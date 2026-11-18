// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Encoding conventions (format section 1): little-endian fixed-width
//! integers, LEB128 variable-length integers, and tagged values.

use alloc::vec::Vec;
use core::fmt;

use eurasia_core::Value;

/// A failure to decode bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended at `offset` in the middle of a field.
    UnexpectedEnd {
        /// Byte offset where more input was needed.
        offset: usize,
    },
    /// A variable-length integer at `offset` does not fit in 64 bits.
    VarintTooLong {
        /// Byte offset of the integer's first byte.
        offset: usize,
    },
    /// A SmallInteger at `offset` is outside the 63-bit range.
    SmallIntegerOutOfRange {
        /// Byte offset of the value.
        offset: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnexpectedEnd { offset } => {
                write!(f, "snapshot ends unexpectedly at byte {offset}")
            }
            DecodeError::VarintTooLong { offset } => {
                write!(f, "integer at byte {offset} does not fit in 64 bits")
            }
            DecodeError::SmallIntegerOutOfRange { offset } => {
                write!(f, "SmallInteger at byte {offset} is out of range")
            }
        }
    }
}

/// Appends encoded fields to a byte buffer.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Writer {
        Writer::default()
    }

    /// The bytes written so far.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Raw bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        self.bytes.extend_from_slice(b);
    }

    /// `u8`.
    pub fn u8(&mut self, n: u8) {
        self.bytes.push(n);
    }

    /// `u16`, little-endian.
    pub fn u16(&mut self, n: u16) {
        self.bytes(&n.to_le_bytes());
    }

    /// `uv`: unsigned LEB128.
    pub fn uv(&mut self, mut n: u64) {
        loop {
            let low = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                self.bytes.push(low);
                return;
            }
            self.bytes.push(low | 0x80);
        }
    }

    /// `sv`: signed LEB128 of the zigzag encoding.
    pub fn sv(&mut self, n: i64) {
        self.uv(((n << 1) ^ (n >> 63)) as u64);
    }

    /// A value: SmallIntegers zigzagged then tagged; references as
    /// `ref << 1 | 1`, where `ref` is a node number.
    pub fn value(&mut self, v: Value) {
        match v.as_small_integer() {
            Some(n) => self.uv((((n << 1) ^ (n >> 63)) as u64) << 1),
            None => self.uv(v.to_bits()),
        }
    }
}

/// Reads encoded fields from a byte slice, tracking the offset for errors.
pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    /// A reader at the start of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, offset: 0 }
    }

    /// The current byte offset.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Whether every byte has been read.
    pub fn is_at_end(&self) -> bool {
        self.offset == self.bytes.len()
    }

    /// `n` raw bytes.
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(n)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(DecodeError::UnexpectedEnd {
                offset: self.offset,
            })?;
        let b = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(b)
    }

    /// The next byte, without consuming it.
    pub fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    /// `u8`.
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.bytes(1)?[0])
    }

    /// `u16`, little-endian.
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    /// `uv`: unsigned LEB128, at most 64 bits.
    pub fn uv(&mut self) -> Result<u64, DecodeError> {
        let start = self.offset;
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            let low = u64::from(byte & 0x7f);
            if shift == 63 && low > 1 || shift > 63 {
                return Err(DecodeError::VarintTooLong { offset: start });
            }
            result |= low << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// `sv`: signed LEB128 of the zigzag encoding.
    pub fn sv(&mut self) -> Result<i64, DecodeError> {
        let z = self.uv()?;
        Ok(((z >> 1) as i64) ^ -((z & 1) as i64))
    }

    /// A value (see [`Writer::value`]).
    pub fn value(&mut self) -> Result<Value, DecodeError> {
        let start = self.offset;
        let w = self.uv()?;
        if w & 1 == 1 {
            return Ok(Value::from_bits(w));
        }
        let z = w >> 1;
        let n = ((z >> 1) as i64) ^ -((z & 1) as i64);
        Value::small_integer(n).ok_or(DecodeError::SmallIntegerOutOfRange { offset: start })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_uv(n: u64) -> usize {
        let mut w = Writer::new();
        w.uv(n);
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.uv(), Ok(n));
        assert!(r.is_at_end());
        bytes.len()
    }

    #[test]
    fn unsigned_varints_round_trip_with_expected_lengths() {
        assert_eq!(round_trip_uv(0), 1);
        assert_eq!(round_trip_uv(127), 1);
        assert_eq!(round_trip_uv(128), 2);
        assert_eq!(round_trip_uv(16_383), 2);
        assert_eq!(round_trip_uv(u64::MAX), 10);
    }

    #[test]
    fn signed_varints_round_trip() {
        for n in [0, 1, -1, 63, -64, 64, -65, i64::MAX, i64::MIN] {
            let mut w = Writer::new();
            w.sv(n);
            let bytes = w.into_bytes();
            assert_eq!(Reader::new(&bytes).sv(), Ok(n));
        }
    }

    #[test]
    fn small_negative_integers_stay_short() {
        let mut w = Writer::new();
        w.value(Value::small_integer(-1).unwrap());
        assert_eq!(w.into_bytes().len(), 1);
    }

    #[test]
    fn values_round_trip() {
        for v in [
            Value::small_integer(0).unwrap(),
            Value::small_integer(-1).unwrap(),
            Value::small_integer(Value::MAX_SMALL_INTEGER).unwrap(),
            Value::small_integer(Value::MIN_SMALL_INTEGER).unwrap(),
            Value::reference(1).unwrap(),
            Value::reference(1_000_000).unwrap(),
        ] {
            let mut w = Writer::new();
            w.value(v);
            let bytes = w.into_bytes();
            assert_eq!(Reader::new(&bytes).value(), Ok(v));
        }
    }

    #[test]
    fn truncated_input_reports_the_offset() {
        let mut r = Reader::new(&[0x80]);
        assert_eq!(r.uv(), Err(DecodeError::UnexpectedEnd { offset: 1 }));
        assert_eq!(
            Reader::new(&[1]).u16(),
            Err(DecodeError::UnexpectedEnd { offset: 0 })
        );
    }

    #[test]
    fn overlong_varints_are_rejected() {
        let bytes = [0xff; 11];
        assert_eq!(
            Reader::new(&bytes).uv(),
            Err(DecodeError::VarintTooLong { offset: 0 })
        );
        // Ten bytes whose last group carries more than the 64th bit.
        let mut too_big = [0xffu8; 10];
        too_big[9] = 0x02;
        assert_eq!(
            Reader::new(&too_big).uv(),
            Err(DecodeError::VarintTooLong { offset: 0 })
        );
    }
}
