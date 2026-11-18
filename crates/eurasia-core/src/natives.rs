// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The native catalog of kernel version 1 (operation list, section 3; rules
//! Native-Answer, Native-Tail and Native-Fail of `kernel-semantics.tex`).
//!
//! Every native is total: it answers, makes a defined tail send, or fails
//! with a reason. There are no fallback bodies. Integers are SmallIntegers
//! (63-bit immediates) or LargeIntegers (heap objects); a result that fits
//! a SmallInteger is always one. Floats are IEEE 754 binary64 with one
//! canonical NaN, and every function on them comes from the bundled `libm`,
//! so results are bit-identical on every host (decided 2026-11-10).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::bigint::BigInt;
use crate::host::Host;
use crate::object::{Body, FROZEN, Heap, Space};
use crate::roles::{Role, Roles};
use crate::value::{SmallInteger, Value};

macro_rules! natives {
    ($($name:ident = $id:literal / $arity:literal: $doc:literal,)*) => {
        /// A catalog entry, numbered as in method headers.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u32)]
        pub enum Native {
            $(#[doc = $doc] $name = $id,)*
        }

        impl Native {
            /// Every entry, in id order.
            pub const ALL: &'static [Native] = &[$(Native::$name,)*];

            /// The entry with catalog id `id`.
            pub fn from_id(id: u32) -> Option<Native> {
                match id {
                    $($id => Some(Native::$name),)*
                    _ => None,
                }
            }

            /// The number of arguments, not counting the receiver.
            pub fn arity(self) -> usize {
                match self {
                    $(Native::$name => $arity,)*
                }
            }

            /// The entry's name, for diagnostics and failure objects.
            pub fn name(self) -> &'static str {
                match self {
                    $(Native::$name => stringify!($name),)*
                }
            }
        }
    };
}

natives! {
    IntegerAdd = 1 / 1: "`Integer>>+`; a non-integer argument tail-sends `addFromInteger:`.",
    IntegerSubtract = 2 / 1: "`Integer>>-`; otherwise `subtractFromInteger:`.",
    IntegerMultiply = 3 / 1: "`Integer>>*`; otherwise `multiplyFromInteger:`.",
    IntegerLess = 4 / 1: "`Number>><` on integers and floats; otherwise the reversed comparison `>`.",
    IntegerGreater = 5 / 1: "`Number>>>`; otherwise `<`.",
    IntegerLessEqual = 6 / 1: "`Number>><=`; otherwise `>=`.",
    IntegerGreaterEqual = 7 / 1: "`Number>>>=`; otherwise `<=`.",
    NumberEqual = 8 / 1: "`Number>>=`: false for a non-number (decided 2026-11-10).",
    NumberAsString = 9 / 0: "`Number>>asString`.",
    ObjectIdentical = 10 / 1: "`Object>>==`.",
    ObjectClass = 11 / 0: "`Object>>class`.",
    ObjectIdentityHash = 12 / 0: "`Object>>identityHash`: from the allocation sequence number.",
    StringConcatenate = 13 / 1: "`String>>,`.",
    StringSize = 14 / 0: "`String>>size`, in bytes.",
    NumberFloorDivide = 15 / 1: "`Number>>//`: floored.",
    NumberFloorModulo = 16 / 1: "`Number>>\\\\`: floored; the sign of the divisor.",
    NumberQuo = 17 / 1: "`Number>>quo:`: truncated.",
    NumberRem = 18 / 1: "`Number>>rem:`: truncated; the sign of the dividend.",
    VmPrint = 20 / 1: "`vm print:` (output).",
    VmExit = 21 / 1: "`vm exit:`.",
    IntegerBitAnd = 22 / 1: "`Integer>>bitAnd:` and `&`.",
    IntegerBitOr = 23 / 1: "`Integer>>bitOr:` and `|`.",
    IntegerBitXor = 24 / 1: "`Integer>>bitXor:`.",
    IntegerShiftLeft = 25 / 1: "`Integer>><<`.",
    IntegerShiftRight = 26 / 1: "`Integer>>>>`: arithmetic.",
    NumberAsFloat = 27 / 0: "`Number>>asFloat`.",
    NumberAsInteger = 28 / 0: "`Number>>asInteger`: truncated.",
    ClosureValue = 30 / 0: "`Closure>>value`.",
    ClosureValue1 = 31 / 1: "`Closure>>value:`.",
    ClosureValue2 = 32 / 2: "`Closure>>value:value:`.",
    ClosureValue3 = 33 / 3: "`Closure>>value:value:value:`.",
    ClosureValueWithArguments = 34 / 1: "`Closure>>valueWithArguments:`.",
    ClosureNumArgs = 35 / 0: "`Closure>>numArgs`.",
    FloatAdd = 40 / 1: "`Float>>+`; otherwise `addFromFloat:`.",
    FloatSubtract = 41 / 1: "`Float>>-`; otherwise `subtractFromFloat:`.",
    FloatMultiply = 42 / 1: "`Float>>*`; otherwise `multiplyFromFloat:`.",
    FloatDivide = 43 / 1: "`Float>>/`; otherwise `divideFromFloat:`.",
    FloatFloor = 44 / 0: "`Float>>floor`.",
    FloatCeiling = 45 / 0: "`Float>>ceiling`.",
    FloatRounded = 46 / 0: "`Float>>rounded`: half away from zero.",
    FloatSqrt = 48 / 0: "`Float>>sqrt`.",
    FloatSin = 49 / 0: "`Float>>sin`.",
    FloatCos = 50 / 0: "`Float>>cos`.",
    FloatTan = 51 / 0: "`Float>>tan`.",
    FloatExp = 52 / 0: "`Float>>exp`.",
    FloatLn = 53 / 0: "`Float>>ln`.",
    FloatLog = 54 / 0: "`Float>>log`: base 10.",
    FloatPow = 55 / 1: "`Float>>pow:`.",
    FloatArcSin = 56 / 0: "`Float>>arcSin`.",
    FloatArcCos = 57 / 0: "`Float>>arcCos`.",
    FloatArcTan = 58 / 0: "`Float>>arcTan`.",
    FloatArcTan2 = 59 / 1: "`Float>>arcTan:`.",
    FloatAsStringFixed = 60 / 1: "`Float>>asStringFixed:`.",
    FloatAsStringExponential = 61 / 1: "`Float>>asStringExponential:`.",
    FloatAsStringPrecision = 62 / 1: "`Float>>asStringPrecision:`.",
    FloatParse = 63 / 1: "`Float class>>parse:`.",
    ClassBasicNew = 70 / 0: "`Class>>basicNew`.",
    IndexedNew = 71 / 1: "`new:` on the Array, WeakArray and ByteArray classes.",
    IndexedAt = 72 / 1: "`at:` on Array, WeakArray, ByteArray and String (a byte).",
    IndexedAtPut = 73 / 2: "`at:put:` on Array, WeakArray and ByteArray.",
    IndexedSize = 74 / 0: "`size` on Array, WeakArray and ByteArray.",
    CopyFromTo = 75 / 2: "`copyFrom:to:` on Array, ByteArray and String: the same kind.",
    CopyStringFromTo = 76 / 2: "`copyStringFrom:to:` on ByteArray and String.",
    CopyByteArrayFromTo = 77 / 2: "`copyByteArrayFrom:to:` on ByteArray and String.",
    ReplaceFromToWithStartingAt = 78 / 4: "`replaceFrom:to:with:startingAt:` on Array and ByteArray.",
    ByteArrayWithAll = 79 / 1: "`ByteArray class>>withAll:` from a ByteArray or String.",
    StringWithAll = 80 / 1: "`String class>>withAll:` from a ByteArray or String.",
    StringWith = 81 / 1: "`String class>>with:` a byte.",
    StringHash = 82 / 0: "`String>>hash`: a function of the bytes.",
    StringEquals = 83 / 1: "`String>>=`: false for a non-String.",
    StartsWith = 84 / 1: "`startsWith:` on String and ByteArray.",
    EndsWith = 85 / 1: "`endsWith:` on String and ByteArray.",
    IndexOfStartingAt = 86 / 2: "`indexOf:startingAt:`: 0 if absent.",
    LastIndexOfStartingAt = 87 / 2: "`lastIndexOf:startingAt:`: 0 if absent.",
    ObjectPerformWithArguments = 89 / 2: "`Object>>perform:withArguments:`: an ordinary send.",
    VmSnapshot = 90 / 0: "`vm snapshot`: the heap, saved without frames, as a ByteArray.",
    VmWriteFile = 91 / 2: "`vm writeBytes:toFile:` (output; interim host operation).",
    VmReadFile = 92 / 1: "`vm readFile:` (input; interim host operation).",
    VmSetRole = 93 / 2: "`vm setRole:to:`: assign a role by its snapshot id.",
}

/// The largest shift `<<` performs; beyond it the result would exhaust
/// memory, and the native fails with `#exhausted: #memory`.
pub const MAX_SHIFT: i64 = 1 << 24;

/// A resource whose bound can be exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// Heap memory.
    Memory,
    /// Stack depth.
    Stack,
    /// Callback nesting.
    Nesting,
    /// The alien handle table.
    Handles,
}

/// Why a native failed (operation list, section 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Argument `i` (0 = receiver) is of an unacceptable kind.
    WrongKind(u32),
    /// Argument `i` is an index or size out of range.
    OutOfRange(u32),
    /// Division or modulus by zero.
    ZeroDivide,
    /// Wrong number of arguments.
    Arity,
    /// The class cannot be instantiated this way.
    NotInstantiable,
    /// A dead activation, alien or expat.
    Dead,
    /// A resource limit.
    Exhausted(Resource),
    /// The host raised; the value is the host exception.
    Host(Value),
    /// Not provided on this host, or not yet implemented.
    Unsupported,
}

impl Reason {
    /// The reason's name, as stored in failure objects.
    pub fn name(self) -> &'static str {
        match self {
            Reason::WrongKind(_) => "wrongKind",
            Reason::OutOfRange(_) => "outOfRange",
            Reason::ZeroDivide => "zeroDivide",
            Reason::Arity => "arity",
            Reason::NotInstantiable => "notInstantiable",
            Reason::Dead => "dead",
            Reason::Exhausted(_) => "exhausted",
            Reason::Host(_) => "host",
            Reason::Unsupported => "unsupported",
        }
    }

    /// The reason's argument: an index, a resource name, or the host value.
    pub fn argument(self, ctx: &mut NativeContext<'_>) -> Value {
        match self {
            Reason::WrongKind(i) | Reason::OutOfRange(i) => small(i64::from(i)),
            Reason::Exhausted(r) => {
                let name = match r {
                    Resource::Memory => "memory",
                    Resource::Stack => "stack",
                    Resource::Nesting => "nesting",
                    Resource::Handles => "handles",
                };
                ctx.new_string(name)
            }
            Reason::Host(v) => v,
            _ => ctx.roles.get(Role::Nil),
        }
    }
}

/// What a native did (the three outcomes, plus control transfers the
/// interpreter performs).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Rule Native-Answer.
    Answer(Value),
    /// Rule Native-Tail: an ordinary send of `selector` to `receiver`.
    TailSend {
        /// The new receiver.
        receiver: Value,
        /// The selector.
        selector: &'static str,
        /// The arguments.
        args: Vec<Value>,
    },
    /// Rule Native-Fail.
    Fail(Reason),
    /// `vm exit:` ended the program.
    Exit(i32),
    /// A closure's `value` family: run `closure` with `args` in a new frame.
    /// The arity has been checked.
    ActivateClosure {
        /// The closure.
        closure: Value,
        /// Its arguments.
        args: Vec<Value>,
    },
    /// `vm snapshot`: the interpreter serializes the heap and answers it.
    Snapshot,
    /// `vm setRole:to:`: the interpreter assigns the role.
    SetRole {
        /// The role.
        role: Role,
        /// Its new value.
        value: Value,
    },
    /// `perform:withArguments:`: an ordinary send whose selector is a
    /// String object. The arity has been checked.
    Perform {
        /// The receiver.
        receiver: Value,
        /// The selector (a String).
        selector: Value,
        /// The arguments.
        args: Vec<Value>,
    },
}

/// What a native may touch.
pub struct NativeContext<'a> {
    /// The heap.
    pub heap: &'a mut Heap,
    /// The roles.
    pub roles: &'a Roles,
    /// The host.
    pub host: &'a mut dyn Host,
}

impl NativeContext<'_> {
    /// Allocates a String.
    pub fn new_string(&mut self, s: &str) -> Value {
        self.new_bytes_object(Role::StringClass, Body::String(s.as_bytes().to_vec()))
    }

    fn new_bytes_object(&mut self, role: Role, body: Body) -> Value {
        let class = self.roles.get(role);
        self.heap.allocate(class, Space::Actor(0), body)
    }

    fn boolean(&self, b: bool) -> Value {
        self.roles.get(if b { Role::True } else { Role::False })
    }

    fn nil(&self) -> Value {
        self.roles.get(Role::Nil)
    }

    /// An integer value: a SmallInteger when it fits, else a LargeInteger.
    fn integer(&mut self, n: BigInt) -> Value {
        if let Some(v) = n.to_i64().and_then(Value::small_integer) {
            return v;
        }
        let (negative, magnitude) = n.to_bytes();
        let class = self.roles.get(Role::LargeIntegerClass);
        self.heap.allocate(
            class,
            Space::Actor(0),
            Body::LargeInteger {
                negative,
                magnitude,
            },
        )
    }

    /// A Float value, with NaN canonical.
    fn float(&mut self, f: f64) -> Value {
        let f = if f.is_nan() { f64::NAN } else { f };
        let class = self.roles.get(Role::FloatClass);
        self.heap.allocate(class, Space::Actor(0), Body::Float(f))
    }
}

fn small(n: SmallInteger) -> Value {
    Value::small_integer(n).expect("small enough")
}

/// A number, as natives see it.
#[derive(Clone, Debug)]
enum Num {
    Int(BigInt),
    Float(f64),
}

fn integer_of(heap: &Heap, v: Value) -> Option<BigInt> {
    if let Some(n) = v.as_small_integer() {
        return Some(BigInt::from_i64(n));
    }
    match &heap.get(v)?.body {
        Body::LargeInteger {
            negative,
            magnitude,
        } => Some(BigInt::from_bytes(*negative, magnitude)),
        _ => None,
    }
}

fn float_of(heap: &Heap, v: Value) -> Option<f64> {
    match &heap.get(v)?.body {
        Body::Float(f) => Some(*f),
        _ => None,
    }
}

fn number_of(heap: &Heap, v: Value) -> Option<Num> {
    integer_of(heap, v)
        .map(Num::Int)
        .or_else(|| float_of(heap, v).map(Num::Float))
}

fn as_f64(n: &Num) -> f64 {
    match n {
        Num::Int(i) => i.to_f64(),
        Num::Float(f) => *f,
    }
}

fn small_index(v: Value) -> Option<i64> {
    v.as_small_integer()
}

/// Integer arithmetic: an integer argument computes; anything else
/// tail-sends the double-dispatch selector to the argument.
fn integer_arithmetic(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(&BigInt, &BigInt) -> BigInt,
    tail: &'static str,
) -> Outcome {
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match integer_of(ctx.heap, arg) {
        Some(b) => Outcome::Answer(ctx.integer(op(&a, &b))),
        None => Outcome::TailSend {
            receiver: arg,
            selector: tail,
            args: alloc::vec![receiver],
        },
    }
}

/// Float arithmetic: an integer or float argument computes; anything else
/// tail-sends the double-dispatch selector.
fn float_arithmetic(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(f64, f64) -> f64,
    tail: &'static str,
    divides: bool,
) -> Outcome {
    let Some(a) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match number_of(ctx.heap, arg) {
        Some(b) => {
            let b = as_f64(&b);
            if divides && b == 0.0 {
                return Outcome::Fail(Reason::ZeroDivide);
            }
            Outcome::Answer(ctx.float(op(a, b)))
        }
        None => Outcome::TailSend {
            receiver: arg,
            selector: tail,
            args: alloc::vec![receiver],
        },
    }
}

fn compare(a: &Num, b: &Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(y)),
        _ => as_f64(a).partial_cmp(&as_f64(b)),
    }
}

/// A comparison. A non-number argument tail-sends the reversed comparison
/// (`a < b` becomes `b > a`), so other kinds of number can answer.
fn comparison(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    test: fn(Ordering) -> bool,
    reversed: &'static str,
) -> Outcome {
    let Some(a) = number_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match number_of(ctx.heap, arg) {
        Some(b) => Outcome::Answer(ctx.boolean(compare(&a, &b).is_some_and(test))),
        None => Outcome::TailSend {
            receiver: arg,
            selector: reversed,
            args: alloc::vec![receiver],
        },
    }
}

#[derive(Clone, Copy)]
enum Division {
    FloorDivide,
    FloorModulo,
    Quo,
    Rem,
}

fn division(ctx: &mut NativeContext<'_>, receiver: Value, arg: Value, kind: Division) -> Outcome {
    let Some(a) = number_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(b) = number_of(ctx.heap, arg) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        let result = match kind {
            Division::FloorDivide | Division::FloorModulo => x.div_mod(y),
            Division::Quo | Division::Rem => x.quo_rem(y),
        };
        let Some((q, r)) = result else {
            return Outcome::Fail(Reason::ZeroDivide);
        };
        let v = match kind {
            Division::FloorDivide | Division::Quo => q,
            Division::FloorModulo | Division::Rem => r,
        };
        return Outcome::Answer(ctx.integer(v));
    }
    let (x, y) = (as_f64(&a), as_f64(&b));
    if y == 0.0 {
        return Outcome::Fail(Reason::ZeroDivide);
    }
    let quotient = match kind {
        Division::FloorDivide | Division::FloorModulo => libm::floor(x / y),
        Division::Quo | Division::Rem => libm::trunc(x / y),
    };
    match kind {
        Division::FloorDivide | Division::Quo => match BigInt::from_f64_integral(quotient) {
            Some(q) => Outcome::Answer(ctx.integer(q)),
            None => Outcome::Fail(Reason::OutOfRange(0)),
        },
        Division::FloorModulo | Division::Rem => Outcome::Answer(ctx.float(x - y * quotient)),
    }
}

fn integer_bits(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(&BigInt, &BigInt) -> BigInt,
) -> Outcome {
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(b) = integer_of(ctx.heap, arg) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    Outcome::Answer(ctx.integer(op(&a, &b)))
}

fn shift(ctx: &mut NativeContext<'_>, receiver: Value, arg: Value, left: bool) -> Outcome {
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(k) = arg.as_small_integer() else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    if k < 0 {
        return Outcome::Fail(Reason::OutOfRange(1));
    }
    if left {
        if k > MAX_SHIFT && !a.is_zero() {
            return Outcome::Fail(Reason::Exhausted(Resource::Memory));
        }
        Outcome::Answer(ctx.integer(a.shl(k.min(MAX_SHIFT) as usize)))
    } else {
        let k = usize::try_from(k)
            .unwrap_or(usize::MAX)
            .min(a.bit_length() + 1);
        Outcome::Answer(ctx.integer(a.shr(k)))
    }
}

/// The shortest decimal that reads back as the same Float, with `.0` on
/// integral values, and `NaN`, `Infinity`, `-Infinity`.
pub fn float_to_string(f: f64) -> String {
    if f.is_nan() {
        return String::from("NaN");
    }
    if f.is_infinite() {
        return String::from(if f > 0.0 { "Infinity" } else { "-Infinity" });
    }
    format!("{f:?}")
}

fn float_unary(ctx: &mut NativeContext<'_>, receiver: Value, f: fn(f64) -> f64) -> Outcome {
    match float_of(ctx.heap, receiver) {
        Some(x) => Outcome::Answer(ctx.float(f(x))),
        None => Outcome::Fail(Reason::WrongKind(0)),
    }
}

fn float_to_integer(ctx: &mut NativeContext<'_>, receiver: Value, f: fn(f64) -> f64) -> Outcome {
    let Some(x) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match BigInt::from_f64_integral(f(x)) {
        Some(n) => Outcome::Answer(ctx.integer(n)),
        None => Outcome::Fail(Reason::OutOfRange(0)),
    }
}

fn float_format(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    format: fn(f64, usize) -> String,
) -> Outcome {
    let Some(x) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(digits) = arg.as_small_integer().filter(|d| (0..=100).contains(d)) else {
        return Outcome::Fail(Reason::OutOfRange(1));
    };
    if !x.is_finite() {
        let s = float_to_string(x);
        return Outcome::Answer(ctx.new_string(&s));
    }
    let s = format(x, digits as usize);
    Outcome::Answer(ctx.new_string(&s))
}

/// The bytes of a String or ByteArray.
fn bytes_of(heap: &Heap, v: Value) -> Option<&[u8]> {
    match &heap.get(v)?.body {
        Body::String(b) | Body::Bytes(b) => Some(b),
        _ => None,
    }
}

/// The number of instance slots of `class`: every mixin's slot declarations
/// along the superclass chain.
fn instance_slots(heap: &Heap, roles: &Roles, class: Value) -> Option<usize> {
    let nil = roles.get(Role::Nil);
    let mut total = 0usize;
    let mut c = class;
    for _ in 0..100_000 {
        if c == nil {
            return Some(total);
        }
        let b = heap.behavior(c)?;
        total += heap.mixin(b.mixin)?.slot_count as usize;
        c = b.superclass;
    }
    None
}

/// A one-based index into a sequence of `len` elements.
fn index(v: Value, len: usize) -> Result<usize, Reason> {
    match small_index(v) {
        None => Err(Reason::WrongKind(1)),
        Some(i) if i >= 1 && (i as u64) <= len as u64 => Ok(i as usize - 1),
        Some(_) => Err(Reason::OutOfRange(1)),
    }
}

/// A one-based inclusive range `start..=stop` in a sequence of `len`
/// elements; `stop = start - 1` is empty. Arguments are numbered from
/// `first` for failure reasons.
fn range(
    start: Value,
    stop: Value,
    len: usize,
    first: u32,
) -> Result<core::ops::Range<usize>, Reason> {
    let (Some(a), Some(b)) = (small_index(start), small_index(stop)) else {
        return Err(Reason::WrongKind(if small_index(start).is_none() {
            first
        } else {
            first + 1
        }));
    };
    if a < 1 || b < a - 1 || b > len as i64 {
        return Err(Reason::OutOfRange(if a < 1 { first } else { first + 1 }));
    }
    Ok(a as usize - 1..b as usize)
}

fn find(haystack: &[u8], needle: &[u8], from: usize, last: bool) -> usize {
    if needle.is_empty() || needle.len() > haystack.len() {
        return 0;
    }
    let positions = 0..=haystack.len() - needle.len();
    let hit = |&i: &usize| haystack[i..i + needle.len()] == *needle;
    let found = if last {
        positions.filter(|&i| i <= from).rev().find(hit)
    } else {
        positions.filter(|&i| i >= from).find(hit)
    };
    found.map_or(0, |i| i + 1)
}

/// The number of arguments a selector takes.
fn selector_arity(selector: &[u8]) -> usize {
    match selector.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == b'_' => {
            selector.iter().filter(|&&b| b == b':').count()
        }
        Some(_) => 1,
        None => 0,
    }
}

fn activate_closure(receiver: Value, args: Vec<Value>, ctx: &NativeContext<'_>) -> Outcome {
    match ctx.heap.closure(receiver) {
        Some(c) if c.num_args as usize == args.len() => Outcome::ActivateClosure {
            closure: receiver,
            args,
        },
        Some(_) => Outcome::Fail(Reason::Arity),
        None => Outcome::Fail(Reason::WrongKind(0)),
    }
}

/// Runs a native. `args` excludes the receiver and has the native's arity.
pub fn invoke(
    native: Native,
    receiver: Value,
    args: &[Value],
    ctx: &mut NativeContext<'_>,
) -> Outcome {
    debug_assert_eq!(args.len(), native.arity());
    let arg = |i: usize| args[i];
    match native {
        Native::IntegerAdd => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::add, "addFromInteger:")
        }
        Native::IntegerSubtract => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::sub, "subtractFromInteger:")
        }
        Native::IntegerMultiply => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::mul, "multiplyFromInteger:")
        }
        Native::IntegerLess => comparison(ctx, receiver, arg(0), Ordering::is_lt, ">"),
        Native::IntegerGreater => comparison(ctx, receiver, arg(0), Ordering::is_gt, "<"),
        Native::IntegerLessEqual => comparison(ctx, receiver, arg(0), Ordering::is_le, ">="),
        Native::IntegerGreaterEqual => comparison(ctx, receiver, arg(0), Ordering::is_ge, "<="),
        Native::NumberEqual => {
            let Some(a) = number_of(ctx.heap, receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            let equal = number_of(ctx.heap, arg(0))
                .is_some_and(|b| compare(&a, &b) == Some(Ordering::Equal));
            Outcome::Answer(ctx.boolean(equal))
        }
        Native::NumberAsString => match number_of(ctx.heap, receiver) {
            Some(Num::Int(n)) => Outcome::Answer(ctx.new_string(&n.to_decimal())),
            Some(Num::Float(f)) => Outcome::Answer(ctx.new_string(&float_to_string(f))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::NumberFloorDivide => division(ctx, receiver, arg(0), Division::FloorDivide),
        Native::NumberFloorModulo => division(ctx, receiver, arg(0), Division::FloorModulo),
        Native::NumberQuo => division(ctx, receiver, arg(0), Division::Quo),
        Native::NumberRem => division(ctx, receiver, arg(0), Division::Rem),
        Native::IntegerBitAnd => integer_bits(ctx, receiver, arg(0), BigInt::and),
        Native::IntegerBitOr => integer_bits(ctx, receiver, arg(0), BigInt::or),
        Native::IntegerBitXor => integer_bits(ctx, receiver, arg(0), BigInt::xor),
        Native::IntegerShiftLeft => shift(ctx, receiver, arg(0), true),
        Native::IntegerShiftRight => shift(ctx, receiver, arg(0), false),
        Native::NumberAsFloat => match number_of(ctx.heap, receiver) {
            Some(n) => Outcome::Answer(ctx.float(as_f64(&n))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::NumberAsInteger => match number_of(ctx.heap, receiver) {
            Some(Num::Int(_)) => Outcome::Answer(receiver),
            Some(Num::Float(_)) => float_to_integer(ctx, receiver, libm::trunc),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::ObjectIdentical => Outcome::Answer(ctx.boolean(receiver == arg(0))),
        Native::ObjectClass => Outcome::Answer(match ctx.heap.get(receiver) {
            Some(o) => o.header.class,
            None => ctx.roles.get(Role::SmallIntegerClass),
        }),
        Native::ObjectIdentityHash => match ctx.heap.get(receiver) {
            // eq:kernel-identity-hash with h the identity on SmallIntegers.
            Some(o) => Outcome::Answer(small((o.header.seq & ((1 << 62) - 1)) as i64)),
            None => Outcome::Answer(receiver),
        },
        Native::StringConcatenate => {
            let (Some(a), Some(b)) = (
                ctx.heap.string_bytes(receiver),
                ctx.heap.string_bytes(arg(0)),
            ) else {
                return Outcome::Fail(Reason::WrongKind(
                    if ctx.heap.string_bytes(receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let mut joined = a.to_vec();
            joined.extend_from_slice(b);
            Outcome::Answer(ctx.new_bytes_object(Role::StringClass, Body::String(joined)))
        }
        Native::StringSize => match ctx.heap.string_bytes(receiver) {
            Some(b) => Outcome::Answer(small(b.len() as i64)),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::VmPrint => match ctx.heap.string_bytes(arg(0)) {
            Some(b) => {
                let text = String::from_utf8_lossy(b).into_owned();
                ctx.host.print(&text);
                Outcome::Answer(receiver)
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmExit => match arg(0)
            .as_small_integer()
            .and_then(|n| i32::try_from(n).ok())
        {
            Some(code) => Outcome::Exit(code),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureValue
        | Native::ClosureValue1
        | Native::ClosureValue2
        | Native::ClosureValue3 => activate_closure(receiver, args.to_vec(), ctx),
        Native::ClosureValueWithArguments => match ctx.heap.array_elements(arg(0)) {
            Some(elements) => {
                let elements = elements.to_vec();
                activate_closure(receiver, elements, ctx)
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureNumArgs => match ctx.heap.closure(receiver) {
            Some(c) => Outcome::Answer(small(i64::from(c.num_args))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::FloatAdd => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a + b, "addFromFloat:", false)
        }
        Native::FloatSubtract => float_arithmetic(
            ctx,
            receiver,
            arg(0),
            |a, b| a - b,
            "subtractFromFloat:",
            false,
        ),
        Native::FloatMultiply => float_arithmetic(
            ctx,
            receiver,
            arg(0),
            |a, b| a * b,
            "multiplyFromFloat:",
            false,
        ),
        Native::FloatDivide => float_arithmetic(
            ctx,
            receiver,
            arg(0),
            |a, b| a / b,
            "divideFromFloat:",
            true,
        ),
        Native::FloatFloor => float_to_integer(ctx, receiver, libm::floor),
        Native::FloatCeiling => float_to_integer(ctx, receiver, libm::ceil),
        Native::FloatRounded => float_to_integer(ctx, receiver, libm::round),
        Native::FloatSqrt => float_unary(ctx, receiver, libm::sqrt),
        Native::FloatSin => float_unary(ctx, receiver, libm::sin),
        Native::FloatCos => float_unary(ctx, receiver, libm::cos),
        Native::FloatTan => float_unary(ctx, receiver, libm::tan),
        Native::FloatExp => float_unary(ctx, receiver, libm::exp),
        Native::FloatLn => float_unary(ctx, receiver, libm::log),
        Native::FloatLog => float_unary(ctx, receiver, libm::log10),
        Native::FloatArcSin => float_unary(ctx, receiver, libm::asin),
        Native::FloatArcCos => float_unary(ctx, receiver, libm::acos),
        Native::FloatArcTan => float_unary(ctx, receiver, libm::atan),
        Native::FloatPow | Native::FloatArcTan2 => {
            let Some(x) = float_of(ctx.heap, receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            let Some(y) = number_of(ctx.heap, arg(0)).map(|n| as_f64(&n)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let r = if native == Native::FloatPow {
                libm::pow(x, y)
            } else {
                libm::atan2(x, y)
            };
            Outcome::Answer(ctx.float(r))
        }
        Native::FloatAsStringFixed => {
            float_format(ctx, receiver, arg(0), |x, d| format!("{x:.d$}"))
        }
        Native::FloatAsStringExponential => {
            float_format(ctx, receiver, arg(0), |x, d| format!("{x:.d$e}"))
        }
        Native::FloatAsStringPrecision => float_format(ctx, receiver, arg(0), |x, d| {
            let d = d.max(1);
            let e = format!("{x:.p$e}", p = d - 1);
            let exponent: i32 = e
                .rsplit('e')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if exponent < -6 || exponent >= d as i32 {
                e
            } else {
                let decimals = (d as i32 - 1 - exponent).max(0) as usize;
                format!("{x:.decimals$}")
            }
        }),
        Native::FloatParse => {
            let Some(b) = ctx.heap.string_bytes(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match core::str::from_utf8(b)
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
            {
                Some(f) => Outcome::Answer(ctx.float(f)),
                None => Outcome::Fail(Reason::WrongKind(1)),
            }
        }
        Native::ClassBasicNew => {
            let special = [
                Role::ArrayClass,
                Role::WeakArrayClass,
                Role::ByteArrayClass,
                Role::StringClass,
                Role::FloatClass,
                Role::LargeIntegerClass,
                Role::SmallIntegerClass,
                Role::ClosureClass,
                Role::ActivationClass,
                Role::EphemeronClass,
            ];
            if special.iter().any(|&r| ctx.roles.get(r) == receiver) {
                return Outcome::Fail(Reason::NotInstantiable);
            }
            match instance_slots(ctx.heap, ctx.roles, receiver) {
                Some(n) => {
                    let nil = ctx.nil();
                    Outcome::Answer(ctx.heap.allocate(
                        receiver,
                        Space::Actor(0),
                        Body::Regular(alloc::vec![nil; n]),
                    ))
                }
                None => Outcome::Fail(Reason::NotInstantiable),
            }
        }
        Native::IndexedNew => {
            let Some(n) = small_index(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            if !(0..=(1 << 28)).contains(&n) {
                return Outcome::Fail(if n < 0 {
                    Reason::OutOfRange(1)
                } else {
                    Reason::Exhausted(Resource::Memory)
                });
            }
            let n = n as usize;
            let nil = ctx.nil();
            let body = if receiver == ctx.roles.get(Role::ArrayClass) {
                Body::Array(alloc::vec![nil; n])
            } else if receiver == ctx.roles.get(Role::WeakArrayClass) {
                Body::WeakArray(alloc::vec![nil; n])
            } else if receiver == ctx.roles.get(Role::ByteArrayClass) {
                Body::Bytes(alloc::vec![0; n])
            } else {
                return Outcome::Fail(Reason::NotInstantiable);
            };
            Outcome::Answer(ctx.heap.allocate(receiver, Space::Actor(0), body))
        }
        Native::IndexedAt => {
            let result = match ctx.heap.get(receiver).map(|o| &o.body) {
                Some(Body::Array(e)) | Some(Body::WeakArray(e)) => {
                    index(arg(0), e.len()).map(|i| e[i])
                }
                Some(Body::Bytes(b)) | Some(Body::String(b)) => {
                    index(arg(0), b.len()).map(|i| small(i64::from(b[i])))
                }
                _ => Err(Reason::WrongKind(0)),
            };
            match result {
                Ok(v) => Outcome::Answer(v),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::IndexedAtPut => {
            let value = arg(1);
            let Some(o) = ctx.heap.get_mut(receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            if o.header.flags & FROZEN != 0 {
                return Outcome::Fail(Reason::WrongKind(0));
            }
            let result = match &mut o.body {
                Body::Array(e) | Body::WeakArray(e) => index(arg(0), e.len()).map(|i| e[i] = value),
                Body::Bytes(b) => match value.as_small_integer() {
                    Some(byte @ 0..=255) => index(arg(0), b.len()).map(|i| b[i] = byte as u8),
                    Some(_) => Err(Reason::OutOfRange(2)),
                    None => Err(Reason::WrongKind(2)),
                },
                _ => Err(Reason::WrongKind(0)),
            };
            match result {
                Ok(()) => Outcome::Answer(value),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::IndexedSize => match ctx.heap.get(receiver).map(|o| &o.body) {
            Some(Body::Array(e)) | Some(Body::WeakArray(e)) => {
                Outcome::Answer(small(e.len() as i64))
            }
            Some(Body::Bytes(b)) | Some(Body::String(b)) => Outcome::Answer(small(b.len() as i64)),
            _ => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::CopyFromTo | Native::CopyStringFromTo | Native::CopyByteArrayFromTo => {
            let class = ctx.heap.get(receiver).map(|o| o.header.class);
            let copy = match ctx.heap.get(receiver).map(|o| &o.body) {
                Some(Body::Array(e)) if native == Native::CopyFromTo => {
                    range(arg(0), arg(1), e.len(), 1).map(|r| Body::Array(e[r].to_vec()))
                }
                Some(Body::Bytes(b)) | Some(Body::String(b)) => range(arg(0), arg(1), b.len(), 1)
                    .map(|r| {
                        let bytes = b[r].to_vec();
                        match native {
                            Native::CopyStringFromTo => Body::String(bytes),
                            Native::CopyByteArrayFromTo => Body::Bytes(bytes),
                            _ => match ctx.heap.get(receiver).map(|o| &o.body) {
                                Some(Body::String(_)) => Body::String(bytes),
                                _ => Body::Bytes(bytes),
                            },
                        }
                    }),
                _ => Err(Reason::WrongKind(0)),
            };
            match copy {
                Ok(body) => {
                    let class = match (&body, native) {
                        (Body::String(_), Native::CopyStringFromTo) => {
                            ctx.roles.get(Role::StringClass)
                        }
                        (Body::Bytes(_), Native::CopyByteArrayFromTo) => {
                            ctx.roles.get(Role::ByteArrayClass)
                        }
                        _ => class.expect("receiver"),
                    };
                    Outcome::Answer(ctx.heap.allocate(class, Space::Actor(0), body))
                }
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::ReplaceFromToWithStartingAt => {
            let (start, stop, source, source_start) = (arg(0), arg(1), arg(2), arg(3));
            enum Src {
                Values(Vec<Value>),
                Bytes(Vec<u8>),
            }
            let src = match ctx.heap.get(source).map(|o| &o.body) {
                Some(Body::Array(e)) | Some(Body::WeakArray(e)) => Src::Values(e.clone()),
                Some(Body::Bytes(b)) | Some(Body::String(b)) => Src::Bytes(b.clone()),
                _ => return Outcome::Fail(Reason::WrongKind(3)),
            };
            let Some(o) = ctx.heap.get_mut(receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            if o.header.flags & FROZEN != 0 {
                return Outcome::Fail(Reason::WrongKind(0));
            }
            let src_len = match &src {
                Src::Values(v) => v.len(),
                Src::Bytes(b) => b.len(),
            };
            let result = (|| -> Result<(), Reason> {
                let len = match &o.body {
                    Body::Array(e) | Body::WeakArray(e) => e.len(),
                    Body::Bytes(b) => b.len(),
                    _ => return Err(Reason::WrongKind(0)),
                };
                let target = range(start, stop, len, 1)?;
                let Some(s0) = small_index(source_start) else {
                    return Err(Reason::WrongKind(4));
                };
                if s0 < 1 || (s0 as u64 - 1) + target.len() as u64 > src_len as u64 {
                    return Err(Reason::OutOfRange(4));
                }
                let s0 = s0 as usize - 1;
                match (&mut o.body, &src) {
                    (Body::Array(e) | Body::WeakArray(e), Src::Values(v)) => {
                        e[target.clone()].copy_from_slice(&v[s0..s0 + target.len()]);
                    }
                    (Body::Bytes(b), Src::Bytes(s)) => {
                        b[target.clone()].copy_from_slice(&s[s0..s0 + target.len()]);
                    }
                    _ => return Err(Reason::WrongKind(3)),
                }
                Ok(())
            })();
            match result {
                Ok(()) => Outcome::Answer(receiver),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::ByteArrayWithAll | Native::StringWithAll => match bytes_of(ctx.heap, arg(0)) {
            Some(b) => {
                let b = b.to_vec();
                Outcome::Answer(if native == Native::StringWithAll {
                    ctx.new_bytes_object(Role::StringClass, Body::String(b))
                } else {
                    ctx.new_bytes_object(Role::ByteArrayClass, Body::Bytes(b))
                })
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::StringWith => match arg(0).as_small_integer() {
            Some(byte @ 0..=255) => Outcome::Answer(
                ctx.new_bytes_object(Role::StringClass, Body::String(alloc::vec![byte as u8])),
            ),
            Some(_) => Outcome::Fail(Reason::OutOfRange(1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::StringHash => match ctx.heap.string_bytes(receiver) {
            Some(b) => {
                // FNV-1a, 64-bit, folded into the SmallInteger range.
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for &byte in b {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                Outcome::Answer(small((h >> 2) as i64))
            }
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::StringEquals => match ctx.heap.string_bytes(receiver) {
            Some(a) => {
                let equal = ctx.heap.string_bytes(arg(0)) == Some(a);
                Outcome::Answer(ctx.boolean(equal))
            }
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::StartsWith | Native::EndsWith => {
            let (Some(a), Some(b)) = (bytes_of(ctx.heap, receiver), bytes_of(ctx.heap, arg(0)))
            else {
                return Outcome::Fail(Reason::WrongKind(
                    if bytes_of(ctx.heap, receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let r = if native == Native::StartsWith {
                a.starts_with(b)
            } else {
                a.ends_with(b)
            };
            Outcome::Answer(ctx.boolean(r))
        }
        Native::IndexOfStartingAt | Native::LastIndexOfStartingAt => {
            let (Some(a), Some(b)) = (bytes_of(ctx.heap, receiver), bytes_of(ctx.heap, arg(0)))
            else {
                return Outcome::Fail(Reason::WrongKind(
                    if bytes_of(ctx.heap, receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let Some(from) = small_index(arg(1)) else {
                return Outcome::Fail(Reason::WrongKind(2));
            };
            if from < 1 || from > a.len() as i64 + 1 {
                return Outcome::Fail(Reason::OutOfRange(2));
            }
            let i = find(
                a,
                b,
                from as usize - 1,
                native == Native::LastIndexOfStartingAt,
            );
            Outcome::Answer(small(i as i64))
        }
        Native::VmSnapshot => Outcome::Snapshot,
        Native::VmWriteFile => {
            let (Some(bytes), Some(path)) =
                (bytes_of(ctx.heap, arg(0)), ctx.heap.string_bytes(arg(1)))
            else {
                return Outcome::Fail(Reason::WrongKind(if bytes_of(ctx.heap, arg(0)).is_none() {
                    1
                } else {
                    2
                }));
            };
            let (bytes, path) = (bytes.to_vec(), String::from_utf8_lossy(path).into_owned());
            match ctx.host.write_file(&path, &bytes) {
                Ok(()) => Outcome::Answer(receiver),
                Err(message) => {
                    let m = ctx.new_string(&message);
                    Outcome::Fail(Reason::Host(m))
                }
            }
        }
        Native::VmReadFile => {
            let Some(path) = ctx.heap.string_bytes(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let path = String::from_utf8_lossy(path).into_owned();
            match ctx.host.read_file(&path) {
                Ok(bytes) => {
                    Outcome::Answer(ctx.new_bytes_object(Role::ByteArrayClass, Body::Bytes(bytes)))
                }
                Err(message) => {
                    let m = ctx.new_string(&message);
                    Outcome::Fail(Reason::Host(m))
                }
            }
        }
        Native::VmSetRole => {
            let role = arg(0)
                .as_small_integer()
                .and_then(|id| u64::try_from(id).ok())
                .and_then(Role::from_id);
            match role {
                Some(role) => Outcome::SetRole {
                    role,
                    value: arg(1),
                },
                None => Outcome::Fail(Reason::OutOfRange(1)),
            }
        }
        Native::ObjectPerformWithArguments => {
            let selector = arg(0);
            let Some(sel) = ctx.heap.string_bytes(selector) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let arity = selector_arity(sel);
            let Some(arguments) = ctx.heap.array_elements(arg(1)) else {
                return Outcome::Fail(Reason::WrongKind(2));
            };
            if arguments.len() != arity {
                return Outcome::Fail(Reason::Arity);
            }
            Outcome::Perform {
                receiver,
                selector,
                args: arguments.to_vec(),
            }
        }
    }
}

/// A description of every catalog entry, for documentation and the
/// packager's natives table: (id, name, arity).
pub fn catalog() -> Vec<(u32, &'static str, usize)> {
    Native::ALL
        .iter()
        .map(|&n| (n as u32, n.name(), n.arity()))
        .collect()
}

/// The decimal string of an integer value, if it is one (for tests and
/// diagnostics).
pub fn integer_to_string(heap: &Heap, v: Value) -> Option<String> {
    integer_of(heap, v).map(|n| n.to_decimal())
}
