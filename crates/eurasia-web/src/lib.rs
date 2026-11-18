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
//! The VM reaches the host through four imports from the module `eurasia`:
//! `print(ptr, len)`, `monotonic_nanos()`, `utc_nanos()` and
//! `random_bytes(ptr, len)`. The call-based alien interface, expats and
//! callbacks come next in M5.

// `no_mangle` exports and the host imports are the unsafe constructs here.
#![allow(unsafe_code)]

use std::cell::RefCell;

use eurasia_core::host::Host;
use eurasia_core::{BYTECODE_VERSION, Completion, KERNEL_VERSION, Vm, VmError};
use eurasia_snapshot::{FORMAT_VERSION, read_image, write_image};

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
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
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

/// The monotonic time to resume at, after [`WAITING`], in nanoseconds (an
/// f64, exact below 2^53).
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_wakeup_nanos() -> f64 {
    STATE.with_borrow(|s| s.wakeup as f64)
}

/// Loads the snapshot in the buffer and runs it, with no arguments, as
/// psoup's web host does.
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_start() -> i32 {
    STATE.with_borrow_mut(|s| {
        let (heap, roles) = match read_image(&s.buffer) {
            Ok(image) => image,
            Err(e) => return fail(s, &format!("the snapshot was refused: {e}")),
        };
        s.buffer = Vec::new();
        let mut vm = Vm::new(heap, roles);
        vm.set_snapshot_writer(write_image);
        let outcome = vm.run(&mut WebHost, &[]);
        s.vm = Some(vm);
        report(s, outcome)
    })
}

/// Continues a run that answered [`WAITING`].
#[unsafe(no_mangle)]
pub extern "C" fn eurasia_resume() -> i32 {
    STATE.with_borrow_mut(|s| {
        let Some(vm) = s.vm.as_mut() else {
            return fail(s, "nothing to resume");
        };
        let outcome = vm.resume(&mut WebHost);
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

    fn description(&self) -> &str {
        "web"
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
