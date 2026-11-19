// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Building images in Rust, for tests and for bootstrapping before the
//! packager exists. The real kernel is built by the packager (Newspeak); this
//! builder makes small images with a handful of classes and methods.

use alloc::vec::Vec;

use crate::bytecode::COMMON_SELECTORS;
use crate::natives::Native;
use crate::object::{
    Access, Behavior, Body, FROZEN, Heap, Method, MethodHeader, MethodKind, Mixin, Space,
};
use crate::roles::{Role, Roles};
use crate::value::Value;

/// How a method built by [`ImageBuilder::method`] runs.
pub enum MethodSpec {
    /// A native from the catalog.
    Native(Native),
    /// Bytecode with its argument count, local count and literals.
    Bytecode {
        /// The number of parameters.
        args: u32,
        /// The number of locals.
        locals: u32,
        /// The bytecode.
        code: Vec<u8>,
        /// The literals.
        literals: Vec<Value>,
    },
    /// A getter of slot `slot`.
    Getter(u32),
    /// A setter of slot `slot`.
    Setter(u32),
    /// Answers the receiver; takes `args` arguments.
    ReturnSelf(u32),
}

/// Builds a small image.
pub struct ImageBuilder {
    /// The heap under construction.
    pub heap: Heap,
    /// nil.
    pub nil: Value,
    /// true.
    pub true_: Value,
    /// false.
    pub false_: Value,
    /// The Object class.
    pub object: Value,
    metaclass_class: Value,
    roles: Vec<(Role, Value)>,
}

impl Default for ImageBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageBuilder {
    /// An image with nil, true and false, the Object, UndefinedObject, True,
    /// False, Metaclass, SmallInteger, String, Array and Message classes, and
    /// the common-selector table. Classes start with no methods.
    pub fn new() -> ImageBuilder {
        let mut heap = Heap::new();
        // nil first, with a placeholder class patched below.
        let placeholder = Value::small_integer(0).expect("small");
        let nil = heap.allocate(placeholder, Space::Actor(0), Body::Regular(Vec::new()));
        let mut b = ImageBuilder {
            heap,
            nil,
            true_: nil,
            false_: nil,
            object: nil,
            metaclass_class: nil,
            roles: Vec::new(),
        };
        b.metaclass_class = b.define_class("Metaclass", nil);
        b.object = b.define_class("Object", nil);
        let undefined = b.define_class("UndefinedObject", b.object);
        b.heap.get_mut(nil).expect("nil").header.class = undefined;
        let true_class = b.define_class("True", b.object);
        let false_class = b.define_class("False", b.object);
        b.true_ = b
            .heap
            .allocate(true_class, Space::Actor(0), Body::Regular(Vec::new()));
        b.false_ = b
            .heap
            .allocate(false_class, Space::Actor(0), Body::Regular(Vec::new()));
        let small_integer = b.define_class("SmallInteger", b.object);
        let string = b.define_class("String", b.object);
        let array = b.define_class("Array", b.object);
        let message = b.define_class("Message", b.object);
        let float = b.define_class("Float", b.object);
        let large_integer = b.define_class("LargeInteger", b.object);
        let byte_array = b.define_class("ByteArray", b.object);
        let weak_array = b.define_class("WeakArray", b.object);
        let closure = b.define_class("Closure", b.object);
        let activation = b.define_class("Activation", b.object);
        b.roles.extend([
            (Role::Nil, nil),
            (Role::True, b.true_),
            (Role::False, b.false_),
            (Role::SmallIntegerClass, small_integer),
            (Role::StringClass, string),
            (Role::ArrayClass, array),
            (Role::MessageClass, message),
            (Role::FloatClass, float),
            (Role::LargeIntegerClass, large_integer),
            (Role::ByteArrayClass, byte_array),
            (Role::WeakArrayClass, weak_array),
            (Role::ClosureClass, closure),
            (Role::ActivationClass, activation),
        ]);
        // Strings and arrays allocated before their classes existed get them now.
        let fix: Vec<(Value, bool)> = b
            .heap
            .iter()
            .filter_map(|(v, o)| match o.body {
                Body::String(_) => Some((v, true)),
                Body::Array(_) => Some((v, false)),
                _ => None,
            })
            .collect();
        for (v, is_string) in fix {
            b.heap.get_mut(v).expect("object").header.class =
                if is_string { string } else { array };
        }
        let mut table = Vec::new();
        for (selector, arity) in COMMON_SELECTORS {
            table.push(b.string(selector));
            table.push(Value::small_integer(arity.into()).expect("small"));
        }
        let table = b.array(table);
        b.roles.push((Role::CommonSelectors, table));
        for (role, selector) in [
            (Role::DoesNotUnderstandSelector, "doesNotUnderstand:"),
            (Role::CannotReturnSelector, "cannotReturn:"),
            (Role::AboutToReturnThroughSelector, "aboutToReturn:through:"),
            (Role::NonBooleanReceiverSelector, "nonBooleanReceiver:"),
        ] {
            let s = b.string(selector);
            b.roles.push((role, s));
        }
        b
    }

    /// The value assigned to `role` so far, if any.
    pub fn role(&self, role: Role) -> Option<Value> {
        self.roles
            .iter()
            .find(|&&(r, _)| r == role)
            .map(|&(_, v)| v)
    }

    /// A frozen String in the program space.
    pub fn string(&mut self, s: &str) -> Value {
        let class = self.role(Role::StringClass).unwrap_or(self.nil);
        let v = self
            .heap
            .allocate(class, Space::Program, Body::String(s.as_bytes().to_vec()));
        self.heap.get_mut(v).expect("string").header.flags = FROZEN;
        v
    }

    /// An Array.
    pub fn array(&mut self, elements: Vec<Value>) -> Value {
        let class = self.role(Role::ArrayClass).unwrap_or(self.nil);
        self.heap
            .allocate(class, Space::Actor(0), Body::Array(elements))
    }

    /// A class named `name` with superclass `superclass` (nil for none), its
    /// own mixin with no methods, and its metaclass.
    pub fn define_class(&mut self, name: &str, superclass: Value) -> Value {
        let name_string = self.string(name);
        let mixin = self.new_mixin(name_string, self.nil);
        let meta_mixin = self.new_mixin(name_string, self.nil);
        if let Some(Body::Mixin(m)) = self.heap.get_mut(mixin).map(|o| &mut o.body) {
            m.class_mixin = meta_mixin;
        }
        let meta_super = if superclass == self.nil {
            self.nil
        } else {
            self.heap.get(superclass).expect("superclass").header.class
        };
        let metaclass = self.heap.allocate(
            self.metaclass_class,
            Space::Actor(0),
            Body::Metaclass(Behavior {
                mixin: meta_mixin,
                superclass: meta_super,
                enclosing_object: self.nil,
                name: self.nil,
                this_class: self.nil,
                library: self.nil,
            }),
        );
        let class = self.heap.allocate(
            metaclass,
            Space::Actor(0),
            Body::Class(Behavior {
                mixin,
                superclass,
                enclosing_object: self.nil,
                name: name_string,
                this_class: self.nil,
                library: self.nil,
            }),
        );
        if let Some(Body::Metaclass(b)) = self.heap.get_mut(metaclass).map(|o| &mut o.body) {
            b.this_class = class;
        }
        if self.metaclass_class == self.nil {
            // The first class defined is Metaclass itself.
            self.metaclass_class = class;
            self.heap
                .get_mut(metaclass)
                .expect("metaclass")
                .header
                .class = class;
        }
        class
    }

    fn new_mixin(&mut self, name: Value, enclosing: Value) -> Value {
        let methods = self.array(Vec::new());
        let slot_names = self.array(Vec::new());
        let v = self.heap.allocate(
            self.nil,
            Space::Program,
            Body::Mixin(Mixin {
                name,
                methods,
                enclosing_mixin: enclosing,
                slot_names,
                slot_count: 0,
                class_mixin: self.nil,
                library: self.nil,
            }),
        );
        self.heap.get_mut(v).expect("mixin").header.flags = FROZEN;
        v
    }

    /// Declares `count` slots in `class`'s mixin.
    pub fn set_slot_count(&mut self, class: Value, count: u32) {
        let mixin = self.mixin_of(class);
        if let Some(Body::Mixin(m)) = self.heap.get_mut(mixin).map(|o| &mut o.body) {
            m.slot_count = count;
        }
    }

    /// The mixin of `class`.
    pub fn mixin_of(&self, class: Value) -> Value {
        self.heap.behavior(class).expect("class").mixin
    }

    /// Adds a method to `class`'s mixin.
    pub fn method(
        &mut self,
        class: Value,
        selector: &str,
        access: Access,
        spec: MethodSpec,
    ) -> Value {
        let mixin = self.mixin_of(class);
        let selector_string = self.string(selector);
        let (kind, args, locals, native, slot, code, literals) = match spec {
            MethodSpec::Native(n) => (
                MethodKind::Native,
                n.arity() as u32,
                0,
                n as u32,
                0,
                Vec::new(),
                Vec::new(),
            ),
            MethodSpec::Bytecode {
                args,
                locals,
                code,
                literals,
            } => (MethodKind::Ordinary, args, locals, 0, 0, code, literals),
            MethodSpec::Getter(slot) => (MethodKind::Getter, 0, 0, 0, slot, Vec::new(), Vec::new()),
            MethodSpec::Setter(slot) => (MethodKind::Setter, 1, 0, 0, slot, Vec::new(), Vec::new()),
            MethodSpec::ReturnSelf(args) => (
                MethodKind::ReturnSelf,
                args,
                0,
                0,
                0,
                Vec::new(),
                Vec::new(),
            ),
        };
        let literals = self.array(literals);
        let method_class = self.role(Role::MethodClass).unwrap_or(self.nil);
        let m = self.heap.allocate(
            method_class,
            Space::Program,
            Body::Method(Method {
                header: MethodHeader {
                    kind,
                    access,
                    arg_count: args,
                    local_count: locals,
                    native,
                    slot,
                },
                bytecode: code.into(),
                literals,
                mixin,
                selector: selector_string,
                source: self.nil,
                library: self.nil,
            }),
        );
        self.heap.get_mut(m).expect("method").header.flags = FROZEN;
        let methods = self.heap.mixin(mixin).expect("mixin").methods;
        if let Some(Body::Array(elements)) = self.heap.get_mut(methods).map(|o| &mut o.body) {
            elements.push(m);
        }
        m
    }

    /// Assigns a role.
    pub fn set_role(&mut self, role: Role, value: Value) {
        self.roles.retain(|&(r, _)| r != role);
        self.roles.push((role, value));
    }

    /// The heap and its roles. Roles not assigned are nil.
    pub fn finish(self) -> (Heap, Roles) {
        let mut pairs = self.roles.clone();
        for &role in Role::ALL {
            if !pairs.iter().any(|&(r, _)| r == role) {
                pairs.push((role, self.nil));
            }
        }
        let roles = Roles::new(&pairs).expect("every role assigned once");
        (self.heap, roles)
    }
}

/// The M1 first-light image: `main: vm args: args` prints `3 + 4` (via
/// `asString`) and answers nil. It has SmallInteger `+` and `asString`
/// natives, a VM mirror with `print:` and `exit:`, a failure class whose
/// `signal` answers the reason name, and `Object>>doesNotUnderstand:`
/// answering the message's selector. Used by the snapshot and CLI tests.
pub fn first_light_image() -> (Heap, Roles) {
    use crate::bytecode::{Assembler, SendKind, op};

    let mut b = ImageBuilder::new();
    let object = b.object;
    let small_integer = b.role(Role::SmallIntegerClass).expect("role");
    b.method(
        small_integer,
        "+",
        Access::Public,
        MethodSpec::Native(Native::IntegerAdd),
    );
    b.method(
        small_integer,
        "asString",
        Access::Public,
        MethodSpec::Native(Native::NumberAsString),
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
        .allocate(mirror_class, Space::Actor(0), Body::Regular(Vec::new()));
    b.set_role(Role::VmMirror, mirror);
    let failure = b.define_class("NativeFailure", object);
    b.method(failure, "signal", Access::Public, MethodSpec::Getter(3));
    b.set_role(Role::FailureClass, failure);
    let message = b.role(Role::MessageClass).expect("role");
    b.method(message, "selector", Access::Public, MethodSpec::Getter(0));
    let selector = b.string("selector");
    let mut dnu = Assembler::new();
    dnu.push_parameter(1)
        .send(SendKind::Ordinary, 0, 0)
        .return_top();
    b.method(
        object,
        "doesNotUnderstand:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 1,
            locals: 0,
            code: dnu.finish(),
            literals: alloc::vec![selector],
        },
    );
    let entry_class = b.define_class("Main", object);
    let entry = b
        .heap
        .allocate(entry_class, Space::Actor(0), Body::Regular(Vec::new()));
    b.set_role(Role::Entry, entry);
    let print = b.string("print:");
    let as_string = b.string("asString");
    let mut main = Assembler::new();
    main.push_parameter(2)
        .push_integer(3)
        .push_integer(4)
        .common_send(0)
        .send(SendKind::Ordinary, 1, 0)
        .send(SendKind::Ordinary, 0, 1)
        .byte(op::RETURN_NIL);
    b.method(
        entry_class,
        "main:args:",
        Access::Public,
        MethodSpec::Bytecode {
            args: 2,
            locals: 0,
            code: main.finish(),
            literals: alloc::vec![print, as_string],
        },
    );
    b.finish()
}
