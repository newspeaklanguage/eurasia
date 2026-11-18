// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! M1 first light: small images built in Rust, run by the interpreter.

use eurasia_core::builder::{ImageBuilder, MethodSpec};
use eurasia_core::bytecode::{Assembler, SendKind, op};
use eurasia_core::host::Host;
use eurasia_core::natives::Native;
use eurasia_core::object::{Access, Body, Space};
use eurasia_core::roles::Role;
use eurasia_core::{Completion, Value, Vm, VmError};

#[derive(Default)]
struct TestHost {
    printed: Vec<String>,
}

impl Host for TestHost {
    fn print(&mut self, text: &str) {
        self.printed.push(text.to_string());
    }
    fn monotonic_nanos(&mut self) -> u64 {
        0
    }
    fn utc_nanos(&mut self) -> i64 {
        0
    }
    fn random_bytes(&mut self, buffer: &mut [u8]) {
        buffer.fill(0);
    }
    fn description(&self) -> &str {
        "test"
    }
}

/// A kernel slice: SmallInteger and String natives, a VM mirror with
/// print: and exit:, a failure class whose #signal answers the reason name,
/// and a doesNotUnderstand: on Object that answers the message's selector.
struct Kernel {
    b: ImageBuilder,
    entry_class: Value,
}

fn kernel() -> Kernel {
    let mut b = ImageBuilder::new();
    let object = b.object;
    let small_integer = b.role(Role::SmallIntegerClass).unwrap();
    let string = b.role(Role::StringClass).unwrap();
    for (selector, native) in [
        ("+", Native::IntegerAdd),
        ("-", Native::IntegerSubtract),
        ("*", Native::IntegerMultiply),
        ("<", Native::IntegerLess),
        (">", Native::IntegerGreater),
        ("=", Native::NumberEqual),
        ("asString", Native::NumberAsString),
    ] {
        b.method(
            small_integer,
            selector,
            Access::Public,
            MethodSpec::Native(native),
        );
    }
    b.method(
        string,
        ",",
        Access::Public,
        MethodSpec::Native(Native::StringConcatenate),
    );
    b.method(
        string,
        "size",
        Access::Public,
        MethodSpec::Native(Native::StringSize),
    );

    let mirror_class = b.define_class("VMMirror", object);
    b.method(
        mirror_class,
        "print:",
        Access::Public,
        MethodSpec::Native(Native::VmPrint),
    );
    b.method(
        mirror_class,
        "exit:",
        Access::Public,
        MethodSpec::Native(Native::VmExit),
    );
    let mirror = b
        .heap
        .allocate(mirror_class, Space::Actor(0), Body::Regular(vec![]));
    b.set_role(Role::VmMirror, mirror);

    let failure = b.define_class("NativeFailure", object);
    // Slots: native name, receiver, arguments, reason name, reason argument.
    b.method(failure, "signal", Access::Public, MethodSpec::Getter(3));
    b.set_role(Role::FailureClass, failure);

    let message_class = b.role(Role::MessageClass).unwrap();
    b.method(
        message_class,
        "selector",
        Access::Public,
        MethodSpec::Getter(0),
    );
    let dnu_code = {
        let mut a = Assembler::new();
        a.push_parameter(1)
            .send(SendKind::Ordinary, 0, 0)
            .return_top();
        a.finish()
    };
    let selector_literal = b.string("selector");
    b.method(
        object,
        "doesNotUnderstand:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 1,
            locals: 0,
            code: dnu_code,
            literals: vec![selector_literal],
        },
    );

    let entry_class = b.define_class("Main", object);
    let entry = b
        .heap
        .allocate(entry_class, Space::Actor(0), Body::Regular(vec![]));
    b.set_role(Role::Entry, entry);
    Kernel { b, entry_class }
}

/// Defines `main: vm args: args` on the entry class and runs it.
fn run_main(
    mut k: Kernel,
    locals: u32,
    code: Vec<u8>,
    literals: &[&str],
) -> (Result<Completion, VmError>, Vec<String>) {
    let lits: Vec<_> = literals.iter().map(|s| k.b.string(s)).collect();
    k.b.method(
        k.entry_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals,
            code,
            literals: lits,
        },
    );
    let (heap, roles) = k.b.finish();
    let mut vm = Vm::new(heap, roles);
    let mut host = TestHost::default();
    let result = vm.run(&mut host, &[]);
    (result, host.printed)
}

/// `vm` is the first parameter of main:args:, which is parameter 2.
const VM: u8 = 2;

#[test]
fn three_plus_four_prints_seven() {
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .push_integer(3)
        .push_integer(4)
        .common_send(0) // +
        .send(SendKind::Ordinary, 1, 0) // asString
        .send(SendKind::Ordinary, 0, 1) // print:
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(kernel(), 0, a.finish(), &["print:", "asString"]);
    assert!(matches!(result, Ok(Completion::Returned(_))), "{result:?}");
    assert_eq!(printed, ["7"]);
}

#[test]
fn a_native_failure_signals_through_the_failure_class() {
    // vm print: ('abc' , 3)  — `,` fails with wrongKind 1; the failure's
    // #signal answers the reason name, which is printed.
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .push_literal(1)
        .push_integer(3)
        .common_send(27) // ,
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(kernel(), 0, a.finish(), &["print:", "abc"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["wrongKind"]);
}

#[test]
fn integer_plus_non_number_tail_sends_and_reaches_dnu() {
    // vm print: (3 + nil)  — the native tail-sends #addFromInteger: to nil,
    // which nil does not understand; Object>>doesNotUnderstand: answers the
    // selector.
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .push_integer(3)
        .byte(op::PUSH_NIL)
        .common_send(0)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(kernel(), 0, a.finish(), &["print:"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["addFromInteger:"]);
}

#[test]
fn exit_ends_the_run_with_its_code() {
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .push_integer(3)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(kernel(), 0, a.finish(), &["exit:"]);
    assert_eq!(result, Ok(Completion::Exited(3)));
    assert!(printed.is_empty());
}

#[test]
fn self_sends_locals_and_branches() {
    // max: a with: b = ( a > b ifTrue: [^a]. ^b )   (inlined branch)
    // main: t := self max: 7 with: 11. vm print: t asString
    let mut k = kernel();
    let mut m = Assembler::new();
    // parameters: a is 2, b is 1
    m.push_parameter(2).push_parameter(1).common_send(10); // >
    m.byte(op::BRANCH_IF_FALSE).byte(2).byte(0); // skip the 2-byte return below
    m.push_parameter(2).return_top();
    m.push_parameter(1).return_top();
    k.b.method(
        k.entry_class,
        "max:with:",
        Access::Private,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: m.finish(),
            literals: vec![],
        },
    );
    let mut a = Assembler::new();
    a.push_integer(7)
        .push_integer(2)
        .common_send(2) // 7 * 2 = 14
        .push_integer(2)
        .push_integer(2)
        .common_send(0) // 4
        .send(SendKind::SelfSend, 2, 2) // max:with:
        .pop_into_local(0)
        .push_parameter(VM)
        .push_local(0)
        .send(SendKind::Ordinary, 1, 0) // asString
        .send(SendKind::Ordinary, 0, 1) // print:
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(k, 1, a.finish(), &["print:", "asString", "max:with:"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["14"]);
}

#[test]
fn ordinary_sends_do_not_see_protected_or_private_methods() {
    let mut k = kernel();
    let secret = {
        let mut a = Assembler::new();
        a.push_integer(1).return_top();
        a.finish()
    };
    k.b.method(
        k.entry_class,
        "secret",
        Access::Protected,
        MethodSpec::Bytecode {
            args: 0,
            locals: 0,
            code: secret,
            literals: vec![],
        },
    );
    // vm print: self secret  — as an ordinary send to self, it reaches DNU.
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .byte(op::PUSH_SELF)
        .send(SendKind::Ordinary, 1, 0)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(k, 0, a.finish(), &["print:", "secret"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["secret"]);
}

#[test]
fn self_sends_see_protected_methods() {
    let mut k = kernel();
    let secret = {
        let mut a = Assembler::new();
        a.push_integer(1).return_top();
        a.finish()
    };
    k.b.method(
        k.entry_class,
        "secret",
        Access::Protected,
        MethodSpec::Bytecode {
            args: 0,
            locals: 0,
            code: secret,
            literals: vec![],
        },
    );
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .send(SendKind::SelfSend, 1, 0)
        .send(SendKind::Ordinary, 2, 0) // asString
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(k, 0, a.finish(), &["print:", "secret", "asString"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["1"]);
}

#[test]
fn invalid_bytecode_is_an_error_not_a_crash() {
    let (result, _) = run_main(kernel(), 0, vec![157], &[]);
    assert!(
        matches!(result, Err(VmError::InvalidBytecode { .. })),
        "{result:?}"
    );
    // Running off the end of a method.
    let (result, _) = run_main(kernel(), 0, vec![op::PUSH_NIL], &[]);
    assert!(
        matches!(result, Err(VmError::InvalidBytecode { .. })),
        "{result:?}"
    );
    // A parameter index beyond the method's arguments.
    let (result, _) = run_main(kernel(), 0, vec![115, op::RETURN_TOP], &[]);
    assert!(
        matches!(result, Err(VmError::InvalidBytecode { .. })),
        "{result:?}"
    );
}

#[test]
fn identity_hash_is_the_allocation_sequence_number() {
    let k = kernel();
    let (heap, _) = k.b.finish();
    let mut seqs: Vec<u64> = heap.iter().map(|(_, o)| o.header.seq).collect();
    let sorted = {
        let mut s = seqs.clone();
        s.sort_unstable();
        s
    };
    assert_eq!(seqs, sorted);
    seqs.dedup();
    assert_eq!(seqs.len(), heap.len());
    let _ = Value::small_integer(0);
}

#[test]
fn accessor_slots_are_relative_to_their_mixin() {
    // A declares one slot; B (a subclass) declares one. B's getter for its
    // slot 0 reads absolute slot 1 of a B instance.
    let mut k = kernel();
    let object = k.b.object;
    let a = k.b.define_class("A", object);
    k.b.set_slot_count(a, 1);
    k.b.method(a, "first", Access::Public, MethodSpec::Getter(0));
    let b = k.b.define_class("B", a);
    k.b.set_slot_count(b, 1);
    k.b.method(b, "second", Access::Public, MethodSpec::Getter(0));
    k.b.method(b, "second:", Access::Public, MethodSpec::Setter(0));
    let ten = Value::small_integer(10).unwrap();
    let twenty = Value::small_integer(20).unwrap();
    let instance =
        k.b.heap
            .allocate(b, Space::Actor(0), Body::Regular(vec![ten, twenty]));
    // main: vm print: (instance second: (instance first + instance second)) second asString
    let mut m = Assembler::new();
    m.push_parameter(VM)
        .push_literal(4) // instance
        .push_literal(4)
        .send(SendKind::Ordinary, 1, 0) // first
        .push_literal(4)
        .send(SendKind::Ordinary, 2, 0) // second
        .common_send(0) // +
        .send(SendKind::Ordinary, 3, 1) // second:
        .send(SendKind::Ordinary, 2, 0) // second
        .send(SendKind::Ordinary, 5, 0) // asString
        .send(SendKind::Ordinary, 0, 1) // print:
        .byte(op::RETURN_NIL);
    let mut lits: Vec<Value> = ["print:", "first", "second", "second:"]
        .iter()
        .map(|s| k.b.string(s))
        .collect();
    lits.push(instance);
    lits.push(k.b.string("asString"));
    k.b.method(
        k.entry_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: m.finish(),
            literals: lits,
        },
    );
    let (heap, roles) = k.b.finish();
    let mut vm = Vm::new(heap, roles);
    let mut host = TestHost::default();
    let result = vm.run(&mut host, &[]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(host.printed, ["30"]);
}
