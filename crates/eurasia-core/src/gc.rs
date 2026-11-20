// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The collector (implementation plan M3; `gc-semantics.tex`).
//!
//! A collection is the semantics' rule GC-Collect taking all garbage, which
//! is always admissible: it marks what the roots strongly reach, clears weak
//! observations of everything else, and drops it. Survivors keep their
//! allocation order and their sequence numbers, so identity order and
//! identity hashes (`eq:kernel-identity-hash`) are unchanged and no identity
//! is ever reused (`eq:gc-wrapper-state`); only arena offsets change, and
//! every reference is rewritten through a [`Forwarding`]. Survivors slide
//! down the arena in place; the byte space and the code table are compacted
//! with them.
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

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use crate::object::{Code, FROZEN, Format, HEADER, Heap, SHARED, Space, VALUE};
use crate::value::Value;

/// Where each object of a collected heap went: a compressor's tables. A
/// bit per arena word, set over every surviving object's words, and for
/// each block of 64 words the number of surviving words before it; an
/// object's new offset is its block's count plus the surviving words before
/// it in its block.
pub struct Forwarding {
    live: Vec<u64>,
    base: Vec<u64>,
}

impl Forwarding {
    #[inline]
    fn is_live(&self, o: usize) -> bool {
        self.live[o / 64] & (1 << (o % 64)) != 0
    }

    #[inline]
    fn new_offset(&self, o: usize) -> u64 {
        let below = self.live[o / 64] & ((1u64 << (o % 64)) - 1);
        self.base[o / 64] + u64::from(below.count_ones())
    }

    /// The value after the collection. A reference to a collected object is
    /// a VM bug: roots and strong fields always survive.
    #[inline]
    pub fn forward(&self, v: Value) -> Value {
        match v.as_reference() {
            Some(i) => {
                debug_assert!(
                    self.is_live(i as usize),
                    "a reference to a collected object"
                );
                Value::reference(self.new_offset(i as usize)).expect("offset in range")
            }
            None => v,
        }
    }

    /// Whether the object `v` referred to survived (immediates always do).
    pub fn survived(&self, v: Value) -> bool {
        v.as_reference().is_none_or(|i| self.is_live(i as usize))
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

impl Heap {
    /// Forwards identities (`vm commit:`'s forwarding pairs): every
    /// reference to an object that `forward` maps is replaced by its image,
    /// in every object's class and fields, except an activation's method:
    /// frames already running keep their code (`reflection-semantics.tex`,
    /// the observation table).
    pub fn forward_identities(&mut self, forward: &impl Fn(Value) -> Value) {
        for v in self.objects() {
            let o = self.start(v).expect("an object");
            self.words[o] = forward(self.words[o]);
            let code = self.code_at(o);
            if !code.has_references() {
                continue;
            }
            for (i, w) in self.contents_mut(o).iter_mut().enumerate() {
                if !(code == Code::Activation && i == 1) {
                    *w = forward(*w);
                }
            }
        }
    }
}

/// The marker's state: the strongly reachable set (by word offset) and the
/// objects still to scan.
struct Marker {
    /// A bit per arena word: set at the start of every marked object.
    marked: Vec<u64>,
    work: Vec<usize>,
}

impl Marker {
    #[inline]
    fn bit(&self, o: usize) -> bool {
        self.marked[o / 64] & (1 << (o % 64)) != 0
    }

    #[inline]
    fn mark(&mut self, heap: &Heap, v: Value) {
        if let Some(o) = heap.start(v)
            && !self.bit(o)
        {
            self.marked[o / 64] |= 1 << (o % 64);
            self.work.push(o);
        }
    }

    /// Whether `v` is strongly reachable so far (immediates always are).
    fn is_marked(&self, v: Value) -> bool {
        v.as_reference().is_none_or(|i| {
            let i = i as usize;
            i / 64 >= self.marked.len() || self.bit(i)
        })
    }
}

impl Heap {
    /// Collects everything not strongly reachable from `roots`, clearing weak
    /// observations of it to `nil`. Answers where each object went; the
    /// caller rewrites its own references (the roots) through it.
    pub fn collect(&mut self, roots: &[Value], nil: Value) -> (Forwarding, Statistics) {
        self.collect_with(roots, nil, false)
    }

    /// Collects everything not reachable from `roots` at all, weakly or
    /// strongly, so that no weak observation changes: the heap a checkpoint
    /// saves (M7), whose weak references then clear at the same collections
    /// as the running VM's.
    pub fn collect_keeping_weak(
        &mut self,
        roots: &[Value],
        nil: Value,
    ) -> (Forwarding, Statistics) {
        self.collect_with(roots, nil, true)
    }

    fn collect_with(
        &mut self,
        roots: &[Value],
        nil: Value,
        keep_weak: bool,
    ) -> (Forwarding, Statistics) {
        let before = self.count;
        let blocks = self.words.len().div_ceil(64);
        let mut m = Marker {
            marked: vec![0; blocks],
            work: Vec::new(),
        };
        m.mark(self, nil);
        roots.iter().for_each(|&v| m.mark(self, v));
        // Ephemerons whose key is not (yet) known to be reachable.
        let mut pending: Vec<usize> = Vec::new();
        loop {
            while let Some(o) = m.work.pop() {
                m.mark(self, self.words[o]);
                match self.code_at(o) {
                    Code::Ephemeron => {
                        let c = self.contents(o);
                        let (key, value, finalizer) = (c[0], c[1], c[2]);
                        if keep_weak {
                            m.mark(self, key);
                        }
                        if m.is_marked(key) {
                            m.mark(self, value);
                            m.mark(self, finalizer);
                        } else {
                            pending.push(o);
                        }
                    }
                    Code::WeakArray if !keep_weak => {}
                    code if code.has_references() => {
                        let n = self.contents(o).len();
                        for i in 0..n {
                            let v = self.words[o + HEADER + i];
                            m.mark(self, v);
                        }
                    }
                    _ => {}
                }
            }
            // An ephemeron whose key has become reachable enables its value.
            let mut enabled = Vec::new();
            pending.retain(|&o| {
                let c = self.contents(o);
                if m.is_marked(c[0]) {
                    enabled.push(c[1]);
                    enabled.push(c[2]);
                    false
                } else {
                    true
                }
            });
            if enabled.is_empty() {
                break;
            }
            enabled.into_iter().for_each(|v| m.mark(self, v));
        }

        // Clear weak observations of collected objects.
        for &o in &pending {
            self.contents_mut(o).fill(nil);
        }
        // The live words: every word of every marked object. Weak arrays'
        // collected elements become nil on the way.
        let mut live = vec![0u64; blocks];
        let mut o = 0;
        while o < self.words.len() {
            let n = self.object_words(o);
            if m.bit(o) {
                if self.code_at(o) == Code::WeakArray {
                    for w in &mut self.words[o + HEADER..o + n] {
                        if !m.is_marked(*w) {
                            *w = nil;
                        }
                    }
                }
                set_bits(&mut live, o, o + n);
            }
            o += n;
        }
        let mut base = Vec::with_capacity(blocks);
        let mut total = 0u64;
        for &bits in &live {
            base.push(total);
            total += u64::from(bits.count_ones());
        }
        let forwarding = Forwarding { live, base };

        // Slide the survivors down, compacting bytes and codes with them.
        let mut bytes = Vec::new();
        let mut codes = Vec::new();
        let mut count = 0;
        let mut o = 0;
        let end = self.words.len();
        while o < end {
            let n = self.object_words(o);
            if forwarding.is_live(o) {
                let to = forwarding.new_offset(o) as usize;
                let code = self.code_at(o);
                self.words.copy_within(o..o + n, to);
                if code.is_bytes() {
                    let range = self.byte_range(to);
                    let at = bytes.len();
                    bytes.extend_from_slice(&self.bytes[range]);
                    self.words[to + HEADER] = Value::small_integer(at as i64).expect("small");
                } else if code == Code::Method {
                    let c = self.words[to + HEADER + 7];
                    let i = c.as_small_integer().expect("a code index") as usize;
                    self.words[to + HEADER + 7] =
                        Value::small_integer(codes.len() as i64).expect("small");
                    codes.push(self.codes[i].clone());
                }
                count += 1;
            }
            o += n;
        }
        self.words.truncate(total as usize);
        self.bytes = bytes;
        self.codes = codes;
        self.count = count;
        // Rewrite references.
        let forward = |v: Value| forwarding.forward(v);
        let mut o = 0;
        while o < self.words.len() {
            self.words[o] = forward(self.words[o]);
            let n = self.object_words(o);
            if self.code_at(o).has_references() {
                for w in &mut self.words[o + HEADER..o + n] {
                    *w = forward(*w);
                }
            }
            o += n;
        }
        let statistics = Statistics {
            before,
            after: self.count,
        };
        (forwarding, statistics)
    }
}

/// Sets bits `from` to `to` (exclusive) of `bits`.
fn set_bits(bits: &mut [u64], from: usize, to: usize) {
    let mut i = from;
    while i < to {
        let w = i / 64;
        let lo = i % 64;
        let hi = (to - w * 64).min(64);
        let mask = if hi - lo == 64 {
            u64::MAX
        } else {
            ((1u64 << (hi - lo)) - 1) << lo
        };
        bits[w] |= mask;
        i = w * 64 + hi;
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
        let mut seen: BTreeMap<usize, ()> = BTreeMap::new();
        // Breadth first; `order` is both the queue and the discovery order.
        let mut order: Vec<usize> = Vec::new();
        let visit =
            |v: Value, heap: &Heap, seen: &mut BTreeMap<usize, ()>, order: &mut Vec<usize>| {
                if let Some(o) = heap.start(v)
                    && !seen.contains_key(&o)
                    && !heap.is_shared(o)
                {
                    seen.insert(o, ());
                    order.push(o);
                }
            };
        visit(root, self, &mut seen, &mut order);
        let mut head = 0;
        while head < order.len() {
            let o = order[head];
            head += 1;
            let mut refs = alloc::vec![self.words[o]];
            if self.code_at(o).has_references() {
                refs.extend_from_slice(self.contents(o));
            }
            for v in refs {
                visit(v, self, &mut seen, &mut order);
            }
        }
        // Allocate the copies, in discovery order, then rewrite them.
        let mut copy_of: BTreeMap<u64, Value> = BTreeMap::new();
        let mut copies = Vec::with_capacity(order.len());
        for &o in &order {
            let v = Value::reference(o as u64).expect("in range");
            let original = self.object(v).expect("an object");
            let copy = self.allocate(original.header.class, space, original.body);
            self.set_flags(copy, original.header.flags);
            copy_of.insert(o as u64, copy);
            copies.push(copy);
        }
        let forward = |v: Value| match v.as_reference() {
            Some(i) => copy_of.get(&i).copied().unwrap_or(v),
            None => v,
        };
        for copy in copies {
            let o = self.start(copy).expect("a copy");
            self.words[o] = forward(self.words[o]);
            if self.code_at(o).has_references() {
                for w in self.contents_mut(o) {
                    *w = forward(*w);
                }
            }
        }
        forward(root)
    }

    /// Whether object `o` is shared by every actor rather than copied with
    /// a platform (see [`Heap::clone_graph`]).
    fn is_shared(&self, o: usize) -> bool {
        let v = Value::reference(o as u64).expect("in range");
        if self.flags(v) & (FROZEN | VALUE | SHARED) != 0 || self.space(v) == Some(Space::Program) {
            return true;
        }
        match self.format(v) {
            Some(Format::Method | Format::Mixin | Format::FarRef | Format::Promise) => true,
            Some(Format::Class | Format::Metaclass) => {
                match self.start(self.behavior(v).expect("a class").enclosing_object()) {
                    Some(e) => e == o || self.is_shared(e),
                    None => true,
                }
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Body, Space};

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
        h.seq(v).unwrap()
    }

    #[test]
    fn unreachable_objects_and_cycles_go_and_survivors_keep_their_identity() {
        let (mut h, nil, class) = heap();
        let kept = object(&mut h, class, Vec::new());
        let a = object(&mut h, class, alloc::vec![nil]);
        let b = object(&mut h, class, alloc::vec![a]);
        h.regular_slots_mut(a).unwrap()[0] = b; // a cycle with no root
        let root = object(&mut h, class, alloc::vec![kept]);
        let (root_seq, kept_seq) = (seq(&h, root), seq(&h, kept));
        let (fwd, stats) = h.collect(&[class, root], nil);
        assert_eq!((stats.before, stats.after), (6, 4));
        assert!(!fwd.survived(a) && !fwd.survived(b));
        let root = fwd.forward(root);
        assert_eq!(seq(&h, root), root_seq);
        let s = h.regular_slots(root).unwrap();
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
        let e = h.fields(fwd.forward(weak)).unwrap();
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
        let [key, value, _] = h.ephemeron(e).unwrap();
        (key, value)
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
