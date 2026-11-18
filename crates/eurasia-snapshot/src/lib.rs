// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Eurasia snapshots: the format of `docs/SNAPSHOT_FORMAT.md`.
//!
//! M0 provides the encoding primitives and the header. Sections, nodes and
//! edges follow in M1, together with the packager that writes them.

#![no_std]

extern crate alloc;

pub mod encoding;
pub mod header;

pub use encoding::{DecodeError, Reader, Writer};
pub use header::{FORMAT_VERSION, Header, HeaderError, MAGIC, WORD_SIZE};
