// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Recording and replay (M7; `EURASIA_M7_SNAPSHOTS_REPLAY_DESIGN_2026-11-18.md`
//! section 3.1).
//!
//! The log sits at the [`Host`] boundary. A [`Recorder`] wraps a real host
//! and appends, in the order the VM meets them, every input (clock readings,
//! entropy, host-operation results, callbacks into the VM, file contents)
//! and every output (printed lines, host-operation requests, callback
//! results, checkpoints, files written). The embedder appends the top-level
//! events (a start, a resume, an idle callback) and how each ended, through
//! [`record_run`], [`record_resume`] and [`record_callback`].
//!
//! A [`Replayer`] answers the inputs from the log and never calls a host; it
//! compares each output with the recorded one, and the first mismatch is a
//! [`Divergence`]. [`replay`] drives a VM through a log's top-level events,
//! so that a session recorded in a browser replays on the command line.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::host::{Host, HostCallbacks, HostError, HostOp, HostValue};
use crate::interpreter::{Completion, Vm, VmError};

/// How a top-level event ended (an output).
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    /// The run returned.
    Returned,
    /// The program exited with this code.
    Exited(i32),
    /// The VM waits for this monotonic time.
    Waiting(u64),
    /// The VM failed; its description.
    Failed(String),
}

impl Ending {
    /// How `outcome` ended.
    pub fn of(outcome: &Result<Completion, VmError>) -> Ending {
        match outcome {
            Ok(Completion::Returned(_)) => Ending::Returned,
            Ok(Completion::Exited(c)) => Ending::Exited(*c),
            Ok(Completion::Waiting(t)) => Ending::Waiting(*t),
            // Recording hosts install no code, so never see this.
            Ok(Completion::Compiled(..)) => {
                Ending::Failed(String::from("compiled code while recording"))
            }
            Err(e) => Ending::Failed(format!("{e}")),
        }
    }
}

/// One entry of a log.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Top level: the run started with these arguments.
    Start(Vec<String>),
    /// Top level: the host resumed the VM.
    Resume,
    /// Top level: the host called an expat while the VM was idle.
    Idle {
        /// The expat.
        expat: u32,
        /// Its arguments.
        args: Vec<HostValue>,
    },
    /// Output: how the last top-level event ended.
    Ended(Ending),
    /// Output: an idle callback's result.
    IdleResult(Result<HostValue, HostError>),
    /// Input: a monotonic clock reading.
    Monotonic(u64),
    /// Input: a wall-clock reading.
    Utc(i64),
    /// Input: entropy.
    Random(Vec<u8>),
    /// Output: a host-operation request.
    HostOp {
        /// The operation.
        op: HostOp,
        /// Its receiver.
        receiver: u32,
        /// Its arguments.
        args: Vec<HostValue>,
    },
    /// Input: a host operation's result.
    HostOpResult(Result<HostValue, HostError>),
    /// Input: the host called an expat during a host operation.
    Callback {
        /// The expat.
        expat: u32,
        /// Its arguments.
        args: Vec<HostValue>,
    },
    /// Output: that callback's result.
    CallbackResult(Result<HostValue, HostError>),
    /// Output: a printed line.
    Print(String),
    /// Output: a file written (its contents by digest).
    WriteFile {
        /// The path.
        path: String,
        /// The length of the contents.
        length: u64,
        /// Their digest ([`digest`]).
        digest: u64,
    },
    /// Input: whether writing it worked.
    WriteFileResult(Result<(), String>),
    /// Output: a file read.
    ReadFile(String),
    /// Input: its contents.
    ReadFileResult(Result<Vec<u8>, String>),
    /// Output: a checkpoint, kept in the log as [`Log::checkpoints`]`[index]`.
    Checkpoint {
        /// Its index among the log's checkpoints.
        index: u32,
        /// The digest of its bytes.
        digest: u64,
    },
}

impl Event {
    fn is_top_level(&self) -> bool {
        matches!(self, Event::Start(_) | Event::Resume | Event::Idle { .. })
    }
}

/// A recorded session.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Log {
    /// The recording host's description (`vm hostDescription`).
    pub description: String,
    /// Whether the recording host blocks ([`Host::blocks`]).
    pub blocks: bool,
    /// The checkpoint policy the embedder set at the start (allocations; 0
    /// none); a replay from the start sets the same.
    pub checkpoint_every: u64,
    /// The events, in order.
    pub events: Vec<Event>,
    /// Checkpoints: snapshot bytes. The first is the image the run started
    /// from, when the recording began at a start.
    pub checkpoints: Vec<Vec<u8>>,
    /// The position of each checkpoint: the position just after its
    /// [`Event::Checkpoint`]; 0 for the starting image.
    pub checkpoint_positions: Vec<u64>,
    /// The position of `events[0]`. Positions are absolute: they count every
    /// event recorded, kept or dropped, so a VM's `vm logPosition` and a
    /// checkpoint's position mean the same in a trimmed log.
    pub base: u64,
    /// The number of `checkpoints[0]` among every checkpoint recorded.
    pub first_checkpoint: u32,
    /// How many checkpoints the log keeps (0: all). Recording a checkpoint
    /// beyond them drops the oldest, and the events before the oldest one
    /// kept ([`Log::trim`]), so that a long session's log stays bounded;
    /// it still starts at a checkpoint, from which it replays.
    pub keep_checkpoints: u32,
}

impl Log {
    /// The position after the last event.
    pub fn end(&self) -> u64 {
        self.base + self.events.len() as u64
    }

    /// The event at `position`, if the log has it.
    pub fn event_at(&self, position: u64) -> Option<&Event> {
        let index = position.checked_sub(self.base)?;
        self.events.get(usize::try_from(index).ok()?)
    }

    /// Checkpoint number `k` (counting every checkpoint recorded) and its
    /// position, if the log still has it.
    pub fn checkpoint(&self, k: u32) -> Option<(&[u8], u64)> {
        let index = k.checked_sub(self.first_checkpoint)? as usize;
        Some((
            self.checkpoints.get(index)?.as_slice(),
            self.checkpoint_positions[index],
        ))
    }

    /// The numbers of the checkpoints the log has.
    pub fn checkpoint_numbers(&self) -> core::ops::Range<u32> {
        self.first_checkpoint..self.first_checkpoint + self.checkpoints.len() as u32
    }

    /// Drops the oldest checkpoints beyond [`Log::keep_checkpoints`], and
    /// the events before the oldest checkpoint kept.
    pub fn trim(&mut self) {
        let keep = self.keep_checkpoints as usize;
        if keep == 0 || self.checkpoints.len() <= keep {
            return;
        }
        let dropped = self.checkpoints.len() - keep;
        let new_base = self.checkpoint_positions[dropped];
        self.events.drain(..(new_base - self.base) as usize);
        self.checkpoints.drain(..dropped);
        self.checkpoint_positions.drain(..dropped);
        self.first_checkpoint += dropped as u32;
        self.base = new_base;
    }
}

/// A 64-bit FNV-1a digest, for outputs the log keeps by digest.
pub fn digest(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ----- Recording -----

/// A host that records: it forwards every call to `inner` and appends what
/// crosses to `log`.
pub struct Recorder<'a> {
    /// The real host.
    pub inner: &'a mut dyn Host,
    /// The log.
    pub log: &'a mut Log,
}

/// The VM's callback interface during a recorded host operation: it records
/// each callback and hands the VM a recorder over the host the callback came
/// through.
struct RecordingCallbacks<'a> {
    log: &'a mut Log,
    vm: &'a mut dyn HostCallbacks,
}

impl HostCallbacks for RecordingCallbacks<'_> {
    fn callback(
        &mut self,
        host: &mut dyn Host,
        expat: u32,
        args: &[HostValue],
    ) -> Result<HostValue, HostError> {
        self.log.events.push(Event::Callback {
            expat,
            args: args.to_vec(),
        });
        let mut recorder = Recorder {
            inner: host,
            log: &mut *self.log,
        };
        let result = self.vm.callback(&mut recorder, expat, args);
        self.log.events.push(Event::CallbackResult(result.clone()));
        result
    }
}

impl Host for Recorder<'_> {
    fn print(&mut self, text: &str) {
        self.log.events.push(Event::Print(String::from(text)));
        self.inner.print(text);
    }

    fn monotonic_nanos(&mut self) -> u64 {
        let t = self.inner.monotonic_nanos();
        self.log.events.push(Event::Monotonic(t));
        t
    }

    fn utc_nanos(&mut self) -> i64 {
        let t = self.inner.utc_nanos();
        self.log.events.push(Event::Utc(t));
        t
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        self.inner.random_bytes(buffer);
        self.log.events.push(Event::Random(buffer.to_vec()));
    }

    fn wait_until(&mut self, nanos: u64) {
        self.inner.wait_until(nanos);
    }

    fn blocks(&self) -> bool {
        self.inner.blocks()
    }

    fn host_operation(
        &mut self,
        op: HostOp,
        receiver: u32,
        args: &[HostValue],
        callbacks: &mut dyn HostCallbacks,
    ) -> Result<HostValue, HostError> {
        self.log.events.push(Event::HostOp {
            op,
            receiver,
            args: args.to_vec(),
        });
        let mut recording = RecordingCallbacks {
            log: &mut *self.log,
            vm: callbacks,
        };
        let result = self
            .inner
            .host_operation(op, receiver, args, &mut recording);
        self.log.events.push(Event::HostOpResult(result.clone()));
        result
    }

    fn checkpoint(&mut self, bytes: Vec<u8>) {
        let index = self.log.first_checkpoint + self.log.checkpoints.len() as u32;
        self.log.events.push(Event::Checkpoint {
            index,
            digest: digest(&bytes),
        });
        let position = self.log.end();
        self.log.checkpoint_positions.push(position);
        self.log.checkpoints.push(bytes);
        self.log.trim();
    }

    // Not an event: the program is told only that the image was handed
    // over, and a replay saves nothing (the Replayer keeps the default).
    fn save_image(&mut self, name: &str, bytes: Vec<u8>) {
        self.inner.save_image(name, bytes);
    }

    // Not an event either, for the same reason.
    fn deliver_image(&mut self, name: &str, bytes: &[u8]) {
        self.inner.deliver_image(name, bytes);
    }

    fn wants_checkpoints(&self) -> bool {
        true
    }

    fn log_position(&self) -> Option<u64> {
        Some(self.log.end())
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        self.log.events.push(Event::WriteFile {
            path: String::from(path),
            length: bytes.len() as u64,
            digest: digest(bytes),
        });
        let result = self.inner.write_file(path, bytes);
        self.log.events.push(Event::WriteFileResult(result.clone()));
        result
    }

    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        self.log.events.push(Event::ReadFile(String::from(path)));
        let result = self.inner.read_file(path);
        self.log.events.push(Event::ReadFileResult(result.clone()));
        result
    }
}

/// Runs `vm` from its start as [`Vm::run`] does, recording into `log`.
pub fn record_run(
    vm: &mut Vm,
    host: &mut dyn Host,
    log: &mut Log,
    args: &[&str],
) -> Result<Completion, VmError> {
    log.description = String::from(host.description());
    log.blocks = host.blocks();
    log.events.push(Event::Start(
        args.iter().map(|&a| String::from(a)).collect(),
    ));
    let outcome = vm.run(
        &mut Recorder {
            inner: &mut *host,
            log: &mut *log,
        },
        args,
    );
    log.events.push(Event::Ended(Ending::of(&outcome)));
    outcome
}

/// Resumes `vm` ([`Vm::resume`]), recording into `log`.
pub fn record_resume(
    vm: &mut Vm,
    host: &mut dyn Host,
    log: &mut Log,
) -> Result<Completion, VmError> {
    log.events.push(Event::Resume);
    let outcome = vm.resume(&mut Recorder {
        inner: &mut *host,
        log: &mut *log,
    });
    log.events.push(Event::Ended(Ending::of(&outcome)));
    outcome
}

/// An idle callback of expat `expat` ([`HostCallbacks::callback`] with no
/// turn under way), recording into `log`.
pub fn record_callback(
    vm: &mut Vm,
    host: &mut dyn Host,
    log: &mut Log,
    expat: u32,
    args: &[HostValue],
) -> Result<HostValue, HostError> {
    log.events.push(Event::Idle {
        expat,
        args: args.to_vec(),
    });
    let result = vm.callback(
        &mut Recorder {
            inner: &mut *host,
            log: &mut *log,
        },
        expat,
        args,
    );
    log.events.push(Event::IdleResult(result.clone()));
    result
}

// ----- Replaying -----

/// Where a replay first differed from its log.
#[derive(Clone, Debug, PartialEq)]
pub struct Divergence {
    /// The position in the log's events.
    pub position: u64,
    /// The recorded event there, or "the end of the log".
    pub expected: String,
    /// What the replay did instead.
    pub found: String,
}

/// A host that replays a log: inputs come from it, outputs are compared
/// with it. After the first divergence it answers defaults and asks the VM
/// to stop ([`Host::interrupted`]).
pub struct Replayer<'a> {
    log: &'a Log,
    /// The position of the next event.
    pub position: u64,
    /// The first divergence, if any.
    pub divergence: Option<Divergence>,
    /// Printed lines go here too, if set (to show the replayed output).
    pub echo: Option<fn(&str)>,
}

/// Whether two host values are the same (numbers by their bits).
fn same(a: &HostValue, b: &HostValue) -> bool {
    match (a, b) {
        (HostValue::Number(x), HostValue::Number(y)) => x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn same_all(a: &[HostValue], b: &[HostValue]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
}

fn same_result(a: &Result<HostValue, HostError>, b: &Result<HostValue, HostError>) -> bool {
    match (a, b) {
        (Ok(x), Ok(y)) => same(x, y),
        _ => a == b,
    }
}

impl<'a> Replayer<'a> {
    /// A replayer of `log` from event `position`.
    pub fn new(log: &'a Log, position: u64) -> Replayer<'a> {
        Replayer {
            log,
            position,
            divergence: None,
            echo: None,
        }
    }

    fn peek(&self) -> Option<&'a Event> {
        if self.divergence.is_some() {
            return None;
        }
        self.log.event_at(self.position)
    }

    fn diverge(&mut self, found: String) {
        if self.divergence.is_none() {
            let expected = match self.log.event_at(self.position) {
                Some(e) => format!("{e:?}"),
                None => String::from("the end of the log"),
            };
            self.divergence = Some(Divergence {
                position: self.position,
                expected,
                found,
            });
        }
    }

    /// Takes the next event if `accept` accepts it; otherwise records a
    /// divergence, `found` describing what the replay did.
    fn take<T>(
        &mut self,
        found: impl FnOnce() -> String,
        accept: impl FnOnce(&'a Event) -> Option<T>,
    ) -> Option<T> {
        let event = self.peek()?;
        match accept(event) {
            Some(t) => {
                self.position += 1;
                Some(t)
            }
            None => {
                self.diverge(found());
                None
            }
        }
    }

    /// The next top-level event, or `None` at the end of the log (or after
    /// a divergence).
    pub fn next_top_level(&mut self) -> Option<&'a Event> {
        let event = self.peek()?;
        if event.is_top_level() {
            self.position += 1;
            Some(event)
        } else {
            self.diverge(String::from("the end of a top-level event"));
            None
        }
    }

    /// Compares how a top-level event ended.
    pub fn ended(&mut self, outcome: &Result<Completion, VmError>) {
        let ending = Ending::of(outcome);
        self.take(
            || format!("{:?}", Event::Ended(ending.clone())),
            |e| (*e == Event::Ended(ending.clone())).then_some(()),
        );
    }

    /// Compares an idle callback's result.
    pub fn idle_result(&mut self, result: &Result<HostValue, HostError>) {
        self.take(
            || format!("{:?}", Event::IdleResult(result.clone())),
            |e| match e {
                Event::IdleResult(r) if same_result(r, result) => Some(()),
                _ => None,
            },
        );
    }
}

impl Host for Replayer<'_> {
    fn print(&mut self, text: &str) {
        if let Some(echo) = self.echo {
            echo(text);
        }
        self.take(
            || format!("{:?}", Event::Print(String::from(text))),
            |e| match e {
                Event::Print(s) if s == text => Some(()),
                _ => None,
            },
        );
    }

    fn monotonic_nanos(&mut self) -> u64 {
        self.take(
            || String::from("a monotonic clock reading"),
            |e| match e {
                Event::Monotonic(t) => Some(*t),
                _ => None,
            },
        )
        .unwrap_or(u64::MAX)
    }

    fn utc_nanos(&mut self) -> i64 {
        self.take(
            || String::from("a wall-clock reading"),
            |e| match e {
                Event::Utc(t) => Some(*t),
                _ => None,
            },
        )
        .unwrap_or(0)
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        let n = buffer.len();
        if let Some(bytes) = self.take(
            || format!("{n} random bytes"),
            |e| match e {
                Event::Random(b) if b.len() == n => Some(b),
                _ => None,
            },
        ) {
            buffer.copy_from_slice(bytes);
        }
    }

    fn blocks(&self) -> bool {
        self.log.blocks
    }

    fn host_operation(
        &mut self,
        op: HostOp,
        receiver: u32,
        args: &[HostValue],
        callbacks: &mut dyn HostCallbacks,
    ) -> Result<HostValue, HostError> {
        let request = || {
            format!(
                "{:?}",
                Event::HostOp {
                    op,
                    receiver,
                    args: args.to_vec(),
                }
            )
        };
        let diverged = || Err(HostError::Raised(String::from("the replay diverged")));
        if self
            .take(request, |e| match e {
                Event::HostOp {
                    op: o,
                    receiver: r,
                    args: a,
                } if *o == op && *r == receiver && same_all(a, args) => Some(()),
                _ => None,
            })
            .is_none()
        {
            return diverged();
        }
        loop {
            match self.peek() {
                Some(Event::Callback { expat, args }) => {
                    self.position += 1;
                    let result = callbacks.callback(self, *expat, args);
                    self.take(
                        || format!("{:?}", Event::CallbackResult(result.clone())),
                        |e| match e {
                            Event::CallbackResult(r) if same_result(r, &result) => Some(()),
                            _ => None,
                        },
                    );
                }
                Some(Event::HostOpResult(r)) => {
                    self.position += 1;
                    return r.clone();
                }
                _ => {
                    self.diverge(String::from("the end of a host operation"));
                    return diverged();
                }
            }
        }
    }

    fn checkpoint(&mut self, bytes: Vec<u8>) {
        let d = digest(&bytes);
        self.take(
            || format!("a checkpoint with digest {d:#x}"),
            |e| match e {
                Event::Checkpoint { digest, .. } if *digest == d => Some(()),
                _ => None,
            },
        );
    }

    fn wants_checkpoints(&self) -> bool {
        true
    }

    fn log_position(&self) -> Option<u64> {
        Some(self.position)
    }

    fn interrupted(&self) -> bool {
        self.divergence.is_some()
    }

    fn description(&self) -> &str {
        &self.log.description
    }

    fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let (length, d) = (bytes.len() as u64, digest(bytes));
        let wrote = self.take(
            || {
                format!(
                    "{:?}",
                    Event::WriteFile {
                        path: String::from(path),
                        length,
                        digest: d
                    }
                )
            },
            |e| match e {
                Event::WriteFile {
                    path: p,
                    length: l,
                    digest: g,
                } if p == path && *l == length && *g == d => Some(()),
                _ => None,
            },
        );
        if wrote.is_none() {
            return Err(String::from("the replay diverged"));
        }
        self.take(
            || String::from("the result of writing a file"),
            |e| match e {
                Event::WriteFileResult(r) => Some(r.clone()),
                _ => None,
            },
        )
        .unwrap_or_else(|| Err(String::from("the replay diverged")))
    }

    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let read = self.take(
            || format!("{:?}", Event::ReadFile(String::from(path))),
            |e| match e {
                Event::ReadFile(p) if p == path => Some(()),
                _ => None,
            },
        );
        if read.is_none() {
            return Err(String::from("the replay diverged"));
        }
        self.take(
            || String::from("a file's contents"),
            |e| match e {
                Event::ReadFileResult(r) => Some(r.clone()),
                _ => None,
            },
        )
        .unwrap_or_else(|| Err(String::from("the replay diverged")))
    }
}

/// What a replay did.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayReport {
    /// The top-level events replayed (counting a continued one).
    pub top_level_events: u64,
    /// The log position reached.
    pub position: u64,
    /// The first divergence, if any; `None` means the replay produced every
    /// recorded output and consumed the whole log.
    pub divergence: Option<Divergence>,
}

/// Replays `log` on `vm` from event `position`. With `continuing`, `vm` was
/// restored from the checkpoint at that position, in the middle of a
/// top-level event, which it first finishes ([`Vm::continue_run`]);
/// otherwise `vm` is fresh and the log's events start at `position`.
pub fn replay(
    vm: &mut Vm,
    log: &Log,
    position: u64,
    continuing: bool,
    echo: Option<fn(&str)>,
) -> ReplayReport {
    let mut r = Replayer::new(log, position);
    r.echo = echo;
    let mut top_level_events = 0;
    if !continuing && position == 0 {
        vm.set_checkpoint_policy(log.checkpoint_every);
    }
    if continuing {
        top_level_events += 1;
        let outcome = vm.continue_run(&mut r);
        r.ended(&outcome);
    }
    while r.divergence.is_none() {
        let Some(event) = r.next_top_level() else {
            break;
        };
        top_level_events += 1;
        match event {
            Event::Start(args) => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                let outcome = vm.run(&mut r, &args);
                r.ended(&outcome);
            }
            Event::Resume => {
                let outcome = vm.resume(&mut r);
                r.ended(&outcome);
            }
            Event::Idle { expat, args } => {
                let result = vm.callback(&mut r, *expat, args);
                r.idle_result(&result);
            }
            _ => unreachable!("top-level events only"),
        }
    }
    if r.divergence.is_none() && r.position < log.end() {
        r.diverge(String::from("nothing more"));
    }
    ReplayReport {
        top_level_events,
        position: r.position,
        divergence: r.divergence,
    }
}

// ----- Encoding -----

/// The log file's magic number.
pub const LOG_MAGIC: [u8; 4] = *b"EURL";
/// The log format's version.
pub const LOG_VERSION: u16 = 2;

/// Why a log could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogError(pub String);

struct Out(Vec<u8>);

impl Out {
    fn u8(&mut self, b: u8) {
        self.0.push(b);
    }
    fn uv(&mut self, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                self.0.push(b);
                return;
            }
            self.0.push(b | 0x80);
        }
    }
    fn sv(&mut self, n: i64) {
        self.uv(((n << 1) ^ (n >> 63)) as u64);
    }
    fn bytes(&mut self, b: &[u8]) {
        self.uv(b.len() as u64);
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn value(&mut self, v: &HostValue) {
        match v {
            HostValue::Undefined => self.u8(0),
            HostValue::Null => self.u8(1),
            HostValue::Bool(b) => {
                self.u8(2);
                self.u8(u8::from(*b));
            }
            HostValue::Number(x) => {
                self.u8(3);
                self.0.extend_from_slice(&x.to_bits().to_le_bytes());
            }
            HostValue::String(s) => {
                self.u8(4);
                self.str(s);
            }
            HostValue::Ref(r) => {
                self.u8(5);
                self.uv((*r).into());
            }
            HostValue::Expat(e) => {
                self.u8(6);
                self.uv((*e).into());
            }
        }
    }
    fn values(&mut self, vs: &[HostValue]) {
        self.uv(vs.len() as u64);
        vs.iter().for_each(|v| self.value(v));
    }
    fn result(&mut self, r: &Result<HostValue, HostError>) {
        match r {
            Ok(v) => {
                self.u8(0);
                self.value(v);
            }
            Err(HostError::Unsupported) => self.u8(1),
            Err(HostError::Raised(s)) => {
                self.u8(2);
                self.str(s);
            }
            Err(HostError::Dead) => self.u8(3),
        }
    }
    fn string_result(&mut self, r: Result<&[u8], &String>) {
        match r {
            Ok(b) => {
                self.u8(0);
                self.bytes(b);
            }
            Err(s) => {
                self.u8(1);
                self.str(s);
            }
        }
    }
}

fn op_code(op: HostOp) -> u8 {
    match op {
        HostOp::Get => 0,
        HostOp::Set => 1,
        HostOp::Delete => 2,
        HostOp::Has => 3,
        HostOp::Call => 4,
        HostOp::Apply => 5,
        HostOp::New => 6,
        HostOp::InstanceOf => 7,
    }
}

impl Log {
    /// The log as bytes: magic, version, the header fields, the events,
    /// then the checkpoints.
    pub fn encode(&self) -> Vec<u8> {
        let mut o = Out(Vec::new());
        o.0.extend_from_slice(&LOG_MAGIC);
        o.0.extend_from_slice(&LOG_VERSION.to_le_bytes());
        o.str(&self.description);
        o.u8(u8::from(self.blocks));
        o.uv(self.checkpoint_every);
        o.uv(self.base);
        o.uv(self.first_checkpoint.into());
        o.uv(self.keep_checkpoints.into());
        o.uv(self.events.len() as u64);
        for e in &self.events {
            match e {
                Event::Start(args) => {
                    o.u8(1);
                    o.uv(args.len() as u64);
                    args.iter().for_each(|a| o.str(a));
                }
                Event::Resume => o.u8(2),
                Event::Idle { expat, args } => {
                    o.u8(3);
                    o.uv((*expat).into());
                    o.values(args);
                }
                Event::Ended(ending) => {
                    o.u8(4);
                    match ending {
                        Ending::Returned => o.u8(0),
                        Ending::Exited(c) => {
                            o.u8(1);
                            o.sv((*c).into());
                        }
                        Ending::Waiting(t) => {
                            o.u8(2);
                            o.uv(*t);
                        }
                        Ending::Failed(s) => {
                            o.u8(3);
                            o.str(s);
                        }
                    }
                }
                Event::IdleResult(r) => {
                    o.u8(5);
                    o.result(r);
                }
                Event::Monotonic(t) => {
                    o.u8(6);
                    o.uv(*t);
                }
                Event::Utc(t) => {
                    o.u8(7);
                    o.sv(*t);
                }
                Event::Random(b) => {
                    o.u8(8);
                    o.bytes(b);
                }
                Event::HostOp { op, receiver, args } => {
                    o.u8(9);
                    o.u8(op_code(*op));
                    o.uv((*receiver).into());
                    o.values(args);
                }
                Event::HostOpResult(r) => {
                    o.u8(10);
                    o.result(r);
                }
                Event::Callback { expat, args } => {
                    o.u8(11);
                    o.uv((*expat).into());
                    o.values(args);
                }
                Event::CallbackResult(r) => {
                    o.u8(12);
                    o.result(r);
                }
                Event::Print(s) => {
                    o.u8(13);
                    o.str(s);
                }
                Event::WriteFile {
                    path,
                    length,
                    digest,
                } => {
                    o.u8(14);
                    o.str(path);
                    o.uv(*length);
                    o.uv(*digest);
                }
                Event::WriteFileResult(r) => {
                    o.u8(15);
                    o.string_result(r.as_ref().map(|()| &[][..]));
                }
                Event::ReadFile(p) => {
                    o.u8(16);
                    o.str(p);
                }
                Event::ReadFileResult(r) => {
                    o.u8(17);
                    o.string_result(r.as_ref().map(Vec::as_slice));
                }
                Event::Checkpoint { index, digest } => {
                    o.u8(18);
                    o.uv((*index).into());
                    o.uv(*digest);
                }
            }
        }
        o.uv(self.checkpoints.len() as u64);
        for (c, &p) in self.checkpoints.iter().zip(&self.checkpoint_positions) {
            o.uv(p);
            o.bytes(c);
        }
        o.0
    }

    /// Reads a log written by [`Log::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Log, LogError> {
        let mut i = In { bytes, at: 0 };
        if i.take(4)? != LOG_MAGIC {
            return Err(LogError(String::from("not a Eurasia log")));
        }
        let version = u16::from_le_bytes(i.take(2)?.try_into().expect("2 bytes"));
        // Version 1 had no base, first checkpoint or retention: it kept
        // every event and checkpoint.
        if version != 1 && version != LOG_VERSION {
            return Err(LogError(format!(
                "log format {version}; this VM reads 1 and {LOG_VERSION}"
            )));
        }
        let mut log = Log {
            description: i.string()?,
            blocks: i.u8()? != 0,
            checkpoint_every: i.uv()?,
            ..Log::default()
        };
        if version >= 2 {
            log.base = i.uv()?;
            log.first_checkpoint = u32::try_from(i.uv()?)
                .map_err(|_| LogError(String::from("a first checkpoint out of range")))?;
            log.keep_checkpoints = u32::try_from(i.uv()?)
                .map_err(|_| LogError(String::from("a retention out of range")))?;
        }
        let n = i.uv()?;
        for _ in 0..n {
            let event = match i.u8()? {
                1 => {
                    let k = i.uv()?;
                    let mut args = Vec::new();
                    for _ in 0..k {
                        args.push(i.string()?);
                    }
                    Event::Start(args)
                }
                2 => Event::Resume,
                3 => Event::Idle {
                    expat: i.u32()?,
                    args: i.values()?,
                },
                4 => Event::Ended(match i.u8()? {
                    0 => Ending::Returned,
                    1 => Ending::Exited(i.sv()? as i32),
                    2 => Ending::Waiting(i.uv()?),
                    3 => Ending::Failed(i.string()?),
                    t => return Err(LogError(format!("unknown ending {t}"))),
                }),
                5 => Event::IdleResult(i.result()?),
                6 => Event::Monotonic(i.uv()?),
                7 => Event::Utc(i.sv()?),
                8 => Event::Random(i.byte_vec()?),
                9 => Event::HostOp {
                    op: match i.u8()? {
                        0 => HostOp::Get,
                        1 => HostOp::Set,
                        2 => HostOp::Delete,
                        3 => HostOp::Has,
                        4 => HostOp::Call,
                        5 => HostOp::Apply,
                        6 => HostOp::New,
                        7 => HostOp::InstanceOf,
                        t => return Err(LogError(format!("unknown operation {t}"))),
                    },
                    receiver: i.u32()?,
                    args: i.values()?,
                },
                10 => Event::HostOpResult(i.result()?),
                11 => Event::Callback {
                    expat: i.u32()?,
                    args: i.values()?,
                },
                12 => Event::CallbackResult(i.result()?),
                13 => Event::Print(i.string()?),
                14 => Event::WriteFile {
                    path: i.string()?,
                    length: i.uv()?,
                    digest: i.uv()?,
                },
                15 => Event::WriteFileResult(match i.u8()? {
                    0 => {
                        i.byte_vec()?;
                        Ok(())
                    }
                    _ => Err(i.string()?),
                }),
                16 => Event::ReadFile(i.string()?),
                17 => Event::ReadFileResult(match i.u8()? {
                    0 => Ok(i.byte_vec()?),
                    _ => Err(i.string()?),
                }),
                18 => Event::Checkpoint {
                    index: i.u32()?,
                    digest: i.uv()?,
                },
                t => return Err(LogError(format!("unknown event {t}"))),
            };
            log.events.push(event);
        }
        let k = i.uv()?;
        for _ in 0..k {
            log.checkpoint_positions.push(i.uv()?);
            log.checkpoints.push(i.byte_vec()?);
        }
        if i.at != bytes.len() {
            return Err(LogError(String::from("bytes after the log")));
        }
        Ok(log)
    }
}

struct In<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], LogError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or_else(|| LogError(String::from("the log ends early")))?;
        let b = &self.bytes[self.at..end];
        self.at = end;
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, LogError> {
        Ok(self.take(1)?[0])
    }
    fn uv(&mut self) -> Result<u64, LogError> {
        let mut n: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            n |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
        }
        Err(LogError(String::from("a number too long")))
    }
    fn u32(&mut self) -> Result<u32, LogError> {
        u32::try_from(self.uv()?).map_err(|_| LogError(String::from("a number out of range")))
    }
    fn sv(&mut self) -> Result<i64, LogError> {
        let z = self.uv()?;
        Ok(((z >> 1) as i64) ^ -((z & 1) as i64))
    }
    fn byte_vec(&mut self) -> Result<Vec<u8>, LogError> {
        let n = usize::try_from(self.uv()?).map_err(|_| LogError(String::from("too long")))?;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, LogError> {
        String::from_utf8(self.byte_vec()?).map_err(|_| LogError(String::from("bad UTF-8")))
    }
    fn value(&mut self) -> Result<HostValue, LogError> {
        Ok(match self.u8()? {
            0 => HostValue::Undefined,
            1 => HostValue::Null,
            2 => HostValue::Bool(self.u8()? != 0),
            3 => HostValue::Number(f64::from_bits(u64::from_le_bytes(
                self.take(8)?.try_into().expect("8 bytes"),
            ))),
            4 => HostValue::String(self.string()?),
            5 => HostValue::Ref(self.u32()?),
            6 => HostValue::Expat(self.u32()?),
            t => return Err(LogError(format!("unknown host value {t}"))),
        })
    }
    fn values(&mut self) -> Result<Vec<HostValue>, LogError> {
        let n = self.uv()?;
        let mut vs = Vec::new();
        for _ in 0..n {
            vs.push(self.value()?);
        }
        Ok(vs)
    }
    fn result(&mut self) -> Result<Result<HostValue, HostError>, LogError> {
        Ok(match self.u8()? {
            0 => Ok(self.value()?),
            1 => Err(HostError::Unsupported),
            2 => Err(HostError::Raised(self.string()?)),
            3 => Err(HostError::Dead),
            t => return Err(LogError(format!("unknown result {t}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_round_trips() {
        let log = Log {
            description: String::from("test"),
            blocks: false,
            checkpoint_every: 7,
            events: alloc::vec![
                Event::Start(alloc::vec![String::from("a")]),
                Event::Monotonic(5),
                Event::Utc(-3),
                Event::Random(alloc::vec![1, 2]),
                Event::HostOp {
                    op: HostOp::Call,
                    receiver: 1,
                    args: alloc::vec![
                        HostValue::String(String::from("x")),
                        HostValue::Number(1.5),
                        HostValue::Expat(2),
                        HostValue::Undefined,
                    ],
                },
                Event::Callback {
                    expat: 2,
                    args: alloc::vec![HostValue::Bool(true)],
                },
                Event::CallbackResult(Err(HostError::Raised(String::from("no")))),
                Event::HostOpResult(Ok(HostValue::Ref(9))),
                Event::Print(String::from("hi")),
                Event::Checkpoint {
                    index: 0,
                    digest: 99,
                },
                Event::Ended(Ending::Waiting(12)),
                Event::Resume,
                Event::Ended(Ending::Exited(-1)),
                Event::Idle {
                    expat: 0,
                    args: Vec::new(),
                },
                Event::IdleResult(Ok(HostValue::Null)),
            ],
            checkpoints: alloc::vec![alloc::vec![9, 9]],
            checkpoint_positions: alloc::vec![10 + 40],
            base: 40,
            first_checkpoint: 3,
            keep_checkpoints: 2,
        };
        assert_eq!(Log::decode(&log.encode()), Ok(log));
    }

    /// A recorder that keeps two checkpoints: the log starts at the older
    /// one kept, positions and checkpoint numbers stay absolute.
    #[test]
    fn a_log_keeps_its_last_checkpoints() {
        let mut log = Log {
            keep_checkpoints: 2,
            checkpoints: alloc::vec![alloc::vec![0]],
            checkpoint_positions: alloc::vec![0],
            ..Log::default()
        };
        struct Quiet;
        impl Host for Quiet {
            fn print(&mut self, _: &str) {}
            fn monotonic_nanos(&mut self) -> u64 {
                0
            }
            fn utc_nanos(&mut self) -> i64 {
                0
            }
            fn random_bytes(&mut self, _: &mut [u8]) {}
            fn description(&self) -> &str {
                "quiet"
            }
        }
        let mut host = Quiet;
        let mut r = Recorder {
            inner: &mut host,
            log: &mut log,
        };
        for k in 1..=3u8 {
            r.print("a");
            r.print("b");
            r.checkpoint(alloc::vec![k]);
        }
        r.print("c");
        assert_eq!(r.log_position(), Some(10));
        assert_eq!(log.checkpoint_numbers(), 2..4);
        assert_eq!(log.base, 6);
        assert_eq!(log.checkpoint(2), Some((&[2u8][..], 6)));
        assert_eq!(log.checkpoint(3), Some((&[3u8][..], 9)));
        assert_eq!(log.checkpoint(1), None);
        assert_eq!(log.event_at(5), None);
        assert_eq!(log.event_at(6), Some(&Event::Print(String::from("a"))));
        assert_eq!(log.event_at(9), Some(&Event::Print(String::from("c"))));
        assert_eq!(log.end(), 10);
        assert_eq!(Log::decode(&log.encode()), Ok(log));
    }
}
