// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Activation natives over live frames (genesis plan, section 2): what
//! psoup's exception library uses, notably returning to an earlier frame by
//! rewriting the current frame's sender.

mod common;

use common::*;
use eurasia_core::Value;
use eurasia_core::builder::MethodSpec;
use eurasia_core::bytecode::{Assembler, SendKind, op};
use eurasia_core::natives::Native;
use eurasia_core::object::Access;
use eurasia_core::roles::Role;

fn code(f: impl FnOnce(&mut Assembler)) -> Vec<u8> {
    let mut a = Assembler::new();
    f(&mut a);
    a.finish()
}

/// The test kernel plus `Object>>thisActivation` and the Activation natives.
fn activation_kernel() -> Kernel {
    let mut k = kernel();
    let object = k.b.object;
    k.b.method(
        object,
        "thisActivation",
        Access::Public,
        MethodSpec::Native(Native::CurrentActivation),
    );
    let activation = k.b.role(Role::ActivationClass).unwrap();
    for (selector, native) in [
        ("sender", Native::ActivationSender),
        ("sender:", Native::ActivationSetSender),
        ("method", Native::ActivationMethod),
        ("receiver", Native::ActivationReceiver),
        ("closure", Native::ActivationClosure),
        ("tempAt:", Native::ActivationTempAt),
        ("tempAt:put:", Native::ActivationTempAtPut),
        ("size", Native::ActivationSize),
        ("bci", Native::ActivationBci),
    ] {
        k.b.method(
            activation,
            selector,
            Access::Public,
            MethodSpec::Native(native),
        );
    }
    k
}

fn method(
    k: &mut Kernel,
    selector: &str,
    args: u32,
    locals: u32,
    literals: &[&str],
    body: Vec<u8>,
) {
    let lits: Vec<Value> = literals.iter().map(|s| k.b.string(s)).collect();
    k.b.method(
        k.entry_class,
        selector,
        Access::Protected,
        MethodSpec::Bytecode {
            args,
            locals,
            code: body,
            literals: lits,
        },
    );
}

/// main prints `self <selector>` asString.
fn print_self_send(
    k: Kernel,
    selector: &str,
) -> (
    Result<eurasia_core::Completion, eurasia_core::VmError>,
    Vec<String>,
) {
    let body = code(|a| {
        a.push_parameter(VM)
            .send(SendKind::SelfSend, 2, 0)
            .send(SendKind::Ordinary, 1, 0)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    run_main(k, 0, body, &["print:", "asString", selector])
}

#[test]
fn cutting_back_returns_to_an_earlier_frame() {
    // outer  = ( ^self middle + 1 )
    // middle = ( self inner. ^0 )
    // inner  = ( thisActivation sender: thisActivation sender sender. ^41 )
    // inner's return skips middle and lands in outer: 41 + 1 = 42.
    let mut k = activation_kernel();
    method(
        &mut k,
        "outer",
        0,
        0,
        &["middle"],
        code(|a| {
            a.send(SendKind::SelfSend, 0, 0)
                .push_integer(1)
                .common_send(0)
                .return_top();
        }),
    );
    method(
        &mut k,
        "middle",
        0,
        0,
        &["inner"],
        code(|a| {
            a.send(SendKind::SelfSend, 0, 0)
                .byte(op::POP)
                .push_integer(0)
                .return_top();
        }),
    );
    method(
        &mut k,
        "inner",
        0,
        0,
        &["thisActivation", "sender", "sender:"],
        code(|a| {
            a.send(SendKind::SelfSend, 0, 0)
                .send(SendKind::SelfSend, 0, 0)
                .send(SendKind::Ordinary, 1, 0)
                .send(SendKind::Ordinary, 1, 0)
                .send(SendKind::Ordinary, 2, 1)
                .byte(op::POP)
                .push_integer(41)
                .return_top();
        }),
    );
    let (result, printed) = print_self_send(k, "outer");
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["42"]);
}

#[test]
fn temps_are_arguments_then_locals_then_operands() {
    // probe: x = ( | l | l := 5. thisActivation tempAt: 2 put: 7.
    //   ^(thisActivation tempAt: 1) + l + (thisActivation size) )
    // tempAt: 1 is x (10), tempAt: 2 is l (now 7). When size runs, the
    // native's own receiver is off the stack, so the frame holds x, l and
    // the operand 17: size 3.
    let mut k = activation_kernel();
    method(
        &mut k,
        "probe:",
        1,
        1,
        &["thisActivation", "tempAt:put:", "tempAt:", "size"],
        code(|a| {
            a.push_integer(5).pop_into_local(0);
            a.send(SendKind::SelfSend, 0, 0)
                .push_integer(2)
                .push_integer(7)
                .send(SendKind::Ordinary, 1, 2)
                .byte(op::POP);
            a.send(SendKind::SelfSend, 0, 0)
                .push_integer(1)
                .send(SendKind::Ordinary, 2, 1)
                .push_local(0)
                .common_send(0)
                .send(SendKind::SelfSend, 0, 0)
                .send(SendKind::Ordinary, 3, 0)
                .common_send(0)
                .return_top();
        }),
    );
    let body = code(|a| {
        a.push_parameter(VM)
            .push_integer(10)
            .send(SendKind::SelfSend, 2, 1)
            .send(SendKind::Ordinary, 1, 0)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, printed) = run_main(k, 0, body, &["print:", "asString", "probe:"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["20"], "10 + 7 + 3 (x, l, the operand 17)");
}

#[test]
fn a_returned_activation_is_dead() {
    // escapee = ( ^thisActivation )   main prints (self escapee tempAt: 1):
    // a dead activation fails with dead, and the failure's #signal answers
    // the reason name.
    let mut k = activation_kernel();
    method(
        &mut k,
        "escapee",
        0,
        0,
        &["thisActivation"],
        code(|a| {
            a.send(SendKind::SelfSend, 0, 0).return_top();
        }),
    );
    let body = code(|a| {
        a.push_parameter(VM)
            .send(SendKind::SelfSend, 1, 0)
            .push_integer(1)
            .send(SendKind::Ordinary, 2, 1)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, printed) = run_main(k, 0, body, &["print:", "escapee", "tempAt:"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["dead"]);
}

#[test]
fn sender_can_only_cut_back_to_a_live_frame_below() {
    // bad = ( ^thisActivation sender: thisActivation )  — not below itself.
    let mut k = activation_kernel();
    method(
        &mut k,
        "bad",
        0,
        0,
        &["thisActivation", "sender:"],
        code(|a| {
            a.send(SendKind::SelfSend, 0, 0)
                .send(SendKind::SelfSend, 0, 0)
                .send(SendKind::Ordinary, 1, 1)
                .return_top();
        }),
    );
    let body = code(|a| {
        a.push_parameter(VM)
            .send(SendKind::SelfSend, 1, 0)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, printed) = run_main(k, 0, body, &["print:", "bad"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["wrongKind"]);
}

#[test]
fn the_bottom_frame_has_no_sender() {
    // main: vm print: (thisActivation sender == nil) printString... we print
    // via a selector answering 'bottom' when the sender is nil.
    let mut k = activation_kernel();
    let undefined = k.b.heap.class(k.b.nil).unwrap();
    let lit = k.b.string("bottom");
    k.b.method(
        undefined,
        "describeSender",
        Access::Public,
        MethodSpec::Bytecode {
            args: 0,
            locals: 0,
            code: code(|a| {
                a.push_literal(0).return_top();
            }),
            literals: vec![lit],
        },
    );
    let body = code(|a| {
        a.push_parameter(VM)
            .send(SendKind::SelfSend, 1, 0)
            .send(SendKind::Ordinary, 2, 0)
            .send(SendKind::Ordinary, 3, 0)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, printed) = run_main(
        k,
        0,
        body,
        &["print:", "thisActivation", "sender", "describeSender"],
    );
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["bottom"]);
}
