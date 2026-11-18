// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Eurasia snapshots: the format of `docs/SNAPSHOT_FORMAT.md`.
//!
//! The encoding primitives, the header, and whole images.

#![no_std]

extern crate alloc;

pub mod encoding;
pub mod header;
pub mod image;

pub use encoding::{DecodeError, Reader, Writer};
pub use header::{FORMAT_VERSION, Header, HeaderError, MAGIC, WORD_SIZE};
pub use image::{LoadError, read_checkpoint, read_image, write_image};
