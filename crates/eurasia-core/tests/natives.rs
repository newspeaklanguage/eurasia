// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The native catalog, called directly: each native's answers, tail sends
//! and failures.

use eurasia_core::Value;
use eurasia_core::builder::ImageBuilder;
use eurasia_core::host::Host;
use eurasia_core::natives::{self, Native, NativeContext, Outcome, Reason, Resource};
use eurasia_core::object::{Body, FROZEN, Heap, Space};
use eurasia_core::roles::{Role, Roles};

struct NoHost;

impl Host for NoHost {
    fn print(&mut self, _: &str) {}
    fn monotonic_nanos(&mut self) -> u64 {
        0
    }
    fn utc_nanos(&mut self) -> i64 {
        0
    }
    fn random_bytes(&mut self, b: &mut [u8]) {
        b.fill(0);
    }
    fn description(&self) -> &str {
        "none"
    }
}

struct World {
    heap: Heap,
    roles: Roles,
}

fn world() -> World {
    let (heap, roles) = ImageBuilder::new().finish();
    World { heap, roles }
}

impl World {
    fn call(&mut self, native: Native, receiver: Value, args: &[Value]) -> Outcome {
        let mut host = NoHost;
        let mut ctx = NativeContext {
            heap: &mut self.heap,
            roles: &self.roles,
            host: &mut host,
            space: Space::Actor(0),
        };
        natives::invoke(native, receiver, args, &mut ctx)
    }

    fn answer(&mut self, native: Native, receiver: Value, args: &[Value]) -> Value {
        match self.call(native, receiver, args) {
            Outcome::Answer(v) => v,
            other => panic!("{native:?}: expected an answer, got {other:?}"),
        }
    }

    fn fails(&mut self, native: Native, receiver: Value, args: &[Value]) -> Reason {
        match self.call(native, receiver, args) {
            Outcome::Fail(r) => r,
            other => panic!("{native:?}: expected a failure, got {other:?}"),
        }
    }

    /// The printed form of a number (via the NumberAsString native).
    fn show(&mut self, v: Value) -> String {
        let s = self.answer(Native::NumberAsString, v, &[]);
        String::from_utf8(self.heap.string_bytes(s).unwrap().to_vec()).unwrap()
    }

    fn string(&mut self, s: &str) -> Value {
        let class = self.roles.get(Role::StringClass);
        self.heap
            .allocate(class, Space::Actor(0), Body::String(s.as_bytes().to_vec()))
    }

    fn text(&self, v: Value) -> String {
        String::from_utf8(self.heap.string_bytes(v).unwrap().to_vec()).unwrap()
    }

    fn float(&mut self, f: f64) -> Value {
        let class = self.roles.get(Role::FloatClass);
        self.heap.allocate(class, Space::Actor(0), Body::Float(f))
    }

    fn truth(&self, v: Value) -> bool {
        if v == self.roles.get(Role::True) {
            true
        } else if v == self.roles.get(Role::False) {
            false
        } else {
            panic!("not a Boolean: {v:?}")
        }
    }

    /// An integer from decimal text (built with SmallInteger natives).
    fn int(&mut self, n: i128) -> Value {
        if let Some(v) = i64::try_from(n).ok().and_then(Value::small_integer) {
            return v;
        }
        // n = hi * 2^62 + lo
        let base = Value::small_integer(1 << 30).unwrap();
        let mut acc = Value::small_integer(0).unwrap();
        let negative = n < 0;
        let digits: Vec<i64> = {
            let mut m = n.unsigned_abs();
            let mut v = Vec::new();
            while m > 0 {
                v.push((m % (1 << 30)) as i64);
                m /= 1 << 30;
            }
            v
        };
        for &d in digits.iter().rev() {
            acc = self.answer(Native::IntegerMultiply, acc, &[base]);
            acc = self.answer(Native::IntegerAdd, acc, &[Value::small_integer(d).unwrap()]);
        }
        if negative {
            acc = self.answer(
                Native::IntegerSubtract,
                Value::small_integer(0).unwrap(),
                &[acc],
            );
        }
        acc
    }
}

fn si(n: i64) -> Value {
    Value::small_integer(n).unwrap()
}

#[test]
fn small_integer_overflow_becomes_a_large_integer_and_back() {
    let mut w = world();
    let max = si(Value::MAX_SMALL_INTEGER);
    let big = w.answer(Native::IntegerAdd, max, &[si(1)]);
    assert!(big.as_small_integer().is_none());
    assert_eq!(w.show(big), "4611686018427387904");
    let back = w.answer(Native::IntegerSubtract, big, &[si(1)]);
    assert_eq!(back, max, "results that fit are SmallIntegers again");
    let square = w.answer(Native::IntegerMultiply, big, &[big]);
    assert_eq!(w.show(square), "21267647932558653966460912964485513216");
    let neg = w.int(-170141183460469231731687303715884105728);
    assert_eq!(w.show(neg), "-170141183460469231731687303715884105728");
}

#[test]
fn integer_division_flavours_and_zero() {
    let mut w = world();
    let cases = [
        (Native::NumberFloorDivide, -7, 2, -4),
        (Native::NumberFloorModulo, -7, 2, 1),
        (Native::NumberQuo, -7, 2, -3),
        (Native::NumberRem, -7, 2, -1),
        (Native::NumberFloorDivide, 7, -2, -4),
        (Native::NumberFloorModulo, 7, -2, -1),
    ];
    for (native, a, b, expected) in cases {
        assert_eq!(
            w.answer(native, si(a), &[si(b)]),
            si(expected),
            "{native:?} {a} {b}"
        );
    }
    let big = w.int(1 << 100);
    let q = w.answer(Native::NumberFloorDivide, big, &[si(3)]);
    assert_eq!(w.show(q), "422550200076076467165567735125");
    let r = w.answer(Native::NumberFloorModulo, big, &[si(3)]);
    assert_eq!(r, si(1));
    assert_eq!(
        w.fails(Native::NumberFloorDivide, si(1), &[si(0)]),
        Reason::ZeroDivide
    );
    assert_eq!(
        w.fails(Native::NumberRem, big, &[si(0)]),
        Reason::ZeroDivide
    );
    let nil = w.roles.get(Role::Nil);
    assert_eq!(
        w.fails(Native::NumberQuo, si(1), &[nil]),
        Reason::WrongKind(1)
    );
}

#[test]
fn bit_operations_are_infinite_twos_complement() {
    let mut w = world();
    assert_eq!(w.answer(Native::IntegerBitAnd, si(-1), &[si(255)]), si(255));
    assert_eq!(
        w.answer(Native::IntegerBitOr, si(-256), &[si(15)]),
        si(-241)
    );
    assert_eq!(w.answer(Native::IntegerBitXor, si(-1), &[si(5)]), si(-6));
    let shifted = w.answer(Native::IntegerShiftLeft, si(1), &[si(100)]);
    assert_eq!(w.show(shifted), "1267650600228229401496703205376");
    assert_eq!(
        w.answer(Native::IntegerShiftRight, shifted, &[si(99)]),
        si(2)
    );
    assert_eq!(
        w.answer(Native::IntegerShiftRight, si(-5), &[si(1)]),
        si(-3)
    );
    assert_eq!(
        w.answer(Native::IntegerShiftRight, si(-5), &[si(1000)]),
        si(-1)
    );
    assert_eq!(
        w.fails(Native::IntegerShiftLeft, si(1), &[si(-1)]),
        Reason::OutOfRange(1)
    );
    assert_eq!(
        w.fails(Native::IntegerShiftLeft, si(1), &[si(1 << 40)]),
        Reason::Exhausted(Resource::Memory)
    );
    assert_eq!(
        w.answer(Native::IntegerShiftLeft, si(0), &[si(1 << 40)]),
        si(0)
    );
}

#[test]
fn integer_arithmetic_with_a_non_integer_tail_sends() {
    let mut w = world();
    let x = w.float(0.5);
    match w.call(Native::IntegerAdd, si(3), &[x]) {
        Outcome::TailSend {
            receiver,
            selector,
            args,
        } => {
            assert_eq!(
                (receiver, selector, args),
                (x, "addFromInteger:", vec![si(3)])
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn comparisons_mix_integers_and_floats_and_reverse_otherwise() {
    let mut w = world();
    let half = w.float(3.5);
    let v = w.answer(Native::IntegerLess, si(3), &[half]);
    assert!(w.truth(v));
    let big = w.int(1 << 70);
    let v = w.answer(Native::IntegerGreater, big, &[si(1)]);
    assert!(w.truth(v));
    let nil = w.roles.get(Role::Nil);
    match w.call(Native::IntegerLess, si(3), &[nil]) {
        Outcome::TailSend { selector, .. } => assert_eq!(selector, ">"),
        other => panic!("{other:?}"),
    }
    let three = w.float(3.0);
    let v = w.answer(Native::NumberEqual, si(3), &[three]);
    assert!(w.truth(v));
    let v = w.answer(Native::NumberEqual, si(3), &[nil]);
    assert!(!w.truth(v), "= with a non-number answers false");
    let nan = w.float(f64::NAN);
    let v = w.answer(Native::NumberEqual, nan, &[nan]);
    assert!(!w.truth(v), "NaN equals nothing");
}

#[test]
fn floats_are_libm_and_print_shortest() {
    let mut w = world();
    let a = w.float(0.1);
    let b = w.float(0.2);
    let sum = w.answer(Native::FloatAdd, a, &[b]);
    assert_eq!(w.show(sum), "0.30000000000000004");
    let one = w.float(1.0);
    assert_eq!(w.show(one), "1.0");
    let two = w.float(2.0);
    let root = w.answer(Native::FloatSqrt, two, &[]);
    assert_eq!(w.show(root), "1.4142135623730951");
    for (x, floor, ceiling, rounded) in [(2.5, 2, 3, 3), (-2.5, -3, -2, -3), (2.4, 2, 3, 2)] {
        let f = w.float(x);
        assert_eq!(w.answer(Native::FloatFloor, f, &[]), si(floor));
        assert_eq!(w.answer(Native::FloatCeiling, f, &[]), si(ceiling));
        assert_eq!(
            w.answer(Native::FloatRounded, f, &[]),
            si(rounded),
            "{x} rounded"
        );
    }
    // IEEE division (operation list 3.1; psoup agrees): no failure on zero.
    let zero = w.float(0.0);
    let infinity = w.answer(Native::FloatDivide, one, &[zero]);
    assert_eq!(w.show(infinity), "Infinity");
    let big = w.float(f64::MAX);
    assert_eq!(w.show(big), "1.7976931348623157e+308");
    let small = w.float(1.5e-7);
    assert_eq!(w.show(small), "1.5e-7");
    let tiny = w.float(1.5e-6);
    assert_eq!(w.show(tiny), "0.0000015");
    let e20 = w.float(1e20);
    assert_eq!(w.show(e20), "100000000000000000000.0");
    let e21 = w.float(1e21);
    assert_eq!(w.show(e21), "1e+21");
    let huge = w.float(1e20);
    let n = w.answer(Native::NumberAsInteger, huge, &[]);
    assert_eq!(w.show(n), "100000000000000000000");
    let nan = w.float(f64::NAN);
    assert_eq!(w.fails(Native::FloatFloor, nan, &[]), Reason::OutOfRange(0));
    let inf = w.float(f64::INFINITY);
    assert_eq!(w.show(inf), "Infinity");
    let s = w.string(" 1.5 ");
    let float_class = w.roles.get(Role::FloatClass);
    let parsed = w.answer(Native::FloatParse, float_class, &[s]);
    assert_eq!(w.show(parsed), "1.5");
    let pi = w.float(std::f64::consts::PI);
    let fixed = w.answer(Native::FloatAsStringFixed, pi, &[si(2)]);
    assert_eq!(w.text(fixed), "3.14");
    let precise = w.answer(Native::FloatAsStringPrecision, pi, &[si(4)]);
    assert_eq!(w.text(precise), "3.142");
}

#[test]
fn nan_results_are_canonical() {
    let mut w = world();
    let neg = w.float(-1.0);
    let nan = w.answer(Native::FloatSqrt, neg, &[]);
    match w.heap.get(nan).unwrap().body {
        Body::Float(f) => assert_eq!(f.to_bits(), f64::NAN.to_bits()),
        _ => panic!("not a float"),
    }
}

#[test]
fn allocation_and_indexed_access() {
    let mut w = world();
    let array_class = w.roles.get(Role::ArrayClass);
    let bytes_class = w.roles.get(Role::ByteArrayClass);
    let a = w.answer(Native::IndexedNew, array_class, &[si(3)]);
    assert_eq!(w.answer(Native::IndexedSize, a, &[]), si(3));
    assert_eq!(w.answer(Native::IndexedAtPut, a, &[si(2), si(42)]), si(42));
    assert_eq!(w.answer(Native::IndexedAt, a, &[si(2)]), si(42));
    assert_eq!(
        w.fails(Native::IndexedAt, a, &[si(0)]),
        Reason::OutOfRange(1)
    );
    assert_eq!(
        w.fails(Native::IndexedAt, a, &[si(4)]),
        Reason::OutOfRange(1)
    );
    let nil = w.roles.get(Role::Nil);
    assert_eq!(w.fails(Native::IndexedAt, a, &[nil]), Reason::WrongKind(1));
    let b = w.answer(Native::IndexedNew, bytes_class, &[si(2)]);
    assert_eq!(
        w.fails(Native::IndexedAtPut, b, &[si(1), si(256)]),
        Reason::OutOfRange(2)
    );
    assert_eq!(
        w.answer(Native::IndexedAtPut, b, &[si(1), si(255)]),
        si(255)
    );
    assert_eq!(
        w.fails(Native::IndexedNew, array_class, &[si(-1)]),
        Reason::OutOfRange(1)
    );
    assert_eq!(
        w.fails(Native::ClassBasicNew, array_class, &[]),
        Reason::NotInstantiable
    );
    w.heap.get_mut(a).unwrap().header.flags = FROZEN;
    assert_eq!(
        w.fails(Native::IndexedAtPut, a, &[si(1), si(1)]),
        Reason::WrongKind(0)
    );
}

#[test]
fn basic_new_sizes_instances_from_the_mixin_chain() {
    let mut b = ImageBuilder::new();
    let object = b.object;
    let a = b.define_class("A", object);
    b.set_slot_count(a, 2);
    let c = b.define_class("C", a);
    b.set_slot_count(c, 3);
    let (heap, roles) = b.finish();
    let mut w = World { heap, roles };
    let instance = w.answer(Native::ClassBasicNew, c, &[]);
    match &w.heap.get(instance).unwrap().body {
        Body::Regular(slots) => assert_eq!(slots.len(), 5),
        other => panic!("{other:?}"),
    }
}

#[test]
fn copying_and_replacing() {
    let mut w = world();
    let s = w.string("hello world");
    let part = w.answer(Native::CopyFromTo, s, &[si(7), si(11)]);
    assert_eq!(w.text(part), "world");
    let empty = w.answer(Native::CopyFromTo, s, &[si(3), si(2)]);
    assert_eq!(w.text(empty), "");
    assert_eq!(
        w.fails(Native::CopyFromTo, s, &[si(0), si(2)]),
        Reason::OutOfRange(1)
    );
    assert_eq!(
        w.fails(Native::CopyFromTo, s, &[si(1), si(12)]),
        Reason::OutOfRange(2)
    );
    let bytes = w.answer(Native::CopyByteArrayFromTo, s, &[si(1), si(5)]);
    assert_eq!(
        w.heap.get(bytes).unwrap().header.class,
        w.roles.get(Role::ByteArrayClass)
    );
    // Overlapping replace within one array.
    let array_class = w.roles.get(Role::ArrayClass);
    let a = w.answer(Native::IndexedNew, array_class, &[si(5)]);
    for i in 1..=5 {
        w.answer(Native::IndexedAtPut, a, &[si(i), si(i * 10)]);
    }
    w.answer(
        Native::ReplaceFromToWithStartingAt,
        a,
        &[si(2), si(5), a, si(1)],
    );
    let got: Vec<Value> = (1..=5)
        .map(|i| w.answer(Native::IndexedAt, a, &[si(i)]))
        .collect();
    assert_eq!(got, [si(10), si(10), si(20), si(30), si(40)]);
    assert_eq!(
        w.fails(
            Native::ReplaceFromToWithStartingAt,
            a,
            &[si(1), si(5), a, si(2)]
        ),
        Reason::OutOfRange(4)
    );
}

#[test]
fn string_natives() {
    let mut w = world();
    let s = w.string("abcabc");
    let bc = w.string("bc");
    assert_eq!(w.answer(Native::IndexOfStartingAt, s, &[bc, si(1)]), si(2));
    assert_eq!(w.answer(Native::IndexOfStartingAt, s, &[bc, si(3)]), si(5));
    assert_eq!(
        w.answer(Native::LastIndexOfStartingAt, s, &[bc, si(6)]),
        si(5)
    );
    assert_eq!(
        w.answer(Native::LastIndexOfStartingAt, s, &[bc, si(4)]),
        si(2)
    );
    let zz = w.string("zz");
    assert_eq!(w.answer(Native::IndexOfStartingAt, s, &[zz, si(1)]), si(0));
    let ab = w.string("ab");
    let v = w.answer(Native::StartsWith, s, &[ab]);
    assert!(w.truth(v));
    let v = w.answer(Native::EndsWith, s, &[bc]);
    assert!(w.truth(v));
    let same = w.string("abcabc");
    let v = w.answer(Native::StringEquals, s, &[same]);
    assert!(w.truth(v));
    let h1 = w.answer(Native::StringHash, s, &[]);
    let h2 = w.answer(Native::StringHash, same, &[]);
    assert_eq!(h1, h2, "equal strings hash equally");
    let string_class = w.roles.get(Role::StringClass);
    let a = w.answer(Native::StringWith, string_class, &[si(65)]);
    assert_eq!(w.text(a), "A");
}

#[test]
fn perform_checks_the_selector_arity() {
    let mut w = world();
    let plus = w.string("+");
    let array_class = w.roles.get(Role::ArrayClass);
    let one = w.answer(Native::IndexedNew, array_class, &[si(1)]);
    w.answer(Native::IndexedAtPut, one, &[si(1), si(4)]);
    match w.call(Native::ObjectPerformWithArguments, si(3), &[plus, one]) {
        Outcome::Perform {
            receiver,
            selector,
            args,
        } => {
            assert_eq!((receiver, selector, args), (si(3), plus, vec![si(4)]));
        }
        other => panic!("{other:?}"),
    }
    let none = w.answer(Native::IndexedNew, array_class, &[si(0)]);
    assert_eq!(
        w.fails(Native::ObjectPerformWithArguments, si(3), &[plus, none]),
        Reason::Arity
    );
    let at_put = w.string("at:put:");
    assert_eq!(
        w.fails(Native::ObjectPerformWithArguments, si(3), &[at_put, one]),
        Reason::Arity
    );
}

#[test]
fn every_catalog_entry_has_a_unique_id() {
    let catalog = natives::catalog();
    let mut ids: Vec<u32> = catalog.iter().map(|&(id, _, _)| id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), catalog.len());
    for &(id, _, arity) in &catalog {
        assert_eq!(Native::from_id(id).unwrap().arity(), arity);
    }
}

#[test]
fn mirror_slot_access_reads_contract_fields_and_writes_only_regular_objects() {
    let mut b = ImageBuilder::new();
    let object = b.object;
    let a = b.define_class("A", object);
    b.set_slot_count(a, 2);
    let (heap, roles) = b.finish();
    let mut w = World { heap, roles };
    let vm = w.roles.get(Role::Nil); // the receiver is not consulted
    // A class's contract fields: mixin, superclass, enclosing, name, library.
    assert_eq!(w.answer(Native::VmSlotOfAt, vm, &[a, si(2)]), object);
    let name = w.answer(Native::VmSlotOfAt, vm, &[a, si(4)]);
    assert_eq!(w.text(name), "A");
    assert_eq!(
        w.fails(Native::VmSlotOfAt, vm, &[a, si(6)]),
        Reason::OutOfRange(2)
    );
    assert_eq!(
        w.fails(Native::VmSlotOfAtPut, vm, &[a, si(1), si(0)]),
        Reason::WrongKind(1)
    );
    // A regular instance.
    let instance = w.answer(Native::VmInstantiate, vm, &[a]);
    assert_eq!(
        w.answer(Native::VmSlotOfAtPut, vm, &[instance, si(2), si(9)]),
        si(9)
    );
    assert_eq!(w.answer(Native::VmSlotOfAt, vm, &[instance, si(2)]), si(9));
    assert_eq!(w.answer(Native::VmClassOf, vm, &[instance]), a);
    let v = w.answer(Native::VmIsCanonical, vm, &[instance]);
    assert!(!w.truth(v));
    w.answer(Native::VmSetCanonical, vm, &[instance]);
    let v = w.answer(Native::VmIsCanonical, vm, &[instance]);
    assert!(w.truth(v));
    assert_eq!(
        w.fails(Native::VmSlotOfAtPut, vm, &[instance, si(1), si(1)]),
        Reason::WrongKind(1)
    );
    let v = w.answer(Native::VmIsCanonical, vm, &[si(3)]);
    assert!(w.truth(v), "immediates are values");
}

#[test]
fn mirror_perform_numbers_its_arguments_from_the_object() {
    let mut w = world();
    let vm = w.roles.get(Role::Nil);
    let plus = w.string("+");
    let nil = w.roles.get(Role::Nil);
    assert_eq!(
        w.fails(Native::VmPerformWith, vm, &[si(3), plus, nil]),
        Reason::WrongKind(3)
    );
    let array_class = w.roles.get(Role::ArrayClass);
    let args = w.answer(Native::IndexedNew, array_class, &[si(1)]);
    assert!(matches!(
        w.call(Native::VmPerformWith, vm, &[si(3), plus, args]),
        Outcome::Perform { .. }
    ));
}
