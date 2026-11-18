// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The host interface of the Eurasia VM.
//!
//! Everything nondeterministic or external reaches the VM through [`Host`]:
//! these are the *input* and *output* effects of the kernel layer
//! (`kernel-semantics.tex`, section 13.3), which replay logs and feeds back.
//! M0 defines only the operations M1 needs; calls, callbacks and expats
//! (operation list, section 10) come with the web host in M5.

#![no_std]

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

    /// Input (`vm hostDescription`): a short description of the host.
    fn description(&self) -> &str;

    /// The turn layer's host hook (`vm finishTurn:`): the VM has finished a
    /// turn and asks to be resumed when an event arrives or `wakeup_nanos`
    /// (a monotonic time) passes, if given.
    fn finish_turn(&mut self, wakeup_nanos: Option<u64>);

    /// `vm exit:`: the program asked to end with `code`.
    fn exit(&mut self, code: i32);
}
