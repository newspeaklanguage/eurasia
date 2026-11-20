// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Whole images: sections, nodes and edges (format sections 3-8).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::fmt;

use eurasia_core::Value;
use eurasia_core::machine::{ActorState, FrameState, MachineState};
use eurasia_core::object::Detached;
use eurasia_core::object::{
    Access, Activation, Behavior, Body, Closure, FLAGS, Format, Heap, HeapError, Method,
    MethodHeader, MethodKind, Mixin, Object, PromiseRecord, Space,
};
use eurasia_core::roles::{Role, Roles, RolesError};

use crate::encoding::{DecodeError, Reader, Writer};
use crate::header::{Header, HeaderError};

const SECTION_ROLES: u8 = 1;
const SECTION_PROGRAM: u8 = 2;
const SECTION_ACTOR: u8 = 3;
const SECTION_MACHINE: u8 = 4;

const KIND_REGULAR: u8 = 1;
const KIND_BYTES: u8 = 2;
const KIND_STRING: u8 = 3;
const KIND_ARRAY: u8 = 4;
const KIND_WEAK_ARRAY: u8 = 5;
const KIND_EPHEMERON: u8 = 6;
const KIND_FLOAT: u8 = 7;
const KIND_LARGE_INTEGER: u8 = 8;
const KIND_METHOD: u8 = 9;
const KIND_MIXIN: u8 = 10;
const KIND_CLASS: u8 = 11;
const KIND_METACLASS: u8 = 12;
const KIND_CLOSURE: u8 = 13;
const KIND_ACTIVATION: u8 = 14;
const KIND_FAR_REFERENCE: u8 = 15;
const KIND_PROMISE: u8 = 16;

/// Why an image could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    /// The header was refused.
    Header(HeaderError),
    /// The bytes could not be decoded.
    Decode(DecodeError),
    /// A section kind this format version does not define.
    UnknownSection(u8),
    /// The roles section is missing or not last.
    RolesNotLast,
    /// Node `node` has an unknown kind.
    UnknownKind {
        /// The node number.
        node: u64,
        /// The kind byte.
        kind: u8,
    },
    /// Node `node` has unknown flags.
    UnknownFlags {
        /// The node number.
        node: u64,
        /// The flags byte.
        flags: u8,
    },
    /// Node `node`'s method header is inconsistent with its kind.
    BadMethodHeader {
        /// The node number.
        node: u64,
    },
    /// A reference names no node.
    DanglingReference {
        /// The node whose edges hold the reference (0 for the roles section).
        node: u64,
        /// The referenced node number.
        target: u64,
    },
    /// Sequence numbers are not increasing within a section, not unique, or
    /// not below the allocation counter.
    BadSequence {
        /// The node number.
        node: u64,
    },
    /// The header's node or section count does not match the file.
    CountMismatch,
    /// Bytes remain after the last section.
    TrailingBytes,
    /// The role table was refused.
    Roles(RolesError),
    /// The machine section is malformed, or given twice.
    BadMachineState,
}

impl From<DecodeError> for LoadError {
    fn from(e: DecodeError) -> LoadError {
        LoadError::Decode(e)
    }
}

impl From<HeaderError> for LoadError {
    fn from(e: HeaderError) -> LoadError {
        LoadError::Header(e)
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Header(e) => write!(f, "{e}"),
            LoadError::Decode(e) => write!(f, "{e}"),
            LoadError::UnknownSection(k) => write!(f, "unknown section kind {k}"),
            LoadError::RolesNotLast => write!(f, "the roles section must be present and last"),
            LoadError::UnknownKind { node, kind } => write!(f, "node {node}: unknown kind {kind}"),
            LoadError::UnknownFlags { node, flags } => {
                write!(f, "node {node}: unknown flags {flags:#04x}")
            }
            LoadError::BadMethodHeader { node } => {
                write!(f, "node {node}: inconsistent method header")
            }
            LoadError::DanglingReference { node, target } => {
                write!(f, "node {node}: reference to missing node {target}")
            }
            LoadError::BadSequence { node } => write!(f, "node {node}: bad sequence number"),
            LoadError::CountMismatch => {
                write!(f, "node or section count does not match the header")
            }
            LoadError::TrailingBytes => write!(f, "bytes after the last section"),
            LoadError::Roles(e) => write!(f, "roles: {e:?}"),
            LoadError::BadMachineState => write!(f, "malformed machine state"),
        }
    }
}

// ----- Writing -----

/// Writes an image. Objects keep their sequence numbers; the program space
/// is written first, then each actor heap in actor order, then the machine
/// state of a checkpoint, then the roles. Without a machine state, live
/// activations are saved dead (a snapshot starts afresh).
pub fn write_image(heap: &Heap, roles: &Roles, machine: Option<&MachineState>) -> Vec<u8> {
    // Group objects by space, each group in allocation order (sequence
    // order: a reshaped object can sit later in the arena than its number).
    let mut groups: BTreeMap<(u8, u32), Vec<(Value, Object)>> = BTreeMap::new();
    for v in heap.objects() {
        let o = heap.object(v).expect("an object");
        let key = match o.header.space {
            Space::Program => (0, 0),
            Space::Actor(n) => (1, n),
        };
        groups.entry(key).or_default().push((v, o));
    }
    for objects in groups.values_mut() {
        objects.sort_by_key(|(_, o)| o.header.seq);
    }
    // Node numbers in file order.
    let mut node_of: BTreeMap<u64, u64> = BTreeMap::new();
    let mut next = 1;
    for objects in groups.values() {
        for (v, _) in objects {
            node_of.insert(v.as_reference().expect("reference"), next);
            next += 1;
        }
    }
    let to_node = |v: Value| -> Value {
        match v.as_reference() {
            Some(i) => Value::reference(node_of[&i]).expect("node number in range"),
            None => v,
        }
    };

    let mut w = Writer::new();
    let sections = groups.len() as u64 + 1 + u64::from(machine.is_some());
    Header::current(heap.allocation_counter(), next - 1, sections).write(&mut w);
    for (&(space_kind, actor), objects) in &groups {
        w.u8(if space_kind == 0 {
            SECTION_PROGRAM
        } else {
            SECTION_ACTOR
        });
        w.uv(u64::from(actor));
        w.uv(objects.len() as u64);
        let mut previous = None;
        for (_, o) in objects {
            w.uv(match previous {
                None => o.header.seq,
                Some(p) => o.header.seq - p,
            });
            previous = Some(o.header.seq);
            write_node(&mut w, o, machine.is_some());
        }
        for (_, o) in objects {
            write_edges(&mut w, o, &to_node);
        }
    }
    if let Some(m) = machine {
        w.u8(SECTION_MACHINE);
        w.uv(0);
        w.uv(0);
        write_machine(&mut w, m, &to_node);
    }
    w.u8(SECTION_ROLES);
    w.uv(0);
    w.uv(0);
    w.uv(Role::ALL.len() as u64);
    for (role, value) in roles.pairs() {
        w.uv(role as u64);
        w.value(to_node(value));
    }
    w.into_bytes()
}

fn write_machine(w: &mut Writer, m: &MachineState, to_node: &dyn Fn(Value) -> Value) {
    let opt = |w: &mut Writer, v: Option<Value>| match v {
        Some(v) => {
            w.u8(1);
            w.value(to_node(v));
        }
        None => w.u8(0),
    };
    w.uv(m.stack.len() as u64);
    m.stack.iter().for_each(|&v| w.value(to_node(v)));
    w.uv(m.frames.len() as u64);
    for f in &m.frames {
        w.uv(f.serial);
        w.value(to_node(f.method));
        w.value(to_node(f.receiver));
        w.uv(f.bp);
        w.uv(f.arg_count);
        w.uv(f.lp);
        w.uv(f.ip);
        opt(w, f.activation);
        opt(w, f.closure);
    }
    w.uv(m.actors.len() as u64);
    for a in &m.actors {
        w.value(to_node(a.message_loop));
        w.uv(a.mailbox.len() as u64);
        a.mailbox.iter().for_each(|&v| w.value(to_node(v)));
        w.uv(a.wakeup);
        w.u8(u8::from(a.poked));
    }
    w.uv(m.current);
    w.uv(m.expats.len() as u64);
    for &(c, owner) in &m.expats {
        w.value(to_node(c));
        w.uv(owner.into());
    }
    w.uv(m.symbols.len() as u64);
    for (name, v) in &m.symbols {
        w.uv(name.len() as u64);
        w.bytes(name.as_bytes());
        w.value(to_node(*v));
    }
    for n in [
        m.next_serial,
        m.gc_counter,
        m.gc_budget,
        m.collections,
        m.program_version,
        m.checkpoint_every,
        m.last_checkpoint,
    ] {
        w.uv(n);
    }
    match m.finished_turn {
        Some(t) => {
            w.u8(1);
            w.uv(t);
        }
        None => w.u8(0),
    }
    w.u8(u8::from(m.has_loop));
}

/// Reads a machine section's body; its values are still node numbers.
fn read_machine(r: &mut Reader<'_>) -> Result<MachineState, LoadError> {
    let n = |r: &mut Reader<'_>| -> Result<usize, LoadError> {
        usize::try_from(r.uv()?).map_err(|_| LoadError::BadMachineState)
    };
    let opt = |r: &mut Reader<'_>| -> Result<Option<Value>, LoadError> {
        Ok(match r.u8()? {
            0 => None,
            1 => Some(r.value()?),
            _ => return Err(LoadError::BadMachineState),
        })
    };
    let flag = |r: &mut Reader<'_>| -> Result<bool, LoadError> {
        match r.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LoadError::BadMachineState),
        }
    };
    let mut stack = Vec::new();
    for _ in 0..n(r)? {
        stack.push(r.value()?);
    }
    let mut frames = Vec::new();
    for _ in 0..n(r)? {
        frames.push(FrameState {
            serial: r.uv()?,
            method: r.value()?,
            receiver: r.value()?,
            bp: r.uv()?,
            arg_count: r.uv()?,
            lp: r.uv()?,
            ip: r.uv()?,
            activation: opt(r)?,
            closure: opt(r)?,
        });
    }
    let mut actors = Vec::new();
    for _ in 0..n(r)? {
        let message_loop = r.value()?;
        let mut mailbox = Vec::new();
        for _ in 0..n(r)? {
            mailbox.push(r.value()?);
        }
        actors.push(ActorState {
            message_loop,
            mailbox,
            wakeup: r.uv()?,
            poked: flag(r)?,
        });
    }
    let current = r.uv()?;
    let mut expats = Vec::new();
    for _ in 0..n(r)? {
        let c = r.value()?;
        let owner = u32::try_from(r.uv()?).map_err(|_| LoadError::BadMachineState)?;
        expats.push((c, owner));
    }
    let mut symbols = Vec::new();
    for _ in 0..n(r)? {
        let len = n(r)?;
        let name = core::str::from_utf8(r.bytes(len)?).map_err(|_| LoadError::BadMachineState)?;
        symbols.push((alloc::string::String::from(name), r.value()?));
    }
    let mut c = [0u64; 7];
    for x in &mut c {
        *x = r.uv()?;
    }
    let finished_turn = if flag(r)? { Some(r.uv()?) } else { None };
    let has_loop = flag(r)?;
    Ok(MachineState {
        stack,
        frames,
        actors,
        current,
        expats,
        symbols,
        next_serial: c[0],
        gc_counter: c[1],
        gc_budget: c[2],
        collections: c[3],
        program_version: c[4],
        checkpoint_every: c[5],
        last_checkpoint: c[6],
        finished_turn,
        has_loop,
    })
}

/// Resolves a machine state's node numbers to arena references.
fn resolve_machine(
    m: MachineState,
    resolve: &dyn Fn(Value) -> Result<Value, LoadError>,
) -> Result<MachineState, LoadError> {
    let opt = |v: Option<Value>| v.map(resolve).transpose();
    let all =
        |vs: Vec<Value>| -> Result<Vec<Value>, LoadError> { vs.into_iter().map(resolve).collect() };
    Ok(MachineState {
        stack: all(m.stack)?,
        frames: m
            .frames
            .into_iter()
            .map(|f| {
                Ok(FrameState {
                    method: resolve(f.method)?,
                    receiver: resolve(f.receiver)?,
                    activation: opt(f.activation)?,
                    closure: opt(f.closure)?,
                    ..f
                })
            })
            .collect::<Result<_, LoadError>>()?,
        actors: m
            .actors
            .into_iter()
            .map(|a| {
                Ok(ActorState {
                    message_loop: resolve(a.message_loop)?,
                    mailbox: all(a.mailbox)?,
                    ..a
                })
            })
            .collect::<Result<_, LoadError>>()?,
        expats: m
            .expats
            .into_iter()
            .map(|(c, o)| Ok((resolve(c)?, o)))
            .collect::<Result<_, LoadError>>()?,
        symbols: m
            .symbols
            .into_iter()
            .map(|(k, v)| Ok((k, resolve(v)?)))
            .collect::<Result<_, LoadError>>()?,
        ..m
    })
}

fn write_node(w: &mut Writer, o: &Object, keep_frames: bool) {
    let kind = match &o.body {
        Body::Regular(_) => KIND_REGULAR,
        Body::Bytes(_) => KIND_BYTES,
        Body::String(_) => KIND_STRING,
        Body::Array(_) => KIND_ARRAY,
        Body::WeakArray(_) => KIND_WEAK_ARRAY,
        Body::Ephemeron { .. } => KIND_EPHEMERON,
        Body::Float(_) => KIND_FLOAT,
        Body::LargeInteger { .. } => KIND_LARGE_INTEGER,
        Body::Method(_) => KIND_METHOD,
        Body::Mixin(_) => KIND_MIXIN,
        Body::Class(_) => KIND_CLASS,
        Body::Metaclass(_) => KIND_METACLASS,
        Body::Closure(_) => KIND_CLOSURE,
        Body::Activation(_) => KIND_ACTIVATION,
        Body::FarRef { .. } => KIND_FAR_REFERENCE,
        Body::Promise(_) => KIND_PROMISE,
    };
    w.u8(kind);
    w.u8(o.header.flags);
    match &o.body {
        Body::Regular(slots) => w.uv(slots.len() as u64),
        Body::Bytes(b) | Body::String(b) => {
            w.uv(b.len() as u64);
            w.bytes(b);
        }
        Body::Array(e) | Body::WeakArray(e) => w.uv(e.len() as u64),
        Body::Ephemeron { .. } | Body::Class(_) | Body::Metaclass(_) => {}
        Body::Float(f) => {
            let bits = if f.is_nan() {
                f64::NAN.to_bits()
            } else {
                f.to_bits()
            };
            w.bytes(&bits.to_le_bytes());
        }
        Body::LargeInteger {
            negative,
            magnitude,
        } => {
            w.u8(u8::from(*negative));
            w.uv(magnitude.len() as u64);
            w.bytes(magnitude);
        }
        Body::Method(m) => {
            let h = &m.header;
            w.u8(match h.kind {
                MethodKind::Ordinary => 0,
                MethodKind::Native => 1,
                MethodKind::Getter => 2,
                MethodKind::Setter => 3,
                MethodKind::ReturnSelf => 4,
            });
            w.u8(match h.access {
                Access::Public => 0,
                Access::Protected => 1,
                Access::Private => 2,
            });
            w.uv(h.arg_count.into());
            w.uv(h.local_count.into());
            match h.kind {
                MethodKind::Native => w.uv(h.native.into()),
                MethodKind::Getter | MethodKind::Setter => w.uv(h.slot.into()),
                _ => {}
            }
            w.uv(m.bytecode.len() as u64);
            w.bytes(&m.bytecode);
        }
        Body::Mixin(m) => w.uv(m.slot_count.into()),
        Body::Closure(c) => {
            w.uv(c.initial_ip.into());
            w.uv(c.num_args.into());
            w.uv(c.copied.len() as u64);
        }
        // A frame's handle is saved live only with the frames (a
        // checkpoint); a detached activation with its state.
        Body::Activation(a) => match (&a.detached, a.frame) {
            (Some(_), _) => w.u8(2),
            (None, Some(serial)) if keep_frames => {
                w.u8(1);
                w.uv(serial);
            }
            (None, _) => w.u8(0),
        },
        Body::FarRef { actor, .. } => w.uv((*actor).into()),
        Body::Promise(p) => w.uv(p.state.into()),
    }
}

fn write_edges(w: &mut Writer, o: &Object, to_node: &dyn Fn(Value) -> Value) {
    let mut v = |x: Value| w.value(to_node(x));
    v(o.header.class);
    match &o.body {
        Body::Regular(e) | Body::Array(e) | Body::WeakArray(e) => e.iter().for_each(|&x| v(x)),
        Body::Ephemeron {
            key,
            value,
            finalizer,
        } => {
            v(*key);
            v(*value);
            v(*finalizer);
        }
        Body::Method(m) => [m.literals, m.mixin, m.selector, m.source, m.library]
            .into_iter()
            .for_each(v),
        Body::Mixin(m) => [
            m.name,
            m.methods,
            m.enclosing_mixin,
            m.slot_names,
            m.class_mixin,
            m.library,
        ]
        .into_iter()
        .for_each(v),
        Body::Class(b) => [b.mixin, b.superclass, b.enclosing_object, b.name, b.library]
            .into_iter()
            .for_each(v),
        Body::Metaclass(b) => [
            b.mixin,
            b.superclass,
            b.enclosing_object,
            b.this_class,
            b.library,
        ]
        .into_iter()
        .for_each(v),
        Body::Closure(c) => {
            v(c.defining);
            c.copied.iter().for_each(|&x| v(x));
        }
        Body::Activation(a) => {
            [a.method, a.receiver, a.closure]
                .into_iter()
                .for_each(&mut v);
            if let Some(d) = &a.detached {
                v(d.sender);
                v(d.bci);
                v(d.temps);
            }
        }
        Body::FarRef { target, .. } => v(*target),
        Body::Promise(p) => [p.value, p.owner, p.waiters, p.queue]
            .into_iter()
            .for_each(v),
        Body::Bytes(_) | Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => {}
    }
}

// ----- Reading -----

struct Pending {
    node: u64,
    seq: u64,
    flags: u8,
    space: Space,
    body: Body,
}

/// Reads an image, checking everything section 8 of the format requires.
/// A checkpoint's machine state is ignored: the image starts afresh.
pub fn read_image(bytes: &[u8]) -> Result<(Heap, Roles), LoadError> {
    read_checkpoint(bytes).map(|(heap, roles, _)| (heap, roles))
}

/// Reads an image with its machine state, if it is a checkpoint (M7):
/// [`eurasia_core::Vm::restore`] continues it.
pub fn read_checkpoint(bytes: &[u8]) -> Result<(Heap, Roles, Option<MachineState>), LoadError> {
    let mut r = Reader::new(bytes);
    let header = Header::read(&mut r)?;
    let placeholder = Value::small_integer(0).expect("small");
    let mut nodes: Vec<Pending> = Vec::new();
    // Edges are read after all nodes of a section; keep (node index, edge values).
    let mut edges: Vec<Vec<Value>> = Vec::new();
    let mut role_pairs: Option<Vec<(u64, Value)>> = None;
    let mut machine: Option<MachineState> = None;
    for _ in 0..header.section_count {
        if role_pairs.is_some() {
            return Err(LoadError::RolesNotLast);
        }
        let kind = r.u8()?;
        let owner = r.uv()?;
        let count = r.uv()?;
        let space = match kind {
            SECTION_ROLES => {
                if count != 0 {
                    return Err(LoadError::RolesNotLast);
                }
                let n = r.uv()?;
                let mut pairs = Vec::new();
                for _ in 0..n {
                    pairs.push((r.uv()?, r.value()?));
                }
                role_pairs = Some(pairs);
                continue;
            }
            SECTION_MACHINE => {
                if count != 0 || machine.is_some() {
                    return Err(LoadError::BadMachineState);
                }
                machine = Some(read_machine(&mut r)?);
                continue;
            }
            SECTION_PROGRAM => Space::Program,
            SECTION_ACTOR => {
                Space::Actor(u32::try_from(owner).map_err(|_| LoadError::UnknownSection(kind))?)
            }
            other => return Err(LoadError::UnknownSection(other)),
        };
        let first = nodes.len();
        let mut previous: Option<u64> = None;
        for _ in 0..count {
            let node = nodes.len() as u64 + 1;
            let delta = r.uv()?;
            let seq = match previous {
                None => delta,
                Some(p) => {
                    if delta == 0 {
                        return Err(LoadError::BadSequence { node });
                    }
                    p.checked_add(delta)
                        .ok_or(LoadError::BadSequence { node })?
                }
            };
            previous = Some(seq);
            let (flags, body) = read_node(&mut r, node, placeholder)?;
            nodes.push(Pending {
                node,
                seq,
                flags,
                space,
                body,
            });
        }
        for pending in &nodes[first..] {
            edges.push(read_edges(&mut r, pending)?);
        }
    }
    if nodes.len() as u64 != header.node_count {
        return Err(LoadError::CountMismatch);
    }
    let role_pairs = role_pairs.ok_or(LoadError::RolesNotLast)?;
    if !r.is_at_end() {
        return Err(LoadError::TrailingBytes);
    }

    // Allocation order: sort by sequence number, then map node numbers to
    // the arena offsets the objects will have, in that order.
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by_key(|&i| nodes[i].seq);
    let mut index_of_node = alloc::vec![0u64; nodes.len()];
    let mut offset = 0u64;
    for &i in &order {
        index_of_node[i] = offset;
        offset += Heap::words_for(&nodes[i].body) as u64;
    }
    let resolve = |node: u64, v: Value| -> Result<Value, LoadError> {
        match v.as_reference() {
            None => Ok(v),
            Some(target) => {
                if target == 0 || target > index_of_node.len() as u64 {
                    return Err(LoadError::DanglingReference { node, target });
                }
                Ok(Value::reference(index_of_node[target as usize - 1]).expect("in range"))
            }
        }
    };

    let mut heap = Heap::new();
    let mut taken: Vec<Option<Pending>> = nodes.into_iter().map(Some).collect();
    for &i in &order {
        let p = taken[i].take().expect("each node once");
        let e = &edges[i];
        let class = resolve(p.node, e[0])?;
        let body = fill_body(p.body, &e[1..], &|v| resolve(p.node, v))?;
        heap.insert_loaded(p.seq, class, p.flags, p.space, body)
            .map_err(|_: HeapError| LoadError::BadSequence { node: p.node })?;
    }
    heap.set_allocation_counter(header.allocation_counter)
        .map_err(|_| LoadError::BadSequence { node: 0 })?;

    let mut pairs = Vec::new();
    for (id, v) in role_pairs {
        let role = Role::from_id(id).ok_or(LoadError::Roles(RolesError::UnknownRole(id)))?;
        pairs.push((role, resolve(0, v)?));
    }
    let roles = Roles::new(&pairs).map_err(LoadError::Roles)?;
    let machine = match machine {
        Some(m) => Some(resolve_machine(m, &|v| resolve(0, v))?),
        None => None,
    };
    // Fields the format does not store (a class's this_class, a metaclass's
    // name) are nil.
    let nil = roles.get(Role::Nil);
    for v in heap.objects() {
        match heap.format(v) {
            Some(Format::Class) => {
                heap.set_this_class(v, nil);
            }
            Some(Format::Metaclass) => {
                heap.set_behavior_field(v, 3, nil);
            }
            _ => {}
        }
    }
    // A loaded image is old: the first minor collection looks only at what
    // runs after it.
    heap.seal();
    Ok((heap, roles, machine))
}

fn read_node(r: &mut Reader<'_>, node: u64, placeholder: Value) -> Result<(u8, Body), LoadError> {
    let kind = r.u8()?;
    let flags = r.u8()?;
    if flags & !FLAGS != 0 {
        return Err(LoadError::UnknownFlags { node, flags });
    }
    let len = |r: &mut Reader<'_>| -> Result<usize, LoadError> {
        usize::try_from(r.uv()?)
            .map_err(|_| LoadError::Decode(DecodeError::UnexpectedEnd { offset: r.offset() }))
    };
    let p = placeholder;
    let body = match kind {
        KIND_REGULAR => Body::Regular(alloc::vec![p; len(r)?]),
        KIND_BYTES => {
            let n = len(r)?;
            Body::Bytes(r.bytes(n)?.to_vec())
        }
        KIND_STRING => {
            let n = len(r)?;
            Body::String(r.bytes(n)?.to_vec())
        }
        KIND_ARRAY => Body::Array(alloc::vec![p; len(r)?]),
        KIND_WEAK_ARRAY => Body::WeakArray(alloc::vec![p; len(r)?]),
        KIND_EPHEMERON => Body::Ephemeron {
            key: p,
            value: p,
            finalizer: p,
        },
        KIND_FLOAT => {
            let b = r.bytes(8)?;
            Body::Float(f64::from_le_bytes(b.try_into().expect("8 bytes")))
        }
        KIND_LARGE_INTEGER => {
            let negative = r.u8()? != 0;
            let n = len(r)?;
            Body::LargeInteger {
                negative,
                magnitude: r.bytes(n)?.to_vec(),
            }
        }
        KIND_METHOD => {
            let kind = match r.u8()? {
                0 => MethodKind::Ordinary,
                1 => MethodKind::Native,
                2 => MethodKind::Getter,
                3 => MethodKind::Setter,
                4 => MethodKind::ReturnSelf,
                _ => return Err(LoadError::BadMethodHeader { node }),
            };
            let access = match r.u8()? {
                0 => Access::Public,
                1 => Access::Protected,
                2 => Access::Private,
                _ => return Err(LoadError::BadMethodHeader { node }),
            };
            let small = |r: &mut Reader<'_>| -> Result<u32, LoadError> {
                u32::try_from(r.uv()?).map_err(|_| LoadError::BadMethodHeader { node })
            };
            let arg_count = small(r)?;
            let local_count = small(r)?;
            let (native, slot) = match kind {
                MethodKind::Native => (small(r)?, 0),
                MethodKind::Getter | MethodKind::Setter => (0, small(r)?),
                _ => (0, 0),
            };
            let n = len(r)?;
            let bytecode: alloc::rc::Rc<[u8]> = r.bytes(n)?.into();
            let consistent = match kind {
                MethodKind::Ordinary => !bytecode.is_empty(),
                MethodKind::Getter => bytecode.is_empty() && arg_count == 0,
                MethodKind::Setter => bytecode.is_empty() && arg_count == 1,
                MethodKind::Native | MethodKind::ReturnSelf => bytecode.is_empty(),
            };
            if !consistent {
                return Err(LoadError::BadMethodHeader { node });
            }
            Body::Method(Method {
                header: MethodHeader {
                    kind,
                    access,
                    arg_count,
                    local_count,
                    native,
                    slot,
                },
                bytecode,
                literals: p,
                mixin: p,
                selector: p,
                source: p,
                library: p,
            })
        }
        KIND_MIXIN => Body::Mixin(Mixin {
            name: p,
            methods: p,
            enclosing_mixin: p,
            slot_names: p,
            slot_count: u32::try_from(r.uv()?)
                .map_err(|_| LoadError::UnknownKind { node, kind })?,
            class_mixin: p,
            library: p,
        }),
        KIND_CLASS | KIND_METACLASS => {
            let b = Behavior {
                mixin: p,
                superclass: p,
                enclosing_object: p,
                name: p,
                this_class: p,
                library: p,
            };
            if kind == KIND_CLASS {
                Body::Class(b)
            } else {
                Body::Metaclass(b)
            }
        }
        KIND_CLOSURE => {
            let initial_ip =
                u32::try_from(r.uv()?).map_err(|_| LoadError::UnknownKind { node, kind })?;
            let num_args =
                u32::try_from(r.uv()?).map_err(|_| LoadError::UnknownKind { node, kind })?;
            let copied = len(r)?;
            Body::Closure(Closure {
                defining: p,
                initial_ip,
                num_args,
                copied: alloc::vec![p; copied],
            })
        }
        KIND_ACTIVATION => {
            let (frame, detached) = match r.u8()? {
                0 => (None, None),
                1 => (Some(r.uv()?), None),
                2 => (
                    None,
                    Some(Detached {
                        sender: p,
                        bci: p,
                        temps: p,
                    }),
                ),
                _ => return Err(LoadError::UnknownKind { node, kind }),
            };
            Body::Activation(Activation {
                frame,
                method: p,
                receiver: p,
                closure: p,
                detached,
            })
        }
        KIND_FAR_REFERENCE => Body::FarRef {
            actor: u32::try_from(r.uv()?).map_err(|_| LoadError::UnknownKind { node, kind })?,
            target: p,
        },
        KIND_PROMISE => {
            let state = u8::try_from(r.uv()?)
                .ok()
                .filter(|&s| s <= PromiseRecord::FORWARDED)
                .ok_or(LoadError::UnknownKind { node, kind })?;
            Body::Promise(PromiseRecord {
                state,
                value: p,
                owner: p,
                waiters: p,
                queue: p,
            })
        }
        _ => return Err(LoadError::UnknownKind { node, kind }),
    };
    Ok((flags, body))
}

fn edge_count(body: &Body) -> usize {
    1 + match body {
        Body::Regular(e) | Body::Array(e) | Body::WeakArray(e) => e.len(),
        Body::Ephemeron { .. } => 3,
        Body::Mixin(_) => 6,
        Body::Method(_) | Body::Class(_) | Body::Metaclass(_) => 5,
        Body::Closure(c) => 1 + c.copied.len(),
        Body::Activation(a) => 3 + a.detached.as_ref().map_or(0, |_| 3),
        Body::FarRef { .. } => 1,
        Body::Promise(_) => 4,
        Body::Bytes(_) | Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => 0,
    }
}

fn read_edges(r: &mut Reader<'_>, p: &Pending) -> Result<Vec<Value>, LoadError> {
    let n = edge_count(&p.body);
    let mut e = Vec::with_capacity(n);
    for _ in 0..n {
        e.push(r.value()?);
    }
    Ok(e)
}

fn fill_body(
    body: Body,
    e: &[Value],
    resolve: &dyn Fn(Value) -> Result<Value, LoadError>,
) -> Result<Body, LoadError> {
    let all =
        |e: &[Value]| -> Result<Vec<Value>, LoadError> { e.iter().map(|&v| resolve(v)).collect() };
    Ok(match body {
        Body::Regular(_) => Body::Regular(all(e)?),
        Body::Array(_) => Body::Array(all(e)?),
        Body::WeakArray(_) => Body::WeakArray(all(e)?),
        Body::Ephemeron { .. } => Body::Ephemeron {
            key: resolve(e[0])?,
            value: resolve(e[1])?,
            finalizer: resolve(e[2])?,
        },
        Body::Method(m) => Body::Method(Method {
            literals: resolve(e[0])?,
            mixin: resolve(e[1])?,
            selector: resolve(e[2])?,
            source: resolve(e[3])?,
            library: resolve(e[4])?,
            ..m
        }),
        Body::Mixin(m) => Body::Mixin(Mixin {
            name: resolve(e[0])?,
            methods: resolve(e[1])?,
            enclosing_mixin: resolve(e[2])?,
            slot_names: resolve(e[3])?,
            class_mixin: resolve(e[4])?,
            library: resolve(e[5])?,
            ..m
        }),
        Body::Class(_) => Body::Class(Behavior {
            mixin: resolve(e[0])?,
            superclass: resolve(e[1])?,
            enclosing_object: resolve(e[2])?,
            name: resolve(e[3])?,
            this_class: Value::small_integer(0).expect("small"),
            library: resolve(e[4])?,
        }),
        Body::Metaclass(_) => Body::Metaclass(Behavior {
            mixin: resolve(e[0])?,
            superclass: resolve(e[1])?,
            enclosing_object: resolve(e[2])?,
            name: Value::small_integer(0).expect("small"),
            this_class: resolve(e[3])?,
            library: resolve(e[4])?,
        }),
        Body::Closure(c) => Body::Closure(Closure {
            defining: resolve(e[0])?,
            copied: all(&e[1..])?,
            ..c
        }),
        Body::Activation(a) => Body::Activation(Activation {
            frame: a.frame,
            method: resolve(e[0])?,
            receiver: resolve(e[1])?,
            closure: resolve(e[2])?,
            detached: match a.detached {
                Some(_) => Some(Detached {
                    sender: resolve(e[3])?,
                    bci: resolve(e[4])?,
                    temps: resolve(e[5])?,
                }),
                None => None,
            },
        }),
        Body::FarRef { actor, .. } => Body::FarRef {
            actor,
            target: resolve(e[0])?,
        },
        Body::Promise(p) => Body::Promise(PromiseRecord {
            value: resolve(e[0])?,
            owner: resolve(e[1])?,
            waiters: resolve(e[2])?,
            queue: resolve(e[3])?,
            ..p
        }),
        other => other,
    })
}
