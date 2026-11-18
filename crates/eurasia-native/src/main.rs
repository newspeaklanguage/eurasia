// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! `eurasia`: the command-line Eurasia VM.
//!
//! Usage: `eurasia SNAPSHOT [ARGS...]`, or `eurasia --version`.
//!
//! Exit status: the code given to `vm exit:`; 0 when the entry send returns;
//! 65 when the snapshot cannot be loaded; 70 when the VM stops with an error;
//! 2 for a usage error.

use std::io::Write;
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use eurasia_core::host::Host;
use eurasia_core::{BYTECODE_VERSION, Completion, KERNEL_VERSION, Vm};
use eurasia_snapshot::{FORMAT_VERSION, read_image, write_image};

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

    fn description(&self) -> &str {
        std::env::consts::OS
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.get(1).map(String::as_str) {
        None => {
            eprintln!("usage: eurasia SNAPSHOT [ARGS...]");
            return ExitCode::from(2);
        }
        Some("--version") => {
            println!("{}", version());
            return ExitCode::SUCCESS;
        }
        Some(path) => path,
    };
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("eurasia: cannot read {path}: {e}");
            return ExitCode::from(65);
        }
    };
    let (heap, roles) = match read_image(&bytes) {
        Ok(image) => image,
        Err(e) => {
            eprintln!("eurasia: {path}: {e}");
            return ExitCode::from(65);
        }
    };
    let program_args: Vec<&str> = args[2..].iter().map(String::as_str).collect();
    let mut vm = Vm::new(heap, roles);
    vm.set_snapshot_writer(write_image);
    let mut host = CommandLineHost {
        start: Instant::now(),
    };
    match vm.run(&mut host, &program_args) {
        Ok(Completion::Returned(_)) => ExitCode::SUCCESS,
        Ok(Completion::Exited(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("eurasia: {e}");
            ExitCode::from(70)
        }
    }
}
