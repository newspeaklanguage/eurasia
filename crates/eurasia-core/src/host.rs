// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The host interface.
//!
//! Everything nondeterministic or external reaches the VM through [`Host`]:
//! these are the *input* and *output* effects of the kernel layer
//! (`kernel-semantics.tex`, section 13.3), which replay logs and feeds back.
//! The trait lives in the core because natives call it; host implementations
//! live in `eurasia-native` and `eurasia-web`. Calls, callbacks and expats
//! (operation list, section 10) come with the web host in M5.

/// A value crossing to or from the host (operation list, section 10): the
/// bilingual values by value, anything else as a host reference, an index
/// into the host's table of objects. Reference 0 is the host's `undefined`
/// and 1 its global object.
#[derive(Clone, Debug, PartialEq)]
pub enum HostValue {
    /// JavaScript `undefined`, which arrives in Newspeak as an Alien.
    Undefined,
    /// `null`, which arrives as nil.
    Null,
    /// A Boolean.
    Bool(bool),
    /// A number. Integral numbers of magnitude at most 2^53 arrive as
    /// Integers, others as Floats.
    Number(f64),
    /// A string.
    String(alloc::string::String),
    /// Any other host object: an index into the host's table.
    Ref(u32),
    /// To the host only: a Newspeak closure as a host-callable function, by
    /// its index in the VM's expat table. Calling it calls
    /// [`HostCallbacks::callback`].
    Expat(u32),
}

/// Calling back into the VM from the host (design doc section 4). The host
/// gets this during a host operation, for callbacks that arrive while the
/// operation runs (sub-steps of the current turn); a host whose event loop
/// calls an expat while the VM is idle calls the VM's own implementation.
pub trait HostCallbacks {
    /// Runs expat `expat` with `args` and answers its result. Extra
    /// arguments are dropped and missing ones are nil. An exception in the
    /// callback, a non-local return out of it, too deep a nesting, or a dead
    /// expat fail it, and the host caller sees a host exception.
    fn callback(
        &mut self,
        host: &mut dyn Host,
        expat: u32,
        args: &[HostValue],
    ) -> Result<HostValue, HostError>;
}

/// The reference to the host's global object.
pub const HOST_GLOBAL: u32 = 1;

/// A host-object operation (operation list, section 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOp {
    /// `receiver[args[0]]`.
    Get,
    /// `receiver[args[0]] = args[1]`.
    Set,
    /// `delete receiver[args[0]]`.
    Delete,
    /// `args[0] in receiver`.
    Has,
    /// `receiver[args[0]](args[1..])`, with `this` the receiver.
    Call,
    /// `receiver(args..)`: the receiver is a function.
    Apply,
    /// `new receiver(args..)`.
    New,
    /// `receiver instanceof args[0]`.
    InstanceOf,
}

/// Why a host operation failed.
#[derive(Clone, Debug, PartialEq)]
pub enum HostError {
    /// This host has no host objects (the command line, before a native FFI).
    Unsupported,
    /// The host raised; its description of the exception.
    Raised(alloc::string::String),
    /// The reference names no host object.
    Dead,
}

/// The services a host provides to the VM.
pub trait Host {
    /// Output (`vm print:`): send a line of text to the host's console.
    fn print(&mut self, text: &str);

    /// Input (`vm monotonicNanos`): a monotonic clock in nanoseconds.
    fn monotonic_nanos(&mut self) -> u64;

    /// Input (`vm utcNanos`): wall-clock time in nanoseconds since the epoch.
    fn utc_nanos(&mut self) -> i64;

    /// Input (`vm randomBytes:`): fill `buffer` with host entropy.
    fn random_bytes(&mut self, buffer: &mut [u8]);

    /// Waits until the monotonic clock reaches `nanos` (the next timer), or
    /// an event arrives. The default does not wait: the event loop then
    /// polls the clock, which is correct but busy.
    fn wait_until(&mut self, nanos: u64) {
        let _ = nanos;
    }

    /// Whether [`Host::wait_until`] may block. A host that cannot (the
    /// browser's, whose event loop must keep running) answers false: the VM
    /// then returns [`crate::Completion::Waiting`] instead of waiting.
    fn blocks(&self) -> bool {
        true
    }

    /// Input and output (operation list, section 10): a host-object
    /// operation on the object `receiver` refers to. Callbacks into the VM
    /// while it runs go through `callbacks`. The default host has none.
    fn host_operation(
        &mut self,
        op: HostOp,
        receiver: u32,
        args: &[HostValue],
        callbacks: &mut dyn HostCallbacks,
    ) -> Result<HostValue, HostError> {
        let _ = (op, receiver, args, callbacks);
        Err(HostError::Unsupported)
    }

    /// Output (M7): a checkpoint, as snapshot bytes the VM can be restored
    /// from. The VM makes one only when [`Host::wants_checkpoints`].
    fn checkpoint(&mut self, bytes: alloc::vec::Vec<u8>) {
        let _ = bytes;
    }

    /// Whether this host keeps checkpoints (a recording or replaying host).
    fn wants_checkpoints(&self) -> bool {
        false
    }

    /// `vm logPosition`: the position in this host's input log, if it has
    /// one.
    fn log_position(&self) -> Option<u64> {
        None
    }

    /// Whether the VM should stop (a replay that diverged); the event loop
    /// asks now and then and stops with [`crate::VmError::Interrupted`].
    fn interrupted(&self) -> bool {
        false
    }

    /// Input (`vm hostDescription`): a short description of the host.
    fn description(&self) -> &str;

    /// Output (`vm writeBytes:toFile:`): write `bytes` to the file `path`.
    /// An interim host operation until the FFI (M5) makes file I/O a library;
    /// hosts without files keep the default, which refuses.
    fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), alloc::string::String> {
        let _ = (path, bytes);
        Err(alloc::string::String::from("this host has no files"))
    }

    /// Input (`vm readFile:`): the contents of the file `path`.
    fn read_file(&mut self, path: &str) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        let _ = path;
        Err(alloc::string::String::from("this host has no files"))
    }
}
