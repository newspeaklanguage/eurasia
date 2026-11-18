// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The V5 interpreter.
//!
//! Frames live on a Rust-managed stack (implementation plan, section 2.4).
//! The operand stack holds, for each frame, the message receiver, then the
//! arguments, then the locals, then the frame's operands.
//!
//! Send lookup follows psoup exactly (`interpreter.cc`: `OrdinarySendMiss`,
//! `LexicalSend`, `ProtectedSend`, `ImplicitReceiverSendMiss`,
//! `OuterSendMiss`, `SuperSendMiss`, `DNUSend`), which is also the lookup of
//! the formal semantics.
//!
//! M1 scope: sends, lookup, DNU, natives with their three outcomes, the
//! failure path, local returns, branches, literals, parameters, locals and
//! arrays. Closures, non-local return, eventual sends and
//! `nonBooleanReceiver:` arrive in M2 and are reported as unimplemented.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::bytecode::{SendKind, instruction_length, op};
use crate::gc;
use crate::host::Host;
use crate::natives::{self, Native, NativeContext, Outcome, Reason};
use crate::object::{Access, Body, FROZEN, Heap, MethodKind, Space};
use crate::roles::{Role, Roles};
use crate::value::Value;

/// The deepest frame stack M1 allows. Exceeding it is an error in M1; in M2
/// it becomes the failure `#exhausted: #stack` (operation list, section 0).
pub const MAX_FRAMES: usize = 100_000;

/// Why execution stopped abnormally. These are VM-level errors, not Newspeak
/// exceptions: a correct image never causes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmError {
    /// A value that should be an object of some kind is not.
    Malformed(&'static str),
    /// Bytecode that does not decode, or an operand out of range.
    InvalidBytecode {
        /// The method's selector.
        selector: String,
        /// The zero-based bytecode index.
        index: usize,
    },
    /// An instruction or feature that a later milestone implements.
    Unimplemented {
        /// What is missing.
        what: &'static str,
        /// The method's selector.
        selector: String,
    },
    /// A method's class chain does not contain the mixin it was compiled in.
    MixinApplicationNotFound,
    /// No `doesNotUnderstand:` method anywhere in the class chain.
    RecursiveDoesNotUnderstand,
    /// A method header names an unknown native, or the wrong arity.
    BadNative(u32),
    /// The frame stack exceeded [`MAX_FRAMES`]; the selectors of the
    /// innermost frames, innermost first.
    StackExhausted(Vec<String>),
    /// A getter or setter ran on an object without that slot.
    BadSlotAccess {
        /// The accessor's selector.
        selector: String,
        /// The receiver's class name, if it has one.
        receiver_class: String,
    },
    /// The step limit set by [`Vm::set_step_limit`] was reached.
    StepLimitReached,
    /// The image has no method for a VM upcall (named by its role).
    MissingUpcall(&'static str),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::Malformed(what) => write!(f, "malformed image: {what}"),
            VmError::InvalidBytecode { selector, index } => {
                write!(f, "invalid bytecode in {selector} at index {index}")
            }
            VmError::Unimplemented { what, selector } => {
                write!(f, "{what} is not implemented yet (in {selector})")
            }
            VmError::MixinApplicationNotFound => write!(f, "cannot find mixin application"),
            VmError::RecursiveDoesNotUnderstand => write!(f, "recursive #doesNotUnderstand:"),
            VmError::BadNative(id) => write!(f, "bad native method (catalog id {id})"),
            VmError::StackExhausted(top) => {
                write!(f, "stack exhausted; innermost frames: {}", top.join(" < "))
            }
            VmError::BadSlotAccess {
                selector,
                receiver_class,
            } => write!(
                f,
                "accessor {selector} on an instance of {receiver_class}, which has no such slot"
            ),
            VmError::StepLimitReached => write!(f, "step limit reached"),
            VmError::MissingUpcall(role) => write!(f, "the image has no method for {role}"),
        }
    }
}

/// How a run ended normally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    /// The entry send returned this value.
    Returned(Value),
    /// The program sent `vm exit:` with this code.
    Exited(i32),
}

#[derive(Clone, Debug)]
struct Frame {
    /// A number unique to this frame for the life of the VM.
    serial: u64,
    method: Value,
    receiver: Value,
    /// Index of the message receiver; the arguments follow it.
    bp: usize,
    arg_count: usize,
    /// Index of the first local.
    lp: usize,
    ip: usize,
    /// The frame's activation handle, once something has asked for it.
    activation: Option<Value>,
    /// For a closure frame, the closure.
    closure: Option<Value>,
}

enum Step {
    Continue,
    Done(Completion),
}

/// A VM: a heap, its roles, and the execution state of its one actor (M1).
pub struct Vm {
    /// The heap.
    pub heap: Heap,
    /// The roles.
    pub roles: Roles,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    symbols: BTreeMap<&'static str, Value>,
    step_limit: Option<u64>,
    next_serial: u64,
    snapshot_writer: Option<SnapshotWriter>,
    /// The allocation counter after the last collection.
    gc_counter: u64,
    /// Allocations after which the next safepoint collects.
    gc_budget: u64,
    /// Stress testing: collect after this many allocations, whatever the
    /// live size.
    gc_stress: Option<u64>,
    /// Collections so far.
    collections: u64,
}

/// The fewest allocations between scheduled collections.
const MIN_GC_BUDGET: u64 = 1 << 20;

/// Serializes a heap and its roles (the snapshot format's writer). The core
/// cannot depend on `eurasia-snapshot`, which depends on it, so the embedder
/// supplies the writer; `vm snapshot` fails with `unsupported` without one.
pub type SnapshotWriter = fn(&Heap, &Roles) -> Vec<u8>;

impl Vm {
    /// A VM over a loaded heap.
    pub fn new(heap: Heap, roles: Roles) -> Vm {
        Vm {
            heap,
            roles,
            stack: Vec::new(),
            frames: Vec::new(),
            symbols: BTreeMap::new(),
            step_limit: None,
            next_serial: 0,
            snapshot_writer: None,
            gc_counter: 0,
            gc_budget: MIN_GC_BUDGET,
            gc_stress: None,
            collections: 0,
        }
    }

    /// Stress mode: with `Some(n)`, collect at the first safepoint after
    /// every `n` allocations (`n` = 1: after every allocation), so that a
    /// reference the VM holds across a safepoint without rooting it shows up
    /// at once (implementation plan, M3). `None` is normal scheduling.
    pub fn set_gc_stress(&mut self, interval: Option<u64>) {
        self.gc_stress = interval.map(|n| n.max(1));
    }

    /// The number of collections so far.
    pub fn collections(&self) -> u64 {
        self.collections
    }

    fn gc_due(&self) -> bool {
        let allocated = self.heap.allocation_counter() - self.gc_counter;
        allocated >= self.gc_stress.unwrap_or(self.gc_budget)
    }

    /// A full collection at a safepoint. The roots are the roles, the
    /// operand stack, the frames and the VM's own selector strings; every
    /// reference the VM holds is rewritten. The next scheduled collection
    /// comes after as many allocations as there are live objects (at least
    /// [`MIN_GC_BUDGET`]), so the heap stays within about twice its live size.
    pub fn collect_garbage(&mut self) -> gc::Statistics {
        let mut roots: Vec<Value> = self.roles.pairs().map(|(_, v)| v).collect();
        roots.extend(self.stack.iter().copied());
        for f in &self.frames {
            roots.extend([f.method, f.receiver]);
            roots.extend(f.activation);
            roots.extend(f.closure);
        }
        roots.extend(self.symbols.values().copied());
        let nil = self.roles.get(Role::Nil);
        let (fwd, statistics) = self.heap.collect(&roots, nil);
        let pairs: Vec<(Role, Value)> = self.roles.pairs().collect();
        for (role, v) in pairs {
            self.roles.set(role, fwd.forward(v));
        }
        for v in &mut self.stack {
            *v = fwd.forward(*v);
        }
        for f in &mut self.frames {
            f.method = fwd.forward(f.method);
            f.receiver = fwd.forward(f.receiver);
            f.activation = f.activation.map(|v| fwd.forward(v));
            f.closure = f.closure.map(|v| fwd.forward(v));
        }
        for v in self.symbols.values_mut() {
            *v = fwd.forward(*v);
        }
        self.gc_counter = self.heap.allocation_counter();
        self.gc_budget = MIN_GC_BUDGET.max(statistics.after as u64);
        self.collections += 1;
        statistics
    }

    /// The heap and roles a snapshot saves: what the roles reach, collected
    /// into a copy. A saved image has no frames (snapshot format, section
    /// 5), so objects only the running program's frames reach are not
    /// saved; the running program is not disturbed.
    fn snapshot_heap(&self) -> (Heap, Roles) {
        let mut heap = self.heap.clone();
        let roots: Vec<Value> = self.roles.pairs().map(|(_, v)| v).collect();
        let (fwd, _) = heap.collect(&roots, self.roles.get(Role::Nil));
        let mut roles = self.roles.clone();
        let pairs: Vec<(Role, Value)> = self.roles.pairs().collect();
        for (role, v) in pairs {
            roles.set(role, fwd.forward(v));
        }
        (heap, roles)
    }

    /// Supplies the snapshot writer that `vm snapshot` uses.
    pub fn set_snapshot_writer(&mut self, writer: SnapshotWriter) {
        self.snapshot_writer = Some(writer);
    }

    /// Limits a run to `limit` instructions; `None` (the default) is no
    /// limit. A watchdog for embedders and for tests of untrusted images.
    pub fn set_step_limit(&mut self, limit: Option<u64>) {
        self.step_limit = limit;
    }

    fn nil(&self) -> Value {
        self.roles.get(Role::Nil)
    }

    /// Runs the program: sends `main:args:` to the entry object with the VM
    /// mirror and an Array of the argument strings.
    pub fn run(&mut self, host: &mut dyn Host, args: &[&str]) -> Result<Completion, VmError> {
        let strings: Vec<Value> = args.iter().map(|a| self.new_string(a)).collect();
        let array_class = self.roles.get(Role::ArrayClass);
        let args_array = self
            .heap
            .allocate(array_class, Space::Actor(0), Body::Array(strings));
        let entry = self.roles.get(Role::Entry);
        let vm_mirror = self.roles.get(Role::VmMirror);
        self.stack.clear();
        self.frames.clear();
        self.stack.extend([entry, vm_mirror, args_array]);
        let selector = self.symbol("main:args:");
        if let Some(done) = self.ordinary_send(selector, 2, host)? {
            return Ok(done);
        }
        let mut steps: u64 = 0;
        loop {
            if let Some(limit) = self.step_limit {
                if steps >= limit {
                    return Err(VmError::StepLimitReached);
                }
                steps += 1;
            }
            if self.frames.is_empty() {
                let result = self.stack.pop().ok_or(VmError::Malformed("empty stack"))?;
                return Ok(Completion::Returned(result));
            }
            // A safepoint: every value the VM holds is in the roots.
            if self.gc_due() {
                self.collect_garbage();
            }
            if let Step::Done(done) = self.step(host)? {
                return Ok(done);
            }
        }
    }

    // ----- Heap helpers -----

    fn new_string(&mut self, s: &str) -> Value {
        let class = self.roles.get(Role::StringClass);
        self.heap
            .allocate(class, Space::Actor(0), Body::String(s.as_bytes().to_vec()))
    }

    /// A String for a selector the VM sends itself, allocated once.
    fn symbol(&mut self, s: &'static str) -> Value {
        if let Some(&v) = self.symbols.get(s) {
            return v;
        }
        let v = self.new_string(s);
        self.symbols.insert(s, v);
        v
    }

    fn class_of(&self, v: Value) -> Value {
        match self.heap.get(v) {
            Some(o) => o.header.class,
            None => self.roles.get(Role::SmallIntegerClass),
        }
    }

    fn superclass(&self, class: Value) -> Result<Option<Value>, VmError> {
        let b = self
            .heap
            .behavior(class)
            .ok_or(VmError::Malformed("class expected"))?;
        Ok(if b.superclass == self.nil() {
            None
        } else {
            Some(b.superclass)
        })
    }

    /// The method `selector` defined by `class`'s own mixin, if any.
    fn method_at(&self, class: Value, selector: &[u8]) -> Result<Option<Value>, VmError> {
        let b = self
            .heap
            .behavior(class)
            .ok_or(VmError::Malformed("class expected"))?;
        let mixin = self
            .heap
            .mixin(b.mixin)
            .ok_or(VmError::Malformed("mixin expected"))?;
        let methods = self
            .heap
            .array_elements(mixin.methods)
            .ok_or(VmError::Malformed("method array expected"))?;
        for &m in methods {
            let method = self
                .heap
                .method(m)
                .ok_or(VmError::Malformed("method expected"))?;
            if self.heap.string_bytes(method.selector) == Some(selector) {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    fn access(&self, method: Value) -> Access {
        self.heap.method(method).expect("method").header.access
    }

    fn find_application_of(&self, mixin: Value, mut class: Value) -> Result<Value, VmError> {
        loop {
            let b = self
                .heap
                .behavior(class)
                .ok_or(VmError::MixinApplicationNotFound)?;
            if b.mixin == mixin {
                return Ok(class);
            }
            class = self
                .superclass(class)?
                .ok_or(VmError::MixinApplicationNotFound)?;
        }
    }

    /// The number of slots an instance of `class` has: the slot declarations
    /// of every mixin application in its chain.
    fn instance_slot_count(&self, class: Value) -> Result<usize, VmError> {
        let mut total = 0usize;
        let mut c = Some(class);
        let mut guard = 0usize;
        while let Some(k) = c {
            guard += 1;
            if guard > MAX_FRAMES {
                return Err(VmError::Malformed("cyclic superclass chain"));
            }
            let b = self
                .heap
                .behavior(k)
                .ok_or(VmError::Malformed("class expected"))?;
            let mixin = self
                .heap
                .mixin(b.mixin)
                .ok_or(VmError::Malformed("mixin expected"))?;
            total += mixin.slot_count as usize;
            c = self.superclass(k)?;
        }
        Ok(total)
    }

    /// The absolute index of slot `relative` of `method`'s mixin in
    /// `receiver`: the slots of the application's superclasses come first
    /// (the application-qualified slot `(C, j)` of the formal semantics).
    fn slot_index(&self, method: Value, receiver: Value, relative: u32) -> Result<usize, VmError> {
        let mixin = self
            .heap
            .method(method)
            .ok_or(VmError::Malformed("method expected"))?
            .mixin;
        let application = self.find_application_of(mixin, self.class_of(receiver))?;
        let above = match self.superclass(application)? {
            Some(s) => self.instance_slot_count(s)?,
            None => 0,
        };
        Ok(above + relative as usize)
    }

    fn selector_bytes(&self, selector: Value) -> Result<Vec<u8>, VmError> {
        self.heap
            .string_bytes(selector)
            .map(<[u8]>::to_vec)
            .ok_or(VmError::Malformed("selector must be a String"))
    }

    // ----- Sends -----

    /// An ordinary send: the receiver is below `n` arguments on the stack.
    fn ordinary_send(
        &mut self,
        selector: Value,
        n: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let receiver = self.stack[self.stack.len() - 1 - n];
        let receiver_class = self.class_of(receiver);
        let sel = self.selector_bytes(selector)?;
        let mut class = Some(receiver_class);
        while let Some(c) = class {
            if let Some(m) = self.method_at(c, &sel)? {
                match self.access(m) {
                    Access::Public => return self.activate(m, n, host),
                    Access::Protected => {
                        return self.does_not_understand(
                            selector,
                            n,
                            receiver,
                            receiver_class,
                            true,
                            host,
                        );
                    }
                    Access::Private => {}
                }
            }
            class = self.superclass(c)?;
        }
        self.does_not_understand(selector, n, receiver, receiver_class, true, host)
    }

    /// Inserts an absent receiver below the top `n` arguments.
    fn insert_receiver(&mut self, receiver: Value, n: usize) {
        let at = self.stack.len() - n;
        self.stack.insert(at, receiver);
    }

    fn lexical_send(
        &mut self,
        selector: Value,
        n: usize,
        receiver: Value,
        mixin: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let receiver_class = self.class_of(receiver);
        let application = self.find_application_of(mixin, receiver_class)?;
        let sel = self.selector_bytes(selector)?;
        if let Some(m) = self.method_at(application, &sel)?
            && self.access(m) == Access::Private
        {
            self.insert_receiver(receiver, n);
            return self.activate(m, n, host);
        }
        self.protected_send(selector, n, receiver, receiver_class, host)
    }

    fn protected_send(
        &mut self,
        selector: Value,
        n: usize,
        receiver: Value,
        start: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let sel = self.selector_bytes(selector)?;
        let mut class = Some(start);
        while let Some(c) = class {
            if let Some(m) = self.method_at(c, &sel)?
                && self.access(m) != Access::Private
            {
                self.insert_receiver(receiver, n);
                return self.activate(m, n, host);
            }
            class = self.superclass(c)?;
        }
        self.does_not_understand(selector, n, receiver, start, false, host)
    }

    fn does_not_understand(
        &mut self,
        selector: Value,
        n: usize,
        receiver: Value,
        lookup_class: Value,
        present_receiver: bool,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let dnu_selector = self.roles.get(Role::DoesNotUnderstandSelector);
        let dnu = self.selector_bytes(dnu_selector)?;
        let mut class = Some(lookup_class);
        let mut method = None;
        while let Some(c) = class {
            if let Some(m) = self.method_at(c, &dnu)? {
                method = Some(m);
                break;
            }
            class = self.superclass(c)?;
        }
        let method = method.ok_or(VmError::RecursiveDoesNotUnderstand)?;
        let args = self.stack.split_off(self.stack.len() - n);
        let array_class = self.roles.get(Role::ArrayClass);
        let arguments = self
            .heap
            .allocate(array_class, Space::Actor(0), Body::Array(args));
        let message_class = self.roles.get(Role::MessageClass);
        let message = self.heap.allocate(
            message_class,
            Space::Actor(0),
            Body::Regular(alloc::vec![selector, arguments]),
        );
        if !present_receiver {
            self.stack.push(receiver);
        }
        self.stack.push(message);
        self.activate(method, 1, host)
    }

    /// Runs `method` with the receiver and `n` arguments on top of the stack.
    fn activate(
        &mut self,
        method: Value,
        n: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let m = self
            .heap
            .method(method)
            .ok_or(VmError::Malformed("method expected"))?;
        let header = m.header.clone();
        let base = self.stack.len() - 1 - n;
        match header.kind {
            MethodKind::Native => {
                let native = Native::from_id(header.native)
                    .filter(|nat| nat.arity() == n)
                    .ok_or(VmError::BadNative(header.native))?;
                self.run_native(native, base, n, host)
            }
            MethodKind::Getter => {
                let receiver = self.stack[base];
                let index = self.slot_index(method, receiver, header.slot)?;
                // A contract object (class, metaclass, mixin, method) answers
                // its contract field at that position, as `vm slotOf:at:` does.
                let value = match self.heap.get(receiver).map(|o| &o.body) {
                    Some(Body::Regular(slots)) => slots.get(index).copied(),
                    Some(body) => {
                        natives::readable_fields(body).and_then(|f| f.get(index).copied())
                    }
                    None => None,
                }
                .ok_or_else(|| self.bad_slot_access(method, receiver))?;
                self.stack.truncate(base);
                self.stack.push(value);
                Ok(None)
            }
            MethodKind::Setter => {
                let receiver = self.stack[base];
                let value = self.stack[base + 1];
                let index = self.slot_index(method, receiver, header.slot)?;
                match self.heap.get_mut(receiver).map(|o| &mut o.body) {
                    Some(Body::Regular(slots)) if index < slots.len() => {
                        slots[index] = value;
                    }
                    // The kernel's Ephemeron declares key, value, finalizer.
                    Some(Body::Ephemeron {
                        key,
                        value: v,
                        finalizer,
                    }) if index < 3 => {
                        *[key, v, finalizer][index] = value;
                    }
                    _ => return Err(self.bad_slot_access(method, receiver)),
                }
                self.stack.truncate(base + 1);
                Ok(None)
            }
            MethodKind::ReturnSelf => {
                self.stack.truncate(base + 1);
                Ok(None)
            }
            MethodKind::Ordinary => {
                if self.frames.len() >= MAX_FRAMES {
                    return Err(self.stack_exhausted());
                }
                let receiver = self.stack[base];
                let lp = self.stack.len();
                let nil = self.nil();
                self.stack
                    .extend(core::iter::repeat_n(nil, header.local_count as usize));
                let serial = self.next_serial;
                self.next_serial += 1;
                self.frames.push(Frame {
                    serial,
                    method,
                    receiver,
                    bp: base,
                    arg_count: n,
                    lp,
                    ip: 0,
                    activation: None,
                    closure: None,
                });
                Ok(None)
            }
        }
    }

    fn run_native(
        &mut self,
        native: Native,
        base: usize,
        n: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let receiver = self.stack[base];
        let args: Vec<Value> = self.stack[base + 1..base + 1 + n].to_vec();
        let outcome = {
            let mut ctx = NativeContext {
                heap: &mut self.heap,
                roles: &self.roles,
                host,
            };
            natives::invoke(native, receiver, &args, &mut ctx)
        };
        self.stack.truncate(base);
        let outcome = match outcome {
            Outcome::Frame(op) => self.frame_native(op, receiver, &args),
            other => other,
        };
        match outcome {
            // Rule Native-Answer.
            Outcome::Answer(v) => {
                self.stack.push(v);
                Ok(None)
            }
            // Rule Native-Tail: an ordinary send in the caller's frame.
            Outcome::TailSend {
                receiver,
                selector,
                args,
            } => {
                let count = args.len();
                self.stack.push(receiver);
                self.stack.extend(args);
                let selector = self.symbol(selector);
                self.ordinary_send(selector, count, host)
            }
            // Rule Native-Fail: allocate the failure exception and send it
            // #signal; its answer is the send's answer.
            Outcome::Fail(reason) => {
                let failure = self.new_failure(native, receiver, args, reason, host);
                self.stack.push(failure);
                let signal = self.symbol("signal");
                self.ordinary_send(signal, 0, host)
            }
            Outcome::Exit(code) => Ok(Some(Completion::Exited(code))),
            Outcome::Frame(_) => unreachable!("frame natives are resolved above"),
            Outcome::Collect => {
                // A safepoint: the receiver is back on the stack first.
                self.stack.push(receiver);
                self.collect_garbage();
                Ok(None)
            }
            Outcome::Snapshot => match self.snapshot_writer {
                Some(write) => {
                    let (heap, roles) = self.snapshot_heap();
                    let bytes = write(&heap, &roles);
                    let class = self.roles.get(Role::ByteArrayClass);
                    let v = self
                        .heap
                        .allocate(class, Space::Actor(0), Body::Bytes(bytes));
                    self.stack.push(v);
                    Ok(None)
                }
                None => {
                    let failure =
                        self.new_failure(native, receiver, args, Reason::Unsupported, host);
                    self.stack.push(failure);
                    let signal = self.symbol("signal");
                    self.ordinary_send(signal, 0, host)
                }
            },
            Outcome::SetRole { role, value } => {
                self.roles.set(role, value);
                self.stack.push(receiver);
                Ok(None)
            }
            Outcome::Perform {
                receiver,
                selector,
                args,
            } => {
                let count = args.len();
                self.stack.push(receiver);
                self.stack.extend(args);
                self.ordinary_send(selector, count, host)
            }
            Outcome::ActivateClosure { closure, args } => {
                self.stack.push(closure);
                let n = args.len();
                self.stack.extend(args);
                self.activate_closure(closure, n)?;
                Ok(None)
            }
        }
    }

    /// Pushes a frame running `closure`, which is on the stack below its `n`
    /// arguments. Its method and receiver are its defining activation's; its
    /// copied values become its first locals.
    fn activate_closure(&mut self, closure: Value, n: usize) -> Result<(), VmError> {
        if self.frames.len() >= MAX_FRAMES {
            return Err(self.stack_exhausted());
        }
        let c = self
            .heap
            .closure(closure)
            .ok_or(VmError::Malformed("closure expected"))?
            .clone();
        let defining = self
            .heap
            .activation(c.defining)
            .ok_or(VmError::Malformed("activation expected"))?;
        let (method, receiver) = (defining.method, defining.receiver);
        let code_len = self
            .heap
            .method(method)
            .ok_or(VmError::Malformed("method expected"))?
            .bytecode
            .len();
        if c.initial_ip as usize >= code_len {
            return Err(VmError::Malformed("closure body outside its method"));
        }
        let base = self.stack.len() - 1 - n;
        let lp = self.stack.len();
        self.stack.extend(c.copied.iter().copied());
        let serial = self.next_serial;
        self.next_serial += 1;
        self.frames.push(Frame {
            serial,
            method,
            receiver,
            bp: base,
            arg_count: n,
            lp,
            ip: c.initial_ip as usize,
            activation: None,
            closure: Some(closure),
        });
        Ok(())
    }

    /// The activation handle of frame `index`, created on first request.
    fn ensure_activation(&mut self, index: usize) -> Value {
        if let Some(a) = self.frames[index].activation {
            return a;
        }
        let f = &self.frames[index];
        let nil = self.nil();
        let record = crate::object::Activation {
            frame: Some(f.serial),
            method: f.method,
            receiver: f.receiver,
            closure: f.closure.unwrap_or(nil),
        };
        let class = self.roles.get(Role::ActivationClass);
        let a = self
            .heap
            .allocate(class, Space::Actor(0), Body::Activation(record));
        self.frames[index].activation = Some(a);
        a
    }

    /// The index of the live frame an activation handle names.
    fn frame_index(&self, activation: Value) -> Option<usize> {
        let serial = self.heap.activation(activation)?.frame?;
        self.frames.iter().rposition(|f| f.serial == serial)
    }

    /// The stack range a frame's temps and operands occupy: after its
    /// message receiver, up to the next frame's receiver (or the top).
    fn frame_extent(&self, index: usize) -> core::ops::Range<usize> {
        let start = self.frames[index].bp + 1;
        let end = match self.frames.get(index + 1) {
            Some(above) => above.bp,
            None => self.stack.len(),
        };
        start..end
    }

    /// An activation native (genesis plan, section 2). The current frame is
    /// the one that sent the message; the native's receiver and arguments
    /// are already off the stack.
    fn frame_native(&mut self, native: Native, receiver: Value, args: &[Value]) -> Outcome {
        let nil = self.nil();
        if native == Native::CurrentActivation {
            let top = self.frames.len() - 1;
            return Outcome::Answer(self.ensure_activation(top));
        }
        let Some(record) = self.heap.activation(receiver).cloned() else {
            return Outcome::Fail(Reason::WrongKind(0));
        };
        match native {
            Native::ActivationMethod => return Outcome::Answer(record.method),
            Native::ActivationReceiver => return Outcome::Answer(record.receiver),
            Native::ActivationClosure => return Outcome::Answer(record.closure),
            _ => {}
        }
        let Some(k) = self.frame_index(receiver) else {
            // A dead activation: no sender, no bci, no temps.
            return match native {
                Native::ActivationSender | Native::ActivationBci => Outcome::Answer(nil),
                Native::ActivationSize => Outcome::Answer(Value::small_integer(0).expect("small")),
                _ => Outcome::Fail(Reason::Dead),
            };
        };
        let extent = self.frame_extent(k);
        let temp = |v: Value| -> Result<usize, Reason> {
            match v.as_small_integer() {
                Some(i) if i >= 1 && (i as u64) <= extent.len() as u64 => {
                    Ok(extent.start + i as usize - 1)
                }
                Some(_) => Err(Reason::OutOfRange(1)),
                None => Err(Reason::WrongKind(1)),
            }
        };
        match native {
            Native::ActivationSender => Outcome::Answer(if k == 0 {
                nil
            } else {
                self.ensure_activation(k - 1)
            }),
            Native::ActivationBci => {
                Outcome::Answer(Value::small_integer(self.frames[k].ip as i64 + 1).expect("small"))
            }
            Native::ActivationSize => {
                Outcome::Answer(Value::small_integer(extent.len() as i64).expect("small"))
            }
            Native::ActivationTempAt => match temp(args[0]) {
                Ok(slot) => Outcome::Answer(self.stack[slot]),
                Err(r) => Outcome::Fail(r),
            },
            Native::ActivationTempAtPut => match temp(args[0]) {
                Ok(slot) => {
                    self.stack[slot] = args[1];
                    Outcome::Answer(args[1])
                }
                Err(r) => Outcome::Fail(r),
            },
            Native::ActivationSetSender => match self.frame_index(args[0]) {
                Some(j) if j < k => {
                    self.cut_back(k, j);
                    Outcome::Answer(receiver)
                }
                _ => Outcome::Fail(Reason::WrongKind(1)),
            },
            _ => Outcome::Fail(Reason::Unsupported),
        }
    }

    /// Makes frame `j` the sender of frame `k` (j < k): the frames in between
    /// are retired and their stack contents removed, so a return from `k`
    /// lands in `j`, which receives the answer where its pending send's
    /// receiver was.
    fn cut_back(&mut self, k: usize, j: usize) {
        if j + 1 == k {
            return;
        }
        let from = self.frames[j + 1].bp;
        let to = self.frames[k].bp;
        let removed: Vec<Frame> = self.frames.drain(j + 1..k).collect();
        for f in &removed {
            self.retire(f);
        }
        self.stack.drain(from..to);
        let delta = to - from;
        for f in &mut self.frames[j + 1..] {
            f.bp -= delta;
            f.lp -= delta;
        }
    }

    /// Marks the handle of a frame that has returned or been unwound as dead.
    fn retire(&mut self, frame: &Frame) {
        if let Some(a) = frame.activation
            && let Some(Body::Activation(record)) = self.heap.get_mut(a).map(|o| &mut o.body)
        {
            record.frame = None;
        }
    }

    /// Sends `selector` (a role) to the current frame's activation with
    /// `args`, as psoup does for #cannotReturn:, #aboutToReturn:through: and
    /// #nonBooleanReceiver:. Lookup ignores access, like DNU.
    fn upcall(
        &mut self,
        selector: Role,
        args: &[Value],
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let _ = host;
        let top = self.ensure_activation(self.frames.len() - 1);
        let sel = self.selector_bytes(self.roles.get(selector))?;
        let mut class = Some(self.class_of(top));
        let mut method = None;
        while let Some(c) = class {
            if let Some(m) = self.method_at(c, &sel)? {
                method = Some(m);
                break;
            }
            class = self.superclass(c)?;
        }
        let method = method.ok_or(VmError::MissingUpcall(selector.name()))?;
        self.stack.push(top);
        self.stack.extend(args.iter().copied());
        self.activate(method, args.len(), host)
    }

    /// The home method activation of a closure: its defining activation,
    /// followed outward through enclosing closures.
    fn home_of(&self, closure: Value) -> Result<Value, VmError> {
        let nil = self.nil();
        let mut home = self
            .heap
            .closure(closure)
            .ok_or(VmError::Malformed("closure expected"))?
            .defining;
        for _ in 0..MAX_FRAMES {
            let a = self
                .heap
                .activation(home)
                .ok_or(VmError::Malformed("activation expected"))?;
            if a.closure == nil {
                return Ok(home);
            }
            home = self
                .heap
                .closure(a.closure)
                .ok_or(VmError::Malformed("closure expected"))?
                .defining;
        }
        Err(VmError::Malformed("closure chain too deep"))
    }

    /// `^result` inside a closure: return from the home method, following
    /// psoup's protocol. An intervening ensure: frame gets
    /// #aboutToReturn:through:; a dead home, or a simulation-root or
    /// callback-dispatch frame in between, gets #cannotReturn:.
    fn non_local_return(
        &mut self,
        result: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let closure = self.frames.last().expect("a frame").closure;
        let Some(closure) = closure else {
            // In a method frame, ^ is a local return.
            self.local_return(result);
            return Ok(None);
        };
        let home = self.home_of(closure)?;
        let home_serial = self.heap.activation(home).and_then(|a| a.frame);
        let Some(home_serial) = home_serial else {
            return self.upcall(Role::CannotReturnSelector, &[result], host);
        };
        let ensure = self.roles.get(Role::EnsureMethod);
        let blockers = [
            self.roles.get(Role::SimulationRootMethod),
            self.roles.get(Role::CallbackDispatchMethod),
        ];
        let nil = self.nil();
        let top = self.frames.len() - 1;
        for i in (0..top).rev() {
            if self.frames[i].serial == home_serial {
                let popped = self.frames.split_off(i);
                for f in &popped {
                    self.retire(f);
                }
                self.stack.truncate(popped[0].bp);
                self.stack.push(result);
                return Ok(None);
            }
            let method = self.frames[i].method;
            if method == ensure && ensure != nil {
                let unwind = self.ensure_activation(i);
                return self.upcall(Role::AboutToReturnThroughSelector, &[result, unwind], host);
            }
            if blockers.iter().any(|&b| b == method && b != nil) {
                break;
            }
        }
        self.upcall(Role::CannotReturnSelector, &[result], host)
    }

    /// The failure object of `eq:native-failure-reification`: an instance of
    /// the failure class with slots native name, receiver, arguments, reason
    /// and reason argument. No user code runs.
    fn new_failure(
        &mut self,
        native: Native,
        receiver: Value,
        args: Vec<Value>,
        reason: Reason,
        host: &mut dyn Host,
    ) -> Value {
        let name = self.new_string(native.name());
        let array_class = self.roles.get(Role::ArrayClass);
        let arguments = self
            .heap
            .allocate(array_class, Space::Actor(0), Body::Array(args));
        let reason_name = self.new_string(reason.name());
        let reason_argument = {
            let mut ctx = NativeContext {
                heap: &mut self.heap,
                roles: &self.roles,
                host,
            };
            reason.argument(&mut ctx)
        };
        // The five failure fields are the failure class's own last five
        // slots, after any its superclasses declare (it is normally an
        // Exception subclass).
        let class = self.roles.get(Role::FailureClass);
        let nil = self.nil();
        let total = self.instance_slot_count(class).unwrap_or(5).max(5);
        let mut slots = alloc::vec![nil; total];
        slots[total - 5..].copy_from_slice(&[
            name,
            receiver,
            arguments,
            reason_name,
            reason_argument,
        ]);
        self.heap
            .allocate(class, Space::Actor(0), Body::Regular(slots))
    }

    // ----- Execution -----

    fn invalid(&self) -> VmError {
        let f = self.frames.last().expect("a frame");
        VmError::InvalidBytecode {
            selector: self.method_selector(f.method),
            index: f.ip,
        }
    }

    fn unimplemented(&self, what: &'static str) -> VmError {
        let f = self.frames.last().expect("a frame");
        VmError::Unimplemented {
            what,
            selector: self.method_selector(f.method),
        }
    }

    fn stack_exhausted(&self) -> VmError {
        VmError::StackExhausted(
            self.frames
                .iter()
                .rev()
                .take(24)
                .map(|f| self.method_selector(f.method))
                .collect(),
        )
    }

    fn bad_slot_access(&self, method: Value, receiver: Value) -> VmError {
        let class = self.class_of(receiver);
        let receiver_class = self
            .heap
            .behavior(class)
            .and_then(|b| self.heap.string_bytes(b.name))
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| String::from("?"));
        VmError::BadSlotAccess {
            selector: self.method_selector(method),
            receiver_class,
        }
    }

    fn method_selector(&self, method: Value) -> String {
        self.heap
            .method(method)
            .and_then(|m| self.heap.string_bytes(m.selector))
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| String::from("?"))
    }

    fn literal(&self, method: Value, i: usize) -> Option<Value> {
        let m = self.heap.method(method)?;
        self.heap.array_elements(m.literals)?.get(i).copied()
    }

    fn local_return(&mut self, result: Value) -> Step {
        let frame = self.frames.pop().expect("a frame");
        self.retire(&frame);
        self.stack.truncate(frame.bp);
        self.stack.push(result);
        Step::Continue
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        let lp = self.frames.last().expect("a frame").lp;
        if self.stack.len() <= lp {
            return Err(self.invalid());
        }
        Ok(self.stack.pop().expect("checked"))
    }

    fn top(&self) -> Result<Value, VmError> {
        let lp = self.frames.last().expect("a frame").lp;
        if self.stack.len() <= lp {
            return Err(self.invalid());
        }
        Ok(*self.stack.last().expect("checked"))
    }

    fn local_slot(&self, i: usize) -> Result<usize, VmError> {
        let f = self.frames.last().expect("a frame");
        let slot = f.lp + i;
        if slot >= self.stack.len() {
            return Err(self.invalid());
        }
        Ok(slot)
    }

    fn send_by_literal(
        &mut self,
        kind: SendKind,
        selector_index: usize,
        n: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let (method, receiver) = {
            let f = self.frames.last().expect("a frame");
            (f.method, f.receiver)
        };
        let selector = self
            .literal(method, selector_index)
            .ok_or_else(|| self.invalid())?;
        let available = self.stack.len() - self.frames.last().expect("a frame").lp;
        let needed = if kind == SendKind::Ordinary { n + 1 } else { n };
        if available < needed {
            return Err(self.invalid());
        }
        let mixin = self.heap.method(method).expect("method").mixin;
        match kind {
            SendKind::Ordinary => self.ordinary_send(selector, n, host),
            SendKind::SelfSend => self.lexical_send(selector, n, receiver, mixin, host),
            SendKind::Super => {
                let class = self.class_of(receiver);
                let application = self.find_application_of(mixin, class)?;
                match self.superclass(application)? {
                    Some(start) => self.protected_send(selector, n, receiver, start, host),
                    None => {
                        self.does_not_understand(selector, n, receiver, application, false, host)
                    }
                }
            }
            SendKind::ImplicitReceiver => {
                let sel = self.selector_bytes(selector)?;
                let mut candidate_receiver = receiver;
                let mut candidate_mixin = mixin;
                loop {
                    let application = self
                        .find_application_of(candidate_mixin, self.class_of(candidate_receiver))?;
                    if self.method_at(application, &sel)?.is_some() {
                        return self.lexical_send(
                            selector,
                            n,
                            candidate_receiver,
                            candidate_mixin,
                            host,
                        );
                    }
                    let enclosing = self
                        .heap
                        .mixin(candidate_mixin)
                        .ok_or(VmError::Malformed("mixin expected"))?
                        .enclosing_mixin;
                    if enclosing == self.nil() {
                        break;
                    }
                    candidate_mixin = enclosing;
                    candidate_receiver = self
                        .heap
                        .behavior(application)
                        .expect("class")
                        .enclosing_object;
                }
                let class = self.class_of(receiver);
                self.protected_send(selector, n, receiver, class, host)
            }
        }
    }

    fn outer_send(
        &mut self,
        selector_index: usize,
        n: usize,
        depth: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let (method, mut receiver) = {
            let f = self.frames.last().expect("a frame");
            (f.method, f.receiver)
        };
        let selector = self
            .literal(method, selector_index)
            .ok_or_else(|| self.invalid())?;
        let mut mixin = self.heap.method(method).expect("method").mixin;
        for _ in 0..depth {
            let application = self.find_application_of(mixin, self.class_of(receiver))?;
            receiver = self
                .heap
                .behavior(application)
                .expect("class")
                .enclosing_object;
            mixin = self
                .heap
                .mixin(mixin)
                .ok_or(VmError::Malformed("mixin expected"))?
                .enclosing_mixin;
        }
        self.lexical_send(selector, n, receiver, mixin, host)
    }

    fn enclosing_object(&self, depth: usize) -> Result<Value, VmError> {
        let f = self.frames.last().expect("a frame");
        let mut receiver = f.receiver;
        let mut mixin = self.heap.method(f.method).expect("method").mixin;
        for _ in 0..depth {
            let application = self.find_application_of(mixin, self.class_of(receiver))?;
            receiver = self
                .heap
                .behavior(application)
                .expect("class")
                .enclosing_object;
            mixin = self
                .heap
                .mixin(mixin)
                .ok_or(VmError::Malformed("mixin expected"))?
                .enclosing_mixin;
        }
        Ok(receiver)
    }

    /// A conditional branch. A non-Boolean sends #nonBooleanReceiver: to the
    /// current activation, as psoup does; its answer stays on the stack and
    /// the branch is not taken (the kernel's handler normally raises).
    fn branch_if(
        &mut self,
        condition: bool,
        forward: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let v = self.pop()?;
        let taken = if v == self.roles.get(Role::True) {
            condition
        } else if v == self.roles.get(Role::False) {
            !condition
        } else {
            return self.upcall(Role::NonBooleanReceiverSelector, &[v], host);
        };
        if taken {
            self.frames.last_mut().expect("a frame").ip += forward;
        }
        Ok(None)
    }

    fn step(&mut self, host: &mut dyn Host) -> Result<Step, VmError> {
        let (method, ip) = {
            let f = self.frames.last().expect("a frame");
            (f.method, f.ip)
        };
        let bytes: [u8; 4] = {
            let code = &self
                .heap
                .method(method)
                .ok_or(VmError::Malformed("method expected"))?
                .bytecode;
            let Some(&first) = code.get(ip) else {
                return Err(self.invalid());
            };
            let len = instruction_length(first);
            if ip + len > code.len() {
                return Err(self.invalid());
            }
            let mut b = [0u8; 4];
            b[..len].copy_from_slice(&code[ip..ip + len]);
            b
        };
        let opcode = bytes[0];
        let next = ip + instruction_length(opcode);
        self.frames.last_mut().expect("a frame").ip = next;
        let wide = || u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
        let ext_selector = || bytes[1] as usize | ((bytes[2] as usize & 0x0f) << 8);
        let ext_args = || (bytes[2] >> 4) as usize;
        let sent = match opcode {
            0..=15 => {
                let target = next
                    .checked_sub((opcode & 15) as usize)
                    .ok_or_else(|| self.invalid())?;
                self.frames.last_mut().expect("a frame").ip = target;
                None
            }
            16..=31 => {
                self.frames.last_mut().expect("a frame").ip += (opcode & 15) as usize;
                None
            }
            32..=47 => self.branch_if(true, (opcode & 15) as usize, host)?,
            48..=63 => self.branch_if(false, (opcode & 15) as usize, host)?,
            64..=111 => {
                let kind = match opcode {
                    64..=79 => SendKind::Ordinary,
                    80..=95 => SendKind::SelfSend,
                    _ => SendKind::ImplicitReceiver,
                };
                self.send_by_literal(
                    kind,
                    (opcode & 7) as usize,
                    ((opcode >> 3) & 1) as usize,
                    host,
                )?
            }
            112..=119 => {
                let p = (opcode & 7) as usize;
                self.push_parameter(p)?;
                None
            }
            120..=127 => {
                let slot = self.local_slot((opcode & 7) as usize)?;
                self.stack.push(self.stack[slot]);
                None
            }
            128..=135 => {
                let slot = self.local_slot((opcode & 7) as usize)?;
                let v = self.pop()?;
                self.stack[slot] = v;
                None
            }
            136..=143 => {
                let slot = self.local_slot((opcode & 7) as usize)?;
                self.stack[slot] = self.top()?;
                None
            }
            144..=151 => {
                let v = self
                    .literal(method, (opcode & 7) as usize)
                    .ok_or_else(|| self.invalid())?;
                self.stack.push(v);
                None
            }
            op::PUSH_NIL => {
                self.stack.push(self.nil());
                None
            }
            op::PUSH_FALSE => {
                self.stack.push(self.roles.get(Role::False));
                None
            }
            op::PUSH_TRUE => {
                self.stack.push(self.roles.get(Role::True));
                None
            }
            op::PUSH_SELF => {
                let r = self.frames.last().expect("a frame").receiver;
                self.stack.push(r);
                None
            }
            op::PUSH_MIXIN => {
                let mixin = self.heap.method(method).expect("method").mixin;
                self.stack.push(mixin);
                None
            }
            op::POP => {
                self.pop()?;
                None
            }
            op::DUP => {
                let v = self.top()?;
                self.stack.push(v);
                None
            }
            160..=163 => {
                self.stack
                    .push(Value::small_integer(i64::from(opcode) - 161).expect("small"));
                None
            }
            op::RETURN_NIL => return Ok(self.local_return(self.nil())),
            op::RETURN_FALSE => return Ok(self.local_return(self.roles.get(Role::False))),
            op::RETURN_TRUE => return Ok(self.local_return(self.roles.get(Role::True))),
            op::RETURN_SELF => {
                let r = self.frames.last().expect("a frame").receiver;
                return Ok(self.local_return(r));
            }
            op::RETURN_TOP => {
                let v = self.pop()?;
                return Ok(self.local_return(v));
            }
            171..=175 => {
                let result = match opcode {
                    171 => self.nil(),
                    172 => self.roles.get(Role::False),
                    173 => self.roles.get(Role::True),
                    174 => self.frames.last().expect("a frame").receiver,
                    _ => self.pop()?,
                };
                self.non_local_return(result, host)?
            }
            176..=207 => {
                let k = (opcode - 176) as usize;
                let table = self.roles.get(Role::CommonSelectors);
                let entries = self
                    .heap
                    .array_elements(table)
                    .ok_or(VmError::Malformed("common selectors must be an Array"))?;
                let (Some(&selector), Some(&arity)) = (entries.get(2 * k), entries.get(2 * k + 1))
                else {
                    return Err(VmError::Malformed("common-selector table too short"));
                };
                let n = arity
                    .as_small_integer()
                    .and_then(|a| usize::try_from(a).ok())
                    .ok_or(VmError::Malformed("common-selector arity"))?;
                if self.stack.len() - self.frames.last().expect("a frame").lp < n + 1 {
                    return Err(self.invalid());
                }
                self.ordinary_send(selector, n, host)?
            }
            op::PUSH_NEW_ARRAY => {
                let nil = self.nil();
                let class = self.roles.get(Role::ArrayClass);
                let a = self.heap.allocate(
                    class,
                    Space::Actor(0),
                    Body::Array(alloc::vec![nil; bytes[1] as usize]),
                );
                self.stack.push(a);
                None
            }
            op::PUSH_NEW_ARRAY_WITH_ELEMENTS => {
                let n = bytes[1] as usize;
                if self.stack.len() - self.frames.last().expect("a frame").lp < n {
                    return Err(self.invalid());
                }
                let elements = self.stack.split_off(self.stack.len() - n);
                let class = self.roles.get(Role::ArrayClass);
                let a = self
                    .heap
                    .allocate(class, Space::Actor(0), Body::Array(elements));
                self.stack.push(a);
                None
            }
            op::PUSH_PARAMETER => {
                self.push_parameter(bytes[1] as usize)?;
                None
            }
            op::PUSH_LOCAL => {
                let slot = self.local_slot(bytes[1] as usize)?;
                self.stack.push(self.stack[slot]);
                None
            }
            op::POP_INTO_LOCAL => {
                let slot = self.local_slot(bytes[1] as usize)?;
                let v = self.pop()?;
                self.stack[slot] = v;
                None
            }
            op::STORE_INTO_LOCAL => {
                let slot = self.local_slot(bytes[1] as usize)?;
                self.stack[slot] = self.top()?;
                None
            }
            op::PUSH_ENCLOSING_OBJECT => {
                let v = self.enclosing_object(bytes[1] as usize)?;
                self.stack.push(v);
                None
            }
            op::EVENTUAL_SEND => return Err(self.unimplemented("eventual send")),
            op::BRANCH_BACK => {
                let target = next.checked_sub(wide()).ok_or_else(|| self.invalid())?;
                self.frames.last_mut().expect("a frame").ip = target;
                None
            }
            op::BRANCH => {
                self.frames.last_mut().expect("a frame").ip += wide();
                None
            }
            op::BRANCH_IF_TRUE => self.branch_if(true, wide(), host)?,
            op::BRANCH_IF_FALSE => self.branch_if(false, wide(), host)?,
            op::PUSH_INDIRECT_LOCAL
            | op::POP_INTO_INDIRECT_LOCAL
            | op::STORE_INTO_INDIRECT_LOCAL => {
                let slot = self.local_slot(bytes[2] as usize)?;
                let vector = self.stack[slot];
                let index = bytes[1] as usize;
                let value = match opcode {
                    op::PUSH_INDIRECT_LOCAL => None,
                    op::POP_INTO_INDIRECT_LOCAL => Some(self.pop()?),
                    _ => Some(self.top()?),
                };
                let invalid = self.invalid();
                match self
                    .heap
                    .get_mut(vector)
                    .map(|o| (&mut o.body, o.header.flags))
                {
                    Some((Body::Array(elements), flags)) if index < elements.len() => match value {
                        None => {
                            let v = elements[index];
                            self.stack.push(v);
                        }
                        Some(_) if flags & FROZEN != 0 => return Err(invalid),
                        Some(v) => elements[index] = v,
                    },
                    _ => return Err(invalid),
                }
                None
            }
            op::PUSH_LITERAL => {
                let v = self.literal(method, wide()).ok_or_else(|| self.invalid())?;
                self.stack.push(v);
                None
            }
            op::PUSH_INTEGER => {
                let n = i16::from_le_bytes([bytes[1], bytes[2]]);
                self.stack
                    .push(Value::small_integer(n.into()).expect("small"));
                None
            }
            op::ORDINARY_SEND => {
                self.send_by_literal(SendKind::Ordinary, ext_selector(), ext_args(), host)?
            }
            op::SELF_SEND => {
                self.send_by_literal(SendKind::SelfSend, ext_selector(), ext_args(), host)?
            }
            op::SUPER_SEND => {
                self.send_by_literal(SendKind::Super, ext_selector(), ext_args(), host)?
            }
            op::IMPLICIT_RECEIVER_SEND => {
                self.send_by_literal(SendKind::ImplicitReceiver, ext_selector(), ext_args(), host)?
            }
            op::OUTER_SEND => {
                self.outer_send(ext_selector(), ext_args(), bytes[3] as usize, host)?
            }
            op::PUSH_CLOSURE => {
                let copied_count = (bytes[1] >> 4) as usize;
                let num_args = u32::from(bytes[1] & 15);
                let length = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
                let code_len = self.heap.method(method).expect("method").bytecode.len();
                if self.stack.len() - self.frames.last().expect("a frame").lp < copied_count
                    || next + length > code_len
                {
                    return Err(self.invalid());
                }
                let copied = self.stack.split_off(self.stack.len() - copied_count);
                let top = self.frames.len() - 1;
                let defining = self.ensure_activation(top);
                let class = self.roles.get(Role::ClosureClass);
                let closure = self.heap.allocate(
                    class,
                    Space::Actor(0),
                    Body::Closure(crate::object::Closure {
                        defining,
                        initial_ip: next as u32,
                        num_args,
                        copied,
                    }),
                );
                self.stack.push(closure);
                self.frames.last_mut().expect("a frame").ip = next + length;
                None
            }
            _ => return Err(self.invalid()),
        };
        Ok(match sent {
            Some(done) => Step::Done(done),
            None => Step::Continue,
        })
    }

    fn push_parameter(&mut self, p: usize) -> Result<(), VmError> {
        let f = self.frames.last().expect("a frame");
        if p == 0 || p > f.arg_count {
            return Err(self.invalid());
        }
        let v = self.stack[f.bp + 1 + (f.arg_count - p)];
        self.stack.push(v);
        Ok(())
    }
}
