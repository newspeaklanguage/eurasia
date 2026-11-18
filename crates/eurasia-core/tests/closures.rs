// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! M2: closures, non-local return, and the VM upcalls
//! #cannotReturn:, #aboutToReturn:through: and #nonBooleanReceiver:.

mod common;

use common::*;
use eurasia_core::builder::MethodSpec;
use eurasia_core::bytecode::{Assembler, SendKind, op};
use eurasia_core::natives::Native;
use eurasia_core::object::Access;
use eurasia_core::roles::Role;
use eurasia_core::{Completion, Vm};

/// Common-selector indices.
const PLUS: u8 = 0;
const TIMES: u8 = 2;
const EQUAL: u8 = 13;
const VALUE: u8 = 19;
const VALUE1: u8 = 20;

fn code(f: impl FnOnce(&mut Assembler)) -> Vec<u8> {
    let mut a = Assembler::new();
    f(&mut a);
    a.finish()
}

/// main: prints (expression) asString, where `expression` leaves a value on
/// the stack above the VM mirror. Literals: 0 print:, 1 asString, then extra.
fn print_main(
    k: Kernel,
    locals: u32,
    extra: &[&str],
    expression: impl FnOnce(&mut Assembler),
) -> (Result<Completion, eurasia_core::VmError>, Vec<String>) {
    let body = code(|a| {
        a.push_parameter(VM);
        expression(a);
        a.send(SendKind::Ordinary, 1, 0)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let mut literals = vec!["print:", "asString"];
    literals.extend_from_slice(extra);
    run_main(k, locals, body, &literals)
}

/// Gives Activation an `exit:`-style handler for `selector`: it exits with
/// `code`, so a test can see that the upcall happened.
fn exit_on_upcall(k: &mut Kernel, selector: &str, args: u32, code_value: i16) {
    let activation = k.b.role(Role::ActivationClass).unwrap();
    let mirror = k.b.role(Role::VmMirror).unwrap();
    let exit = k.b.string("exit:");
    let body = code(|a| {
        a.push_literal(0)
            .push_integer(code_value)
            .send(SendKind::Ordinary, 1, 1)
            .return_top();
    });
    k.b.method(
        activation,
        selector,
        Access::Public,
        MethodSpec::Bytecode {
            args,
            locals: 0,
            code: body,
            literals: vec![mirror, exit],
        },
    );
}

#[test]
fn a_closure_answers_its_body() {
    // [3 + 4] value
    let body = code(|a| {
        a.push_integer(3)
            .push_integer(4)
            .common_send(PLUS)
            .return_top();
    });
    let (result, printed) = print_main(kernel(), 0, &[], |a| {
        a.push_closure(0, 0, &body).common_send(VALUE);
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["7"]);
}

#[test]
fn copied_values_and_parameters() {
    // | x | x := 10. [:y | x + y] value: 5
    let body = code(|a| {
        a.push_local(0)
            .push_parameter(1)
            .common_send(PLUS)
            .return_top();
    });
    let (result, printed) = print_main(kernel(), 1, &[], |a| {
        a.push_integer(10)
            .pop_into_local(0)
            .push_local(0)
            .push_closure(1, 1, &body)
            .push_integer(5)
            .common_send(VALUE1);
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["15"]);
}

#[test]
fn wrong_arity_fails_through_the_failure_class() {
    // [:y | y] value  — the failure's #signal answers 'arity'.
    let body = code(|a| {
        a.push_parameter(1).return_top();
    });
    let main = code(|a| {
        a.push_parameter(VM)
            .push_closure(0, 1, &body)
            .common_send(VALUE)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, printed) = run_main(kernel(), 0, main, &["print:"]);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["arity"]);
}

/// Adds `run: aBlock = ( aBlock value. ^nil )`, an intermediate frame.
fn add_run(k: &mut Kernel) {
    let body = code(|a| {
        a.push_parameter(1)
            .common_send(VALUE)
            .byte(op::POP)
            .byte(op::RETURN_NIL);
    });
    k.b.method(
        k.entry_class,
        "run:",
        Access::Protected,
        MethodSpec::Bytecode {
            args: 1,
            locals: 0,
            code: body,
            literals: vec![],
        },
    );
}

#[test]
fn non_local_return_leaves_the_home_method_through_other_frames() {
    // find = ( self run: [^6 * 7]. ^0 )   main prints self find.
    let mut k = kernel();
    add_run(&mut k);
    let block = code(|a| {
        a.push_integer(6)
            .push_integer(7)
            .common_send(TIMES)
            .non_local_return_top();
    });
    let find = code(|a| {
        a.push_closure(0, 0, &block)
            .send(SendKind::SelfSend, 0, 1)
            .byte(op::POP)
            .push_integer(0)
            .return_top();
    });
    let run = k.b.string("run:");
    k.b.method(
        k.entry_class,
        "find",
        Access::Protected,
        MethodSpec::Bytecode {
            args: 0,
            locals: 0,
            code: find,
            literals: vec![run],
        },
    );
    let (result, printed) = print_main(k, 0, &["find"], |a| {
        a.send(SendKind::SelfSend, 2, 0);
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["42"]);
}

#[test]
fn non_local_return_to_a_dead_home_sends_cannot_return() {
    // escaper = ( ^[^1] )   main: self escaper value  — the home is dead.
    let mut k = kernel();
    exit_on_upcall(&mut k, "cannotReturn:", 1, 42);
    let block = code(|a| {
        a.push_integer(1).non_local_return_top();
    });
    let escaper = code(|a| {
        a.push_closure(0, 0, &block).return_top();
    });
    k.b.method(
        k.entry_class,
        "escaper",
        Access::Protected,
        MethodSpec::Bytecode {
            args: 0,
            locals: 0,
            code: escaper,
            literals: vec![],
        },
    );
    let main = code(|a| {
        a.send(SendKind::SelfSend, 0, 0)
            .common_send(VALUE)
            .byte(op::RETURN_NIL);
    });
    let (result, _) = run_main(k, 0, main, &["escaper"]);
    assert_eq!(result, Ok(Completion::Exited(42)));
}

#[test]
fn non_local_return_through_an_ensure_frame_sends_about_to_return() {
    // Closure>>ensure: is the EnsureMethod role (here: ^self value).
    // main: [^5] ensure: nil  — the return crosses the ensure: frame.
    let mut k = kernel();
    exit_on_upcall(&mut k, "aboutToReturn:through:", 2, 9);
    let closure_class = k.b.role(Role::ClosureClass).unwrap();
    let ensure_body = code(|a| {
        a.byte(op::PUSH_SELF).common_send(VALUE).return_top();
    });
    let ensure = k.b.method(
        closure_class,
        "ensure:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 1,
            locals: 0,
            code: ensure_body,
            literals: vec![],
        },
    );
    k.b.set_role(Role::EnsureMethod, ensure);
    let block = code(|a| {
        a.push_integer(5).non_local_return_top();
    });
    let main = code(|a| {
        a.push_closure(0, 0, &block)
            .byte(op::PUSH_NIL)
            .send(SendKind::Ordinary, 0, 1)
            .byte(op::RETURN_NIL);
    });
    let (result, _) = run_main(k, 0, main, &["ensure:"]);
    assert_eq!(result, Ok(Completion::Exited(9)));
}

#[test]
fn a_non_boolean_condition_sends_non_boolean_receiver() {
    let mut k = kernel();
    exit_on_upcall(&mut k, "nonBooleanReceiver:", 1, 5);
    let main = code(|a| {
        a.push_integer(3).branch_if_false(0).byte(op::RETURN_NIL);
    });
    let (result, _) = run_main(k, 0, main, &[]);
    assert_eq!(result, Ok(Completion::Exited(5)));
}

#[test]
fn a_missing_upcall_method_is_a_vm_error_not_a_crash() {
    let main = code(|a| {
        a.push_integer(3).branch_if_false(0).byte(op::RETURN_NIL);
    });
    let (result, _) = run_main(kernel(), 0, main, &[]);
    assert!(
        matches!(result, Err(eurasia_core::VmError::MissingUpcall(_))),
        "{result:?}"
    );
}

#[test]
fn closures_compare_by_identity_and_report_their_arity() {
    // [:a :b | a] numArgs
    let body = code(|a| {
        a.push_parameter(2).return_top();
    });
    let (result, printed) = print_main(kernel(), 0, &["numArgs"], |a| {
        a.push_closure(0, 2, &body).send(SendKind::Ordinary, 2, 0);
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(printed, ["2"]);
    let _ = (EQUAL, Native::ClosureNumArgs, Vm::set_step_limit);
}
