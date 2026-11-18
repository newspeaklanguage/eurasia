// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Tagged values.
//!
//! Every value is one 64-bit word on every target, wasm32 included (decided
//! 2026-11-17), so the SmallInteger range is the same everywhere. That is
//! required for bit-identical replay and for Croquet.
//!
//! Encoding: the low bit is the tag.
//! - `...xxx0`: a SmallInteger, stored shifted left by one.
//! - `...xxx1`: an object reference, `index << 1 | 1`, where `index` is the
//!   object's slot in an arena (implementation plan section 2.3).

use core::fmt;

/// A Newspeak value: a SmallInteger or an object reference.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Value(u64);

/// A SmallInteger in its untagged form.
pub type SmallInteger = i64;

impl Value {
    /// The smallest SmallInteger: -2^62.
    pub const MIN_SMALL_INTEGER: SmallInteger = -(1 << 62);
    /// The largest SmallInteger: 2^62 - 1.
    pub const MAX_SMALL_INTEGER: SmallInteger = (1 << 62) - 1;
    /// The largest object index a reference can hold.
    pub const MAX_INDEX: u64 = (1 << 63) - 1;

    /// The SmallInteger `n`, or `None` if `n` is outside the SmallInteger range.
    pub fn small_integer(n: SmallInteger) -> Option<Value> {
        if (Self::MIN_SMALL_INTEGER..=Self::MAX_SMALL_INTEGER).contains(&n) {
            Some(Value((n << 1) as u64))
        } else {
            None
        }
    }

    /// A reference to the object at `index`, or `None` if `index` is too large.
    pub fn reference(index: u64) -> Option<Value> {
        if index <= Self::MAX_INDEX {
            Some(Value((index << 1) | 1))
        } else {
            None
        }
    }

    /// Whether this value is a SmallInteger.
    pub fn is_small_integer(self) -> bool {
        self.0 & 1 == 0
    }

    /// The SmallInteger this value holds, if it is one.
    pub fn as_small_integer(self) -> Option<SmallInteger> {
        if self.is_small_integer() {
            Some((self.0 as i64) >> 1)
        } else {
            None
        }
    }

    /// The object index this value refers to, if it is a reference.
    pub fn as_reference(self) -> Option<u64> {
        if self.is_small_integer() {
            None
        } else {
            Some(self.0 >> 1)
        }
    }

    /// The raw tagged word, as stored in a snapshot.
    pub fn to_bits(self) -> u64 {
        self.0
    }

    /// The value with the raw tagged word `bits`.
    pub fn from_bits(bits: u64) -> Value {
        Value(bits)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.as_small_integer() {
            Some(n) => write!(f, "SmallInteger({n})"),
            None => write!(f, "Ref({})", self.0 >> 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_integers_round_trip_across_the_whole_range() {
        for n in [
            0,
            1,
            -1,
            42,
            Value::MIN_SMALL_INTEGER,
            Value::MAX_SMALL_INTEGER,
            Value::MIN_SMALL_INTEGER + 1,
            Value::MAX_SMALL_INTEGER - 1,
        ] {
            let v = Value::small_integer(n).unwrap();
            assert!(v.is_small_integer());
            assert_eq!(v.as_small_integer(), Some(n));
            assert_eq!(v.as_reference(), None);
        }
    }

    #[test]
    fn small_integer_range_is_63_bits_on_every_target() {
        assert_eq!(Value::small_integer(Value::MAX_SMALL_INTEGER + 1), None);
        assert_eq!(Value::small_integer(Value::MIN_SMALL_INTEGER - 1), None);
        // The range does not depend on the pointer width (wasm32 included).
        assert_eq!(Value::MAX_SMALL_INTEGER, 4_611_686_018_427_387_903);
    }

    #[test]
    fn references_round_trip_and_are_not_small_integers() {
        for i in [0, 1, 7, Value::MAX_INDEX] {
            let v = Value::reference(i).unwrap();
            assert!(!v.is_small_integer());
            assert_eq!(v.as_reference(), Some(i));
            assert_eq!(v.as_small_integer(), None);
        }
        assert_eq!(Value::reference(Value::MAX_INDEX + 1), None);
    }

    #[test]
    fn bits_round_trip() {
        let v = Value::small_integer(-12345).unwrap();
        assert_eq!(Value::from_bits(v.to_bits()), v);
        let r = Value::reference(99).unwrap();
        assert_eq!(Value::from_bits(r.to_bits()), r);
    }
}
