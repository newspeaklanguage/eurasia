// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Host implementations of the Eurasia VM's host interface.
//!
//! The [`Host`] trait itself is defined in `eurasia-core`, because natives
//! call it. This crate holds hosts that are not tied to a platform; the
//! command-line and web hosts live in their own crates.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub use eurasia_core::host::Host;

/// A deterministic host for tests: it records printed lines, its clocks
/// advance by one nanosecond per read, and its entropy is a counter.
#[derive(Default)]
pub struct RecordingHost {
    /// Every line printed, in order.
    pub printed: Vec<String>,
    clock: u64,
    entropy: u8,
}

impl RecordingHost {
    /// A host with nothing printed and its clocks at zero.
    pub fn new() -> RecordingHost {
        RecordingHost::default()
    }
}

impl Host for RecordingHost {
    fn print(&mut self, text: &str) {
        self.printed.push(String::from(text));
    }

    fn monotonic_nanos(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn utc_nanos(&mut self) -> i64 {
        self.clock += 1;
        self.clock as i64
    }

    fn random_bytes(&mut self, buffer: &mut [u8]) {
        for b in buffer {
            self.entropy = self.entropy.wrapping_add(1);
            *b = self.entropy;
        }
    }

    fn description(&self) -> &str {
        "recording test host"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_prints_and_is_deterministic() {
        let mut h = RecordingHost::new();
        h.print("a");
        h.print("b");
        assert_eq!(h.printed, ["a", "b"]);
        assert_eq!(h.monotonic_nanos(), 1);
        assert_eq!(h.monotonic_nanos(), 2);
        let mut buf = [0u8; 3];
        h.random_bytes(&mut buf);
        assert_eq!(buf, [1, 2, 3]);
    }
}
