// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Eurasia for the web: the wasm32 build with a JavaScript host.
//!
//! The exports are plain integer functions, needing no generated glue;
//! `web/eurasia.mjs` is the JavaScript side. A run goes:
//!
//! 1. `eurasia_buffer(n)` answers a buffer of `n` bytes in wasm memory, which
//!    the loader fills with a snapshot;
//! 2. `eurasia_start()` loads it and runs until the run is over or nothing
//!    can run before the next timer;
//! 3. while it answers [`WAITING`], the loader waits (without blocking the
//!    browser) until `eurasia_wakeup_nanos()` and calls `eurasia_resume()`.
//!
//! The VM reaches the host through imports from the module `eurasia`:
//! `print(ptr, len)`, `monotonic_nanos()`, `utc_nanos()`,
//! `random_bytes(ptr, len)`, and the host-object operations of the
//! call-based FFI (operation list, section 10): each operation's arguments
//! are staged one by one (`ffi_arg_*`), `ffi_perform(op, receiver)` performs
//! it and answers the result's tag, and `ffi_result_*` read the result.
//! A closure passed to the host arrives as a function (`ffi_arg_expat`);
//! calling it calls the export `eurasia_callback(expat)`, which reads the
//! call's arguments with `cb_arg_count()` and `cb_arg(i)` and stages its
//! result as an argument. A callback during a host operation is a sub-step
//! of the running turn; one while the VM is idle starts a turn, after which
//! the loader resumes the VM.
//!
//! Recording (M7): `eurasia_record(n, k)` before `eurasia_start` records the
//! session (`eurasia_core::replay`), with a checkpoint after every `n`
//! allocations (0: none), keeping the last `k` checkpoints and the events
//! after the oldest of them (0: all), so that a long session's log stays
//! bounded; `eurasia_log()` then encodes the log so far into
//! the buffer, to be saved and replayed on the command line. A checkpoint
//! given to `eurasia_start` is restored into this new host and continues.
//!
//! Saving an image: when the program asks (`saveImage:`), the VM hands the
//! checkpoint to the import `save_image(name_ptr, name_len, ptr, len)`,
//! which copies the bytes out during the call and keeps them as the page
//! sees fit.

// `no_mangle` exports and the host imports are the unsafe constructs here.
#![allow(unsafe_code)]

use std::cell::RefCell;

use eurasia_core::host::{Host, HostCallbacks, HostError, HostOp, HostValue};
use eurasia_core::replay::{Log, record_callback, record_resume, record_run};
use eurasia_core::{BYTECODE_VERSION, Completion, KERNEL_VERSION, Vm, VmError};
use eurasia_snapshot::{FORMAT_VERSION, read_checkpoint, write_image};

/// `eurasia_start` and `eurasia_resume`: the run is over; the entry returned.
pub const RETURNED: i32 = 0;
/// The program sent `vm exit:`; `eurasia_exit_code()` has the code.
pub const EXITED: i32 = 1;
/// Nothing can run before `eurasia_wakeup_nanos()`; then call `eurasia_resume()`.
pub const WAITING: i32 = 2;
/// The snapshot was refused or the VM failed; `eurasia_error()` says why.
pub const FAILED: i32 = -1;

/// The kernel version this VM implements.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_kernel_version() -> u32 {
    KERNEL_VERSION.into()
}

/// The bytecode version this VM implements.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_bytecode_version() -> u32 {
    BYTECODE_VERSION.into()
}

/// The snapshot format version this VM reads.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_snapshot_format_version() -> u32 {
    FORMAT_VERSION.into()
}

#[derive(Default)]
struct State {
    /// The buffer the loader fills (the snapshot), then the error message.
    buffer: Vec<u8>,
    vm: Option<Vm>,
    exit_code: i32,
    wakeup: u64,
    /// The session's log, when recording.
    log: Option<Log>,
    /// The checkpoint policy to record with.
    record_every: Option<u64>,
    /// How many checkpoints the log keeps (0: all).
    record_keep: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// The VM's callback interface for each host operation now running,
    /// innermost last: a callback during one re-enters the VM through it.
    static CALLBACKS: RefCell<Vec<*mut (dyn HostCallbacks + 'static)>> =
        const { RefCell::new(Vec::new()) };
}

/// Calls expat `expat` from the host, with the arguments the host gives
/// (`cb_arg_count`, `cb_arg`). Stages the result (or, on failure, a
/// description) for the host and answers 0, or 1 when the callback failed.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_callback(expat: u32) -> i32 {
    let args = imports::callback_arguments();
    let inner = CALLBACKS.with_borrow(|c| c.last().copied());
    let result = match inner {
        // SAFETY: the pointer is the VM's callback interface for the host
        // operation that is running further up this stack (it was pushed
        // when the operation began and is popped when it ends); the VM uses
        // it only through this call while the operation is suspended.
        Some(callbacks) => unsafe { (*callbacks).callback(&mut WebHost, expat, &args) },
        None => STATE.with_borrow_mut(|s| {
            let State { vm, log, .. } = s;
            match (vm.as_mut(), log.as_mut()) {
                (Some(vm), Some(log)) => record_callback(vm, &mut WebHost, log, expat, &args),
                (Some(vm), None) => vm.callback(&mut WebHost, expat, &args),
                (None, _) => Err(HostError::Dead),
            }
        }),
    };
    match result {
        Ok(v) => {
            imports::stage(&v);
            0
        }
        Err(e) => {
            let description = match e {
                HostError::Raised(m) => m,
                HostError::Dead => String::from("the expat is dead"),
                HostError::Unsupported => String::from("unsupported"),
            };
            imports::stage(&HostValue::String(description));
            1
        }
    }
}

/// A buffer of `length` bytes in wasm memory, for the loader to fill with a
/// snapshot before `eurasia_start`. It stays valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_buffer(length: usize) -> *mut u8 {
    STATE.with_borrow_mut(|s| {
        s.buffer = vec![0; length];
        s.buffer.as_mut_ptr()
    })
}

/// The length of the buffer (after a failure, of the error message in it).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_buffer_length() -> usize {
    STATE.with_borrow(|s| s.buffer.len())
}

/// The buffer's address (after a failure, the error message, UTF-8).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_error() -> *const u8 {
    STATE.with_borrow(|s| s.buffer.as_ptr())
}

/// The code the program exited with, after [`EXITED`].
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_exit_code() -> i32 {
    STATE.with_borrow(|s| s.exit_code)
}

/// Heap statistics, for measuring (not for programs): `which` 0 is the
/// number of objects, 1 the arena's words, 2 the collections so far; -1
/// when there is no VM or it is running.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_heap_stat(which: u32) -> f64 {
    STATE
        .try_with(|s| {
            let Ok(s) = s.try_borrow() else {
                return -1.0;
            };
            match (&s.vm, which) {
                (Some(vm), 0) => vm.heap.len() as f64,
                (Some(vm), 1) => vm.heap.words() as f64,
                (Some(vm), 2) => vm.collections() as f64,
                _ => -1.0,
            }
        })
        .unwrap_or(-1.0)
}

/// The monotonic time to resume at, after [`WAITING`], in nanoseconds (an
/// f64, exact below 2^53).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_wakeup_nanos() -> f64 {
    STATE.with_borrow(|s| s.wakeup as f64)
}

/// Records the session that `eurasia_start` begins, with a checkpoint
/// after every `every` allocations (0: none), keeping the last `keep`
/// checkpoints (0: all).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_record(every: f64, keep: f64) {
    STATE.with_borrow_mut(|s| {
        s.record_every = Some(every as u64);
        s.record_keep = keep as u32;
    });
}

/// Encodes the log recorded so far into the buffer and answers its length
/// (0 when not recording); `eurasia_buffer_address()` is where it is.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_log() -> usize {
    STATE.with_borrow_mut(|s| {
        s.buffer = s.log.as_ref().map(Log::encode).unwrap_or_default();
        s.buffer.len()
    })
}

/// The buffer's address.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_buffer_address() -> *const u8 {
    STATE.with_borrow(|s| s.buffer.as_ptr())
}

/// Loads the snapshot in the buffer and runs it, with no arguments, as
/// psoup's web host does. A checkpoint is restored into this new host and
/// continues instead (M7 design, section 3.4).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_start() -> i32 {
    STATE.with_borrow_mut(|s| {
        let (heap, roles, machine) = match read_checkpoint(&s.buffer) {
            Ok(image) => image,
            Err(e) => return fail(s, &format!("the snapshot was refused: {e}")),
        };
        let bytes = core::mem::take(&mut s.buffer);
        let outcome = match machine {
            Some(state) => {
                // A restored session is not recorded: its replay would need
                // the host it was taken in.
                let mut vm = Vm::restore(heap, roles, state);
                vm.set_snapshot_writer(write_image);
                vm.restored_into_new_host();
                let outcome = vm.continue_run(&mut WebHost);
                s.vm = Some(vm);
                outcome
            }
            None => {
                let mut vm = Vm::new(heap, roles);
                vm.set_snapshot_writer(write_image);
                let outcome = match s.record_every {
                    Some(every) => {
                        vm.set_checkpoint_policy(every);
                        let mut log = Log {
                            checkpoint_every: every,
                            checkpoints: vec![bytes],
                            checkpoint_positions: vec![0],
                            keep_checkpoints: s.record_keep,
                            ..Log::default()
                        };
                        let outcome = record_run(&mut vm, &mut WebHost, &mut log, &[]);
                        s.log = Some(log);
                        outcome
                    }
                    None => vm.run(&mut WebHost, &[]),
                };
                s.vm = Some(vm);
                outcome
            }
        };
        report(s, outcome)
    })
}

/// Continues a run that answered [`WAITING`].
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_resume() -> i32 {
    STATE.with_borrow_mut(|s| {
        let State { vm, log, .. } = s;
        let Some(vm) = vm.as_mut() else {
            return fail(s, "nothing to resume");
        };
        let outcome = match log.as_mut() {
            Some(log) => record_resume(vm, &mut WebHost, log),
            None => vm.resume(&mut WebHost),
        };
        report(s, outcome)
    })
}

fn report(s: &mut State, outcome: Result<Completion, VmError>) -> i32 {
    match outcome {
        Ok(Completion::Returned(_)) => RETURNED,
        Ok(Completion::Exited(code)) => {
            s.exit_code = code;
            EXITED
        }
        Ok(Completion::Waiting(nanos)) => {
            s.wakeup = nanos;
            WAITING
        }
        Err(e) => fail(s, &format!("{e}")),
    }
}

fn fail(s: &mut State, message: &str) -> i32 {
    s.buffer = message.as_bytes().to_vec();
    FAILED
}

/// The JavaScript host. It does not block: waiting for a timer is the
/// loader's, through [`WAITING`].
struct WebHost;

impl Host for WebHost {
    fn print(&mut self, text: &str) {
        imports::print(text.as_bytes());
    }

    fn monotonic_nanos(&mut self) -> u64 {
        imports::monotonic_nanos() as u64
    }

    fn utc_nanos(&mut self) -> i64 {
        imports::utc_nanos() as i64
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        imports::random_bytes(buffer);
    }

    fn blocks(&self) -> bool {
        false
    }

    fn host_operation(
        &mut self,
        op: HostOp,
        receiver: u32,
        args: &[HostValue],
        callbacks: &mut dyn HostCallbacks,
    ) -> Result<HostValue, HostError> {
        let op = match op {
            HostOp::Get => 0,
            HostOp::Set => 1,
            HostOp::Delete => 2,
            HostOp::Has => 3,
            HostOp::Call => 4,
            HostOp::Apply => 5,
            HostOp::New => 6,
            HostOp::InstanceOf => 7,
        };
        for a in args {
            imports::stage(a);
        }
        let callbacks: *mut (dyn HostCallbacks + '_) = callbacks;
        // SAFETY: only the lifetime is erased. The pointer is popped right
        // after the operation, before `callbacks` goes out of scope, so it
        // is never used after.
        let callbacks: *mut (dyn HostCallbacks + 'static) =
            unsafe { core::mem::transmute(callbacks) };
        CALLBACKS.with_borrow_mut(|c| c.push(callbacks));
        let tag = imports::perform(op, receiver);
        CALLBACKS.with_borrow_mut(|c| c.pop());
        match tag {
            0 => Ok(HostValue::Undefined),
            1 => Ok(HostValue::Null),
            2 => Ok(HostValue::Bool(imports::result_number() != 0.0)),
            3 => Ok(HostValue::Number(imports::result_number())),
            4 => Ok(HostValue::String(imports::result_string())),
            5 => Ok(HostValue::Ref(imports::result_number() as u32)),
            6 => Err(HostError::Raised(imports::result_string())),
            7 => Err(HostError::Dead),
            _ => Err(HostError::Raised(String::from(
                "the host answered an unknown tag",
            ))),
        }
    }

    fn description(&self) -> &str {
        "web"
    }

    fn save_image(&mut self, name: &str, bytes: Vec<u8>) {
        imports::save_image(name.as_bytes(), &bytes);
    }
}

/// The host imports, on wasm32; elsewhere (the crate's own tests) inert
/// stand-ins.
#[cfg(target_arch = "wasm32")]
mod imports {
    #[link(wasm_import_module = "eurasia")]
    unsafe extern "C" {
        #[link_name = "print"]
        fn host_print(ptr: *const u8, len: usize);
        #[link_name = "monotonic_nanos"]
        fn host_monotonic_nanos() -> f64;
        #[link_name = "utc_nanos"]
        fn host_utc_nanos() -> f64;
        #[link_name = "random_bytes"]
        fn host_random_bytes(ptr: *mut u8, len: usize);
        #[link_name = "save_image"]
        fn host_save_image(name: *const u8, name_len: usize, ptr: *const u8, len: usize);
        fn ffi_arg_undefined();
        fn ffi_arg_null();
        fn ffi_arg_bool(b: i32);
        fn ffi_arg_number(x: f64);
        fn ffi_arg_string(ptr: *const u8, len: usize);
        fn ffi_arg_ref(r: u32);
        fn ffi_arg_expat(e: u32);
        fn cb_arg_count() -> u32;
        fn cb_arg(i: u32) -> i32;
        fn ffi_perform(op: i32, receiver: u32) -> i32;
        fn ffi_result_number() -> f64;
        fn ffi_result_string_length() -> usize;
        fn ffi_result_string_copy(ptr: *mut u8);
    }

    use eurasia_core::host::HostValue;

    /// Stages one argument of the next operation.
    pub fn stage(v: &HostValue) {
        // SAFETY: plain values; for a string the host reads `len` bytes at
        // `ptr` during the call.
        unsafe {
            match v {
                HostValue::Undefined => ffi_arg_undefined(),
                HostValue::Null => ffi_arg_null(),
                HostValue::Bool(b) => ffi_arg_bool(i32::from(*b)),
                HostValue::Number(x) => ffi_arg_number(*x),
                HostValue::String(s) => ffi_arg_string(s.as_ptr(), s.len()),
                HostValue::Ref(r) => ffi_arg_ref(*r),
                HostValue::Expat(e) => ffi_arg_expat(*e),
            }
        }
    }

    /// The arguments of the host's call of an expat.
    pub fn callback_arguments() -> Vec<HostValue> {
        // SAFETY: plain values; each cb_arg makes that argument the result.
        unsafe {
            (0..cb_arg_count())
                .map(|i| match cb_arg(i) {
                    1 => HostValue::Null,
                    2 => HostValue::Bool(ffi_result_number() != 0.0),
                    3 => HostValue::Number(ffi_result_number()),
                    4 => HostValue::String(result_string()),
                    5 => HostValue::Ref(ffi_result_number() as u32),
                    _ => HostValue::Undefined,
                })
                .collect()
        }
    }

    /// Performs operation `op` on the object `receiver` refers to, with the
    /// staged arguments; answers the result's tag.
    pub fn perform(op: i32, receiver: u32) -> i32 {
        // SAFETY: plain values.
        unsafe { ffi_perform(op, receiver) }
    }

    /// The result, when it is a number, Boolean or reference.
    pub fn result_number() -> f64 {
        // SAFETY: no arguments.
        unsafe { ffi_result_number() }
    }

    /// The result, when it is a string (or an exception's description).
    pub fn result_string() -> String {
        // SAFETY: the host writes exactly ffi_result_string_length() bytes,
        // the UTF-8 of the result, into the buffer.
        unsafe {
            let mut buffer = vec![0u8; ffi_result_string_length()];
            ffi_result_string_copy(buffer.as_mut_ptr());
            String::from_utf8(buffer).unwrap_or_default()
        }
    }

    pub fn print(bytes: &[u8]) {
        // SAFETY: the host reads `len` bytes at `ptr` during the call.
        unsafe { host_print(bytes.as_ptr(), bytes.len()) }
    }

    pub fn monotonic_nanos() -> f64 {
        // SAFETY: no arguments; the host answers a number.
        unsafe { host_monotonic_nanos() }
    }

    pub fn utc_nanos() -> f64 {
        // SAFETY: as above.
        unsafe { host_utc_nanos() }
    }

    pub fn random_bytes(buffer: &mut [u8]) {
        // SAFETY: the host writes at most `len` bytes at `ptr` during the call.
        unsafe { host_random_bytes(buffer.as_mut_ptr(), buffer.len()) }
    }

    pub fn save_image(name: &[u8], bytes: &[u8]) {
        // SAFETY: the host reads both byte ranges during the call.
        unsafe { host_save_image(name.as_ptr(), name.len(), bytes.as_ptr(), bytes.len()) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imports {
    pub fn print(bytes: &[u8]) {
        let _ = bytes;
    }

    pub fn monotonic_nanos() -> f64 {
        0.0
    }

    pub fn utc_nanos() -> f64 {
        0.0
    }

    pub fn random_bytes(buffer: &mut [u8]) {
        buffer.fill(0);
    }

    pub fn save_image(name: &[u8], bytes: &[u8]) {
        let _ = (name, bytes);
    }

    pub fn stage(v: &eurasia_core::host::HostValue) {
        let _ = v;
    }

    pub fn callback_arguments() -> Vec<eurasia_core::host::HostValue> {
        Vec::new()
    }

    /// Without a JavaScript host every operation raises.
    pub fn perform(op: i32, receiver: u32) -> i32 {
        let _ = (op, receiver);
        6
    }

    pub fn result_number() -> f64 {
        0.0
    }

    pub fn result_string() -> String {
        String::from("no JavaScript host")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_report_the_core_versions() {
        assert_eq!(eurasia_kernel_version(), u32::from(KERNEL_VERSION));
        assert_eq!(eurasia_bytecode_version(), u32::from(BYTECODE_VERSION));
        assert_eq!(eurasia_snapshot_format_version(), u32::from(FORMAT_VERSION));
    }

    #[test]
    fn a_refused_snapshot_reports_why() {
        let buffer = eurasia_buffer(3);
        // SAFETY: the buffer has three bytes.
        unsafe { buffer.copy_from_nonoverlapping(b"bad".as_ptr(), 3) };
        assert_eq!(eurasia_start(), FAILED);
        // SAFETY: the error is eurasia_buffer_length() bytes at eurasia_error().
        let message =
            unsafe { std::slice::from_raw_parts(eurasia_error(), eurasia_buffer_length()) };
        assert!(
            std::str::from_utf8(message)
                .unwrap()
                .starts_with("the snapshot was refused")
        );
    }
}
