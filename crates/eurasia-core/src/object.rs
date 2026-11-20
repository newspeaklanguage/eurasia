// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The object model and the heap.
//!
//! Objects live in one arena of words (EURASIA_ARENA_DESIGN_2026-11-19.md):
//! three header words, then the contents; a reference [`Value`] holds the
//! object's word offset. Byte contents live in a separate byte space and
//! bytecode in a table of codes, so every arena word is a Value or a raw
//! header word, and no `unsafe` is needed.
//! Every object records its allocation sequence number, which is its identity
//! order and its identity hash (`kernel-semantics.tex`, `eq:kernel-fresh` and
//! `eq:kernel-identity-hash`). Methods, mixins, classes, metaclasses and
//! messages have the contract layouts of the implementation plan; everything
//! the VM derives from them is computed, never stored in these records.

use alloc::rc::Rc;
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
/// Object flag: a value (`actor-semantics.tex`, the value predicate): deeply
/// immutable, so actors may share it. Strings, floats and large integers
/// are values from allocation; so are nil, true and false, an instance of a
/// value class, and a class whose enclosing object is a value.
pub const VALUE: u8 = 2;
/// Class flag: a value class, one whose superclass chain includes the Value
/// role (decided 2026-11-17, `EURASIA_ACTORS_DESIGN_2026-11-17.md` section
/// 5). Its instances are values.
pub const VALUE_CLASS: u8 = 4;
/// Object flag: shared by every actor of the cohort, though mutable (the
/// kernel module and the VM mirror). Cloning a platform does not copy it.
pub const SHARED: u8 = 8;
/// On a method: a simulation root (psoup's primitive 142). A non-local
/// return stops at it, and an exception in a simulated thread is routed to
/// it (M6 design 4.4).
pub const SIMULATION_ROOT: u8 = 16;
/// Every defined flag (snapshot format, section 5).
pub const FLAGS: u8 = FROZEN | VALUE | VALUE_CLASS | SHARED | SIMULATION_ROOT;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub bytecode: Rc<[u8]>,
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
    /// For an instance-side mixin, its paired class-side mixin; nil for a
    /// class-side mixin.
    pub class_mixin: Value,
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

/// A closure (hidden layout: reached only through natives and kernel
/// operations).
#[derive(Clone, Debug)]
pub struct Closure {
    /// The activation that created it.
    pub defining: Value,
    /// The bytecode index where its body starts, in the defining method.
    pub initial_ip: u32,
    /// The number of parameters.
    pub num_args: u32,
    /// The copied values, which become its frame's first locals.
    pub copied: Vec<Value>,
}

/// An activation handle (hidden layout). A frame's handle is created when
/// something needs it (a closure, reflection, a VM upcall) and is marked dead
/// when the frame returns or is unwound. A *detached* activation (M6 design
/// 4.4) holds its own state instead of naming a frame: the debugger's
/// Simulator makes and runs them (`Activation new`), and
/// `vm resumeActivation:` turns a chain of them into frames.
#[derive(Clone, Debug)]
pub struct Activation {
    /// The live frame's serial number, or `None` once the frame is gone.
    pub frame: Option<u64>,
    /// The method running in the frame.
    pub method: Value,
    /// The method receiver (`self`).
    pub receiver: Value,
    /// For a closure frame, the closure; nil for a method frame.
    pub closure: Value,
    /// A detached activation's own state; `None` for a frame's handle.
    pub detached: Option<Detached>,
}

/// The state of a detached activation, as psoup's heap activations hold it.
#[derive(Clone, Debug)]
pub struct Detached {
    /// The sender (an activation) or nil.
    pub sender: Value,
    /// The bytecode position of the next instruction, from 1, or nil.
    pub bci: Value,
    /// Its temps (the arguments, then the locals, then the operand stack):
    /// an Array, or nil for none.
    pub temps: Value,
}

/// A promise record (hidden layout; `actor-semantics.tex`, the table Π): the
/// state of a promise that has crossed actors, which every actor sees.
/// Values in it are in no actor's terms: values, far references and promise
/// records only.
#[derive(Clone, Debug)]
pub struct PromiseRecord {
    /// [`PromiseRecord::PENDING`], `FULFILLED`, `BROKEN` or `FORWARDED`.
    pub state: u8,
    /// The resolution, the problem, or the promise record forwarded to; nil
    /// while pending.
    pub value: Value,
    /// A far reference to the library promise of the actor that created the
    /// record, which stands for it there; nil for none.
    pub owner: Value,
    /// Far references to library promises waiting for it to settle: an
    /// Array, or nil for none.
    pub waiters: Value,
    /// Eventual sends to it while pending (Route-Pending), in the order
    /// sent, each an Array `{sending actor. selector. result record or nil.
    /// arguments...}`, delivered when it settles: an Array, or nil for none.
    pub queue: Value,
}

impl PromiseRecord {
    /// Not yet settled.
    pub const PENDING: u8 = 0;
    /// Fulfilled with `value`.
    pub const FULFILLED: u8 = 1;
    /// Broken with `value`, the problem.
    pub const BROKEN: u8 = 2;
    /// Resolved to another promise record, `value`.
    pub const FORWARDED: u8 = 3;
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
    /// A closure.
    Closure(Closure),
    /// An activation handle.
    Activation(Activation),
    /// A far reference (hidden layout; `actor-semantics.tex`, the table Φ):
    /// the one place a reference into another actor's heap may be held.
    FarRef {
        /// The actor whose heap holds the target.
        actor: u32,
        /// The target, in that heap.
        target: Value,
    },
    /// A promise record.
    Promise(PromiseRecord),
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

/// The objects of a VM: one arena of words, in allocation order (a moved
/// object, see [`Heap::reshape`], is the exception), with byte contents in
/// a byte space and bytecode in a table of codes.
///
/// An object at word offset `o` is `words[o]` its class, `words[o + 1]` its
/// sequence number (raw), `words[o + 2]` its format, flags, space and size
/// (raw, see [`meta`]), then its contents (EURASIA_ARENA_DESIGN_2026-11-19.md
/// section 2).
#[derive(Default, Debug, Clone)]
pub struct Heap {
    pub(crate) words: Vec<Value>,
    pub(crate) bytes: Vec<u8>,
    pub(crate) codes: Vec<CodeEntry>,
    pub(crate) count: usize,
    next_seq: u64,
    /// The nursery (EURASIA_NURSERY_DESIGN_2026-11-19.md): objects from this
    /// word offset on are young; below it, old. Likewise for the byte
    /// space and the code table.
    pub(crate) boundary: usize,
    pub(crate) byte_boundary: usize,
    pub(crate) code_boundary: usize,
    /// The old objects (word offsets) that may refer to young ones, each
    /// once (its header's [`REMEMBERED`] bit says it is here).
    pub(crate) remembered: Vec<usize>,
    /// The objects below `boundary`, and the old words after the last full
    /// collection (to decide when the next one is due).
    pub(crate) old_count: usize,
    pub(crate) old_words_at_full: usize,
    /// Something rewrote references wholesale (a commit): the next
    /// collection must be full.
    pub(crate) full_due: bool,
}

/// An entry of the code table: a method's bytecode, and the compiler's word
/// for it: an activation count (0 or more), [`JIT_NEVER`], or a compiled
/// function's index `i` as `-2 - i` (newspeak/
/// EURASIA_COMPILER_DESIGN_2026-11-19.md). Kept here, beside the bytecode,
/// rather than in the method object, whose size the lookups feel.
#[derive(Clone, Debug)]
pub(crate) struct CodeEntry {
    pub(crate) bytes: Rc<[u8]>,
    pub(crate) jit: i64,
}

/// The header words of an object.
pub(crate) const HEADER: usize = 3;

/// The arena's format codes. `Filler` marks the old place of a moved object;
/// it is skipped by every walk and dropped by the next collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Code {
    Regular = 0,
    Bytes = 1,
    String = 2,
    Array = 3,
    WeakArray = 4,
    Ephemeron = 5,
    Float = 6,
    LargePositive = 7,
    LargeNegative = 8,
    Method = 9,
    Mixin = 10,
    Class = 11,
    Metaclass = 12,
    Closure = 13,
    Activation = 14,
    FarRef = 15,
    Promise = 16,
    Filler = 17,
}

impl Code {
    #[inline]
    fn from_bits(b: u64) -> Code {
        match b {
            0 => Code::Regular,
            1 => Code::Bytes,
            2 => Code::String,
            3 => Code::Array,
            4 => Code::WeakArray,
            5 => Code::Ephemeron,
            6 => Code::Float,
            7 => Code::LargePositive,
            8 => Code::LargeNegative,
            9 => Code::Method,
            10 => Code::Mixin,
            11 => Code::Class,
            12 => Code::Metaclass,
            13 => Code::Closure,
            14 => Code::Activation,
            15 => Code::FarRef,
            16 => Code::Promise,
            _ => Code::Filler,
        }
    }

    fn format(self) -> Option<Format> {
        Some(match self {
            Code::Regular => Format::Regular,
            Code::Bytes => Format::Bytes,
            Code::String => Format::String,
            Code::Array => Format::Array,
            Code::WeakArray => Format::WeakArray,
            Code::Ephemeron => Format::Ephemeron,
            Code::Float => Format::Float,
            Code::LargePositive | Code::LargeNegative => Format::LargeInteger,
            Code::Method => Format::Method,
            Code::Mixin => Format::Mixin,
            Code::Class => Format::Class,
            Code::Metaclass => Format::Metaclass,
            Code::Closure => Format::Closure,
            Code::Activation => Format::Activation,
            Code::FarRef => Format::FarRef,
            Code::Promise => Format::Promise,
            Code::Filler => return None,
        })
    }

    /// Whether the contents are bytes, in the byte space (one content word:
    /// the offset there; the size is the byte count).
    pub(crate) fn is_bytes(self) -> bool {
        matches!(
            self,
            Code::Bytes | Code::String | Code::LargePositive | Code::LargeNegative
        )
    }

    /// Whether the contents are Values the collector scans (all but bytes,
    /// Float and Filler).
    pub(crate) fn has_references(self) -> bool {
        !self.is_bytes() && !matches!(self, Code::Float | Code::Filler)
    }
}

/// The Program space's code in the header.
const PROGRAM_SPACE: u64 = (1 << 21) - 1;

/// The header bit of an object in the remembered set (the top bit of the
/// space field, which is therefore 21 bits).
pub(crate) const REMEMBERED: u64 = 1 << 31;

/// The third header word: format (5 bits), flags (5), space (21), remembered
/// (1), size (32).
pub(crate) fn meta(code: Code, flags: u8, space: Space, size: usize) -> Value {
    let space = match space {
        Space::Program => PROGRAM_SPACE,
        Space::Actor(n) => {
            assert!(u64::from(n) < PROGRAM_SPACE, "too many actors");
            u64::from(n)
        }
    };
    let size = u32::try_from(size).expect("an object of at most 2^32 fields or bytes");
    Value::from_bits(
        code as u64 | (u64::from(flags & FLAGS) << 5) | (space << 10) | (u64::from(size) << 32),
    )
}

/// The code in a header's third word (the collector's).
#[inline]
pub(crate) fn meta_code_pub(m: Value) -> Code {
    meta_code(m)
}

#[inline]
fn meta_code(m: Value) -> Code {
    Code::from_bits(m.to_bits() & 31)
}
fn meta_flags(m: Value) -> u8 {
    ((m.to_bits() >> 5) & 31) as u8
}
fn meta_space(m: Value) -> Space {
    match (m.to_bits() >> 10) & PROGRAM_SPACE {
        PROGRAM_SPACE => Space::Program,
        n => Space::Actor(n as u32),
    }
}
pub(crate) fn meta_size(m: Value) -> usize {
    (m.to_bits() >> 32) as usize
}

/// The codes whose contents are one word whatever the size: the byte
/// formats (an offset into the byte space) and Float.
const ONE_WORD: u32 = (1 << Code::Bytes as u32)
    | (1 << Code::String as u32)
    | (1 << Code::LargePositive as u32)
    | (1 << Code::LargeNegative as u32)
    | (1 << Code::Float as u32);

/// The number of content words of an object with this code and size.
#[inline]
pub(crate) fn content_words(code: Code, size: usize) -> usize {
    if ONE_WORD & (1 << code as u32) != 0 {
        1
    } else {
        size
    }
}

fn small(n: i64) -> Value {
    Value::small_integer(n).expect("a SmallInteger")
}

fn small_of(v: Value) -> i64 {
    v.as_small_integer().expect("a SmallInteger field")
}

// Field positions in the contents of the program objects.
const M_HEADER1: usize = 0;
const M_HEADER2: usize = 1;
const M_LITERALS: usize = 2;
const M_CODE: usize = 7;

/// The compiler's word for a method it does not compile ([`CodeEntry`]).
pub const JIT_NEVER: i64 = -1;
const MIXIN_SLOT_COUNT: usize = 4;
const C_INITIAL_IP: usize = 1;
const C_NUM_ARGS: usize = 2;
const C_COPIED: usize = 3;
const A_FRAME: usize = 0;
const A_METHOD: usize = 1;
const A_RECEIVER: usize = 2;
const A_CLOSURE: usize = 3;
const A_DETACHED: usize = 4;
const A_SENDER: usize = 5;
const A_BCI: usize = 6;
const A_TEMPS: usize = 7;
const P_STATE: usize = 0;
const P_VALUE: usize = 1;
const P_WAITERS: usize = 3;
const P_QUEUE: usize = 4;

/// The two SmallIntegers a method header is stored as.
fn encode_header(h: &MethodHeader) -> (Value, Value) {
    let kind = match h.kind {
        MethodKind::Ordinary => 0,
        MethodKind::Native => 1,
        MethodKind::Getter => 2,
        MethodKind::Setter => 3,
        MethodKind::ReturnSelf => 4,
    };
    let access = match h.access {
        Access::Public => 0,
        Access::Protected => 1,
        Access::Private => 2,
    };
    assert!(
        h.arg_count < 1 << 24 && h.slot < 1 << 29,
        "method header in range"
    );
    let one =
        kind | (access << 3) | (i64::from(h.arg_count) << 5) | (i64::from(h.local_count) << 29);
    let two = i64::from(h.native) | (i64::from(h.slot) << 32);
    (small(one), small(two))
}

fn decode_header(one: Value, two: Value) -> MethodHeader {
    let (one, two) = (small_of(one), small_of(two));
    MethodHeader {
        kind: match one & 7 {
            0 => MethodKind::Ordinary,
            1 => MethodKind::Native,
            2 => MethodKind::Getter,
            3 => MethodKind::Setter,
            _ => MethodKind::ReturnSelf,
        },
        access: match (one >> 3) & 3 {
            0 => Access::Public,
            1 => Access::Protected,
            _ => Access::Private,
        },
        arg_count: ((one >> 5) & ((1 << 24) - 1)) as u32,
        local_count: ((one >> 29) & 0xffff_ffff) as u32,
        native: (two & 0xffff_ffff) as u32,
        slot: (two >> 32) as u32,
    }
}

impl Heap {
    /// An empty heap.
    pub fn new() -> Heap {
        Heap::default()
    }

    /// The number of objects.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the heap has no objects.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The allocation counter `n_α`: the next sequence number to allocate.
    pub fn allocation_counter(&self) -> u64 {
        self.next_seq
    }

    /// The arena's size in words (for statistics).
    pub fn words(&self) -> usize {
        self.words.len()
    }

    /// Allocates an object with the next sequence number. Strings, floats
    /// and large integers are values from allocation.
    pub fn allocate(&mut self, class: Value, space: Space, body: Body) -> Value {
        let seq = self.next_seq;
        self.next_seq += 1;
        let flags = match body {
            Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => VALUE,
            _ => 0,
        };
        self.push(seq, class, flags, space, body)
    }

    /// Whether `v` is a value by its flag (immediates are).
    pub fn is_value(&self, v: Value) -> bool {
        match self.start(v) {
            Some(o) => meta_flags(self.words[o + 2]) & VALUE != 0,
            None => true,
        }
    }

    /// Adds an object read from a snapshot, with its own sequence number,
    /// which must exceed every one before it.
    pub fn insert_loaded(
        &mut self,
        seq: u64,
        class: Value,
        flags: u8,
        space: Space,
        body: Body,
    ) -> Result<Value, HeapError> {
        if self.count > 0 && seq < self.next_seq {
            return Err(HeapError::SequenceNotIncreasing { seq });
        }
        self.next_seq = seq + 1;
        Ok(self.push(seq, class, flags, space, body))
    }

    /// Sets the allocation counter, which must exceed every sequence number.
    pub fn set_allocation_counter(&mut self, counter: u64) -> Result<(), HeapError> {
        if self.count > 0 && counter < self.next_seq {
            return Err(HeapError::SequenceBeyondCounter {
                seq: self.next_seq - 1,
            });
        }
        self.next_seq = counter;
        Ok(())
    }

    /// The words an object described by `body` takes in the arena.
    pub fn words_for(body: &Body) -> usize {
        HEADER
            + match body {
                Body::Regular(e) | Body::Array(e) | Body::WeakArray(e) => e.len(),
                Body::Bytes(_) | Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => 1,
                Body::Ephemeron { .. } => 3,
                Body::Method(_) => 8,
                Body::Mixin(_) => 7,
                Body::Class(_) | Body::Metaclass(_) => 6,
                Body::Closure(c) => C_COPIED + c.copied.len(),
                Body::Activation(_) => 8,
                Body::FarRef { .. } => 2,
                Body::Promise(_) => 5,
            }
    }

    fn push(&mut self, seq: u64, class: Value, flags: u8, space: Space, body: Body) -> Value {
        let o = self.words.len();
        let v = Value::reference(o as u64).expect("arena offset in the reference range");
        // The header, with the size patched in once the contents are known;
        // the contents go straight into the arena.
        self.words.push(class);
        self.words.push(Value::from_bits(seq));
        self.words.push(Value::from_bits(0));
        let z = small(0);
        let (code, size) = match body {
            Body::Regular(e) => {
                self.words.extend_from_slice(&e);
                (Code::Regular, e.len())
            }
            Body::Array(e) => {
                self.words.extend_from_slice(&e);
                (Code::Array, e.len())
            }
            Body::WeakArray(e) => {
                self.words.extend_from_slice(&e);
                (Code::WeakArray, e.len())
            }
            Body::Bytes(b) => {
                let at = self.push_bytes(&b);
                self.words.push(at);
                (Code::Bytes, b.len())
            }
            Body::String(b) => {
                let at = self.push_bytes(&b);
                self.words.push(at);
                (Code::String, b.len())
            }
            Body::LargeInteger {
                negative,
                magnitude,
            } => {
                let at = self.push_bytes(&magnitude);
                self.words.push(at);
                let code = if negative {
                    Code::LargeNegative
                } else {
                    Code::LargePositive
                };
                (code, magnitude.len())
            }
            Body::Float(f) => {
                self.words.push(Value::from_bits(f.to_bits()));
                (Code::Float, 0)
            }
            Body::Ephemeron {
                key,
                value,
                finalizer,
            } => {
                self.words.extend_from_slice(&[key, value, finalizer]);
                (Code::Ephemeron, 3)
            }
            Body::Method(m) => {
                let (one, two) = encode_header(&m.header);
                let code = small(self.codes.len() as i64);
                self.codes.push(CodeEntry {
                    bytes: m.bytecode,
                    jit: 0,
                });
                self.words.extend_from_slice(&[
                    one, two, m.literals, m.mixin, m.selector, m.source, m.library, code,
                ]);
                (Code::Method, 8)
            }
            Body::Mixin(m) => {
                self.words.extend_from_slice(&[
                    m.name,
                    m.methods,
                    m.enclosing_mixin,
                    m.slot_names,
                    small(i64::from(m.slot_count)),
                    m.class_mixin,
                    m.library,
                ]);
                (Code::Mixin, 7)
            }
            Body::Class(b) => {
                self.words.extend_from_slice(&behavior_words(&b));
                (Code::Class, 6)
            }
            Body::Metaclass(b) => {
                self.words.extend_from_slice(&behavior_words(&b));
                (Code::Metaclass, 6)
            }
            Body::Closure(c) => {
                self.words.extend_from_slice(&[
                    c.defining,
                    small(i64::from(c.initial_ip)),
                    small(i64::from(c.num_args)),
                ]);
                self.words.extend_from_slice(&c.copied);
                (Code::Closure, C_COPIED + c.copied.len())
            }
            Body::Activation(a) => {
                let frame = a.frame.map_or(small(-1), |s| small(s as i64));
                let (detached, sender, bci, temps) = match a.detached {
                    Some(d) => (small(1), d.sender, d.bci, d.temps),
                    None => (z, z, z, z),
                };
                self.words.extend_from_slice(&[
                    frame, a.method, a.receiver, a.closure, detached, sender, bci, temps,
                ]);
                (Code::Activation, 8)
            }
            Body::FarRef { actor, target } => {
                self.words
                    .extend_from_slice(&[small(i64::from(actor)), target]);
                (Code::FarRef, 2)
            }
            Body::Promise(p) => {
                self.words.extend_from_slice(&[
                    small(i64::from(p.state)),
                    p.value,
                    p.owner,
                    p.waiters,
                    p.queue,
                ]);
                (Code::Promise, 5)
            }
        };
        self.words[o + 2] = meta(code, flags, space, size);
        self.count += 1;
        v
    }

    /// Allocates a Closure without building its description first (the
    /// interpreter's `PUSH_CLOSURE`).
    pub fn allocate_closure(
        &mut self,
        class: Value,
        space: Space,
        defining: Value,
        initial_ip: u32,
        num_args: u32,
        copied: &[Value],
    ) -> Value {
        let seq = self.next_seq;
        self.next_seq += 1;
        let o = self.words.len();
        self.words.push(class);
        self.words.push(Value::from_bits(seq));
        self.words
            .push(meta(Code::Closure, 0, space, C_COPIED + copied.len()));
        self.words.extend_from_slice(&[
            defining,
            small(i64::from(initial_ip)),
            small(i64::from(num_args)),
        ]);
        self.words.extend_from_slice(copied);
        self.count += 1;
        Value::reference(o as u64).expect("in range")
    }

    /// Allocates an Array of these elements without building its
    /// description first.
    pub fn allocate_array(&mut self, class: Value, space: Space, elements: &[Value]) -> Value {
        self.allocate_filled(class, space, Code::Array, elements.len(), |w| {
            w.extend_from_slice(elements)
        })
    }

    /// Allocates a Regular object of `n` slots, all `fill` (`basicNew`).
    pub fn allocate_regular(&mut self, class: Value, space: Space, n: usize, fill: Value) -> Value {
        self.allocate_filled(class, space, Code::Regular, n, |w| {
            w.resize(w.len() + n, fill)
        })
    }

    fn allocate_filled(
        &mut self,
        class: Value,
        space: Space,
        code: Code,
        n: usize,
        fill: impl FnOnce(&mut Vec<Value>),
    ) -> Value {
        let seq = self.next_seq;
        self.next_seq += 1;
        let o = self.words.len();
        self.words.push(class);
        self.words.push(Value::from_bits(seq));
        self.words.push(meta(code, 0, space, n));
        fill(&mut self.words);
        self.count += 1;
        Value::reference(o as u64).expect("in range")
    }

    fn push_bytes(&mut self, b: &[u8]) -> Value {
        let at = self.bytes.len();
        self.bytes.extend_from_slice(b);
        small(at as i64)
    }

    // ----- Locating objects -----

    /// The word offset of object `v`, or `None` for a SmallInteger or an
    /// offset outside the arena.
    #[inline]
    pub(crate) fn start(&self, v: Value) -> Option<usize> {
        let o = usize::try_from(v.as_reference()?).ok()?;
        if o + HEADER <= self.words.len() {
            Some(o)
        } else {
            None
        }
    }

    #[inline]
    pub(crate) fn code_at(&self, o: usize) -> Code {
        meta_code(self.words[o + 2])
    }

    /// The words object `o` takes.
    pub(crate) fn object_words(&self, o: usize) -> usize {
        let m = self.words[o + 2];
        HEADER + content_words(meta_code(m), meta_size(m))
    }

    /// The contents of object `o`.
    #[inline]
    pub(crate) fn contents(&self, o: usize) -> &[Value] {
        let m = self.words[o + 2];
        let n = content_words(meta_code(m), meta_size(m));
        &self.words[o + HEADER..o + HEADER + n]
    }

    #[inline]
    pub(crate) fn contents_mut(&mut self, o: usize) -> &mut [Value] {
        let m = self.words[o + 2];
        let n = content_words(meta_code(m), meta_size(m));
        &mut self.words[o + HEADER..o + HEADER + n]
    }

    /// The contents of `v` if its code is `code`, a pointer format (whose
    /// content words are its size).
    #[inline]
    fn contents_of(&self, v: Value, code: Code) -> Option<&[Value]> {
        let o = self.start(v)?;
        let m = self.words[o + 2];
        if meta_code(m) == code {
            let n = if code == Code::Float { 1 } else { meta_size(m) };
            self.words.get(o + HEADER..o + HEADER + n)
        } else {
            None
        }
    }

    #[inline]
    fn contents_of_mut(&mut self, v: Value, code: Code) -> Option<&mut [Value]> {
        let o = self.start(v)?;
        let m = self.words[o + 2];
        if meta_code(m) == code {
            self.remember(o);
            let n = if code == Code::Float { 1 } else { meta_size(m) };
            self.words.get_mut(o + HEADER..o + HEADER + n)
        } else {
            None
        }
    }

    /// The byte range of byte object `o`.
    pub(crate) fn byte_range(&self, o: usize) -> core::ops::Range<usize> {
        let at = small_of(self.words[o + HEADER]) as usize;
        at..at + meta_size(self.words[o + 2])
    }

    /// Every object, in arena order (allocation order but for moved
    /// objects), as a list the caller owns, so that it may change the heap
    /// while it goes through them.
    pub fn objects(&self) -> alloc::vec::IntoIter<Value> {
        let mut all = Vec::with_capacity(self.count);
        let mut o = 0;
        while o < self.words.len() {
            if self.code_at(o) != Code::Filler {
                all.push(Value::reference(o as u64).expect("in range"));
            }
            o += self.object_words(o);
        }
        all.into_iter()
    }

    /// A description of object `v` (the snapshot writer's, and tests').
    pub fn object(&self, v: Value) -> Option<Object> {
        let o = self.start(v)?;
        let m = self.words[o + 2];
        let code = meta_code(m);
        let c = self.contents(o);
        let body = match code {
            Code::Regular => Body::Regular(c.to_vec()),
            Code::Array => Body::Array(c.to_vec()),
            Code::WeakArray => Body::WeakArray(c.to_vec()),
            Code::Bytes => Body::Bytes(self.bytes[self.byte_range(o)].to_vec()),
            Code::String => Body::String(self.bytes[self.byte_range(o)].to_vec()),
            Code::LargePositive | Code::LargeNegative => Body::LargeInteger {
                negative: code == Code::LargeNegative,
                magnitude: self.bytes[self.byte_range(o)].to_vec(),
            },
            Code::Float => Body::Float(f64::from_bits(c[0].to_bits())),
            Code::Ephemeron => Body::Ephemeron {
                key: c[0],
                value: c[1],
                finalizer: c[2],
            },
            Code::Method => {
                let m = self.method(v)?;
                Body::Method(Method {
                    header: m.header(),
                    bytecode: m.bytecode().clone(),
                    literals: m.literals(),
                    mixin: m.mixin(),
                    selector: m.selector(),
                    source: m.source(),
                    library: m.library(),
                })
            }
            Code::Mixin => Body::Mixin(Mixin {
                name: c[0],
                methods: c[1],
                enclosing_mixin: c[2],
                slot_names: c[3],
                slot_count: small_of(c[MIXIN_SLOT_COUNT]) as u32,
                class_mixin: c[5],
                library: c[6],
            }),
            Code::Class => Body::Class(behavior_of(c)),
            Code::Metaclass => Body::Metaclass(behavior_of(c)),
            Code::Closure => Body::Closure(Closure {
                defining: c[0],
                initial_ip: small_of(c[C_INITIAL_IP]) as u32,
                num_args: small_of(c[C_NUM_ARGS]) as u32,
                copied: c[C_COPIED..].to_vec(),
            }),
            Code::Activation => {
                let a = ActivationRef(self, o);
                Body::Activation(Activation {
                    frame: a.frame(),
                    method: a.method(),
                    receiver: a.receiver(),
                    closure: a.closure(),
                    detached: a.detached().map(|d| Detached {
                        sender: d.sender(),
                        bci: d.bci(),
                        temps: c[A_TEMPS],
                    }),
                })
            }
            Code::FarRef => Body::FarRef {
                actor: small_of(c[0]) as u32,
                target: c[1],
            },
            Code::Promise => Body::Promise(PromiseRecord {
                state: small_of(c[0]) as u8,
                value: c[1],
                owner: c[2],
                waiters: c[3],
                queue: c[4],
            }),
            Code::Filler => return None,
        };
        Some(Object {
            header: Header {
                seq: self.words[o + 1].to_bits(),
                class: self.words[o],
                flags: meta_flags(m),
                space: meta_space(m),
            },
            body,
        })
    }

    // ----- The object API (EURASIA_ARENA_DESIGN_2026-11-19.md section 6) -----
    //
    // Outside the heap itself (this file, the collector and the snapshot
    // reader and writer), object contents are read and written only
    // through these.

    /// The format of object `v`; `None` for a SmallInteger.
    #[inline]
    pub fn format(&self, v: Value) -> Option<Format> {
        self.code_at(self.start(v)?).format()
    }

    /// The class of object `v`.
    #[inline]
    pub fn class(&self, v: Value) -> Option<Value> {
        Some(self.words[self.start(v)?])
    }

    /// Sets the class of object `v` (bootstrap and commit only).
    pub fn set_class(&mut self, v: Value, class: Value) {
        if let Some(o) = self.start(v) {
            self.remember(o);
            self.words[o] = class;
        }
    }

    /// The flags of object `v` (0 for a SmallInteger).
    #[inline]
    pub fn flags(&self, v: Value) -> u8 {
        self.start(v).map_or(0, |o| meta_flags(self.words[o + 2]))
    }

    /// Sets the flags of object `v`.
    pub fn set_flags(&mut self, v: Value, flags: u8) {
        if let Some(o) = self.start(v) {
            let m = self.words[o + 2];
            let kept = m.to_bits() & REMEMBERED;
            self.words[o + 2] = Value::from_bits(
                meta(meta_code(m), flags, meta_space(m), meta_size(m)).to_bits() | kept,
            );
        }
    }

    /// The write barrier: an old object about to be written may come to
    /// refer to a young one, so it goes into the remembered set (once).
    #[inline]
    pub(crate) fn remember(&mut self, o: usize) {
        if o < self.boundary {
            let m = self.words[o + 2].to_bits();
            if m & REMEMBERED == 0 {
                self.words[o + 2] = Value::from_bits(m | REMEMBERED);
                self.remembered.push(o);
            }
        }
    }

    /// Empties the remembered set (at a collection).
    pub(crate) fn forget_remembered(&mut self) {
        for o in core::mem::take(&mut self.remembered) {
            let m = self.words[o + 2].to_bits();
            self.words[o + 2] = Value::from_bits(m & !REMEMBERED);
        }
    }

    /// Makes everything old (a loaded image, a finished build): the next
    /// minor collection looks only at what is allocated after this.
    pub fn seal(&mut self) {
        self.forget_remembered();
        self.boundary = self.words.len();
        self.byte_boundary = self.bytes.len();
        self.code_boundary = self.codes.len();
        self.old_count = self.count;
        self.old_words_at_full = self.words.len();
    }

    /// A commit rewrote references across the heap: the next collection is
    /// full.
    pub fn require_full_collection(&mut self) {
        self.full_due = true;
    }

    /// Adds `flags` to those of object `v`.
    pub fn add_flags(&mut self, v: Value, flags: u8) {
        let old = self.flags(v);
        self.set_flags(v, old | flags);
    }

    /// The space of object `v`.
    pub fn space(&self, v: Value) -> Option<Space> {
        Some(meta_space(self.words[self.start(v)? + 2]))
    }

    /// The allocation sequence number of object `v`.
    pub fn seq(&self, v: Value) -> Option<u64> {
        Some(self.words[self.start(v)? + 1].to_bits())
    }

    /// The slots of a Regular object or the elements of an Array or
    /// WeakArray.
    pub fn fields(&self, v: Value) -> Option<&[Value]> {
        let o = self.start(v)?;
        matches!(
            self.code_at(o),
            Code::Regular | Code::Array | Code::WeakArray
        )
        .then(|| self.contents(o))
    }

    /// [`Heap::fields`], to write.
    pub fn fields_mut(&mut self, v: Value) -> Option<&mut [Value]> {
        let o = self.start(v)?;
        if matches!(
            self.code_at(o),
            Code::Regular | Code::Array | Code::WeakArray
        ) {
            self.remember(o);
            Some(self.contents_mut(o))
        } else {
            None
        }
    }

    /// The slots of a Regular object (only), for the accessors' fast path.
    #[inline]
    pub fn regular_slots(&self, v: Value) -> Option<&[Value]> {
        self.contents_of(v, Code::Regular)
    }

    /// [`Heap::regular_slots`], to write.
    #[inline]
    pub fn regular_slots_mut(&mut self, v: Value) -> Option<&mut [Value]> {
        self.contents_of_mut(v, Code::Regular)
    }

    /// The elements of an Array object.
    #[inline]
    pub fn array_elements(&self, v: Value) -> Option<&[Value]> {
        self.contents_of(v, Code::Array)
    }

    /// [`Heap::array_elements`], to write.
    #[inline]
    pub fn array_elements_mut(&mut self, v: Value) -> Option<&mut [Value]> {
        self.contents_of_mut(v, Code::Array)
    }

    /// The bytes of a String object.
    #[inline]
    pub fn string_bytes(&self, v: Value) -> Option<&[u8]> {
        let o = self.start(v)?;
        (self.code_at(o) == Code::String).then(|| &self.bytes[self.byte_range(o)])
    }

    /// The bytes of a Bytes or String object.
    pub fn bytes(&self, v: Value) -> Option<&[u8]> {
        let o = self.start(v)?;
        matches!(self.code_at(o), Code::Bytes | Code::String)
            .then(|| &self.bytes[self.byte_range(o)])
    }

    /// [`Heap::bytes`], to write.
    pub fn bytes_mut(&mut self, v: Value) -> Option<&mut [u8]> {
        let o = self.start(v)?;
        if matches!(self.code_at(o), Code::Bytes | Code::String) {
            let r = self.byte_range(o);
            Some(&mut self.bytes[r])
        } else {
            None
        }
    }

    /// The value of a Float object.
    pub fn float(&self, v: Value) -> Option<f64> {
        self.contents_of(v, Code::Float)
            .map(|c| f64::from_bits(c[0].to_bits()))
    }

    /// The sign and magnitude (little-endian bytes) of a LargeInteger.
    pub fn large_integer(&self, v: Value) -> Option<(bool, &[u8])> {
        let o = self.start(v)?;
        match self.code_at(o) {
            Code::LargePositive => Some((false, &self.bytes[self.byte_range(o)])),
            Code::LargeNegative => Some((true, &self.bytes[self.byte_range(o)])),
            _ => None,
        }
    }

    /// The key, value and finalizer of an Ephemeron.
    pub fn ephemeron(&self, v: Value) -> Option<[Value; 3]> {
        self.contents_of(v, Code::Ephemeron)
            .map(|c| [c[0], c[1], c[2]])
    }

    /// Sets field `i` (0 key, 1 value, 2 finalizer) of an Ephemeron.
    pub fn set_ephemeron_field(&mut self, v: Value, i: usize, x: Value) -> bool {
        match self.contents_of_mut(v, Code::Ephemeron) {
            Some(c) if i < 3 => {
                c[i] = x;
                true
            }
            _ => false,
        }
    }

    /// The actor and target of a far reference.
    pub fn far_ref(&self, v: Value) -> Option<(u32, Value)> {
        self.contents_of(v, Code::FarRef)
            .map(|c| (small_of(c[0]) as u32, c[1]))
    }

    /// A Method object.
    #[inline]
    pub fn method(&self, v: Value) -> Option<MethodRef<'_>> {
        self.contents_of(v, Code::Method)
            .map(|c| MethodRef(self, c))
    }

    /// A Mixin object.
    #[inline]
    pub fn mixin(&self, v: Value) -> Option<MixinRef<'_>> {
        self.contents_of(v, Code::Mixin).map(MixinRef)
    }

    /// A Closure object.
    pub fn closure(&self, v: Value) -> Option<ClosureRef<'_>> {
        self.contents_of(v, Code::Closure).map(ClosureRef)
    }

    /// An activation handle.
    pub fn activation(&self, v: Value) -> Option<ActivationRef<'_>> {
        let o = self.start(v)?;
        (self.code_at(o) == Code::Activation).then_some(ActivationRef(self, o))
    }

    /// A class or metaclass.
    #[inline]
    pub fn behavior(&self, v: Value) -> Option<BehaviorRef<'_>> {
        let o = self.start(v)?;
        matches!(self.code_at(o), Code::Class | Code::Metaclass)
            .then(|| BehaviorRef(self.contents(o)))
    }

    /// A promise record.
    pub fn promise(&self, v: Value) -> Option<PromiseRef<'_>> {
        let o = self.start(v)?;
        (self.code_at(o) == Code::Promise).then_some(PromiseRef(self, o))
    }

    // Writes to the program objects, field by field.

    /// Sets field `i` of a Method, in `vm slotOf:at:`'s order (literals,
    /// mixin, selector, source, library).
    pub fn set_method_field(&mut self, v: Value, i: usize, x: Value) -> bool {
        match self.contents_of_mut(v, Code::Method) {
            Some(c) if i < 5 => {
                c[M_LITERALS + i] = x;
                true
            }
            _ => false,
        }
    }

    /// Sets a Method's header.
    pub fn set_method_header(&mut self, v: Value, header: MethodHeader) -> bool {
        let (one, two) = encode_header(&header);
        match self.contents_of_mut(v, Code::Method) {
            Some(c) => {
                c[M_HEADER1] = one;
                c[M_HEADER2] = two;
                true
            }
            None => false,
        }
    }

    /// A Method's compiler word ([`CodeEntry::jit`]).
    #[inline]
    pub fn method_jit(&self, v: Value) -> Option<i64> {
        let c = self.contents_of(v, Code::Method)?;
        Some(self.codes[small_of(c[M_CODE]) as usize].jit)
    }

    /// Sets a Method's compiler word.
    pub fn set_method_jit(&mut self, v: Value, word: i64) {
        if let Some(c) = self.contents_of(v, Code::Method) {
            let i = small_of(c[M_CODE]) as usize;
            self.codes[i].jit = word;
        }
    }

    /// Sets a Method's bytecode.
    pub fn set_method_bytecode(&mut self, v: Value, bytecode: Rc<[u8]>) -> bool {
        let Some(c) = self.contents_of(v, Code::Method) else {
            return false;
        };
        let i = small_of(c[M_CODE]) as usize;
        self.codes[i] = CodeEntry {
            bytes: bytecode,
            jit: 0,
        };
        true
    }

    /// Sets field `i` of a Mixin, in `vm slotOf:at:`'s order (name,
    /// methods, enclosing mixin, slot names, class mixin, library); setting
    /// the slot names also sets the slot count to `slot_count`.
    pub fn set_mixin_field(&mut self, v: Value, i: usize, x: Value, slot_count: u32) -> bool {
        match self.contents_of_mut(v, Code::Mixin) {
            Some(c) if i < 6 => {
                if i == 3 {
                    c[MIXIN_SLOT_COUNT] = small(i64::from(slot_count));
                }
                // The slot count sits between the slot names and the class
                // mixin.
                c[if i < 4 { i } else { i + 1 }] = x;
                true
            }
            _ => false,
        }
    }

    /// Sets a Mixin's slot count alone (the builder's).
    pub fn set_mixin_slot_count(&mut self, v: Value, count: u32) -> bool {
        match self.contents_of_mut(v, Code::Mixin) {
            Some(c) => {
                c[MIXIN_SLOT_COUNT] = small(i64::from(count));
                true
            }
            None => false,
        }
    }

    /// Sets a class's or metaclass's `this_class` field.
    pub fn set_this_class(&mut self, v: Value, this_class: Value) -> bool {
        self.set_behavior_field(v, 4, this_class)
    }

    /// Sets field `i` of a class or metaclass (mixin, superclass, enclosing
    /// object, name, this class, library).
    pub fn set_behavior_field(&mut self, v: Value, i: usize, x: Value) -> bool {
        let Some(o) = self.start(v) else {
            return false;
        };
        if !matches!(self.code_at(o), Code::Class | Code::Metaclass) || i >= 6 {
            return false;
        }
        self.remember(o);
        self.contents_mut(o)[i] = x;
        true
    }

    /// Sets an activation handle's frame (its frame's serial, or `None`
    /// when it has none).
    #[inline]
    pub fn set_activation_frame(&mut self, v: Value, frame: Option<u64>) -> bool {
        match self.contents_of_mut(v, Code::Activation) {
            Some(c) => {
                c[A_FRAME] = frame.map_or(small(-1), |s| small(s as i64));
                true
            }
            None => false,
        }
    }

    /// Sets an activation's method (0), receiver (1) or closure (2).
    pub fn set_activation_field(&mut self, v: Value, i: usize, x: Value) -> bool {
        match self.contents_of_mut(v, Code::Activation) {
            Some(c) if i < 3 => {
                c[A_METHOD + i] = x;
                true
            }
            _ => false,
        }
    }

    /// Makes an activation handle detached, with this sender, bci and temps
    /// (a new Array of class `array_class`): it no longer names a frame.
    pub fn detach_activation(
        &mut self,
        v: Value,
        sender: Value,
        bci: Value,
        temps: Vec<Value>,
        array_class: Value,
    ) -> bool {
        let Some(space) = self.space(v) else {
            return false;
        };
        if self.format(v) != Some(Format::Activation) {
            return false;
        }
        let temps = self.allocate(array_class, space, Body::Array(temps));
        let c = self.contents_of_mut(v, Code::Activation).expect("checked");
        c[A_FRAME] = small(-1);
        c[A_DETACHED] = small(1);
        c[A_SENDER] = sender;
        c[A_BCI] = bci;
        c[A_TEMPS] = temps;
        true
    }

    /// Makes a detached activation handle live again, naming the frame
    /// `serial`, and answers its sender, bci and temps.
    pub fn attach_activation(
        &mut self,
        v: Value,
        serial: u64,
    ) -> Option<(Value, Value, Vec<Value>)> {
        let d = self.activation(v)?.detached()?;
        let state = (d.sender(), d.bci(), d.temps().to_vec());
        let c = self.contents_of_mut(v, Code::Activation)?;
        c[A_FRAME] = small(serial as i64);
        c[A_DETACHED] = small(0);
        let z = small(0);
        c[A_SENDER] = z;
        c[A_BCI] = z;
        c[A_TEMPS] = z;
        Some(state)
    }

    /// Sets a detached activation's sender.
    pub fn set_detached_sender(&mut self, v: Value, sender: Value) -> bool {
        self.set_detached_word(v, A_SENDER, sender)
    }

    /// Sets a detached activation's bci.
    pub fn set_detached_bci(&mut self, v: Value, bci: Value) -> bool {
        self.set_detached_word(v, A_BCI, bci)
    }

    fn set_detached_word(&mut self, v: Value, i: usize, x: Value) -> bool {
        match self.contents_of_mut(v, Code::Activation) {
            Some(c) if c[A_DETACHED] == small(1) => {
                c[i] = x;
                true
            }
            _ => false,
        }
    }

    /// Sets temp `i` (from 0) of a detached activation.
    pub fn set_detached_temp(&mut self, v: Value, i: usize, x: Value) -> bool {
        let Some(temps) = self
            .activation(v)
            .and_then(|a| a.detached())
            .map(|d| d.temps_array())
        else {
            return false;
        };
        match self.array_elements_mut(temps) {
            Some(e) if i < e.len() => {
                e[i] = x;
                true
            }
            _ => false,
        }
    }

    /// Gives a detached activation `n` temps: its first ones, then `fill`;
    /// a new Array of class `array_class`.
    pub fn resize_detached_temps(
        &mut self,
        v: Value,
        n: usize,
        fill: Value,
        array_class: Value,
    ) -> bool {
        let Some(d) = self.activation(v).and_then(|a| a.detached()) else {
            return false;
        };
        let mut temps = d.temps().to_vec();
        temps.resize(n, fill);
        let space = self.space(v).expect("an object");
        let array = self.allocate(array_class, space, Body::Array(temps));
        self.set_detached_word(v, A_TEMPS, array)
    }

    /// Sets a Closure's copied value `i` (from 0).
    pub fn set_closure_copied(&mut self, v: Value, i: usize, x: Value) -> bool {
        match self.contents_of_mut(v, Code::Closure) {
            Some(c) if C_COPIED + i < c.len() => {
                c[C_COPIED + i] = x;
                true
            }
            _ => false,
        }
    }

    /// Sets a promise's state and value.
    pub fn set_promise(&mut self, v: Value, state: u8, value: Value) -> bool {
        match self.contents_of_mut(v, Code::Promise) {
            Some(c) => {
                c[P_STATE] = small(i64::from(state));
                c[P_VALUE] = value;
                true
            }
            None => false,
        }
    }

    /// Adds a waiter to a promise: its waiters become a new, longer Array
    /// of class `array_class`.
    pub fn push_promise_waiter(&mut self, v: Value, waiter: Value, array_class: Value) -> bool {
        self.extend_promise_list(v, P_WAITERS, &[waiter], array_class)
    }

    /// Adds entries to a promise's queue: it becomes a new, longer Array of
    /// class `array_class`.
    pub fn push_promise_queue(&mut self, v: Value, entries: &[Value], array_class: Value) -> bool {
        self.extend_promise_list(v, P_QUEUE, entries, array_class)
    }

    fn extend_promise_list(
        &mut self,
        v: Value,
        field: usize,
        more: &[Value],
        array_class: Value,
    ) -> bool {
        let Some(c) = self.contents_of(v, Code::Promise) else {
            return false;
        };
        let mut list = self
            .array_elements(c[field])
            .map_or_else(Vec::new, <[Value]>::to_vec);
        list.extend_from_slice(more);
        let space = self.space(v).expect("an object");
        let array = self.allocate(array_class, space, Body::Array(list));
        self.contents_of_mut(v, Code::Promise).expect("checked")[field] = array;
        true
    }

    /// Takes a promise's waiters and queue, leaving it none (`nil`).
    pub fn take_promise_lists(&mut self, v: Value, nil: Value) -> Option<(Vec<Value>, Vec<Value>)> {
        let p = self.promise(v)?;
        let lists = (p.waiters().to_vec(), p.queue().to_vec());
        let c = self.contents_of_mut(v, Code::Promise)?;
        c[P_WAITERS] = nil;
        c[P_QUEUE] = nil;
        Some(lists)
    }

    /// Gives Regular object `v` class `class` and these slots (commit's
    /// reshape), keeping its sequence number, flags and space. With as many
    /// slots as before, it stays where it is; otherwise it moves to the end
    /// of the arena, its old place becomes filler, and the answer is where
    /// it is now: the caller forwards every reference to it.
    pub fn reshape(&mut self, v: Value, class: Value, slots: Vec<Value>) -> Value {
        let o = self.start(v).expect("an object");
        let m = self.words[o + 2];
        if meta_size(m) == slots.len() {
            self.words[o] = class;
            self.remember(o);
            self.contents_mut(o).copy_from_slice(&slots);
            return v;
        }
        let seq = self.words[o + 1];
        let (flags, space) = (meta_flags(m), meta_space(m));
        self.words[o + 2] = meta(Code::Filler, 0, Space::Actor(0), meta_size(m));
        let n = self.words.len();
        let moved = Value::reference(n as u64).expect("in range");
        self.words.push(class);
        self.words.push(seq);
        self.words
            .push(meta(Code::Regular, flags, space, slots.len()));
        self.words.extend(slots);
        moved
    }
}

fn behavior_words(b: &Behavior) -> Vec<Value> {
    alloc::vec![
        b.mixin,
        b.superclass,
        b.enclosing_object,
        b.name,
        b.this_class,
        b.library
    ]
}

fn behavior_of(c: &[Value]) -> Behavior {
    Behavior {
        mixin: c[0],
        superclass: c[1],
        enclosing_object: c[2],
        name: c[3],
        this_class: c[4],
        library: c[5],
    }
}

/// The format of an object: what its contents are
/// (EURASIA_ARENA_DESIGN_2026-11-19.md section 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Format {
    Regular,
    Bytes,
    String,
    Array,
    WeakArray,
    Ephemeron,
    Float,
    LargeInteger,
    Method,
    Mixin,
    Class,
    Metaclass,
    Closure,
    Activation,
    FarRef,
    Promise,
}

impl Format {
    /// Whether the contents are bytes (Bytes, String, LargeInteger).
    pub fn is_bytes(self) -> bool {
        matches!(self, Format::Bytes | Format::String | Format::LargeInteger)
    }
}

/// A Method object, read.
#[derive(Clone, Copy, Debug)]
pub struct MethodRef<'h>(&'h Heap, &'h [Value]);

#[allow(missing_docs)]
impl<'h> MethodRef<'h> {
    #[inline]
    fn c(self) -> &'h [Value] {
        self.1
    }
    pub fn header(self) -> MethodHeader {
        let c = self.c();
        decode_header(c[M_HEADER1], c[M_HEADER2])
    }
    pub fn bytecode(self) -> &'h Rc<[u8]> {
        &self.0.codes[small_of(self.c()[M_CODE]) as usize].bytes
    }
    pub fn literals(self) -> Value {
        self.c()[M_LITERALS]
    }
    pub fn mixin(self) -> Value {
        self.c()[3]
    }
    pub fn selector(self) -> Value {
        self.c()[4]
    }
    pub fn source(self) -> Value {
        self.c()[5]
    }
    pub fn library(self) -> Value {
        self.c()[6]
    }
}

/// A Mixin object, read.
#[derive(Clone, Copy, Debug)]
pub struct MixinRef<'h>(&'h [Value]);

#[allow(missing_docs)]
impl MixinRef<'_> {
    pub fn name(self) -> Value {
        self.0[0]
    }
    pub fn methods(self) -> Value {
        self.0[1]
    }
    pub fn enclosing_mixin(self) -> Value {
        self.0[2]
    }
    pub fn slot_names(self) -> Value {
        self.0[3]
    }
    pub fn slot_count(self) -> u32 {
        small_of(self.0[MIXIN_SLOT_COUNT]) as u32
    }
    pub fn class_mixin(self) -> Value {
        self.0[5]
    }
    pub fn library(self) -> Value {
        self.0[6]
    }
}

/// A class or metaclass, read.
#[derive(Clone, Copy, Debug)]
pub struct BehaviorRef<'h>(&'h [Value]);

#[allow(missing_docs)]
impl BehaviorRef<'_> {
    pub fn mixin(self) -> Value {
        self.0[0]
    }
    pub fn superclass(self) -> Value {
        self.0[1]
    }
    pub fn enclosing_object(self) -> Value {
        self.0[2]
    }
    pub fn name(self) -> Value {
        self.0[3]
    }
    pub fn this_class(self) -> Value {
        self.0[4]
    }
    pub fn library(self) -> Value {
        self.0[5]
    }
}

/// A Closure object, read.
#[derive(Clone, Copy, Debug)]
pub struct ClosureRef<'h>(&'h [Value]);

#[allow(missing_docs)]
impl<'h> ClosureRef<'h> {
    pub fn defining(self) -> Value {
        self.0[0]
    }
    pub fn initial_ip(self) -> u32 {
        small_of(self.0[C_INITIAL_IP]) as u32
    }
    pub fn num_args(self) -> u32 {
        small_of(self.0[C_NUM_ARGS]) as u32
    }
    pub fn copied(self) -> &'h [Value] {
        &self.0[C_COPIED..]
    }
}

/// An activation handle, read.
#[derive(Clone, Copy, Debug)]
pub struct ActivationRef<'h>(&'h Heap, usize);

#[allow(missing_docs)]
impl<'h> ActivationRef<'h> {
    fn c(self) -> &'h [Value] {
        self.0.contents(self.1)
    }
    pub fn frame(self) -> Option<u64> {
        let f = small_of(self.c()[A_FRAME]);
        (f >= 0).then_some(f as u64)
    }
    pub fn method(self) -> Value {
        self.c()[A_METHOD]
    }
    pub fn receiver(self) -> Value {
        self.c()[A_RECEIVER]
    }
    pub fn closure(self) -> Value {
        self.c()[A_CLOSURE]
    }
    pub fn detached(self) -> Option<DetachedRef<'h>> {
        (self.c()[A_DETACHED] == small(1)).then_some(DetachedRef(self.0, self.1))
    }
}

/// A detached activation's own state, read.
#[derive(Clone, Copy, Debug)]
pub struct DetachedRef<'h>(&'h Heap, usize);

#[allow(missing_docs)]
impl<'h> DetachedRef<'h> {
    fn c(self) -> &'h [Value] {
        self.0.contents(self.1)
    }
    pub fn sender(self) -> Value {
        self.c()[A_SENDER]
    }
    pub fn bci(self) -> Value {
        self.c()[A_BCI]
    }
    /// The temps Array (or nil).
    pub fn temps_array(self) -> Value {
        self.c()[A_TEMPS]
    }
    pub fn temps(self) -> &'h [Value] {
        self.0.array_elements(self.temps_array()).unwrap_or(&[])
    }
}

/// A promise record, read.
#[derive(Clone, Copy, Debug)]
pub struct PromiseRef<'h>(&'h Heap, usize);

#[allow(missing_docs)]
impl<'h> PromiseRef<'h> {
    fn c(self) -> &'h [Value] {
        self.0.contents(self.1)
    }
    pub fn state(self) -> u8 {
        small_of(self.c()[P_STATE]) as u8
    }
    pub fn value(self) -> Value {
        self.c()[P_VALUE]
    }
    pub fn owner(self) -> Value {
        self.c()[2]
    }
    pub fn waiters(self) -> &'h [Value] {
        self.0.array_elements(self.c()[P_WAITERS]).unwrap_or(&[])
    }
    pub fn queue(self) -> &'h [Value] {
        self.0.array_elements(self.c()[P_QUEUE]).unwrap_or(&[])
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
        assert_eq!(h.seq(a).unwrap(), 0);
        assert_eq!(h.seq(b).unwrap(), 1);
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
        assert_eq!(h.seq(a).unwrap(), 5);
        assert_eq!(h.flags(b), FROZEN);
        assert_eq!(
            h.insert_loaded(9, nil(), 0, Space::Program, Body::Regular(Vec::new()))
                .unwrap_err(),
            HeapError::SequenceNotIncreasing { seq: 9 }
        );
        h.set_allocation_counter(20).unwrap();
        let c = h.allocate(nil(), Space::Actor(0), Body::Regular(Vec::new()));
        assert_eq!(h.seq(c).unwrap(), 20);
        assert!(h.set_allocation_counter(20).is_err());
    }

    #[test]
    fn a_reshape_that_grows_moves_the_object_and_keeps_its_identity() {
        let mut h = Heap::new();
        let n = h.allocate(nil(), Space::Actor(0), Body::Regular(Vec::new()));
        let one = Value::small_integer(1).unwrap();
        let a = h.allocate(nil(), Space::Actor(3), Body::Regular(alloc::vec![one]));
        h.add_flags(a, FROZEN);
        let after = h.allocate(nil(), Space::Actor(0), Body::Regular(alloc::vec![a]));
        let seq = h.seq(a);
        // Same size: in place.
        assert_eq!(h.reshape(a, n, alloc::vec![one]), a);
        // Larger: moved, with the same sequence number, flags and space.
        let moved = h.reshape(a, n, alloc::vec![one, one, one]);
        assert_ne!(moved, a);
        assert_eq!(h.seq(moved), seq);
        assert_eq!(h.flags(moved), FROZEN);
        assert_eq!(h.space(moved), Some(Space::Actor(3)));
        assert_eq!(h.regular_slots(moved).unwrap().len(), 3);
        // The old place is filler: not an object any more.
        assert!(h.format(a).is_none());
        assert_eq!(h.objects().len(), 3);
        // A reference forwarded to the new place survives a collection.
        h.regular_slots_mut(after).unwrap()[0] = moved;
        let (fwd, stats) = h.collect(&[after], n);
        assert_eq!((stats.before, stats.after), (3, 3));
        let after = fwd.forward(after);
        let moved = h.regular_slots(after).unwrap()[0];
        assert_eq!(h.seq(moved), seq);
        assert_eq!(h.regular_slots(moved).unwrap(), &[one, one, one]);
    }

    #[test]
    fn typed_accessors_answer_none_for_other_kinds() {
        let mut h = Heap::new();
        let s = h.allocate(nil(), Space::Actor(0), Body::String(b"abc".to_vec()));
        assert_eq!(h.string_bytes(s), Some(&b"abc"[..]));
        assert!(h.array_elements(s).is_none());
        assert!(h.method(s).is_none());
        assert!(h.format(Value::small_integer(3).unwrap()).is_none());
    }
}
