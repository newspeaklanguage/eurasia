// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Arbitrary-precision integers for LargeInteger natives.
//!
//! Sign and magnitude, with the magnitude as little-endian 32-bit limbs and
//! no leading zero limbs; zero is the empty magnitude and is never negative.
//! Division and modulus come in both Newspeak flavours: floored (`//`, `\\`)
//! and truncated (`quo:`, `rem:`). Bit operations behave as on infinite
//! two's-complement integers, as Newspeak's do. Everything here is
//! deterministic and target-independent.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// An integer of any size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInt {
    negative: bool,
    mag: Vec<u32>,
}

fn trim(mut mag: Vec<u32>) -> Vec<u32> {
    while mag.last() == Some(&0) {
        mag.pop();
    }
    mag
}

fn cmp_mag(a: &[u32], b: &[u32]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => {}
            other => return other,
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    let mut out = Vec::with_capacity(long.len() + 1);
    let mut carry = 0u64;
    for (i, &l) in long.iter().enumerate() {
        let s = u64::from(l) + u64::from(*short.get(i).unwrap_or(&0)) + carry;
        out.push(s as u32);
        carry = s >> 32;
    }
    if carry != 0 {
        out.push(carry as u32);
    }
    out
}

/// `a - b` where `a >= b` in magnitude.
fn sub_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i64;
    for (i, &x) in a.iter().enumerate() {
        let mut d = i64::from(x) - i64::from(*b.get(i).unwrap_or(&0)) - borrow;
        if d < 0 {
            d += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out.push(d as u32);
    }
    trim(out)
}

fn mul_mag(a: &[u32], b: &[u32]) -> Vec<u32> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0u64;
        for (j, &y) in b.iter().enumerate() {
            let t = u64::from(x) * u64::from(y) + u64::from(out[i + j]) + carry;
            out[i + j] = t as u32;
            carry = t >> 32;
        }
        let mut k = i + b.len();
        while carry != 0 {
            let t = u64::from(out[k]) + carry;
            out[k] = t as u32;
            carry = t >> 32;
            k += 1;
        }
    }
    trim(out)
}

fn bit_len(mag: &[u32]) -> usize {
    match mag.last() {
        None => 0,
        Some(&top) => (mag.len() - 1) * 32 + (32 - top.leading_zeros() as usize),
    }
}

fn shl_mag(mag: &[u32], bits: usize) -> Vec<u32> {
    if mag.is_empty() {
        return Vec::new();
    }
    let (limbs, rem) = (bits / 32, bits % 32);
    let mut out = vec![0u32; limbs];
    if rem == 0 {
        out.extend_from_slice(mag);
    } else {
        let mut carry = 0u32;
        for &x in mag {
            out.push((x << rem) | carry);
            carry = x >> (32 - rem);
        }
        if carry != 0 {
            out.push(carry);
        }
    }
    trim(out)
}

fn shr_mag(mag: &[u32], bits: usize) -> Vec<u32> {
    let (limbs, rem) = (bits / 32, bits % 32);
    if limbs >= mag.len() {
        return Vec::new();
    }
    let src = &mag[limbs..];
    let mut out = Vec::with_capacity(src.len());
    for i in 0..src.len() {
        let low = src[i] >> rem;
        let high = if rem == 0 {
            0
        } else {
            src.get(i + 1).map_or(0, |&h| h << (32 - rem))
        };
        out.push(low | high);
    }
    trim(out)
}

/// Magnitude division: (quotient, remainder). Shift-and-subtract, one bit at
/// a time; simple and exact. A faster algorithm can replace it without
/// changing results.
fn divmod_mag(a: &[u32], b: &[u32]) -> (Vec<u32>, Vec<u32>) {
    debug_assert!(!b.is_empty());
    if cmp_mag(a, b) == Ordering::Less {
        return (Vec::new(), a.to_vec());
    }
    if b.len() == 1 {
        let d = u64::from(b[0]);
        let mut q = vec![0u32; a.len()];
        let mut r = 0u64;
        for i in (0..a.len()).rev() {
            let cur = (r << 32) | u64::from(a[i]);
            q[i] = (cur / d) as u32;
            r = cur % d;
        }
        return (trim(q), trim(vec![r as u32]));
    }
    let n = bit_len(a);
    let mut q = vec![0u32; a.len()];
    let mut r: Vec<u32> = Vec::new();
    for i in (0..n).rev() {
        r = shl_mag(&r, 1);
        if (a[i / 32] >> (i % 32)) & 1 == 1 {
            if r.is_empty() {
                r.push(1);
            } else {
                r[0] |= 1;
            }
        }
        if cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            q[i / 32] |= 1 << (i % 32);
        }
    }
    (trim(q), r)
}

impl BigInt {
    /// Zero.
    pub fn zero() -> BigInt {
        BigInt {
            negative: false,
            mag: Vec::new(),
        }
    }

    fn new(negative: bool, mag: Vec<u32>) -> BigInt {
        let mag = trim(mag);
        let negative = negative && !mag.is_empty();
        BigInt { negative, mag }
    }

    /// From a machine integer.
    pub fn from_i64(n: i64) -> BigInt {
        let m = n.unsigned_abs();
        BigInt::new(n < 0, vec![m as u32, (m >> 32) as u32])
    }

    /// From a 128-bit integer.
    pub fn from_i128(n: i128) -> BigInt {
        let m = n.unsigned_abs();
        BigInt::new(
            n < 0,
            vec![
                m as u32,
                (m >> 32) as u32,
                (m >> 64) as u32,
                (m >> 96) as u32,
            ],
        )
    }

    /// From a sign and a little-endian byte magnitude (the snapshot form).
    pub fn from_bytes(negative: bool, bytes: &[u8]) -> BigInt {
        let mut mag = vec![0u32; bytes.len().div_ceil(4)];
        for (i, &b) in bytes.iter().enumerate() {
            mag[i / 4] |= u32::from(b) << (8 * (i % 4));
        }
        BigInt::new(negative, mag)
    }

    /// The sign and little-endian byte magnitude, without leading zero bytes.
    pub fn to_bytes(&self) -> (bool, Vec<u8>) {
        let mut bytes = Vec::with_capacity(self.mag.len() * 4);
        for &limb in &self.mag {
            bytes.extend_from_slice(&limb.to_le_bytes());
        }
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        (self.negative, bytes)
    }

    /// The value as an `i64`, if it fits.
    pub fn to_i64(&self) -> Option<i64> {
        if self.mag.len() > 2 {
            return None;
        }
        let m = u64::from(*self.mag.first().unwrap_or(&0))
            | (u64::from(*self.mag.get(1).unwrap_or(&0)) << 32);
        if self.negative {
            if m <= 1 << 63 {
                Some((m as i64).wrapping_neg())
            } else {
                None
            }
        } else {
            i64::try_from(m).ok()
        }
    }

    /// Whether the value is negative.
    pub fn is_negative(&self) -> bool {
        self.negative
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.mag.is_empty()
    }

    /// The nearest `f64` (round half to even), or infinity if too large.
    pub fn to_f64(&self) -> f64 {
        let bits = bit_len(&self.mag);
        let value = if bits <= 64 {
            let m = u64::from(*self.mag.first().unwrap_or(&0))
                | (u64::from(*self.mag.get(1).unwrap_or(&0)) << 32);
            m as f64
        } else {
            // Keep 64 significant bits plus a sticky bit for correct rounding.
            let shift = bits - 64;
            let top = shr_mag(&self.mag, shift);
            let mut m = u64::from(top[0]) | (u64::from(*top.get(1).unwrap_or(&0)) << 32);
            let lost = shl_mag(&top, shift) != self.mag;
            if lost {
                m |= 1;
            }
            let scaled = m as f64;
            if shift > 2000 {
                f64::INFINITY
            } else {
                scaled * libm::pow(2.0, shift as f64)
            }
        };
        if self.negative { -value } else { value }
    }

    /// The exact integer value of a finite `f64` with no fractional part.
    pub fn from_f64_integral(f: f64) -> Option<BigInt> {
        if !f.is_finite() || f != libm::trunc(f) {
            return None;
        }
        let bits = f.abs().to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i64;
        let mantissa = if exponent == 0 {
            (bits & ((1 << 52) - 1)) << 1
        } else {
            (bits & ((1 << 52) - 1)) | (1 << 52)
        };
        let shift = exponent - 1075;
        let m = BigInt::new(false, vec![mantissa as u32, (mantissa >> 32) as u32]);
        let magnitude = if shift >= 0 {
            m.shl(shift as usize)
        } else {
            BigInt::new(false, shr_mag(&m.mag, (-shift) as usize))
        };
        Some(BigInt::new(f < 0.0, magnitude.mag))
    }

    /// Negation.
    pub fn neg(&self) -> BigInt {
        BigInt::new(!self.negative, self.mag.clone())
    }

    /// Sum.
    pub fn add(&self, other: &BigInt) -> BigInt {
        if self.negative == other.negative {
            return BigInt::new(self.negative, add_mag(&self.mag, &other.mag));
        }
        match cmp_mag(&self.mag, &other.mag) {
            Ordering::Less => BigInt::new(other.negative, sub_mag(&other.mag, &self.mag)),
            _ => BigInt::new(self.negative, sub_mag(&self.mag, &other.mag)),
        }
    }

    /// Difference.
    pub fn sub(&self, other: &BigInt) -> BigInt {
        self.add(&other.neg())
    }

    /// Product.
    pub fn mul(&self, other: &BigInt) -> BigInt {
        BigInt::new(
            self.negative != other.negative,
            mul_mag(&self.mag, &other.mag),
        )
    }

    /// Truncated division (`quo:` and `rem:`): the quotient rounds toward
    /// zero and the remainder has the dividend's sign. `None` on zero.
    pub fn quo_rem(&self, other: &BigInt) -> Option<(BigInt, BigInt)> {
        if other.is_zero() {
            return None;
        }
        let (q, r) = divmod_mag(&self.mag, &other.mag);
        Some((
            BigInt::new(self.negative != other.negative, q),
            BigInt::new(self.negative, r),
        ))
    }

    /// Floored division (`//` and `\\`): the quotient rounds toward negative
    /// infinity and the remainder has the divisor's sign. `None` on zero.
    pub fn div_mod(&self, other: &BigInt) -> Option<(BigInt, BigInt)> {
        let (q, r) = self.quo_rem(other)?;
        if !r.is_zero() && (r.negative != other.negative) {
            Some((q.sub(&BigInt::from_i64(1)), r.add(other)))
        } else {
            Some((q, r))
        }
    }

    /// Left shift by `bits` (multiplication by 2^bits).
    pub fn shl(&self, bits: usize) -> BigInt {
        BigInt::new(self.negative, shl_mag(&self.mag, bits))
    }

    /// Arithmetic right shift by `bits` (floored division by 2^bits).
    pub fn shr(&self, bits: usize) -> BigInt {
        if !self.negative {
            return BigInt::new(false, shr_mag(&self.mag, bits));
        }
        // floor(-m / 2^k) = -ceil(m / 2^k)
        let q = shr_mag(&self.mag, bits);
        let exact = shl_mag(&q, bits) == self.mag;
        let q = BigInt::new(false, q);
        let q = if exact {
            q
        } else {
            q.add(&BigInt::from_i64(1))
        };
        q.neg()
    }

    /// The number of bits needed to represent the magnitude.
    pub fn bit_length(&self) -> usize {
        bit_len(&self.mag)
    }

    /// Two's-complement limbs of length `n` (enough for the value plus sign).
    fn twos(&self, n: usize) -> Vec<u32> {
        let mut v = self.mag.clone();
        v.resize(n, 0);
        if self.negative {
            let mut carry = 1u64;
            for limb in &mut v {
                let t = u64::from(!*limb) + carry;
                *limb = t as u32;
                carry = t >> 32;
            }
        }
        v
    }

    fn from_twos(v: Vec<u32>) -> BigInt {
        let negative = v.last().is_some_and(|&top| top >> 31 == 1);
        if !negative {
            return BigInt::new(false, v);
        }
        let mut m = v;
        let mut carry = 1u64;
        for limb in &mut m {
            let t = u64::from(!*limb) + carry;
            *limb = t as u32;
            carry = t >> 32;
        }
        BigInt::new(true, m)
    }

    fn bitwise(&self, other: &BigInt, f: fn(u32, u32) -> u32) -> BigInt {
        let n = self.mag.len().max(other.mag.len()) + 1;
        let (a, b) = (self.twos(n), other.twos(n));
        BigInt::from_twos(a.iter().zip(&b).map(|(&x, &y)| f(x, y)).collect())
    }

    /// Bitwise and, on infinite two's-complement integers.
    pub fn and(&self, other: &BigInt) -> BigInt {
        self.bitwise(other, |x, y| x & y)
    }

    /// Bitwise or.
    pub fn or(&self, other: &BigInt) -> BigInt {
        self.bitwise(other, |x, y| x | y)
    }

    /// Bitwise exclusive or.
    pub fn xor(&self, other: &BigInt) -> BigInt {
        self.bitwise(other, |x, y| x ^ y)
    }

    /// The decimal representation.
    pub fn to_decimal(&self) -> String {
        if self.is_zero() {
            return String::from("0");
        }
        let mut digits = Vec::new();
        let mut mag = self.mag.clone();
        // Peel off nine decimal digits at a time.
        while !mag.is_empty() {
            let (q, r) = divmod_mag(&mag, &[1_000_000_000]);
            let mut chunk = r.first().copied().unwrap_or(0);
            mag = q;
            for _ in 0..9 {
                digits.push(b'0' + (chunk % 10) as u8);
                chunk /= 10;
                if mag.is_empty() && chunk == 0 {
                    break;
                }
            }
        }
        while digits.len() > 1 && digits.last() == Some(&b'0') {
            digits.pop();
        }
        if self.negative {
            digits.push(b'-');
        }
        digits.reverse();
        String::from_utf8(digits).expect("ASCII digits")
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &BigInt) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => cmp_mag(&self.mag, &other.mag),
            (true, true) => cmp_mag(&other.mag, &self.mag),
        }
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &BigInt) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn big(n: i128) -> BigInt {
        BigInt::from_i128(n)
    }

    fn to_i128(b: &BigInt) -> i128 {
        assert!(b.bit_length() <= 128, "result too large for i128: {b:?}");
        let mut v: u128 = 0;
        for (i, &limb) in b.mag.iter().enumerate() {
            v |= u128::from(limb) << (32 * i);
        }
        if b.negative {
            v.wrapping_neg() as i128
        } else {
            v as i128
        }
    }

    /// A spread of values around the interesting boundaries.
    fn samples() -> Vec<i128> {
        let mut v: Vec<i128> = vec![0, 1, -1, 2, -2, 7, -7, 10, -10, 255, 256, -256];
        for shift in [31, 32, 33, 62, 63, 64, 65, 90] {
            let p = 1i128 << shift;
            v.extend([p, -p, p - 1, -(p - 1), p + 1, -(p + 1)]);
        }
        v.extend([123_456_789_012_345_678_901, -98_765_432_109_876_543_210]);
        v
    }

    #[test]
    fn arithmetic_matches_i128() {
        for &a in &samples() {
            for &b in &samples() {
                let (x, y) = (big(a), big(b));
                assert_eq!(to_i128(&x.add(&y)), a + b, "{a} + {b}");
                assert_eq!(to_i128(&x.sub(&y)), a - b, "{a} - {b}");
                if let Some(p) = a.checked_mul(b) {
                    assert_eq!(to_i128(&x.mul(&y)), p, "{a} * {b}");
                }
                assert_eq!(x.cmp(&y), a.cmp(&b), "{a} <=> {b}");
                if b != 0 {
                    let (q, r) = x.quo_rem(&y).unwrap();
                    assert_eq!(
                        (to_i128(&q), to_i128(&r)),
                        (a / b, a % b),
                        "{a} quo/rem {b}"
                    );
                    let (fq, fr) = x.div_mod(&y).unwrap();
                    assert_eq!(
                        (to_i128(&fq), to_i128(&fr)),
                        (
                            a.div_euclid(b) - if b < 0 && a.rem_euclid(b) != 0 { 1 } else { 0 },
                            {
                                let m = a.rem_euclid(b);
                                if b < 0 && m != 0 { m + b } else { m }
                            }
                        ),
                        "{a} //, \\\\ {b}"
                    );
                } else {
                    assert!(x.quo_rem(&y).is_none());
                }
                assert_eq!(to_i128(&x.and(&y)), a & b, "{a} & {b}");
                assert_eq!(to_i128(&x.or(&y)), a | b, "{a} | {b}");
                assert_eq!(to_i128(&x.xor(&y)), a ^ b, "{a} ^ {b}");
            }
        }
    }

    #[test]
    fn shifts_match_i128() {
        for &a in &samples() {
            if a.unsigned_abs() >= 1 << 100 {
                continue;
            }
            for k in [0usize, 1, 5, 31, 32, 33, 20] {
                assert_eq!(to_i128(&big(a).shl(k)), a << k, "{a} << {k}");
                assert_eq!(to_i128(&big(a).shr(k)), a >> k, "{a} >> {k}");
            }
        }
    }

    #[test]
    fn decimal_and_bytes_and_i64() {
        for &a in &samples() {
            assert_eq!(big(a).to_decimal(), a.to_string());
            let (neg, bytes) = big(a).to_bytes();
            assert_eq!(BigInt::from_bytes(neg, &bytes), big(a));
            assert_eq!(big(a).to_i64(), i64::try_from(a).ok());
        }
        assert_eq!(big(i128::from(i64::MIN)).to_i64(), Some(i64::MIN));
    }

    #[test]
    fn floats_convert_exactly_where_possible() {
        for &a in &samples() {
            assert_eq!(big(a).to_f64(), a as f64, "{a} as f64");
        }
        assert_eq!(
            BigInt::from_f64_integral(1e20),
            Some(big(100_000_000_000_000_000_000))
        );
        assert_eq!(BigInt::from_f64_integral(-3.0), Some(big(-3)));
        assert_eq!(BigInt::from_f64_integral(0.5), None);
        assert_eq!(BigInt::from_f64_integral(f64::NAN), None);
    }
}
