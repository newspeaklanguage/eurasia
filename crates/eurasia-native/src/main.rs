// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! `eurasia`: the command-line Eurasia VM.
//!
//! Usage:
//! - `eurasia SNAPSHOT [ARGS...]` runs a snapshot. A checkpoint (M7) is
//!   restored into this new host instead: its Aliens are dead, and it
//!   continues where it was taken.
//! - `eurasia --record LOG [--checkpoint-every N] [--keep-checkpoints K]
//!   SNAPSHOT [ARGS...]` runs a snapshot and writes the session's log (M7
//!   design, section 3.1), with a checkpoint after every `N` allocations.
//!   With `K`, the log keeps only the last `K` checkpoints and the events
//!   after the oldest of them, so that it stays bounded.
//! - `eurasia --replay LOG [CHECKPOINT]` replays a log (recorded here or in
//!   a browser) from its first checkpoint (its starting image, unless the
//!   log was trimmed), or from its checkpoint number `CHECKPOINT`, and
//!   reports the first divergence. Checkpoints are numbered in the order
//!   they were taken, the dropped ones included.
//! - `eurasia --checkpoint LOG CHECKPOINT FILE` writes a log's checkpoint to
//!   a file, to be run (restored) on its own.
//! - `eurasia --methods SNAPSHOT` lists the image's methods, one a line,
//!   sorted: where each is (its mixin, qualified by the mixins enclosing it,
//!   and its side), its selector, header, literals and bytecode. Two images
//!   of the same program list the same lines, whatever their sequence
//!   numbers; the build and deploy design (section 4.4) compares an image
//!   deployed from a running program with one built from source this way.
//! - `eurasia --census SNAPSHOT` counts the image's objects by class (and
//!   their bytes, for byte objects), one class a line, sorted by name: what
//!   an image holds beyond its methods, to see what a deployment took along.
//! - `eurasia --version`.
//!
//! With `EURASIA_GC_STRESS=n` in the environment the VM collects at the first
//! safepoint after every `n` allocations (a test mode; `n` = 1 is after every
//! allocation, and slow).
//!
//! Exit status: the code given to `vm exit:`; 0 when the entry send returns,
//! or a replay matched its log; 1 when a replay diverged; 65 when the
//! snapshot or log cannot be loaded; 70 when the VM stops with an error; 2
//! for a usage error.

use std::io::Write;
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eurasia_core::host::Host;
use eurasia_core::replay::{Log, record_run, replay};
use eurasia_core::{BYTECODE_VERSION, Completion, KERNEL_VERSION, Vm, VmError};
use eurasia_snapshot::{FORMAT_VERSION, read_checkpoint, write_image};

/// The command-line host: stdout, the system clocks, and the OS entropy
/// source.
struct CommandLineHost {
    start: Instant,
}

impl Host for CommandLineHost {
    fn print(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }

    fn monotonic_nanos(&mut self) -> u64 {
        u64::try_from(self.start.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn utc_nanos(&mut self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        // /dev/urandom on macOS and Linux; the VM never runs without entropy.
        let ok = std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, buffer))
            .is_ok();
        assert!(ok, "no entropy source");
    }

    fn wait_until(&mut self, nanos: u64) {
        let now = self.monotonic_nanos();
        if nanos > now {
            std::thread::sleep(std::time::Duration::from_nanos(nanos - now));
        }
    }

    fn description(&self) -> &str {
        std::env::consts::OS
    }

    /// `saveImage:` writes the image to the file `name`, which `eurasia`
    /// can then continue (a checkpoint is restored into a new host).
    fn save_image(&mut self, name: &str, bytes: Vec<u8>) {
        if let Err(e) = std::fs::write(name, bytes) {
            eprintln!("eurasia: cannot save the image as {name}: {e}");
        }
    }

    fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        std::fs::write(path, bytes).map_err(|e| format!("cannot write {path}: {e}"))
    }

    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))
    }
}

fn version() -> String {
    format!(
        "eurasia {} (snapshot format {FORMAT_VERSION}, kernel {KERNEL_VERSION}, bytecode {BYTECODE_VERSION})",
        env!("CARGO_PKG_VERSION")
    )
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: eurasia SNAPSHOT [ARGS...]\n       eurasia --record LOG [--checkpoint-every N] [--keep-checkpoints K] SNAPSHOT [ARGS...]\n       eurasia --replay LOG [CHECKPOINT]\n       eurasia --checkpoint LOG CHECKPOINT FILE\n       eurasia --methods SNAPSHOT\n       eurasia --census SNAPSHOT\n       eurasia --version"
    );
    ExitCode::from(2)
}

fn read(path: &str) -> Result<Vec<u8>, ExitCode> {
    std::fs::read(path).map_err(|e| {
        eprintln!("eurasia: cannot read {path}: {e}");
        ExitCode::from(65)
    })
}

fn read_log(path: &str) -> Result<Log, ExitCode> {
    Log::decode(&read(path)?).map_err(|e| {
        eprintln!("eurasia: {path}: {}", e.0);
        ExitCode::from(65)
    })
}

/// A VM over `bytes`, and whether they were a checkpoint (restored).
fn load(path: &str, bytes: &[u8]) -> Result<(Vm, bool), ExitCode> {
    let (heap, roles, machine) = read_checkpoint(bytes).map_err(|e| {
        eprintln!("eurasia: {path}: {e}");
        ExitCode::from(65)
    })?;
    let restored = machine.is_some();
    let mut vm = match machine {
        Some(state) => Vm::restore(heap, roles, state),
        None => Vm::new(heap, roles),
    };
    vm.set_snapshot_writer(write_image);
    Ok((vm, restored))
}

fn exit_status(outcome: Result<Completion, VmError>) -> ExitCode {
    match outcome {
        Ok(Completion::Returned(_)) => ExitCode::SUCCESS,
        Ok(Completion::Exited(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Ok(Completion::Waiting(_)) => unreachable!("this host blocks"),
        Ok(Completion::Compiled(..)) => unreachable!("this host installs no code"),
        Err(e) => {
            eprintln!("eurasia: {e}");
            ExitCode::from(70)
        }
    }
}

fn gc_stress() -> Option<u64> {
    std::env::var("EURASIA_GC_STRESS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &u64| n > 0)
}

fn new_host() -> CommandLineHost {
    CommandLineHost {
        start: Instant::now(),
    }
}

fn run(args: &[String]) -> Result<ExitCode, ExitCode> {
    let path = &args[0];
    let bytes = read(path)?;
    let (mut vm, restored) = load(path, &bytes)?;
    let stress = gc_stress();
    vm.set_gc_stress(stress);
    let mut host = new_host();
    let outcome = if restored {
        vm.restored_into_new_host();
        vm.continue_run(&mut host)
    } else {
        let program_args: Vec<&str> = args[1..].iter().map(String::as_str).collect();
        vm.run(&mut host, &program_args)
    };
    if stress.is_some() {
        eprintln!("eurasia: {} collections", vm.collections());
    }
    Ok(exit_status(outcome))
}

fn list_methods(args: &[String]) -> Result<ExitCode, ExitCode> {
    let [path] = args else {
        return Err(usage());
    };
    let bytes = read(path)?;
    let (heap, _) = eurasia_snapshot::read_image(&bytes).map_err(|e| {
        eprintln!("eurasia: {path}: {e}");
        ExitCode::from(65)
    })?;
    let mut lines: Vec<String> = heap
        .objects()
        .filter_map(|v| {
            let m = heap.method(v)?;
            let literals: Vec<String> = heap
                .array_elements(m.literals())
                .unwrap_or(&[])
                .iter()
                .map(|&l| literal_text(&heap, l))
                .collect();
            let code: String = m.bytecode().iter().map(|b| format!("{b:02x}")).collect();
            Some(format!(
                "{}>>{} {:?} [{}] {}",
                mixin_path(&heap, m.mixin()),
                text_of(&heap, m.selector()),
                m.header(),
                literals.join(" "),
                code
            ))
        })
        .collect();
    lines.sort();
    let mut out = std::io::stdout().lock();
    for line in lines {
        let _ = writeln!(out, "{line}");
    }
    Ok(ExitCode::SUCCESS)
}

fn census(args: &[String]) -> Result<ExitCode, ExitCode> {
    let [path] = args else {
        return Err(usage());
    };
    let bytes = read(path)?;
    let (heap, _) = eurasia_snapshot::read_image(&bytes).map_err(|e| {
        eprintln!("eurasia: {path}: {e}");
        ExitCode::from(65)
    })?;
    let mut counts: std::collections::BTreeMap<String, (u64, u64)> = Default::default();
    for v in heap.objects() {
        let Some(o) = heap.object(v) else { continue };
        let class = o.header.class;
        let name = match (heap.object(class).map(|c| c.body), heap.behavior(class)) {
            // A metaclass's instance is a class: named by its this-class.
            (Some(eurasia_core::object::Body::Metaclass(_)), Some(b)) => {
                let this = heap.behavior(b.this_class()).map_or(b.name(), |t| t.name());
                format!("{} class", text_of(&heap, this))
            }
            (_, Some(b)) => text_of(&heap, b.name()),
            _ => String::from("?"),
        };
        let size = heap.bytes(v).map_or(0, |b| b.len() as u64);
        let entry = counts.entry(name).or_default();
        entry.0 += 1;
        entry.1 += size;
    }
    let mut out = std::io::stdout().lock();
    for (name, (count, size)) in counts {
        let _ = writeln!(out, "{name}\t{count}\t{size}");
    }
    Ok(ExitCode::SUCCESS)
}

fn text_of(heap: &eurasia_core::object::Heap, v: eurasia_core::Value) -> String {
    heap.string_bytes(v)
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_else(|| String::from("?"))
}

/// A mixin's name, after those of the mixins enclosing it; a class-side
/// mixin (whose library is {its instance mixin}) is its instance mixin's,
/// with ` class`.
fn mixin_path(heap: &eurasia_core::object::Heap, mixin: eurasia_core::Value) -> String {
    let Some(m) = heap.mixin(mixin) else {
        return String::from("?");
    };
    if let Some([instance]) = heap.array_elements(m.library()) {
        return format!("{} class", mixin_path(heap, *instance));
    }
    let name = text_of(heap, m.name());
    match heap.mixin(m.enclosing_mixin()) {
        Some(_) => format!("{}`{name}", mixin_path(heap, m.enclosing_mixin())),
        None => name,
    }
}

/// A literal, as text that does not depend on where objects are.
fn literal_text(heap: &eurasia_core::object::Heap, v: eurasia_core::Value) -> String {
    if let Some(i) = v.as_small_integer() {
        return format!("{i}");
    }
    if let Some(b) = heap.string_bytes(v) {
        return format!("{:?}", String::from_utf8_lossy(b));
    }
    if let Some(elements) = heap.array_elements(v) {
        let inner: Vec<String> = elements.iter().map(|&e| literal_text(heap, e)).collect();
        return format!("{{{}}}", inner.join(" "));
    }
    if let Some(m) = heap.method(v) {
        return format!("<method {}>", text_of(heap, m.selector()));
    }
    match heap.class(v).and_then(|c| heap.behavior(c)) {
        Some(b) => format!("<{}>", text_of(heap, b.name())),
        None => String::from("<?>"),
    }
}

fn record(args: &[String]) -> Result<ExitCode, ExitCode> {
    let (log_path, mut rest) = match args {
        [log, rest @ ..] if !rest.is_empty() => (log, rest),
        _ => return Err(usage()),
    };
    let mut every = 0;
    if rest.first().map(String::as_str) == Some("--checkpoint-every") {
        every = match rest.get(1).and_then(|n| n.parse().ok()) {
            Some(n) if rest.len() > 2 => n,
            _ => return Err(usage()),
        };
        rest = &rest[2..];
    }
    let mut keep = 0;
    if rest.first().map(String::as_str) == Some("--keep-checkpoints") {
        keep = match rest.get(1).and_then(|n| n.parse().ok()) {
            Some(n) if rest.len() > 2 => n,
            _ => return Err(usage()),
        };
        rest = &rest[2..];
    }
    let path = &rest[0];
    let bytes = read(path)?;
    let (mut vm, restored) = load(path, &bytes)?;
    if restored {
        eprintln!("eurasia: {path} is a checkpoint; record from a snapshot");
        return Err(ExitCode::from(2));
    }
    vm.set_checkpoint_policy(every);
    let mut log = Log {
        checkpoint_every: every,
        checkpoints: vec![bytes],
        checkpoint_positions: vec![0],
        keep_checkpoints: keep,
        ..Log::default()
    };
    let program_args: Vec<&str> = rest[1..].iter().map(String::as_str).collect();
    let outcome = record_run(&mut vm, &mut new_host(), &mut log, &program_args);
    if let Err(e) = std::fs::write(log_path, log.encode()) {
        eprintln!("eurasia: cannot write {log_path}: {e}");
        return Err(ExitCode::from(70));
    }
    Ok(exit_status(outcome))
}

/// The checkpoint numbered `arg` (by default the log's first), its bytes
/// and its position.
fn checkpoint_of<'a>(log: &'a Log, arg: Option<&String>) -> Result<(u32, &'a [u8], u64), ExitCode> {
    let k = match arg {
        None => log.first_checkpoint,
        Some(a) => a.parse().map_err(|_| usage())?,
    };
    match log.checkpoint(k) {
        Some((bytes, position)) => Ok((k, bytes, position)),
        None => {
            let numbers = log.checkpoint_numbers();
            if numbers.start == 0 {
                eprintln!(
                    "eurasia: the log has {} checkpoints (0 is its starting image)",
                    numbers.len()
                );
            } else {
                eprintln!(
                    "eurasia: the log has {} checkpoints, {} to {} (it was trimmed)",
                    numbers.len(),
                    numbers.start,
                    numbers.end - 1
                );
            }
            Err(ExitCode::from(2))
        }
    }
}

fn echo(text: &str) {
    println!("{text}");
}

fn replay_log(args: &[String]) -> Result<ExitCode, ExitCode> {
    let Some(log_path) = args.first() else {
        return Err(usage());
    };
    let log = read_log(log_path)?;
    let (k, bytes, position) = checkpoint_of(&log, args.get(1))?;
    let (mut vm, restored) = load(log_path, bytes)?;
    vm.set_gc_stress(gc_stress());
    let report = replay(&mut vm, &log, position, restored, Some(echo));
    eprintln!(
        "eurasia: replayed {} top-level events from checkpoint {k}, to log position {} of {}",
        report.top_level_events,
        report.position,
        log.end()
    );
    match report.divergence {
        None => {
            eprintln!("eurasia: the replay matched the log");
            Ok(ExitCode::SUCCESS)
        }
        Some(d) => {
            eprintln!(
                "eurasia: the replay diverged at log position {}:\n  recorded: {}\n  replayed: {}",
                d.position, d.expected, d.found
            );
            Ok(ExitCode::from(1))
        }
    }
}

fn extract(args: &[String]) -> Result<ExitCode, ExitCode> {
    let [log_path, k, out] = args else {
        return Err(usage());
    };
    let log = read_log(log_path)?;
    let (_, bytes, _) = checkpoint_of(&log, Some(k))?;
    std::fs::write(out, bytes).map_err(|e| {
        eprintln!("eurasia: cannot write {out}: {e}");
        ExitCode::from(70)
    })?;
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        None => Err(usage()),
        Some("--version") => {
            println!("{}", version());
            Ok(ExitCode::SUCCESS)
        }
        Some("--record") => record(&args[2..]),
        Some("--replay") => replay_log(&args[2..]),
        Some("--checkpoint") => extract(&args[2..]),
        Some("--methods") => list_methods(&args[2..]),
        Some("--census") => census(&args[2..]),
        Some(_) => run(&args[1..]),
    };
    result.unwrap_or_else(|code| code)
}
