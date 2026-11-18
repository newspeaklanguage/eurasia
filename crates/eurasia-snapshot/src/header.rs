// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The snapshot header (format section 2).

use core::fmt;

use eurasia_core::{BYTECODE_VERSION, KERNEL_VERSION};

use crate::encoding::{DecodeError, Reader, Writer};

/// The four magic bytes, `EURA`.
pub const MAGIC: [u8; 4] = *b"EURA";
/// The version of `docs/SNAPSHOT_FORMAT.md` this crate implements.
pub const FORMAT_VERSION: u16 = 1;
/// The word size of every snapshot, on every target.
pub const WORD_SIZE: u8 = 64;

/// A snapshot header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The format version.
    pub format_version: u16,
    /// The kernel version the snapshot was built for.
    pub kernel_version: u16,
    /// The bytecode version its methods use.
    pub bytecode_version: u16,
    /// The word size; always 64.
    pub word_size: u8,
    /// Reserved flags; 0.
    pub flags: u8,
    /// The allocation counter `n_α`: the next sequence number to allocate.
    pub allocation_counter: u64,
    /// The number of nodes in the file.
    pub node_count: u64,
    /// The number of sections.
    pub section_count: u64,
}

/// Why a header was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderError {
    /// The bytes could not be decoded.
    Decode(DecodeError),
    /// The file does not start with `EURA`.
    BadMagic,
    /// A version or the word size differs from this VM's.
    Mismatch {
        /// Which field: "format version", "kernel version", "bytecode
        /// version" or "word size".
        field: &'static str,
        /// The snapshot's value.
        snapshot: u64,
        /// This VM's value.
        vm: u64,
    },
    /// Reserved flags are set.
    UnknownFlags(u8),
}

impl From<DecodeError> for HeaderError {
    fn from(e: DecodeError) -> HeaderError {
        HeaderError::Decode(e)
    }
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HeaderError::Decode(e) => write!(f, "{e}"),
            HeaderError::BadMagic => write!(f, "not a Eurasia snapshot (bad magic)"),
            HeaderError::Mismatch {
                field,
                snapshot,
                vm,
            } => write!(
                f,
                "snapshot {field} is {snapshot}, but this VM supports {vm}"
            ),
            HeaderError::UnknownFlags(b) => write!(f, "unknown header flags {b:#04x}"),
        }
    }
}

impl Header {
    /// A header for this VM's versions.
    pub fn current(allocation_counter: u64, node_count: u64, section_count: u64) -> Header {
        Header {
            format_version: FORMAT_VERSION,
            kernel_version: KERNEL_VERSION,
            bytecode_version: BYTECODE_VERSION,
            word_size: WORD_SIZE,
            flags: 0,
            allocation_counter,
            node_count,
            section_count,
        }
    }

    /// Writes the header.
    pub fn write(&self, w: &mut Writer) {
        w.bytes(&MAGIC);
        w.u16(self.format_version);
        w.u16(self.kernel_version);
        w.u16(self.bytecode_version);
        w.u8(self.word_size);
        w.u8(self.flags);
        w.uv(self.allocation_counter);
        w.uv(self.node_count);
        w.uv(self.section_count);
    }

    /// Reads a header, skipping a leading `#!` line, and refuses one this VM
    /// cannot load, naming the field that differs.
    pub fn read(r: &mut Reader<'_>) -> Result<Header, HeaderError> {
        if r.peek() == Some(b'#') {
            while r.u8()? != b'\n' {}
        }
        if r.bytes(4)? != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        let header = Header {
            format_version: r.u16()?,
            kernel_version: r.u16()?,
            bytecode_version: r.u16()?,
            word_size: r.u8()?,
            flags: r.u8()?,
            allocation_counter: r.uv()?,
            node_count: r.uv()?,
            section_count: r.uv()?,
        };
        let checks: [(&'static str, u64, u64); 4] = [
            (
                "format version",
                header.format_version.into(),
                FORMAT_VERSION.into(),
            ),
            (
                "kernel version",
                header.kernel_version.into(),
                KERNEL_VERSION.into(),
            ),
            (
                "bytecode version",
                header.bytecode_version.into(),
                BYTECODE_VERSION.into(),
            ),
            ("word size", header.word_size.into(), WORD_SIZE.into()),
        ];
        for (field, snapshot, vm) in checks {
            if snapshot != vm {
                return Err(HeaderError::Mismatch {
                    field,
                    snapshot,
                    vm,
                });
            }
        }
        if header.flags != 0 {
            return Err(HeaderError::UnknownFlags(header.flags));
        }
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn bytes_of(h: &Header) -> Vec<u8> {
        let mut w = Writer::new();
        h.write(&mut w);
        w.into_bytes()
    }

    #[test]
    fn header_round_trips() {
        let h = Header::current(1234, 1000, 3);
        let bytes = bytes_of(&h);
        assert_eq!(&bytes[..4], b"EURA");
        let mut r = Reader::new(&bytes);
        assert_eq!(Header::read(&mut r), Ok(h));
        assert!(r.is_at_end());
    }

    #[test]
    fn interpreter_line_is_skipped() {
        let mut bytes = b"#!/usr/bin/env eurasia\n".to_vec();
        bytes.extend(bytes_of(&Header::current(1, 0, 1)));
        assert!(Header::read(&mut Reader::new(&bytes)).is_ok());
    }

    #[test]
    fn bad_magic_is_refused() {
        let mut bytes = bytes_of(&Header::current(1, 0, 1));
        bytes[0] = b'X';
        assert_eq!(
            Header::read(&mut Reader::new(&bytes)),
            Err(HeaderError::BadMagic)
        );
    }

    #[test]
    fn a_newer_kernel_is_refused_by_name() {
        let mut h = Header::current(1, 0, 1);
        h.kernel_version = KERNEL_VERSION + 1;
        let err = Header::read(&mut Reader::new(&bytes_of(&h))).unwrap_err();
        assert_eq!(
            err,
            HeaderError::Mismatch {
                field: "kernel version",
                snapshot: (KERNEL_VERSION + 1).into(),
                vm: KERNEL_VERSION.into(),
            }
        );
        assert_eq!(
            alloc::format!("{err}"),
            alloc::format!(
                "snapshot kernel version is {}, but this VM supports {}",
                KERNEL_VERSION + 1,
                KERNEL_VERSION
            )
        );
    }

    #[test]
    fn a_32_bit_snapshot_is_refused() {
        let mut h = Header::current(1, 0, 1);
        h.word_size = 32;
        assert!(matches!(
            Header::read(&mut Reader::new(&bytes_of(&h))),
            Err(HeaderError::Mismatch {
                field: "word size",
                ..
            })
        ));
    }

    #[test]
    fn truncated_header_is_a_decode_error() {
        let bytes = bytes_of(&Header::current(1, 0, 1));
        assert!(matches!(
            Header::read(&mut Reader::new(&bytes[..7])),
            Err(HeaderError::Decode(DecodeError::UnexpectedEnd { .. }))
        ));
    }
}
