// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The object model and the heap.
//!
//! Objects live in an arena; a reference [`Value`] holds the object's index.
//! Every object records its allocation sequence number, which is its identity
//! order and its identity hash (`kernel-semantics.tex`, `eq:kernel-fresh` and
//! `eq:kernel-identity-hash`). Methods, mixins, classes, metaclasses and
//! messages have the contract layouts of the implementation plan; everything
//! the VM derives from them is computed, never stored in these records.

use alloc::vec::Vec;

use crate::value::Value;

/// Which space an object belongs to (implementation plan, section 2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// The cohort's shared program space: installed, frozen methods and mixins.
    Program,
    /// The heap of actor `n`.
    Actor(u32),
}

/// Object flag: frozen (installed, or canonical). Writes to it fail.
pub const FROZEN: u8 = 1;

/// The fields every object has.
#[derive(Clone, Debug)]
pub struct Header {
    /// The allocation sequence number.
    pub seq: u64,
    /// The object's class.
    pub class: Value,
    /// Flags: [`FROZEN`].
    pub flags: u8,
    /// Its space.
    pub space: Space,
}

/// How a method runs (snapshot format, section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    /// Interpreted bytecode.
    Ordinary,
    /// A native from the catalog.
    Native,
    /// Answers slot `slot` of the receiver.
    Getter,
    /// Sets slot `slot` of the receiver to the argument and answers the receiver.
    Setter,
    /// Answers the receiver.
    ReturnSelf,
}

/// A method's access modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Visible to every send.
    Public,
    /// Visible to self, implicit-receiver, outer and super sends.
    Protected,
    /// Visible only to lexical sends from its own mixin.
    Private,
}

/// A method header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodHeader {
    /// How the method runs.
    pub kind: MethodKind,
    /// Its access modifier.
    pub access: Access,
    /// The number of parameters.
    pub arg_count: u32,
    /// The number of locals initialized to nil on activation.
    pub local_count: u32,
    /// The native's catalog id (kind native only).
    pub native: u32,
    /// The slot index (getter and setter kinds only).
    pub slot: u32,
}

/// A method (contract layout).
#[derive(Clone, Debug)]
pub struct Method {
    /// The header.
    pub header: MethodHeader,
    /// V5 bytecode; empty for every kind but ordinary.
    pub bytecode: Vec<u8>,
    /// The literal Array.
    pub literals: Value,
    /// The defining mixin.
    pub mixin: Value,
    /// The selector (a String).
    pub selector: Value,
    /// Source, or nil.
    pub source: Value,
    /// The mirror library's field; the VM never reads it.
    pub library: Value,
}

/// A mixin (contract layout).
#[derive(Clone, Debug)]
pub struct Mixin {
    /// The name (a String).
    pub name: Value,
    /// The methods (an Array of Methods).
    pub methods: Value,
    /// The lexically enclosing mixin, or nil.
    pub enclosing_mixin: Value,
    /// Slot-declaration names (an Array of Strings), in order.
    pub slot_names: Value,
    /// The number of slot declarations.
    pub slot_count: u32,
    /// The mirror library's field; the VM never reads it.
    pub library: Value,
}

/// A class or metaclass (contract layout). A metaclass has `this_class`.
#[derive(Clone, Debug)]
pub struct Behavior {
    /// The applied mixin.
    pub mixin: Value,
    /// The superclass, or nil.
    pub superclass: Value,
    /// The enclosing object.
    pub enclosing_object: Value,
    /// For a class, its name; nil for a metaclass.
    pub name: Value,
    /// For a metaclass, its class; nil for a class.
    pub this_class: Value,
    /// The mirror library's field; the VM never reads it.
    pub library: Value,
}

/// An object's contents.
#[derive(Clone, Debug)]
pub enum Body {
    /// Ordinary slots.
    Regular(Vec<Value>),
    /// A ByteArray.
    Bytes(Vec<u8>),
    /// A String, as UTF-8.
    String(Vec<u8>),
    /// An Array.
    Array(Vec<Value>),
    /// A WeakArray.
    WeakArray(Vec<Value>),
    /// An Ephemeron.
    Ephemeron {
        /// The key.
        key: Value,
        /// The value.
        value: Value,
        /// The finalizer.
        finalizer: Value,
    },
    /// A Float (IEEE 754 binary64).
    Float(f64),
    /// An integer outside the SmallInteger range.
    LargeInteger {
        /// The sign.
        negative: bool,
        /// The magnitude, little-endian.
        magnitude: Vec<u8>,
    },
    /// A method.
    Method(Method),
    /// A mixin.
    Mixin(Mixin),
    /// A class.
    Class(Behavior),
    /// A metaclass.
    Metaclass(Behavior),
}

/// An object.
#[derive(Clone, Debug)]
pub struct Object {
    /// The header.
    pub header: Header,
    /// The contents.
    pub body: Body,
}

/// Why the heap refused an operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeapError {
    /// A sequence number did not exceed the previous one.
    SequenceNotIncreasing {
        /// The offending sequence number.
        seq: u64,
    },
    /// A sequence number is not below the allocation counter.
    SequenceBeyondCounter {
        /// The offending sequence number.
        seq: u64,
    },
}

/// The objects of a VM, in allocation order.
#[derive(Default, Debug)]
pub struct Heap {
    objects: Vec<Object>,
    next_seq: u64,
}

impl Heap {
    /// An empty heap.
    pub fn new() -> Heap {
        Heap::default()
    }

    /// The number of objects.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the heap has no objects.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The allocation counter `n_α`: the next sequence number to allocate.
    pub fn allocation_counter(&self) -> u64 {
        self.next_seq
    }

    /// Allocates an object with the next sequence number.
    pub fn allocate(&mut self, class: Value, space: Space, body: Body) -> Value {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.push(seq, class, 0, space, body)
    }

    /// Adds an object with a given sequence number, as a snapshot loader does.
    /// Sequence numbers must strictly increase.
    pub fn insert_loaded(
        &mut self,
        seq: u64,
        class: Value,
        flags: u8,
        space: Space,
        body: Body,
    ) -> Result<Value, HeapError> {
        if let Some(last) = self.objects.last()
            && seq <= last.header.seq
        {
            return Err(HeapError::SequenceNotIncreasing { seq });
        }
        if seq >= self.next_seq {
            self.next_seq = seq + 1;
        }
        Ok(self.push(seq, class, flags, space, body))
    }

    /// Sets the allocation counter after loading. It may not go backwards past
    /// any existing object.
    pub fn set_allocation_counter(&mut self, counter: u64) -> Result<(), HeapError> {
        if let Some(last) = self.objects.last()
            && counter <= last.header.seq
        {
            return Err(HeapError::SequenceBeyondCounter {
                seq: last.header.seq,
            });
        }
        self.next_seq = counter;
        Ok(())
    }

    fn push(&mut self, seq: u64, class: Value, flags: u8, space: Space, body: Body) -> Value {
        let index = self.objects.len() as u64;
        self.objects.push(Object {
            header: Header {
                seq,
                class,
                flags,
                space,
            },
            body,
        });
        Value::reference(index).expect("heap index exceeds the reference range")
    }

    /// The object a value refers to, if it is a reference to an existing object.
    pub fn get(&self, v: Value) -> Option<&Object> {
        self.objects.get(usize::try_from(v.as_reference()?).ok()?)
    }

    /// Mutable access to the object a value refers to.
    pub fn get_mut(&mut self, v: Value) -> Option<&mut Object> {
        self.objects
            .get_mut(usize::try_from(v.as_reference()?).ok()?)
    }

    /// The objects in allocation order, with their references.
    pub fn iter(&self) -> impl Iterator<Item = (Value, &Object)> {
        self.objects
            .iter()
            .enumerate()
            .map(|(i, o)| (Value::reference(i as u64).expect("index in range"), o))
    }

    /// The bytes of a String object.
    pub fn string_bytes(&self, v: Value) -> Option<&[u8]> {
        match &self.get(v)?.body {
            Body::String(b) => Some(b),
            _ => None,
        }
    }

    /// The elements of an Array object.
    pub fn array_elements(&self, v: Value) -> Option<&[Value]> {
        match &self.get(v)?.body {
            Body::Array(e) => Some(e),
            _ => None,
        }
    }

    /// The method record of a Method object.
    pub fn method(&self, v: Value) -> Option<&Method> {
        match &self.get(v)?.body {
            Body::Method(m) => Some(m),
            _ => None,
        }
    }

    /// The mixin record of a Mixin object.
    pub fn mixin(&self, v: Value) -> Option<&Mixin> {
        match &self.get(v)?.body {
            Body::Mixin(m) => Some(m),
            _ => None,
        }
    }

    /// The behavior record of a class or metaclass.
    pub fn behavior(&self, v: Value) -> Option<&Behavior> {
        match &self.get(v)?.body {
            Body::Class(b) | Body::Metaclass(b) => Some(b),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nil() -> Value {
        Value::reference(0).unwrap()
    }

    #[test]
    fn allocation_assigns_increasing_sequence_numbers() {
        let mut h = Heap::new();
        let a = h.allocate(nil(), Space::Actor(0), Body::Regular(Vec::new()));
        let b = h.allocate(nil(), Space::Actor(0), Body::Bytes(Vec::new()));
        assert_eq!(h.get(a).unwrap().header.seq, 0);
        assert_eq!(h.get(b).unwrap().header.seq, 1);
        assert_eq!(h.allocation_counter(), 2);
    }

    #[test]
    fn loaded_objects_keep_their_sequence_numbers_and_gaps() {
        let mut h = Heap::new();
        let a = h
            .insert_loaded(5, nil(), 0, Space::Program, Body::Regular(Vec::new()))
            .unwrap();
        let b = h
            .insert_loaded(9, nil(), FROZEN, Space::Program, Body::Regular(Vec::new()))
            .unwrap();
        assert_eq!(h.get(a).unwrap().header.seq, 5);
        assert_eq!(h.get(b).unwrap().header.flags, FROZEN);
        assert_eq!(
            h.insert_loaded(9, nil(), 0, Space::Program, Body::Regular(Vec::new()))
                .unwrap_err(),
            HeapError::SequenceNotIncreasing { seq: 9 }
        );
        h.set_allocation_counter(20).unwrap();
        let c = h.allocate(nil(), Space::Actor(0), Body::Regular(Vec::new()));
        assert_eq!(h.get(c).unwrap().header.seq, 20);
        assert!(h.set_allocation_counter(20).is_err());
    }

    #[test]
    fn typed_accessors_answer_none_for_other_kinds() {
        let mut h = Heap::new();
        let s = h.allocate(nil(), Space::Actor(0), Body::String(b"abc".to_vec()));
        assert_eq!(h.string_bytes(s), Some(&b"abc"[..]));
        assert!(h.array_elements(s).is_none());
        assert!(h.method(s).is_none());
        assert!(h.get(Value::small_integer(3).unwrap()).is_none());
    }
}
