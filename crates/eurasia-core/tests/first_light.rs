// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! M1 first light: small images built in Rust, run by the interpreter.

mod common;

use common::*;
use eurasia_core::builder::MethodSpec;
use eurasia_core::bytecode::{Assembler, SendKind, op};
use eurasia_core::object::{Access, Body, Space};
use eurasia_core::{Completion, Value, Vm, VmError};

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

#[test]
fn perform_with_arguments_is_an_ordinary_send() {
    // vm print: (3 perform: '+' withArguments: {4}) asString
    let mut a = Assembler::new();
    a.push_parameter(VM)
        .push_integer(3)
        .push_literal(2)
        .push_integer(4)
        .byte(op::PUSH_NEW_ARRAY_WITH_ELEMENTS)
        .byte(1)
        .send(SendKind::Ordinary, 3, 2)
        .send(SendKind::Ordinary, 1, 0)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    let (result, printed) = run_main(
        kernel(),
        0,
        a.finish(),
        &["print:", "asString", "+", "perform:withArguments:"],
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["7"]);
}
