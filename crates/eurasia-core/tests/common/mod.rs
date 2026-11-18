// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Shared test kernel and helpers for the interpreter's integration tests.
#![allow(dead_code)]

use eurasia_core::builder::{ImageBuilder, MethodSpec};
use eurasia_core::bytecode::{Assembler, SendKind};
use eurasia_core::host::Host;
use eurasia_core::natives::Native;
use eurasia_core::object::{Access, Body, Space};
use eurasia_core::roles::Role;
use eurasia_core::{Completion, Value, Vm, VmError};

#[derive(Default)]
pub struct TestHost {
    pub printed: Vec<String>,
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
pub struct Kernel {
    pub b: ImageBuilder,
    pub entry_class: Value,
}

pub fn kernel() -> Kernel {
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
    let closure = b.role(Role::ClosureClass).unwrap();
    for (selector, native) in [
        ("value", Native::ClosureValue),
        ("value:", Native::ClosureValue1),
        ("value:value:", Native::ClosureValue2),
        ("valueWithArguments:", Native::ClosureValueWithArguments),
        ("numArgs", Native::ClosureNumArgs),
    ] {
        b.method(
            closure,
            selector,
            Access::Public,
            MethodSpec::Native(native),
        );
    }

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
pub fn run_main(
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
pub const VM: u8 = 2;
