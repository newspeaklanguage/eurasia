// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The collector (implementation plan M3; `gc-semantics.tex`).
//!
//! A collection is the semantics' rule GC-Collect taking all garbage, which
//! is always admissible: it marks what the roots strongly reach, clears weak
//! observations of everything else, and drops it. Survivors keep their
//! allocation order and their sequence numbers, so identity order and
//! identity hashes (`eq:kernel-identity-hash`) are unchanged and no identity
//! is ever reused (`eq:gc-wrapper-state`); only arena indices change, and
//! every reference is rewritten through a [`Forwarding`].
//!
//! Weakness (`eq:gc-roots-and-successors`, `eq:gc-reachability`):
//! - a WeakArray's elements are weak; an element whose object is collected
//!   becomes nil (`eq:gc-clear-weak-payload`);
//! - an Ephemeron's key is weak and its value and finalizer are strong only
//!   once its key is strongly reachable (the weak-map entry rule). An
//!   ephemeron whose key is collected is mourned as psoup mourns it: key,
//!   value and finalizer become nil. Finalizers are not run.
//!
//! Immediates and the nil object are always live. Collections happen only at
//! safepoints the interpreter chooses (between instructions, and inside
//! `vm garbageCollect`), never inside a native.

use alloc::vec;
use alloc::vec::Vec;

use crate::object::{Body, FROZEN, Heap, SHARED, Space, VALUE};
use crate::value::Value;

/// Where each object of a collected heap went.
pub struct Forwarding {
    /// Old arena index to new arena index; `u64::MAX` for a collected object.
    new_index: Vec<u64>,
}

/// Marks an object dropped by the collection.
const COLLECTED: u64 = u64::MAX;

impl Forwarding {
    /// The value after the collection. A reference to a collected object is
    /// a VM bug: roots and strong fields always survive.
    pub fn forward(&self, v: Value) -> Value {
        match v.as_reference() {
            Some(i) => {
                let n = self.new_index[i as usize];
                debug_assert_ne!(n, COLLECTED, "a reference to a collected object");
                Value::reference(n).expect("index in range")
            }
            None => v,
        }
    }

    /// Whether the object `v` referred to survived (immediates always do).
    pub fn survived(&self, v: Value) -> bool {
        v.as_reference()
            .is_none_or(|i| self.new_index[i as usize] != COLLECTED)
    }
}

/// What a collection did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Statistics {
    /// Objects before.
    pub before: usize,
    /// Objects after.
    pub after: usize,
}

/// Calls `f` on every strong reference field of `body`: not a WeakArray's
/// elements, nor an Ephemeron's fields (the marker treats those itself).
fn for_each_strong(body: &Body, mut f: impl FnMut(Value)) {
    match body {
        Body::Regular(e) | Body::Array(e) => e.iter().copied().for_each(f),
        Body::WeakArray(_) | Body::Ephemeron { .. } => {}
        Body::Bytes(_) | Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => {}
        Body::Method(m) => [m.literals, m.mixin, m.selector, m.source, m.library]
            .into_iter()
            .for_each(f),
        Body::Mixin(m) => [
            m.name,
            m.methods,
            m.enclosing_mixin,
            m.slot_names,
            m.class_mixin,
            m.library,
        ]
        .into_iter()
        .for_each(f),
        Body::Class(b) | Body::Metaclass(b) => [
            b.mixin,
            b.superclass,
            b.enclosing_object,
            b.name,
            b.this_class,
            b.library,
        ]
        .into_iter()
        .for_each(f),
        Body::Closure(c) => {
            f(c.defining);
            c.copied.iter().copied().for_each(f);
        }
        Body::Activation(a) => [a.method, a.receiver, a.closure].into_iter().for_each(f),
        Body::FarRef { target, .. } => f(*target),
        Body::Promise(p) => {
            f(p.value);
            f(p.owner);
            p.waiters.iter().copied().for_each(&mut f);
            p.queue.iter().copied().for_each(f);
        }
    }
}

impl Heap {
    /// Forwards identities (`vm commit:`'s forwarding pairs): every
    /// reference to an object that `forward` maps is replaced by its image,
    /// in every object's class and fields, except an activation's method:
    /// frames already running keep their code (`reflection-semantics.tex`,
    /// the observation table).
    pub fn forward_identities(&mut self, forward: &impl Fn(Value) -> Value) {
        for object in &mut self.objects {
            object.header.class = forward(object.header.class);
            match &mut object.body {
                Body::Activation(a) => {
                    a.receiver = forward(a.receiver);
                    a.closure = forward(a.closure);
                }
                body => map_all(body, forward),
            }
        }
    }
}

/// Rewrites every reference field of `body`, weak ones included.
fn map_all(body: &mut Body, f: &impl Fn(Value) -> Value) {
    let each = |e: &mut Vec<Value>| e.iter_mut().for_each(|v| *v = f(*v));
    match body {
        Body::Regular(e) | Body::Array(e) | Body::WeakArray(e) => each(e),
        Body::Ephemeron {
            key,
            value,
            finalizer,
        } => {
            *key = f(*key);
            *value = f(*value);
            *finalizer = f(*finalizer);
        }
        Body::Bytes(_) | Body::String(_) | Body::Float(_) | Body::LargeInteger { .. } => {}
        Body::Method(m) => {
            for v in [
                &mut m.literals,
                &mut m.mixin,
                &mut m.selector,
                &mut m.source,
                &mut m.library,
            ] {
                *v = f(*v);
            }
        }
        Body::Mixin(m) => {
            for v in [
                &mut m.name,
                &mut m.methods,
                &mut m.enclosing_mixin,
                &mut m.slot_names,
                &mut m.class_mixin,
                &mut m.library,
            ] {
                *v = f(*v);
            }
        }
        Body::Class(b) | Body::Metaclass(b) => {
            for v in [
                &mut b.mixin,
                &mut b.superclass,
                &mut b.enclosing_object,
                &mut b.name,
                &mut b.this_class,
                &mut b.library,
            ] {
                *v = f(*v);
            }
        }
        Body::Closure(c) => {
            c.defining = f(c.defining);
            each(&mut c.copied);
        }
        Body::Activation(a) => {
            a.method = f(a.method);
            a.receiver = f(a.receiver);
            a.closure = f(a.closure);
        }
        Body::FarRef { target, .. } => *target = f(*target),
        Body::Promise(p) => {
            p.value = f(p.value);
            p.owner = f(p.owner);
            each(&mut p.waiters);
            each(&mut p.queue);
        }
    }
}

/// The marker's state: the strongly reachable set and the objects still to
/// scan.
struct Marker {
    marked: Vec<bool>,
    work: Vec<usize>,
}

impl Marker {
    fn mark(&mut self, v: Value) {
        if let Some(i) = v.as_reference() {
            let i = i as usize;
            if let Some(m) = self.marked.get_mut(i)
                && !*m
            {
                *m = true;
                self.work.push(i);
            }
        }
    }

    /// Whether `v` is strongly reachable so far (immediates always are).
    fn is_marked(&self, v: Value) -> bool {
        v.as_reference()
            .is_none_or(|i| self.marked.get(i as usize).copied().unwrap_or(true))
    }
}

impl Heap {
    /// Collects everything not strongly reachable from `roots`, clearing weak
    /// observations of it to `nil`. Answers where each object went; the
    /// caller rewrites its own references (the roots) through it.
    pub fn collect(&mut self, roots: &[Value], nil: Value) -> (Forwarding, Statistics) {
        let n = self.objects.len();
        let mut m = Marker {
            marked: vec![false; n],
            work: Vec::new(),
        };
        m.mark(nil);
        roots.iter().for_each(|&v| m.mark(v));
        // Ephemerons whose key is not (yet) known to be reachable.
        let mut pending: Vec<usize> = Vec::new();
        loop {
            while let Some(i) = m.work.pop() {
                let object = &self.objects[i];
                m.mark(object.header.class);
                match &object.body {
                    Body::Ephemeron {
                        key,
                        value,
                        finalizer,
                    } => {
                        if m.is_marked(*key) {
                            m.mark(*value);
                            m.mark(*finalizer);
                        } else {
                            pending.push(i);
                        }
                    }
                    body => {
                        let mut refs = Vec::new();
                        for_each_strong(body, |v| refs.push(v));
                        refs.into_iter().for_each(|v| m.mark(v));
                    }
                }
            }
            // An ephemeron whose key has become reachable enables its value.
            let mut enabled = Vec::new();
            pending.retain(|&i| {
                let Body::Ephemeron {
                    key,
                    value,
                    finalizer,
                } = &self.objects[i].body
                else {
                    unreachable!("pending ephemerons are ephemerons")
                };
                if m.is_marked(*key) {
                    enabled.push(*value);
                    enabled.push(*finalizer);
                    false
                } else {
                    true
                }
            });
            if enabled.is_empty() {
                break;
            }
            enabled.into_iter().for_each(|v| m.mark(v));
        }

        // Clear weak observations of collected objects.
        for &i in &pending {
            if let Body::Ephemeron {
                key,
                value,
                finalizer,
            } = &mut self.objects[i].body
            {
                *key = nil;
                *value = nil;
                *finalizer = nil;
            }
        }
        let marked = m.marked;
        let alive = |v: Value| {
            v.as_reference()
                .is_none_or(|i| marked.get(i as usize).copied().unwrap_or(true))
        };
        for (i, object) in self.objects.iter_mut().enumerate() {
            if marked[i]
                && let Body::WeakArray(e) = &mut object.body
            {
                for v in e.iter_mut() {
                    if !alive(*v) {
                        *v = nil;
                    }
                }
            }
        }

        // Keep the survivors in allocation order and rewrite references.
        let mut new_index = vec![COLLECTED; n];
        let mut next = 0u64;
        for (i, &keep) in marked.iter().enumerate() {
            if keep {
                new_index[i] = next;
                next += 1;
            }
        }
        let forwarding = Forwarding { new_index };
        let old = core::mem::take(&mut self.objects);
        self.objects = old
            .into_iter()
            .zip(marked.iter())
            .filter_map(|(o, &keep)| keep.then_some(o))
            .collect();
        let forward = |v: Value| forwarding.forward(v);
        for object in &mut self.objects {
            object.header.class = forward(object.header.class);
            map_all(&mut object.body, &forward);
        }
        let statistics = Statistics {
            before: n,
            after: self.objects.len(),
        };
        (forwarding, statistics)
    }
}

impl Heap {
    /// Clones the mutable graph reachable from `root` into `space`, as
    /// actor creation copies the platform template
    /// (`EURASIA_ACTORS_DESIGN_2026-11-17.md` section 5). Shared, and not
    /// copied: immediates, values, frozen objects, objects marked
    /// [`SHARED`], program objects (methods, mixins), far references, and a
    /// class or metaclass whose enclosing object is shared. A class nested
    /// in a copied object is copied with it, so the copies' code reaches the
    /// copies' state. Copies are allocated in discovery order (fresh
    /// sequence numbers, deterministic) and keep their flags; weak elements
    /// are copied like strong ones (a template is never collected into).
    /// Answers the copy of `root`, or `root` itself if it is shared.
    pub fn clone_graph(&mut self, root: Value, space: Space) -> Value {
        let n = self.objects.len();
        let mut copy_of: Vec<u64> = vec![COLLECTED; n];
        // Breadth first; `order` is both the queue and the discovery order.
        let mut order: Vec<usize> = Vec::new();
        let visit = |v: Value, heap: &Heap, copy_of: &mut Vec<u64>, order: &mut Vec<usize>| {
            if let Some(i) = v.as_reference() {
                let i = i as usize;
                if i < n && copy_of[i] == COLLECTED && !heap.is_shared(i) {
                    copy_of[i] = 0;
                    order.push(i);
                }
            }
        };
        visit(root, self, &mut copy_of, &mut order);
        let mut head = 0;
        while head < order.len() {
            let i = order[head];
            head += 1;
            let object = &self.objects[i];
            let mut refs = Vec::new();
            refs.push(object.header.class);
            every_reference(&object.body, |v| refs.push(v));
            for v in refs {
                visit(v, self, &mut copy_of, &mut order);
            }
        }
        // Allocate the copies, in discovery order, then rewrite them.
        let mut new_index: Vec<u64> = (0..n as u64).collect();
        for &i in &order {
            let original = self.objects[i].clone();
            let copy = self.allocate(original.header.class, space, original.body);
            if let Some(o) = self.get_mut(copy) {
                o.header.flags = original.header.flags;
            }
            new_index[i] = copy.as_reference().expect("a reference");
        }
        let forward = |v: Value| match v.as_reference() {
            Some(i) if (i as usize) < n => {
                Value::reference(new_index[i as usize]).expect("in range")
            }
            _ => v,
        };
        for &i in &order {
            let k = new_index[i] as usize;
            let object = &mut self.objects[k];
            object.header.class = forward(object.header.class);
            map_all(&mut object.body, &forward);
        }
        forward(root)
    }

    /// Whether object `i` is shared by every actor rather than copied with
    /// a platform (see [`Heap::clone_graph`]).
    fn is_shared(&self, i: usize) -> bool {
        let object = &self.objects[i];
        if object.header.flags & (FROZEN | VALUE | SHARED) != 0
            || object.header.space == Space::Program
        {
            return true;
        }
        match &object.body {
            Body::Method(_) | Body::Mixin(_) | Body::FarRef { .. } | Body::Promise(_) => true,
            Body::Class(b) | Body::Metaclass(b) => match b.enclosing_object.as_reference() {
                Some(e) => (e as usize) == i || self.is_shared(e as usize),
                None => true,
            },
            _ => false,
        }
    }
}

/// Calls `f` on every reference field of `body`, weak ones included.
fn every_reference(body: &Body, f: impl FnMut(Value)) {
    match body {
        Body::WeakArray(e) => e.iter().copied().for_each(f),
        Body::Ephemeron {
            key,
            value,
            finalizer,
        } => [*key, *value, *finalizer].into_iter().for_each(f),
        other => for_each_strong(other, f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Space;

    /// A heap whose object 0 is nil and object 1 a class for everything.
    fn heap() -> (Heap, Value, Value) {
        let mut h = Heap::new();
        let nil = h.allocate(
            Value::reference(0).unwrap(),
            Space::Actor(0),
            Body::Regular(Vec::new()),
        );
        let class = h.allocate(nil, Space::Actor(0), Body::Regular(Vec::new()));
        (h, nil, class)
    }

    fn object(h: &mut Heap, class: Value, slots: Vec<Value>) -> Value {
        h.allocate(class, Space::Actor(0), Body::Regular(slots))
    }

    fn seq(h: &Heap, v: Value) -> u64 {
        h.get(v).unwrap().header.seq
    }

    #[test]
    fn unreachable_objects_and_cycles_go_and_survivors_keep_their_identity() {
        let (mut h, nil, class) = heap();
        let kept = object(&mut h, class, Vec::new());
        let a = object(&mut h, class, Vec::new());
        let b = object(&mut h, class, alloc::vec![a]);
        if let Body::Regular(s) = &mut h.get_mut(a).unwrap().body {
            s.push(b); // a cycle with no root
        }
        let root = object(&mut h, class, alloc::vec![kept]);
        let (root_seq, kept_seq) = (seq(&h, root), seq(&h, kept));
        let (fwd, stats) = h.collect(&[class, root], nil);
        assert_eq!((stats.before, stats.after), (6, 4));
        assert!(!fwd.survived(a) && !fwd.survived(b));
        let root = fwd.forward(root);
        assert_eq!(seq(&h, root), root_seq);
        let Body::Regular(s) = &h.get(root).unwrap().body else {
            panic!()
        };
        assert_eq!(seq(&h, s[0]), kept_seq);
        // Allocation continues after the highest sequence number ever used.
        let fresh = object(&mut h, class, Vec::new());
        assert_eq!(seq(&h, fresh), 6);
    }

    #[test]
    fn weak_array_elements_are_cleared_but_do_not_retain() {
        let (mut h, nil, class) = heap();
        let gone = object(&mut h, class, Vec::new());
        let kept = object(&mut h, class, Vec::new());
        let weak = h.allocate(
            class,
            Space::Actor(0),
            Body::WeakArray(alloc::vec![gone, kept, Value::small_integer(7).unwrap()]),
        );
        let (fwd, _) = h.collect(&[weak, kept], nil);
        let Body::WeakArray(e) = &h.get(fwd.forward(weak)).unwrap().body else {
            panic!()
        };
        assert_eq!(e[0], fwd.forward(nil));
        assert_eq!(e[1], fwd.forward(kept));
        assert_eq!(e[2], Value::small_integer(7).unwrap());
    }

    fn ephemeron(h: &mut Heap, class: Value, key: Value, value: Value, nil: Value) -> Value {
        h.allocate(
            class,
            Space::Actor(0),
            Body::Ephemeron {
                key,
                value,
                finalizer: nil,
            },
        )
    }

    fn fields(h: &Heap, e: Value) -> (Value, Value) {
        let Body::Ephemeron { key, value, .. } = &h.get(e).unwrap().body else {
            panic!()
        };
        (*key, *value)
    }

    #[test]
    fn an_ephemeron_value_lives_only_while_its_key_does() {
        let (mut h, nil, class) = heap();
        let key = object(&mut h, class, Vec::new());
        let value = object(&mut h, class, Vec::new());
        let e = ephemeron(&mut h, class, key, value, nil);
        // The key is reachable: the value is retained.
        let (fwd, _) = h.collect(&[e, key], nil);
        let (e, key) = (fwd.forward(e), fwd.forward(key));
        assert_eq!(fields(&h, e), (key, fwd.forward(value)));
        // Only the ephemeron is: it is mourned, key and value nil.
        let (fwd, stats) = h.collect(&[e], nil);
        assert_eq!(stats.after, 3);
        let e = fwd.forward(e);
        assert_eq!(fields(&h, e), (fwd.forward(nil), fwd.forward(nil)));
    }

    #[test]
    fn ephemeron_chains_and_cycles_follow_the_least_fixed_point() {
        let (mut h, nil, class) = heap();
        // e1: k1 -> k2; e2: k2 -> v. Rooting k1 keeps the whole chain.
        let k1 = object(&mut h, class, Vec::new());
        let k2 = object(&mut h, class, Vec::new());
        let v = object(&mut h, class, Vec::new());
        let e2 = ephemeron(&mut h, class, k2, v, nil);
        let e1 = ephemeron(&mut h, class, k1, k2, nil);
        // A cycle: e3: a -> b, e4: b -> a; neither key is otherwise reachable.
        let a = object(&mut h, class, Vec::new());
        let b = object(&mut h, class, Vec::new());
        let e3 = ephemeron(&mut h, class, a, b, nil);
        let e4 = ephemeron(&mut h, class, b, a, nil);
        let (fwd, _) = h.collect(&[e1, e2, e3, e4, k1], nil);
        assert!(fwd.survived(k2) && fwd.survived(v));
        assert!(!fwd.survived(a) && !fwd.survived(b));
        assert_eq!(fields(&h, fwd.forward(e2)).1, fwd.forward(v));
        assert_eq!(
            fields(&h, fwd.forward(e3)),
            (fwd.forward(nil), fwd.forward(nil))
        );
    }
}
