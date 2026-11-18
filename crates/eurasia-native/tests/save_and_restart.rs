// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! A program saves its own heap; the saved image, run again, continues from
//! the saved state. This is what genesis relies on (implementation plan,
//! "genesis, then full snapshots").

use std::process::Command;

use eurasia_core::Value;
use eurasia_core::builder::{ImageBuilder, MethodSpec};
use eurasia_core::bytecode::{Assembler, SendKind, op};
use eurasia_core::natives::Native;
use eurasia_core::object::{Access, Body, Space};
use eurasia_core::roles::Role;
use eurasia_snapshot::write_image;

/// Main has one slot, `count`. main: vm args: args =
///   count: count + 1.
///   vm print: count asString.
///   vm writeBytes: vm snapshot toFile: (args at: 1)
fn counter_image() -> Vec<u8> {
    let mut b = ImageBuilder::new();
    let object = b.object;
    let small = b.role(Role::SmallIntegerClass).unwrap();
    b.method(
        small,
        "+",
        Access::Public,
        MethodSpec::Native(Native::IntegerAdd),
    );
    b.method(
        small,
        "asString",
        Access::Public,
        MethodSpec::Native(Native::NumberAsString),
    );
    let array = b.role(Role::ArrayClass).unwrap();
    b.method(
        array,
        "at:",
        Access::Public,
        MethodSpec::Native(Native::IndexedAt),
    );
    let mirror_class = b.define_class("VMMirror", object);
    for (selector, native) in [
        ("print:", Native::VmPrint),
        ("snapshot", Native::VmSnapshot),
        ("writeBytes:toFile:", Native::VmWriteFile),
    ] {
        b.method(
            mirror_class,
            selector,
            Access::Public,
            MethodSpec::Native(native),
        );
    }
    let mirror = b
        .heap
        .allocate(mirror_class, Space::Actor(0), Body::Regular(vec![]));
    b.set_role(Role::VmMirror, mirror);
    let main_class = b.define_class("Main", object);
    b.set_slot_count(main_class, 1);
    b.method(main_class, "count", Access::Private, MethodSpec::Getter(0));
    b.method(main_class, "count:", Access::Private, MethodSpec::Setter(0));
    let entry = b.heap.allocate(
        main_class,
        Space::Actor(0),
        Body::Regular(vec![Value::small_integer(0).unwrap()]),
    );
    b.set_role(Role::Entry, entry);
    let literals: Vec<Value> = [
        "count",
        "count:",
        "asString",
        "print:",
        "snapshot",
        "writeBytes:toFile:",
    ]
    .iter()
    .map(|s| b.string(s))
    .collect();
    let mut a = Assembler::new();
    // count: count + 1
    a.send(SendKind::SelfSend, 0, 0)
        .push_integer(1)
        .common_send(0)
        .send(SendKind::SelfSend, 1, 1)
        .byte(op::POP);
    // vm print: count asString
    a.push_parameter(2)
        .send(SendKind::SelfSend, 0, 0)
        .send(SendKind::Ordinary, 2, 0)
        .send(SendKind::Ordinary, 3, 1)
        .byte(op::POP);
    // vm writeBytes: vm snapshot toFile: (args at: 1)
    a.push_parameter(2)
        .push_parameter(2)
        .send(SendKind::Ordinary, 4, 0)
        .push_parameter(1)
        .push_integer(1)
        .common_send(16)
        .send(SendKind::Ordinary, 5, 2)
        .byte(op::POP)
        .byte(op::RETURN_NIL);
    b.method(
        main_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: a.finish(),
            literals,
        },
    );
    let (heap, roles) = b.finish();
    write_image(&heap, &roles)
}

fn run(image: &std::path::Path, save_to: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_eurasia"))
        .arg(image)
        .arg(save_to)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn state_survives_a_save_and_a_restart() {
    let dir = std::env::temp_dir().join(format!("eurasia-save-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.eus");
    let b = dir.join("b.eus");
    let c = dir.join("c.eus");
    std::fs::write(&a, counter_image()).unwrap();
    assert_eq!(run(&a, &b), "1\n");
    assert_eq!(run(&b, &c), "2\n", "the saved image remembers count = 1");
    assert_eq!(run(&c, &b), "3\n");
    // The original is unchanged.
    assert_eq!(run(&a, &c), "1\n");
}

/// Genesis's shape: the first image does its work, makes another object the
/// entry, and saves. The saved image starts at the new entry.
#[test]
fn a_program_can_choose_the_entry_of_the_image_it_saves() {
    let mut b = ImageBuilder::new();
    let object = b.object;
    let array = b.role(Role::ArrayClass).unwrap();
    b.method(
        array,
        "at:",
        Access::Public,
        MethodSpec::Native(Native::IndexedAt),
    );
    let mirror_class = b.define_class("VMMirror", object);
    for (selector, native) in [
        ("print:", Native::VmPrint),
        ("snapshot", Native::VmSnapshot),
        ("writeBytes:toFile:", Native::VmWriteFile),
        ("setRole:to:", Native::VmSetRole),
    ] {
        b.method(
            mirror_class,
            selector,
            Access::Public,
            MethodSpec::Native(native),
        );
    }
    let mirror = b
        .heap
        .allocate(mirror_class, Space::Actor(0), Body::Regular(vec![]));
    b.set_role(Role::VmMirror, mirror);

    // The base image's entry: prints "base".
    let base_class = b.define_class("Base", object);
    let base = b
        .heap
        .allocate(base_class, Space::Actor(0), Body::Regular(vec![]));
    let base_literals: Vec<Value> = ["print:", "base"].iter().map(|s| b.string(s)).collect();
    let mut m = Assembler::new();
    m.push_parameter(2)
        .push_literal(1)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    b.method(
        base_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: m.finish(),
            literals: base_literals,
        },
    );

    // The genesis entry: prints "genesis", sets the entry role (32) to the
    // base object, and saves.
    let genesis_class = b.define_class("Genesis", object);
    let genesis = b
        .heap
        .allocate(genesis_class, Space::Actor(0), Body::Regular(vec![]));
    b.set_role(Role::Entry, genesis);
    let mut literals: Vec<Value> = [
        "print:",
        "genesis",
        "setRole:to:",
        "snapshot",
        "writeBytes:toFile:",
    ]
    .iter()
    .map(|s| b.string(s))
    .collect();
    literals.push(base);
    let mut g = Assembler::new();
    g.push_parameter(2)
        .push_literal(1)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::POP);
    g.push_parameter(2)
        .push_integer(32)
        .push_literal(5)
        .send(SendKind::Ordinary, 2, 2)
        .byte(op::POP);
    g.push_parameter(2)
        .push_parameter(2)
        .send(SendKind::Ordinary, 3, 0)
        .push_parameter(1)
        .push_integer(1)
        .common_send(16)
        .send(SendKind::Ordinary, 4, 2)
        .byte(op::POP)
        .byte(op::RETURN_NIL);
    b.method(
        genesis_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: g.finish(),
            literals,
        },
    );
    let (heap, roles) = b.finish();

    let dir = std::env::temp_dir().join(format!("eurasia-genesis-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let first = dir.join("genesis.eus");
    let saved = dir.join("base.eus");
    std::fs::write(&first, write_image(&heap, &roles)).unwrap();
    assert_eq!(run(&first, &saved), "genesis\n");
    assert_eq!(run(&saved, &dir.join("unused.eus")), "base\n");
}
