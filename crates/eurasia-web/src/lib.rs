// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Eurasia for the web: the wasm32 build with a JavaScript host.
//!
//! M0 exports only the versions, so the JavaScript loader can refuse a
//! mismatched VM before fetching a snapshot. The call-based alien interface,
//! expats and callbacks come in M5. The exports avoid `wasm-bindgen` for now:
//! they are plain integer functions, needing no generated glue.

// `no_mangle` exports are the one unsafe construct here: the JavaScript host
// finds them by name. Nothing else in this crate is unsafe.
#![allow(unsafe_code)]

use eurasia_core::{BYTECODE_VERSION, KERNEL_VERSION};
use eurasia_snapshot::FORMAT_VERSION;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_report_the_core_versions() {
        assert_eq!(eurasia_kernel_version(), u32::from(KERNEL_VERSION));
        assert_eq!(eurasia_bytecode_version(), u32::from(BYTECODE_VERSION));
        assert_eq!(eurasia_snapshot_format_version(), u32::from(FORMAT_VERSION));
    }
}
