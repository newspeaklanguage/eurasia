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
use crate::object::{Behavior, Body, FROZEN, Heap, Space, VALUE, VALUE_CLASS};
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
    FloatDivide = 43 / 1: "`Float>>/`; otherwise `divideFromNumber:`, as psoup's `Number>>/`.",
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
    CurrentActivation = 100 / 0: "The activation of the frame that sent this message.",
    ActivationSender = 101 / 0: "`Activation>>sender`: the frame below, or nil.",
    ActivationSetSender = 102 / 1: "`Activation>>sender:`, restricted to cutting back to a live frame below.",
    ActivationMethod = 103 / 0: "`Activation>>method`.",
    ActivationReceiver = 104 / 0: "`Activation>>receiver`.",
    ActivationClosure = 105 / 0: "`Activation>>closure`: nil for a method activation.",
    ActivationTempAt = 106 / 1: "`Activation>>tempAt:`: arguments, then locals, then operands.",
    ActivationTempAtPut = 107 / 2: "`Activation>>tempAt:put:`.",
    ActivationSize = 108 / 0: "`Activation>>size`: the number of temps and operands.",
    ActivationBci = 109 / 0: "`Activation>>bci`: one-based, or nil once dead.",
    DefiningActivationOf = 110 / 1: "The activation that created a closure.",
    VmSlotOfAt = 111 / 2: "`vm slotOf:at:`: a slot of a regular object or ephemeron, or a contract field (format order).",
    VmSlotOfAtPut = 112 / 3: "`vm slotOf:at:put:`: regular objects and ephemerons only, not frozen.",
    VmClassOf = 113 / 1: "`vm classOf:`: the real class, whatever the object answers to `class`.",
    VmIsIdenticalTo = 114 / 2: "`vm is:identicalTo:`.",
    VmIdentityHashOf = 115 / 1: "`vm identityHashOf:`.",
    VmInstantiate = 116 / 1: "`vm instantiate:`: `basicNew` of the argument.",
    VmIsCanonical = 117 / 1: "`vm isCanonical:`: whether the object is frozen.",
    VmSetCanonical = 118 / 1: "`vm setCanonical:`: freeze the object.",
    VmPerformWith = 119 / 3: "`vm object:perform:with:`: an ordinary send, whatever the receiver overrides.",
    VmGarbageCollect = 120 / 0: "`vm garbageCollect`: a full collection; answers the receiver.",
    VmMonotonicNanos = 121 / 0: "`vm monotonicNanos` (input).",
    VmIsValue = 125 / 1: "`vm isValue:`: whether the object is a value (immediates, nil, booleans, strings, numbers, instances of value classes, classes with a value enclosing object).",
    SlotStore = 124 / 1: "Not a native: the failure of a slot setter, for a frozen receiver or a non-value stored into a value.",
    VmFinishTurn = 123 / 1: "`MessageLoop>>finish:`: ends the turn; the argument is the next timer's monotonic time in nanoseconds, or 0 for none.",
    VmApplyMixin = 122 / 3: "`vm applyMixin:superclass:enclosingObject:`: a new class and its metaclass.",
}

/// `vm applyMixin: m superclass: s enclosingObject: eo` (operation list,
/// section 6; formal semantics, mixin application): a new class applying
/// the instance-side mixin `m` to `s`, named after `m`, and its metaclass
/// applying `m`'s class-side mixin (a contract field of `m`). The metaclass's superclass is that of `s`'s metaclass and its class
/// is `s`'s metaclass's class, so the new pair follows the image's own
/// convention (in the genesis kernel, every metaclass's superclass is
/// `Class`). Nothing is installed: the pair is ordinary heap objects, and
/// instances get their layout from the mixins along the chain.
fn apply_mixin(m: Value, s: Value, eo: Value, ctx: &mut NativeContext) -> Outcome {
    // An instance-side mixin: one whose class-side mixin is a mixin.
    let Some((name, cm)) = ctx.heap.mixin(m).map(|x| (x.name, x.class_mixin)) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    if ctx.heap.mixin(cm).is_none() {
        return Outcome::Fail(Reason::WrongKind(1));
    }
    let Some(Body::Class(_)) = ctx.heap.get(s).map(|o| &o.body) else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let super_meta = ctx.heap.get(s).expect("class").header.class;
    let Some(super_meta_object) = ctx.heap.get(super_meta) else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let metaclass_class = super_meta_object.header.class;
    let Body::Metaclass(meta_behavior) = &super_meta_object.body else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let meta_super = meta_behavior.superclass;
    // A value class: the superclass is one (its chain includes Value). Its
    // enclosing object must be a value, and its mixin may declare no mutable
    // slot (a setter other than the private init` ones) (design doc section
    // 5: the checks happen at mixin application until the spec lets the
    // compiler know value classes).
    let value_class = ctx.heap.get(s).expect("class").header.flags & VALUE_CLASS != 0
        || s == ctx.roles.get(Role::ValueClass);
    if value_class {
        if !ctx.is_value(eo) {
            return Outcome::Fail(Reason::WrongKind(3));
        }
        if has_mutable_slot(ctx.heap, m) {
            return Outcome::Fail(Reason::WrongKind(1));
        }
    }
    let class_flags =
        (if ctx.is_value(eo) { VALUE } else { 0 }) | (if value_class { VALUE_CLASS } else { 0 });
    let nil = ctx.nil();
    let metaclass = ctx.heap.allocate(
        metaclass_class,
        Space::Actor(0),
        Body::Metaclass(Behavior {
            mixin: cm,
            superclass: meta_super,
            enclosing_object: eo,
            name: nil,
            this_class: nil,
            library: nil,
        }),
    );
    let class = ctx.heap.allocate(
        metaclass,
        Space::Actor(0),
        Body::Class(Behavior {
            mixin: m,
            superclass: s,
            enclosing_object: eo,
            name,
            this_class: nil,
            library: nil,
        }),
    );
    if let Some(Body::Metaclass(b)) = ctx.heap.get_mut(metaclass).map(|o| &mut o.body) {
        b.this_class = class;
    }
    // A class (and its metaclass) whose enclosing object is a value is one.
    ctx.heap.get_mut(class).expect("class").header.flags |= class_flags;
    ctx.heap.get_mut(metaclass).expect("metaclass").header.flags |= class_flags & VALUE;
    Outcome::Answer(class)
}

/// Whether mixin `m` declares a mutable slot: it has a setter other than the
/// private `init` ones that initializers use, and the nested-class cache
/// setters (`` X`slot: ``), which memoize a class; that class is a value when
/// its enclosing object, the value being checked, is one.
fn has_mutable_slot(heap: &Heap, m: Value) -> bool {
    let Some(methods) = heap.mixin(m).and_then(|x| heap.array_elements(x.methods)) else {
        return false;
    };
    methods.iter().any(|&method| {
        heap.method(method).is_some_and(|record| {
            record.header.kind == crate::object::MethodKind::Setter
                && !heap
                    .string_bytes(record.selector)
                    .is_some_and(|s| s.starts_with(b"init`") || s.ends_with(b"`slot:"))
        })
    })
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
    /// Collect garbage now (`vm garbageCollect`), then answer the receiver.
    Collect,
    /// End the turn (`finish:`): the stack unwinds and the VM's event loop
    /// waits until the given monotonic time in nanoseconds (0: no timer).
    FinishTurn(u64),
    /// An activation native: the interpreter, which owns the frames, carries
    /// it out and answers or fails.
    Frame(Native),
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

    /// Whether `v` is a value: an immediate, nil, true, false, or an object
    /// with the value bit.
    pub fn is_value(&self, v: Value) -> bool {
        v == self.nil()
            || v == self.roles.get(Role::True)
            || v == self.roles.get(Role::False)
            || self.heap.is_value(v)
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
) -> Outcome {
    let Some(a) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match number_of(ctx.heap, arg) {
        Some(b) => {
            // IEEE 754: a zero divisor gives an infinity or NaN, as in psoup.
            Outcome::Answer(ctx.float(op(a, as_f64(&b))))
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
    let k = match arg.as_small_integer() {
        Some(k) => k,
        // A LargeInteger count: every bit shifts out to the right; to the
        // left the result could not be represented.
        None => match integer_of(ctx.heap, arg) {
            Some(big) if big.is_negative() => return Outcome::Fail(Reason::OutOfRange(1)),
            Some(_) => i64::MAX,
            None => return Outcome::Fail(Reason::WrongKind(1)),
        },
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

/// The shortest decimal that reads back as the same Float, printed as psoup
/// prints it (double-conversion's shortest form with JavaScript's limits):
/// decimal notation, with `.0` on integral values, when the decimal
/// exponent is from -6 to 20, otherwise `d.ddde+X`; and `NaN`, `Infinity`,
/// `-Infinity`.
pub fn float_to_string(f: f64) -> String {
    if f.is_nan() {
        return String::from("NaN");
    }
    if f.is_infinite() {
        return String::from(if f > 0.0 { "Infinity" } else { "-Infinity" });
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    // Rust's exponential form has the shortest round-tripping digits.
    let e = format!("{:e}", f.abs());
    let (mantissa, exponent) = e.split_once('e').expect("exponential form");
    let exponent: i32 = exponent.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let n = digits.len() as i32;
    if (-6..21).contains(&exponent) {
        if exponent < 0 {
            let zeros = "0".repeat((-exponent - 1) as usize);
            format!("{sign}0.{zeros}{digits}")
        } else if n <= exponent + 1 {
            let zeros = "0".repeat((exponent + 1 - n) as usize);
            format!("{sign}{digits}{zeros}.0")
        } else {
            let (int, frac) = digits.split_at(exponent as usize + 1);
            format!("{sign}{int}.{frac}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { "-" } else { "+" };
        format!(
            "{sign}{first}{point}{rest}e{exponent_sign}{}",
            exponent.abs()
        )
    }
}

/// `Float parse:`: decimal and exponential notation, `NaN`, `Infinity` and
/// `-Infinity`, as psoup accepts them; not Rust's other spellings
/// (`inf`, `nan`, any case).
fn parse_float(s: &str) -> Option<f64> {
    match s {
        "NaN" => return Some(f64::NAN),
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    if s.bytes()
        .any(|b| !(b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E')))
    {
        return None;
    }
    s.parse().ok()
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

/// `asStringFixed:`, `asStringExponential:` and `asStringPrecision:`, as
/// JavaScript's `toFixed`, `toExponential` and `toPrecision` (psoup's
/// behaviour): the digit count must be in `range`.
fn float_format(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    range: core::ops::RangeInclusive<i64>,
    large_is_shortest: bool,
    format: fn(f64, usize) -> String,
) -> Outcome {
    let Some(x) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    // Operation list 3.1: any unusable digit count is #outOfRange: 1.
    let Some(digits) = arg.as_small_integer().filter(|d| range.contains(d)) else {
        return Outcome::Fail(Reason::OutOfRange(1));
    };
    // JavaScript prints non-finite values, and toFixed values of 1e21 and
    // beyond, as their shortest form.
    if !x.is_finite() || (large_is_shortest && x.abs() >= 1e21) {
        let s = float_to_string(x);
        return Outcome::Answer(ctx.new_string(&s));
    }
    let s = format(x, digits as usize);
    Outcome::Answer(ctx.new_string(&s))
}

/// JavaScript's exponent: `3e0` becomes `3e+0`; `3e-1` stays.
fn signed_exponent(s: String) -> String {
    match s.find('e') {
        Some(i) if !s[i + 1..].starts_with('-') => format!("{}e+{}", &s[..i], &s[i + 1..]),
        _ => s,
    }
}

/// The bytes of a String or ByteArray.
fn bytes_of(heap: &Heap, v: Value) -> Option<&[u8]> {
    match &heap.get(v)?.body {
        Body::String(b) | Body::Bytes(b) => Some(b),
        _ => None,
    }
}

/// The fields `vm slotOf:at:` reads: a regular object's slots, an
/// ephemeron's key, value and finalizer, or a contract object's fields in the
/// snapshot format's edge order (format section 5).
pub(crate) fn readable_fields(body: &Body) -> Option<Vec<Value>> {
    Some(match body {
        Body::Regular(slots) => slots.clone(),
        Body::Ephemeron {
            key,
            value,
            finalizer,
        } => alloc::vec![*key, *value, *finalizer],
        Body::Class(b) => alloc::vec![b.mixin, b.superclass, b.enclosing_object, b.name, b.library],
        Body::Metaclass(b) => alloc::vec![
            b.mixin,
            b.superclass,
            b.enclosing_object,
            b.this_class,
            b.library
        ],
        Body::Mixin(m) => alloc::vec![
            m.name,
            m.methods,
            m.enclosing_mixin,
            m.slot_names,
            m.class_mixin,
            m.library
        ],
        Body::Method(m) => alloc::vec![m.literals, m.mixin, m.selector, m.source, m.library],
        _ => return None,
    })
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

/// The one-based position of `needle` in `haystack`, searching forward from,
/// or (`last`) backward from, the zero-based position `from`; 0 when absent.
/// An empty needle is found at `from` itself, as in psoup.
fn find(haystack: &[u8], needle: &[u8], from: usize, last: bool) -> usize {
    if needle.is_empty() {
        return from + 1;
    }
    if needle.len() > haystack.len() {
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
        Native::FloatAdd => float_arithmetic(ctx, receiver, arg(0), |a, b| a + b, "addFromFloat:"),
        Native::FloatSubtract => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a - b, "subtractFromFloat:")
        }
        Native::FloatMultiply => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a * b, "multiplyFromFloat:")
        }
        Native::FloatDivide => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a / b, "divideFromNumber:")
        }
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
        Native::FloatAsStringFixed => float_format(ctx, receiver, arg(0), 0..=20, true, |x, d| {
            format!("{x:.d$}")
        }),
        Native::FloatAsStringExponential => {
            float_format(ctx, receiver, arg(0), 0..=20, false, |x, d| {
                signed_exponent(format!("{x:.d$e}"))
            })
        }
        Native::FloatAsStringPrecision => {
            float_format(ctx, receiver, arg(0), 1..=21, false, |x, d| {
                let d = d.max(1);
                let e = format!("{x:.p$e}", p = d - 1);
                let exponent: i32 = e
                    .rsplit('e')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if exponent < -6 || exponent >= d as i32 {
                    signed_exponent(e)
                } else {
                    let decimals = (d as i32 - 1 - exponent).max(0) as usize;
                    format!("{x:.decimals$}")
                }
            })
        }
        Native::FloatParse => {
            let Some(b) = ctx.heap.string_bytes(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match core::str::from_utf8(b)
                .ok()
                .and_then(|s| parse_float(s.trim()))
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
            if receiver == ctx.roles.get(Role::EphemeronClass) {
                let nil = ctx.nil();
                return Outcome::Answer(ctx.heap.allocate(
                    receiver,
                    Space::Actor(0),
                    Body::Ephemeron {
                        key: nil,
                        value: nil,
                        finalizer: nil,
                    },
                ));
            }
            if special.iter().any(|&r| ctx.roles.get(r) == receiver) {
                return Outcome::Fail(Reason::NotInstantiable);
            }
            match instance_slots(ctx.heap, ctx.roles, receiver) {
                Some(n) => {
                    let nil = ctx.nil();
                    let instance = ctx.heap.allocate(
                        receiver,
                        Space::Actor(0),
                        Body::Regular(alloc::vec![nil; n]),
                    );
                    // An instance of a value class is a value from allocation;
                    // its init setters check what it is given (the end of
                    // construction is not marked yet: design doc section 5).
                    if ctx
                        .heap
                        .get(receiver)
                        .map(|o| o.header.flags & VALUE_CLASS != 0)
                        == Some(true)
                    {
                        ctx.heap.get_mut(instance).expect("new").header.flags |= VALUE;
                    }
                    Outcome::Answer(instance)
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
        Native::CurrentActivation
        | Native::ActivationSender
        | Native::ActivationSetSender
        | Native::ActivationMethod
        | Native::ActivationReceiver
        | Native::ActivationClosure
        | Native::ActivationTempAt
        | Native::ActivationTempAtPut
        | Native::ActivationSize
        | Native::ActivationBci => Outcome::Frame(native),
        Native::DefiningActivationOf => match ctx.heap.closure(arg(0)) {
            Some(c) => Outcome::Answer(c.defining),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
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
        Native::VmSlotOfAt => {
            let (object, i) = (arg(0), arg(1));
            match ctx.heap.get(object).and_then(|o| readable_fields(&o.body)) {
                Some(fields) => match index(i, fields.len()) {
                    Ok(k) => Outcome::Answer(fields[k]),
                    Err(Reason::WrongKind(_)) => Outcome::Fail(Reason::WrongKind(2)),
                    Err(_) => Outcome::Fail(Reason::OutOfRange(2)),
                },
                None => Outcome::Fail(Reason::WrongKind(1)),
            }
        }
        Native::VmSlotOfAtPut => {
            let (object, i, value) = (arg(0), arg(1), arg(2));
            let Some(o) = ctx.heap.get_mut(object) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            if o.header.flags & (FROZEN | VALUE) != 0 {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            let slot = match &mut o.body {
                Body::Regular(slots) => index(i, slots.len()).map(|k| &mut slots[k]),
                Body::Ephemeron {
                    key,
                    value: v,
                    finalizer,
                } => match i.as_small_integer() {
                    Some(1) => Ok(key),
                    Some(2) => Ok(v),
                    Some(3) => Ok(finalizer),
                    Some(_) => Err(Reason::OutOfRange(2)),
                    None => Err(Reason::WrongKind(2)),
                },
                _ => Err(Reason::WrongKind(1)),
            };
            match slot {
                Ok(s) => {
                    *s = value;
                    Outcome::Answer(value)
                }
                Err(Reason::WrongKind(1)) => Outcome::Fail(Reason::WrongKind(1)),
                Err(Reason::WrongKind(_)) => Outcome::Fail(Reason::WrongKind(2)),
                Err(_) => Outcome::Fail(Reason::OutOfRange(2)),
            }
        }
        Native::VmClassOf => Outcome::Answer(match ctx.heap.get(arg(0)) {
            Some(o) => o.header.class,
            None => ctx.roles.get(Role::SmallIntegerClass),
        }),
        Native::VmIsIdenticalTo => Outcome::Answer(ctx.boolean(arg(0) == arg(1))),
        Native::VmIdentityHashOf => match ctx.heap.get(arg(0)) {
            Some(o) => Outcome::Answer(small((o.header.seq & ((1 << 62) - 1)) as i64)),
            None => Outcome::Answer(arg(0)),
        },
        Native::VmInstantiate => invoke(Native::ClassBasicNew, arg(0), &[], ctx),
        Native::VmApplyMixin => apply_mixin(arg(0), arg(1), arg(2), ctx),
        Native::VmIsCanonical => {
            let frozen = match ctx.heap.get(arg(0)) {
                Some(o) => o.header.flags & FROZEN != 0,
                // Immediates are values.
                None => true,
            };
            Outcome::Answer(ctx.boolean(frozen))
        }
        Native::VmSetCanonical => {
            if let Some(o) = ctx.heap.get_mut(arg(0)) {
                o.header.flags |= FROZEN;
            }
            Outcome::Answer(arg(0))
        }
        Native::VmPerformWith => {
            match invoke(
                Native::ObjectPerformWithArguments,
                arg(0),
                &[arg(1), arg(2)],
                ctx,
            ) {
                // Shift argument numbers: the object is argument 1 here.
                Outcome::Fail(Reason::WrongKind(i)) => Outcome::Fail(Reason::WrongKind(i + 1)),
                other => other,
            }
        }
        Native::VmGarbageCollect => Outcome::Collect,
        Native::VmIsValue => Outcome::Answer(ctx.boolean(ctx.is_value(arg(0)))),
        // Never installed: the interpreter reports refused slot stores with it.
        Native::SlotStore => Outcome::Fail(Reason::WrongKind(0)),
        Native::VmFinishTurn => match arg(0).as_small_integer() {
            Some(t) if t >= 0 => Outcome::FinishTurn(t as u64),
            Some(_) => Outcome::Fail(Reason::OutOfRange(1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmMonotonicNanos => {
            let t = ctx.host.monotonic_nanos();
            Outcome::Answer(ctx.integer(BigInt::from_i128(i128::from(t))))
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
