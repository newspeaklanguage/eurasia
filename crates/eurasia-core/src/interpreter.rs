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

// Errors are built lazily (`ok_or_else`) on the hot paths: building one
// eagerly and dropping it cost about a fifth of the run time
// (EURASIA_PERFORMANCE_2026-11-19.md).
#![allow(clippy::unnecessary_lazy_evaluations)]

use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::bytecode::{SendKind, instruction_length, op};
use crate::gc;
use crate::host::Host;
use crate::host::{HostCallbacks, HostError, HostValue};
use crate::machine::{ActorState, FrameState, MachineState};
use crate::natives::{self, Native, NativeContext, Outcome, Reason};
use crate::object::{
    Access, Body, FROZEN, Format, Heap, MethodHeader, MethodKind, PromiseRecord, Space, VALUE,
};
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
    /// The host asked the VM to stop ([`Host::interrupted`]): a replay that
    /// diverged.
    Interrupted,
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
            VmError::Interrupted => write!(f, "interrupted by the host"),
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
    /// Nothing can run before this monotonic time (in nanoseconds), the next
    /// timer, and the host does not block ([`Host::blocks`]): it calls
    /// [`Vm::resume`] once that time has come.
    Waiting(u64),
    /// The top frame is at an entry of a compiled method: the host calls
    /// compiled function `.0` with the VM's address and the entry's block
    /// `.1`, then [`Vm::continue_after_compiled`] with its status
    /// (newspeak/EURASIA_COMPILER_DESIGN_2026-11-19.md).
    Compiled(u32, u32),
}

#[derive(Clone, Debug)]
struct Frame {
    /// A number unique to this frame for the life of the VM.
    serial: u64,
    method: Value,
    /// The method's bytecode, so that an instruction fetch does not look
    /// the method up. A method's bytecode can change only while it is
    /// unfrozen (`Method>>bytecode:`); a frame keeps the code it began with.
    code: Rc<[u8]>,
    /// The method's literal array and mixin, for the same reason; reloaded
    /// whenever the lookup cache is cleared.
    literals: Value,
    mixin: Value,
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

/// How a send looks its method up: the key of the lookup cache besides the
/// receiver's class and the selector (and, for every rule but `Ordinary`,
/// the sending method's mixin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    Ordinary,
    SelfSend,
    Super,
    ImplicitReceiver,
    /// An outer send this many levels out.
    Outer(usize),
}

impl Rule {
    fn code(self) -> u64 {
        match self {
            Rule::Ordinary => 0,
            Rule::SelfSend => 1,
            Rule::Super => 2,
            Rule::ImplicitReceiver => 3,
            Rule::Outer(depth) => 4 + depth as u64,
        }
    }
}

/// Where a send's receiver comes from, once its method is found.
#[derive(Clone, Copy, Debug)]
enum TargetReceiver {
    /// Already on the stack, below the arguments (an ordinary send).
    Present,
    /// The sending frame's receiver, inserted below the arguments.
    Frame,
    /// An enclosing object, inserted below the arguments; it depends only on
    /// the receiver's class and the sending mixin, so it can be cached.
    Fixed(Value),
}

/// What a send runs: the method and its header (so that activating an
/// accessor or a native does not look the method up), its receiver and,
/// for an accessor, the absolute index of its slot in that receiver.
#[derive(Clone, Copy, Debug)]
struct Target {
    method: Value,
    header: MethodHeader,
    receiver: TargetReceiver,
    slot: Option<usize>,
}

/// An entry of the lookup cache: a send's key and what it runs. Only sends
/// that find a method are cached; a #doesNotUnderstand: takes the full
/// lookup every time.
#[derive(Clone, Copy, Debug)]
struct CacheEntry {
    class: Value,
    selector: Value,
    mixin: Value,
    rule: Rule,
    target: Target,
}

/// How [`Vm::run_simple`]'s inner loop ends.
enum Exit {
    /// At the instruction where the event loop polls the host.
    Polls,
    /// At an instruction for `step`.
    Step,
    /// After a send it made itself.
    Sent(Option<Completion>),
}

/// The common selectors (0 to 13: `+` to `=`) with SmallInteger fast paths.
const SMALL_INTEGER_FAST_PATHS: usize = 14;

/// The lookup cache's size, a power of two. It is direct-mapped.
const LOOKUP_CACHE_SIZE: usize = 4096;

/// The lookup cache slot of a key.
fn cache_index(class: Value, selector: Value, mixin: Value, rule: Rule) -> usize {
    let h =
        (class.to_bits() ^ selector.to_bits().rotate_left(21) ^ mixin.to_bits().rotate_left(42))
            .wrapping_add(rule.code())
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (h >> (64 - LOOKUP_CACHE_SIZE.trailing_zeros())) as usize
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
    /// Set when a turn ends with `finish:`: the next timer's time (0: none).
    finished_turn: Option<u64>,
    /// Whether the image runs a message loop (the MessageLoop role is set).
    has_loop: bool,
    /// The cohort's actors (`EURASIA_ACTORS_DESIGN_2026-11-17.md` section
    /// 5); actor 0 runs the image's own platform.
    actors: Vec<Actor>,
    /// The running actor.
    current: usize,
    /// Expats (operation list, section 10): closures the host may call, with
    /// their owning actors; a host function names one by its index. The
    /// table only grows, as psoup's does; expat lifetime is open.
    expats: Vec<(Value, u32)>,
    /// Callbacks now running, nested.
    callback_depth: usize,
    /// Commits so far (`vm programVersion`).
    program_version: u64,
    /// The checkpoint policy (`vm checkpointPolicy:`): a checkpoint at the
    /// first safepoint after this many allocations; 0 is none.
    checkpoint_every: u64,
    /// The allocation counter at the last checkpoint.
    last_checkpoint: u64,
    /// `vm checkpointNow` asked for one at the next safepoint.
    checkpoint_pending: bool,
    /// `saveImage:` asked for the checkpoint at the next safepoint to be
    /// saved under this name.
    save_pending: Option<String>,
    /// Steps until the event loop next asks the host whether to stop.
    interrupt_countdown: u32,
    /// The allocation count at which the next collection or checkpoint can
    /// be due, so that a safepoint costs one comparison until then. Any
    /// change to what decides that sets it to 0, which makes the next
    /// safepoint check in full and compute it again.
    safepoint_at: u64,
    /// Restored into a new host: the next [`Vm::continue_run`] begins a new
    /// incarnation.
    incarnation_pending: bool,
    /// The global lookup cache. It holds references, so it is cleared
    /// whenever objects move or are forwarded (a collection, a commit) and
    /// whenever a method or mixin is written; like psoup's, it is not
    /// cleared when an element of a mixin's (unfrozen) method array is.
    lookup_cache: Vec<Option<CacheEntry>>,
    /// Activations of a method before it is compiled; 0: never.
    jit_threshold: i64,
    /// What compiled code stopped the run with ([`crate::compiler::STOP`]).
    jit_stop: Option<Result<Option<Completion>, VmError>>,
    /// The frame count when the host last entered compiled code: compiled
    /// callees nest in the host's own stack, so their depth is bounded.
    jit_base: usize,
    /// The registers compiled code reads ([`crate::compiler::Regs`]), boxed
    /// so that their address holds while it runs.
    jit_regs: alloc::boxed::Box<crate::compiler::Regs>,
    /// For each arithmetic or comparison common selector (0 to 13), whether
    /// SmallInteger's method for it is the native the fast path computes:
    /// 0 not yet known, 1 yes, 2 no. Cleared with the lookup cache.
    small_integer_fast: [u8; SMALL_INTEGER_FAST_PATHS],
}

/// Which collection [`Vm::collect`] runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Collection {
    /// The nursery only.
    Minor,
    /// Everything, clearing weak references to garbage.
    Full,
    /// Everything not reachable at all, weakly or strongly (a checkpoint's).
    KeepingWeak,
}

/// A VM actor: its message loop (in its platform), its mailbox of packets
/// (Arrays in its heap), and its next timer. Its heap is the objects of
/// space `Actor(n)`. Turns run to completion, so an actor that is not
/// running has no stack (until pausing, M6).
struct Actor {
    message_loop: Value,
    mailbox: VecDeque<Value>,
    /// The monotonic time of its next timer, in nanoseconds; 0: none.
    wakeup: u64,
    /// A callback ran while the VM was idle: its message loop must run (to
    /// drain the sends the callback queued) even with no packet or timer.
    poked: bool,
}

/// How deeply callbacks may nest (design doc section 4; psoup's limit).
const MAX_CALLBACK_NESTING: usize = 64;

/// The fewest allocations between scheduled collections.
const MIN_GC_BUDGET: u64 = 1 << 20;

/// Serializes a heap and its roles (the snapshot format's writer). The core
/// cannot depend on `eurasia-snapshot`, which depends on it, so the embedder
/// supplies the writer; `vm snapshot` fails with `unsupported` without one.
/// Given a machine state, it writes a checkpoint (M7), which a VM restores
/// to continue where it was taken; without one, a snapshot to start afresh.
pub type SnapshotWriter = fn(&Heap, &Roles, Option<&MachineState>) -> Vec<u8>;

/// How often the event loop asks the host whether to stop.
const INTERRUPT_INTERVAL: u32 = 1024;

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
            finished_turn: None,
            has_loop: false,
            actors: Vec::new(),
            current: 0,
            expats: Vec::new(),
            callback_depth: 0,
            program_version: 0,
            checkpoint_every: 0,
            last_checkpoint: 0,
            checkpoint_pending: false,
            save_pending: None,
            interrupt_countdown: INTERRUPT_INTERVAL,
            safepoint_at: 0,
            incarnation_pending: false,
            lookup_cache: alloc::vec![None; LOOKUP_CACHE_SIZE],
            jit_threshold: 0,
            jit_stop: None,
            jit_base: 0,
            jit_regs: alloc::boxed::Box::new(crate::compiler::Regs {
                epoch: 1,
                ..crate::compiler::Regs::default()
            }),
            small_integer_fast: [0; SMALL_INTEGER_FAST_PATHS],
        }
    }

    /// Clears the lookup cache, after anything that moves, forwards or
    /// changes program objects, and reloads what each frame keeps of its
    /// method (its literals and mixin), which such a change may also move.
    fn flush_lookup_cache(&mut self) {
        self.lookup_cache.fill(None);
        // Compiled code's inline caches go with it.
        self.jit_regs.epoch = self.jit_regs.epoch.wrapping_add(1).max(1);
        self.small_integer_fast = [0; SMALL_INTEGER_FAST_PATHS];
        let nil = self.nil();
        for f in &mut self.frames {
            (f.literals, f.mixin) = self
                .heap
                .method(f.method)
                .map_or((nil, nil), |m| (m.literals(), m.mixin()));
        }
    }

    /// Common send `k` (0 to 13) to two SmallIntegers, computed without a
    /// send when SmallInteger's method for it is the expected native and
    /// that native would answer: `None` otherwise (overflow, a zero
    /// divisor, a shift out of range), and the send happens as usual.
    fn small_integer_fast_path(&mut self, k: usize) -> Option<Value> {
        let len = self.stack.len();
        if k >= SMALL_INTEGER_FAST_PATHS || len < self.frames.last()?.lp + 2 {
            return None;
        }
        let a = self.stack[len - 2].as_small_integer()?;
        let b = self.stack[len - 1].as_small_integer()?;
        self.small_integer_op(k, a, b)
    }

    /// Common send `k` (below [`SMALL_INTEGER_FAST_PATHS`]) of SmallIntegers
    /// `a` and `b`, as [`Vm::small_integer_fast_path`] computes it.
    #[inline(always)]
    fn small_integer_op(&mut self, k: usize, a: i64, b: i64) -> Option<Value> {
        if self.small_integer_fast[k] == 0 {
            self.small_integer_fast[k] = if self.small_integer_native_is(k) {
                1
            } else {
                2
            };
        }
        if self.small_integer_fast[k] != 1 {
            return None;
        }
        let boolean =
            |vm: &Vm, t: bool| Some(vm.roles.get(if t { Role::True } else { Role::False }));
        let n = match k {
            0 => a.checked_add(b)?,
            1 => a.checked_sub(b)?,
            2 => a.checked_mul(b)?,
            3 => {
                if b == 0 {
                    return None;
                }
                let q = a.checked_div(b)?;
                if a % b != 0 && ((a < 0) != (b < 0)) {
                    q - 1
                } else {
                    q
                }
            }
            4 => {
                if b == 0 {
                    return None;
                }
                let r = a.checked_rem(b)?;
                if r != 0 && ((r < 0) != (b < 0)) {
                    r + b
                } else {
                    r
                }
            }
            5 => {
                if !(0..=62).contains(&b) {
                    return None;
                }
                let r = a << b;
                if r >> b != a {
                    return None;
                }
                r
            }
            6 => {
                if a < 0 || !(0..=63).contains(&b) {
                    return None;
                }
                a >> b
            }
            7 => a & b,
            8 => a | b,
            9 => return boolean(self, a < b),
            10 => return boolean(self, a > b),
            11 => return boolean(self, a <= b),
            12 => return boolean(self, a >= b),
            _ => return boolean(self, a == b),
        };
        Value::small_integer(n)
    }

    /// Whether SmallInteger's method for common selector `k` is the native
    /// [`Vm::small_integer_fast_path`] computes.
    fn small_integer_native_is(&self, k: usize) -> bool {
        let expected = [
            Native::IntegerAdd,
            Native::IntegerSubtract,
            Native::IntegerMultiply,
            Native::NumberFloorDivide,
            Native::NumberFloorModulo,
            Native::IntegerShiftLeft,
            Native::IntegerShiftRight,
            Native::IntegerBitAnd,
            Native::IntegerBitOr,
            Native::IntegerLess,
            Native::IntegerGreater,
            Native::IntegerLessEqual,
            Native::IntegerGreaterEqual,
            Native::NumberEqual,
        ][k];
        let table = self.roles.get(Role::CommonSelectors);
        let Some(&selector) = self.heap.array_elements(table).and_then(|e| e.get(2 * k)) else {
            return false;
        };
        let class = self.roles.get(Role::SmallIntegerClass);
        let Some(sel) = self.heap.string_bytes(selector) else {
            return false;
        };
        let Ok(Some(m)) = self.lookup_ordinary(class, sel) else {
            return false;
        };
        self.heap.method(m).is_some_and(|m| {
            m.header().kind == MethodKind::Native
                && Native::from_id(m.header().native) == Some(expected)
        })
    }

    /// Stress mode: with `Some(n)`, collect at the first safepoint after
    /// every `n` allocations (`n` = 1: after every allocation), so that a
    /// reference the VM holds across a safepoint without rooting it shows up
    /// at once (implementation plan, M3). `None` is normal scheduling.
    pub fn set_gc_stress(&mut self, interval: Option<u64>) {
        self.gc_stress = interval.map(|n| n.max(1));
        self.safepoint_at = 0;
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
        self.collect(Collection::Full)
    }

    /// A collection the allocation budget schedules: minor, unless a full
    /// one is due (EURASIA_NURSERY_DESIGN_2026-11-19.md section 2). Under
    /// stress, every fourth is full, so that both kinds run.
    fn scheduled_collection(&mut self) -> gc::Statistics {
        let full = self.heap.full_collection_due()
            || (self.gc_stress.is_some() && self.collections % 4 == 3);
        self.collect(if full {
            Collection::Full
        } else {
            Collection::Minor
        })
    }

    fn collect(&mut self, kind: Collection) -> gc::Statistics {
        let roots = self.roots();
        let nil = self.roles.get(Role::Nil);
        let (fwd, statistics) = match kind {
            Collection::Full => self.heap.collect(&roots, nil),
            Collection::Minor => self.heap.collect_minor(&roots, nil),
            Collection::KeepingWeak => self.heap.collect_keeping_weak(&roots, nil),
        };
        for e in &mut self.expats {
            e.0 = fwd.forward(e.0);
        }
        for a in &mut self.actors {
            a.message_loop = fwd.forward(a.message_loop);
            for v in a.mailbox.iter_mut() {
                *v = fwd.forward(*v);
            }
        }
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
        self.flush_lookup_cache();
        self.gc_counter = self.heap.allocation_counter();
        self.gc_budget = MIN_GC_BUDGET.max(statistics.after as u64);
        self.safepoint_at = 0;
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

    /// Every value the VM holds outside the heap: the roots of a collection.
    fn roots(&self) -> Vec<Value> {
        let mut roots: Vec<Value> = self.roles.pairs().map(|(_, v)| v).collect();
        roots.extend(self.stack.iter().copied());
        for f in &self.frames {
            roots.extend([f.method, f.receiver]);
            roots.extend(f.activation);
            roots.extend(f.closure);
        }
        roots.extend(self.symbols.values().copied());
        for a in &self.actors {
            roots.push(a.message_loop);
            roots.extend(a.mailbox.iter().copied());
        }
        roots.extend(self.expats.iter().map(|&(c, _)| c));
        roots
    }

    /// The VM as a checkpoint saves it (M7 design, section 3.2): a copy of
    /// the heap holding everything the VM's roots reach, weakly or strongly
    /// (so that weak observations clear at the same collections as in the
    /// uninterrupted run), its roles, and the machine state, all in the
    /// copy's terms. The running VM is not disturbed.
    pub fn checkpoint_image(&self) -> (Heap, Roles, MachineState) {
        let mut heap = self.heap.clone();
        let (fwd, _) = heap.collect_keeping_weak(&self.roots(), self.nil());
        let f = |v: Value| fwd.forward(v);
        let mut roles = self.roles.clone();
        let pairs: Vec<(Role, Value)> = self.roles.pairs().collect();
        for (role, v) in pairs {
            roles.set(role, f(v));
        }
        let state = MachineState {
            stack: self.stack.iter().map(|&v| f(v)).collect(),
            frames: self
                .frames
                .iter()
                .map(|fr| FrameState {
                    serial: fr.serial,
                    method: f(fr.method),
                    receiver: f(fr.receiver),
                    bp: fr.bp as u64,
                    arg_count: fr.arg_count as u64,
                    lp: fr.lp as u64,
                    ip: fr.ip as u64,
                    activation: fr.activation.map(f),
                    closure: fr.closure.map(f),
                })
                .collect(),
            actors: self
                .actors
                .iter()
                .map(|a| ActorState {
                    message_loop: f(a.message_loop),
                    mailbox: a.mailbox.iter().map(|&v| f(v)).collect(),
                    wakeup: a.wakeup,
                    poked: a.poked,
                })
                .collect(),
            current: self.current as u64,
            expats: self.expats.iter().map(|&(c, o)| (f(c), o)).collect(),
            symbols: self
                .symbols
                .iter()
                .map(|(&k, &v)| (String::from(k), f(v)))
                .collect(),
            next_serial: self.next_serial,
            gc_counter: self.gc_counter,
            gc_budget: self.gc_budget,
            collections: self.collections,
            finished_turn: self.finished_turn,
            has_loop: self.has_loop,
            program_version: self.program_version,
            checkpoint_every: self.checkpoint_every,
            last_checkpoint: self.last_checkpoint,
        };
        (heap, roles, state)
    }

    /// A VM restored from a checkpoint (`checkpoint_image`, or a loaded
    /// one): [`Vm::continue_run`] continues it.
    pub fn restore(heap: Heap, roles: Roles, state: MachineState) -> Vm {
        let mut vm = Vm::new(heap, roles);
        let nil = vm.nil();
        vm.stack = state.stack;
        vm.frames = state
            .frames
            .into_iter()
            .map(|fr| Frame {
                serial: fr.serial,
                method: fr.method,
                code: vm
                    .heap
                    .method(fr.method)
                    .map_or_else(|| Rc::from([]), |m| m.bytecode().clone()),
                literals: vm.heap.method(fr.method).map_or(nil, |m| m.literals()),
                mixin: vm.heap.method(fr.method).map_or(nil, |m| m.mixin()),
                receiver: fr.receiver,
                bp: fr.bp as usize,
                arg_count: fr.arg_count as usize,
                lp: fr.lp as usize,
                ip: fr.ip as usize,
                activation: fr.activation,
                closure: fr.closure,
            })
            .collect();
        vm.actors = state
            .actors
            .into_iter()
            .map(|a| Actor {
                message_loop: a.message_loop,
                mailbox: a.mailbox.into_iter().collect(),
                wakeup: a.wakeup,
                poked: a.poked,
            })
            .collect();
        vm.current = state.current as usize;
        vm.expats = state.expats;
        // The VM's selector names are static strings; a restored VM's are
        // the checkpoint's, kept for its life.
        vm.symbols = state
            .symbols
            .into_iter()
            .map(|(k, v)| (&*alloc::boxed::Box::leak(k.into_boxed_str()), v))
            .collect();
        vm.next_serial = state.next_serial;
        vm.gc_counter = state.gc_counter;
        vm.gc_budget = state.gc_budget;
        vm.collections = state.collections;
        vm.finished_turn = state.finished_turn;
        vm.has_loop = state.has_loop;
        vm.program_version = state.program_version;
        vm.checkpoint_every = state.checkpoint_every;
        vm.last_checkpoint = state.last_checkpoint;
        vm
    }

    /// Continues a restored VM: the turn under way when the checkpoint was
    /// taken, or, for a checkpoint taken while idle (at a resume), that
    /// resume. Answers as [`Vm::run`] does.
    pub fn continue_run(&mut self, host: &mut dyn Host) -> Result<Completion, VmError> {
        if self.incarnation_pending {
            self.incarnation_pending = false;
            self.begin_incarnation(host);
        }
        if self.frames.is_empty()
            && let Some(done) = self.next_turn(host)?
        {
            return Ok(done);
        }
        self.event_loop(host)
    }

    /// Sets the checkpoint policy: a checkpoint at the first safepoint after
    /// every `allocations` allocations; 0 is none (`vm checkpointPolicy:`).
    pub fn set_checkpoint_policy(&mut self, allocations: u64) {
        self.checkpoint_every = allocations;
        self.last_checkpoint = self.heap.allocation_counter();
        self.safepoint_at = 0;
    }

    /// The allocation count at which [`Vm::gc_due`] or
    /// [`Vm::checkpoint_due`] can next be true, if nothing else changes.
    fn next_safepoint(&self) -> u64 {
        if self.checkpoint_pending {
            return 0;
        }
        let gc = self
            .gc_counter
            .saturating_add(self.gc_stress.unwrap_or(self.gc_budget));
        let checkpoint = if self.checkpoint_every == 0 {
            u64::MAX
        } else {
            self.last_checkpoint.saturating_add(self.checkpoint_every)
        };
        gc.min(checkpoint)
    }

    fn checkpoint_due(&self) -> bool {
        self.checkpoint_pending
            || (self.checkpoint_every != 0
                && self.heap.allocation_counter() - self.last_checkpoint >= self.checkpoint_every)
    }

    /// Takes a checkpoint and hands it to the host (M7 design, section 3.3),
    /// at a safepoint with no host operation under way. The counters are
    /// updated whether or not the host wants it, so that recording does not
    /// change the run.
    ///
    /// A checkpoint that `saveImage:` asked for is also handed to the host
    /// to save, and the restore handler is then told, in a turn of its own:
    /// the image is taken first, so a restore of it is not told.
    fn take_checkpoint(&mut self, host: &mut dyn Host) {
        // The running heap becomes what a restore of this checkpoint has: the
        // checkpoint's collection, everything old after it. Whether or not
        // anyone records, so that recording does not change the run.
        self.collect(Collection::KeepingWeak);
        self.checkpoint_pending = false;
        self.last_checkpoint = self.heap.allocation_counter();
        self.safepoint_at = 0;
        let save = self.save_pending.take();
        let wanted = host.wants_checkpoints();
        if let Some(write) = self.snapshot_writer
            && (wanted || save.is_some())
        {
            let (heap, roles, state) = self.checkpoint_image();
            let bytes = write(&heap, &roles, Some(&state));
            match save {
                Some(name) => {
                    if wanted {
                        host.checkpoint(bytes.clone());
                    }
                    host.save_image(&name, bytes);
                    self.notify_restore_handler("imageSaved");
                }
                None => host.checkpoint(bytes),
            }
        }
    }

    /// Queues a turn that sends `selector` to the restore handler (role 40),
    /// if the program set one.
    fn notify_restore_handler(&mut self, selector: &'static str) {
        let nil = self.nil();
        let handler = self.roles.get(Role::RestoreHandler);
        if handler != nil && !self.actors.is_empty() {
            let selector = self.symbol(selector);
            let array_class = self.roles.get(Role::ArrayClass);
            let space = Space::Actor(0);
            // A packet: receiver, selector, resolver, then the arguments.
            let packet = self.heap.allocate(
                array_class,
                space,
                Body::Array(alloc::vec![handler, selector, nil]),
            );
            self.actors[0].mailbox.push_back(packet);
        }
    }

    /// After a restore into a new host (M7 design, section 3.4; contract
    /// design, section 6): every Alien but the host's undefined and global
    /// objects is dead, and lazy slots holding a dead Alien are reset, so
    /// that their next read looks the host object up again. Then the
    /// restore handler, if the program set one, is sent `restored` in a turn
    /// of its own, after the turn under way.
    pub fn restored_into_new_host(&mut self) {
        self.incarnation_pending = true;
        let alien_class = self.roles.get(Role::AlienClass);
        let nil = self.nil();
        let dead = Value::small_integer(-1).expect("small");
        let mut dead_aliens = alloc::collections::BTreeSet::new();
        for v in self.heap.objects() {
            if self.heap.class(v) == Some(alien_class)
                && let Some(slots) = self.heap.regular_slots_mut(v)
                && slots.len() == 1
                && slots[0]
                    .as_small_integer()
                    .is_some_and(|r| r > crate::host::HOST_GLOBAL as i64)
            {
                slots[0] = dead;
                dead_aliens.insert(v.as_reference().expect("an object"));
            }
        }
        // Each class's lazy cache slots (psoup names them x`cache`slot).
        let mut lazy: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for v in self.heap.objects() {
            let class = match self.heap.format(v) {
                Some(Format::Regular) => self.heap.class(v).expect("an object"),
                _ => continue,
            };
            let key = class.as_reference().unwrap_or(u64::MAX);
            let slots = match lazy.get(&key) {
                Some(s) => s.clone(),
                None => {
                    let s = self.lazy_slot_indices(class);
                    lazy.insert(key, s.clone());
                    s
                }
            };
            if slots.is_empty() {
                continue;
            }
            if let Some(values) = self.heap.regular_slots_mut(v) {
                for &j in &slots {
                    if j < values.len()
                        && values[j]
                            .as_reference()
                            .is_some_and(|r| dead_aliens.contains(&r))
                    {
                        values[j] = nil;
                    }
                }
            }
        }
        self.notify_restore_handler("restored");
    }

    /// Begins an incarnation (contract design, section 6): every start, a
    /// cold one or a restore into a new host, mints a fresh id from host
    /// entropy (an input, so a replay reproduces it), and the id the image
    /// was saved with becomes the lineage. A replay continuing from a
    /// checkpoint is the same incarnation, and keeps both.
    fn begin_incarnation(&mut self, host: &mut dyn Host) {
        let saver = self.roles.get(Role::Incarnation);
        self.roles.set(Role::Lineage, saver);
        let mut id = [0u8; 16];
        host.random_bytes(&mut id);
        let class = self.roles.get(Role::ByteArrayClass);
        let v = self
            .heap
            .allocate(class, Space::Actor(0), Body::Bytes(id.to_vec()));
        self.roles.set(Role::Incarnation, v);
    }

    /// The indices of a class's lazy cache slots, in its instances.
    fn lazy_slot_indices(&self, class: Value) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut c = class;
        while let Some(b) = self.heap.behavior(c) {
            chain.push(b.mixin());
            c = b.superclass();
        }
        let mut indices = Vec::new();
        let mut index = 0;
        for &mixin in chain.iter().rev() {
            let Some(m) = self.heap.mixin(mixin) else {
                return Vec::new();
            };
            let names = self.heap.array_elements(m.slot_names()).unwrap_or(&[]);
            for &declaration in names {
                // A slot's name, or since M6 a triple {name. isMutable.
                // access}.
                let name = match self.heap.array_elements(declaration) {
                    Some(triple) if !triple.is_empty() => triple[0],
                    _ => declaration,
                };
                if self
                    .heap
                    .string_bytes(name)
                    .is_some_and(|b| b.ends_with(b"`cache`slot"))
                {
                    indices.push(index);
                }
                index += 1;
            }
        }
        indices
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

    /// Runs the program with an Array of the argument strings.
    ///
    /// An image with a message loop (the MessageLoop role is not nil) runs
    /// as psoup's do: the VM delivers the arguments as the startup message,
    /// `messageLoop dispatchMessage: args port: nil`, and every turn ends
    /// with `finish:`, naming the next timer. The VM then waits for that
    /// time and dispatches again (`dispatchMessage: nil port: nil`); with no
    /// timer (and, until ports, M4, nothing else to wait for) the run is over.
    /// An image without one (first light) sends the entry `main:args:` with
    /// the VM mirror, and the run is over when that returns.
    pub fn run(&mut self, host: &mut dyn Host, args: &[&str]) -> Result<Completion, VmError> {
        self.begin_incarnation(host);
        let strings: Vec<Value> = args.iter().map(|a| self.new_string(a)).collect();
        let array_class = self.roles.get(Role::ArrayClass);
        let args_array = self.heap.allocate(
            array_class,
            Space::Actor(self.current as u32),
            Body::Array(strings),
        );
        self.stack.clear();
        self.frames.clear();
        let message_loop = self.roles.get(Role::MessageLoop);
        let has_loop = message_loop != self.nil();
        self.actors = alloc::vec![Actor {
            message_loop,
            mailbox: VecDeque::new(),
            wakeup: 0,
            poked: false,
        }];
        self.current = 0;
        self.has_loop = has_loop;
        if has_loop {
            if let Some(done) = self.dispatch(args_array, host)? {
                return Ok(done);
            }
        } else {
            let entry = self.roles.get(Role::Entry);
            let vm_mirror = self.roles.get(Role::VmMirror);
            self.stack.extend([entry, vm_mirror, args_array]);
            let selector = self.symbol("main:args:");
            if let Some(done) = self.ordinary_send(selector, 2, host)? {
                return Ok(done);
            }
        }
        self.event_loop(host)
    }

    /// Continues a run that answered [`Completion::Waiting`], once the time
    /// it named has come (a host that does not block calls it from a timer).
    pub fn resume(&mut self, host: &mut dyn Host) -> Result<Completion, VmError> {
        // Idle: a checkpoint due since an idle callback is taken here.
        if self.checkpoint_due() {
            self.take_checkpoint(host);
        }
        if let Some(done) = self.next_turn(host)? {
            return Ok(done);
        }
        self.event_loop(host)
    }

    /// Runs steps and turns until the run is over, or, for a host that does
    /// not block, until nothing can run before the next timer.
    fn event_loop(&mut self, host: &mut dyn Host) -> Result<Completion, VmError> {
        let has_loop = self.has_loop;
        let mut steps: u64 = 0;
        loop {
            if let Some(limit) = self.step_limit {
                if steps >= limit {
                    return Err(VmError::StepLimitReached);
                }
                steps += 1;
            }
            if self.frames.is_empty() {
                match self.finished_turn.take() {
                    // The event loop: the next turn of some actor.
                    Some(wakeup) => {
                        self.actors[self.current].wakeup = wakeup;
                        if let Some(done) = self.next_turn(host)? {
                            return Ok(done);
                        }
                        continue;
                    }
                    None if has_loop => {
                        return Err(VmError::Malformed("a turn returned without finish:"));
                    }
                    None => {
                        let result = self
                            .stack
                            .pop()
                            .ok_or_else(|| VmError::Malformed("empty stack"))?;
                        return Ok(Completion::Returned(result));
                    }
                }
            }
            // A safepoint: every value the VM holds is in the roots, and no
            // host operation is under way.
            if self.heap.allocation_counter() >= self.safepoint_at {
                if self.gc_due() {
                    self.scheduled_collection();
                }
                if self.checkpoint_due() {
                    self.take_checkpoint(host);
                }
                self.safepoint_at = self.next_safepoint();
            }
            self.interrupt_countdown -= 1;
            if self.interrupt_countdown == 0 {
                self.interrupt_countdown = INTERRUPT_INTERVAL;
                if host.interrupted() {
                    return Err(VmError::Interrupted);
                }
            }
            // The top frame at an entry of a compiled method: the host runs
            // it. (Only at the top level: a callback's loop runs
            // interpreted.)
            if self.jit_threshold > 0
                && self.callback_depth == 0
                && let Some((code, block)) = self.compiled_entry()
            {
                self.jit_base = self.frames.len();
                self.fill_jit_regs();
                return Ok(Completion::Compiled(code, block));
            }
            let next = if self.step_limit.is_none() {
                self.run_simple(host)?
            } else {
                self.step(host)?
            };
            if let Step::Done(done) = next {
                return Ok(done);
            }
        }
    }

    // ----- Heap helpers -----

    fn new_string(&mut self, s: &str) -> Value {
        let class = self.roles.get(Role::StringClass);
        self.heap.allocate(
            class,
            Space::Actor(self.current as u32),
            Body::String(s.as_bytes().to_vec()),
        )
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
        match self.heap.class(v) {
            Some(c) => c,
            None => self.roles.get(Role::SmallIntegerClass),
        }
    }

    fn superclass(&self, class: Value) -> Result<Option<Value>, VmError> {
        let b = self
            .heap
            .behavior(class)
            .ok_or_else(|| VmError::Malformed("class expected"))?;
        Ok(if b.superclass() == self.nil() {
            None
        } else {
            Some(b.superclass())
        })
    }

    /// The method `selector` defined by `class`'s own mixin, if any.
    fn method_at(&self, class: Value, selector: &[u8]) -> Result<Option<Value>, VmError> {
        let b = self
            .heap
            .behavior(class)
            .ok_or_else(|| VmError::Malformed("class expected"))?;
        let mixin = self
            .heap
            .mixin(b.mixin())
            .ok_or_else(|| VmError::Malformed("mixin expected"))?;
        let methods = self
            .heap
            .array_elements(mixin.methods())
            .ok_or_else(|| VmError::Malformed("method array expected"))?;
        for &m in methods {
            let method = self
                .heap
                .method(m)
                .ok_or_else(|| VmError::Malformed("method expected"))?;
            if self.heap.string_bytes(method.selector()) == Some(selector) {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    fn access(&self, method: Value) -> Access {
        self.heap.method(method).expect("method").header().access
    }

    fn find_application_of(&self, mixin: Value, mut class: Value) -> Result<Value, VmError> {
        loop {
            let b = self
                .heap
                .behavior(class)
                .ok_or_else(|| VmError::MixinApplicationNotFound)?;
            if b.mixin() == mixin {
                return Ok(class);
            }
            class = self
                .superclass(class)?
                .ok_or_else(|| VmError::MixinApplicationNotFound)?;
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
                .ok_or_else(|| VmError::Malformed("class expected"))?;
            let mixin = self
                .heap
                .mixin(b.mixin())
                .ok_or_else(|| VmError::Malformed("mixin expected"))?;
            total += mixin.slot_count() as usize;
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
            .ok_or_else(|| VmError::Malformed("method expected"))?
            .mixin();
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
            .ok_or_else(|| VmError::Malformed("selector must be a String"))
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
        let nil = self.nil();
        self.send(Rule::Ordinary, selector, n, receiver, nil, host)
    }

    /// A send through the lookup cache. For an ordinary send `receiver` is
    /// the one on the stack and `mixin` is unused; for the others they are
    /// the sending frame's receiver and its method's mixin.
    fn send(
        &mut self,
        rule: Rule,
        selector: Value,
        n: usize,
        receiver: Value,
        mixin: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let class = self.class_of(receiver);
        let i = cache_index(class, selector, mixin, rule);
        let target = match self.lookup_cache[i] {
            Some(e)
                if e.class == class
                    && e.selector == selector
                    && e.mixin == mixin
                    && e.rule == rule =>
            {
                e.target
            }
            _ => match self.resolve(rule, selector, receiver, class, mixin)? {
                Some(target) => {
                    self.lookup_cache[i] = Some(CacheEntry {
                        class,
                        selector,
                        mixin,
                        rule,
                        target,
                    });
                    target
                }
                // Not found: the full lookup, which sends #doesNotUnderstand:.
                None => return self.uncached_send(rule, selector, n, receiver, mixin, host),
            },
        };
        match target.receiver {
            TargetReceiver::Present => {}
            TargetReceiver::Frame => self.insert_receiver(receiver, n),
            TargetReceiver::Fixed(r) => self.insert_receiver(r, n),
        }
        self.activate_with(target.method, target.header, n, host, target.slot)
    }

    /// What a send runs, as [`Vm::uncached_send`] finds it, or `None` when
    /// that would be a #doesNotUnderstand:.
    fn resolve(
        &self,
        rule: Rule,
        selector: Value,
        receiver: Value,
        class: Value,
        mixin: Value,
    ) -> Result<Option<Target>, VmError> {
        let sel = self
            .heap
            .string_bytes(selector)
            .ok_or_else(|| VmError::Malformed("selector must be a String"))?;
        let found = match rule {
            Rule::Ordinary => self
                .lookup_ordinary(class, sel)?
                .map(|m| (m, TargetReceiver::Present, receiver)),
            Rule::SelfSend => self
                .lookup_lexical(class, mixin, sel)?
                .map(|m| (m, TargetReceiver::Frame, receiver)),
            Rule::Super => {
                let application = self.find_application_of(mixin, class)?;
                match self.superclass(application)? {
                    Some(start) => self
                        .lookup_protected(start, sel)?
                        .map(|m| (m, TargetReceiver::Frame, receiver)),
                    None => None,
                }
            }
            Rule::ImplicitReceiver => {
                let mut candidate_receiver = receiver;
                let mut candidate_mixin = mixin;
                let mut depth = 0usize;
                let mut found = None;
                loop {
                    let candidate_class = self.class_of(candidate_receiver);
                    let application = self.find_application_of(candidate_mixin, candidate_class)?;
                    if self.method_at(application, sel)?.is_some() {
                        let how = if depth == 0 {
                            TargetReceiver::Frame
                        } else {
                            TargetReceiver::Fixed(candidate_receiver)
                        };
                        found = Some(
                            self.lookup_lexical(candidate_class, candidate_mixin, sel)?
                                .map(|m| (m, how, candidate_receiver)),
                        );
                        break;
                    }
                    let enclosing = self
                        .heap
                        .mixin(candidate_mixin)
                        .ok_or_else(|| VmError::Malformed("mixin expected"))?
                        .enclosing_mixin();
                    if enclosing == self.nil() {
                        break;
                    }
                    candidate_mixin = enclosing;
                    candidate_receiver = self
                        .heap
                        .behavior(application)
                        .expect("class")
                        .enclosing_object();
                    depth += 1;
                }
                match found {
                    Some(f) => f,
                    None => self
                        .lookup_protected(class, sel)?
                        .map(|m| (m, TargetReceiver::Frame, receiver)),
                }
            }
            Rule::Outer(depth) => {
                let (r, m) = self.enclosing_at(receiver, mixin, depth)?;
                let how = if depth == 0 {
                    TargetReceiver::Frame
                } else {
                    TargetReceiver::Fixed(r)
                };
                self.lookup_lexical(self.class_of(r), m, sel)?
                    .map(|method| (method, how, r))
            }
        };
        let Some((method, how, target_receiver)) = found else {
            return Ok(None);
        };
        let header = self.method_header(method)?;
        // An accessor's slot index, if it has one; if not, activation
        // reports the error.
        let slot = match header.kind {
            MethodKind::Getter | MethodKind::Setter => {
                self.slot_index(method, target_receiver, header.slot).ok()
            }
            _ => None,
        };
        Ok(Some(Target {
            method,
            header,
            receiver: how,
            slot,
        }))
    }

    fn method_header(&self, method: Value) -> Result<MethodHeader, VmError> {
        Ok(self
            .heap
            .method(method)
            .ok_or_else(|| VmError::Malformed("method expected"))?
            .header())
    }

    /// The method an ordinary send of `sel` to an instance of `class` runs:
    /// the first public one up the chain. A protected one, or none, is a
    /// #doesNotUnderstand:; a private one is passed over.
    fn lookup_ordinary(&self, class: Value, sel: &[u8]) -> Result<Option<Value>, VmError> {
        let mut c = Some(class);
        while let Some(k) = c {
            if let Some(m) = self.method_at(k, sel)? {
                match self.access(m) {
                    Access::Public => return Ok(Some(m)),
                    Access::Protected => return Ok(None),
                    Access::Private => {}
                }
            }
            c = self.superclass(k)?;
        }
        Ok(None)
    }

    /// The first method `sel` that is not private, from `start` up.
    fn lookup_protected(&self, start: Value, sel: &[u8]) -> Result<Option<Value>, VmError> {
        let mut c = Some(start);
        while let Some(k) = c {
            if let Some(m) = self.method_at(k, sel)?
                && self.access(m) != Access::Private
            {
                return Ok(Some(m));
            }
            c = self.superclass(k)?;
        }
        Ok(None)
    }

    /// A lexical send's method: the private `sel` of `mixin`'s application
    /// in `class`, or else the protected lookup from `class`.
    fn lookup_lexical(
        &self,
        class: Value,
        mixin: Value,
        sel: &[u8],
    ) -> Result<Option<Value>, VmError> {
        let application = self.find_application_of(mixin, class)?;
        if let Some(m) = self.method_at(application, sel)?
            && self.access(m) == Access::Private
        {
            return Ok(Some(m));
        }
        self.lookup_protected(class, sel)
    }

    /// The object and mixin `depth` levels out from `receiver` in `mixin`.
    fn enclosing_at(
        &self,
        mut receiver: Value,
        mut mixin: Value,
        depth: usize,
    ) -> Result<(Value, Value), VmError> {
        for _ in 0..depth {
            let application = self.find_application_of(mixin, self.class_of(receiver))?;
            receiver = self
                .heap
                .behavior(application)
                .expect("class")
                .enclosing_object();
            mixin = self
                .heap
                .mixin(mixin)
                .ok_or_else(|| VmError::Malformed("mixin expected"))?
                .enclosing_mixin();
        }
        Ok((receiver, mixin))
    }

    /// A send by the full lookup: every rule, #doesNotUnderstand: included.
    fn uncached_send(
        &mut self,
        rule: Rule,
        selector: Value,
        n: usize,
        receiver: Value,
        mixin: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        match rule {
            Rule::Ordinary => self.ordinary_send_uncached(selector, n, receiver, host),
            Rule::SelfSend => self.lexical_send(selector, n, receiver, mixin, host),
            Rule::Super => {
                let class = self.class_of(receiver);
                let application = self.find_application_of(mixin, class)?;
                match self.superclass(application)? {
                    Some(start) => self.protected_send(selector, n, receiver, start, host),
                    None => {
                        self.does_not_understand(selector, n, receiver, application, false, host)
                    }
                }
            }
            Rule::ImplicitReceiver => {
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
                        .ok_or_else(|| VmError::Malformed("mixin expected"))?
                        .enclosing_mixin();
                    if enclosing == self.nil() {
                        break;
                    }
                    candidate_mixin = enclosing;
                    candidate_receiver = self
                        .heap
                        .behavior(application)
                        .expect("class")
                        .enclosing_object();
                }
                let class = self.class_of(receiver);
                self.protected_send(selector, n, receiver, class, host)
            }
            Rule::Outer(depth) => {
                let (r, m) = self.enclosing_at(receiver, mixin, depth)?;
                self.lexical_send(selector, n, r, m, host)
            }
        }
    }

    fn ordinary_send_uncached(
        &mut self,
        selector: Value,
        n: usize,
        receiver: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
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
        // By hand: `Vec::insert` is a call to memmove, and `n` is small.
        self.stack.push(receiver);
        let top = self.stack.len() - 1;
        for i in (top - n..top).rev() {
            self.stack[i + 1] = self.stack[i];
        }
        self.stack[top - n] = receiver;
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
        let method = method.ok_or_else(|| VmError::RecursiveDoesNotUnderstand)?;
        let args = self.stack.split_off(self.stack.len() - n);
        let array_class = self.roles.get(Role::ArrayClass);
        let arguments = self.heap.allocate(
            array_class,
            Space::Actor(self.current as u32),
            Body::Array(args),
        );
        let message_class = self.roles.get(Role::MessageClass);
        let message = self.heap.allocate(
            message_class,
            Space::Actor(self.current as u32),
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
        self.activate_at(method, n, host, None)
    }

    /// [`Vm::activate`], given an accessor's slot index if it is known.
    fn activate_at(
        &mut self,
        method: Value,
        n: usize,
        host: &mut dyn Host,
        slot: Option<usize>,
    ) -> Result<Option<Completion>, VmError> {
        let header = self.method_header(method)?;
        self.activate_with(method, header, n, host, slot)
    }

    /// [`Vm::activate_at`], given the method's header.
    fn activate_with(
        &mut self,
        method: Value,
        header: MethodHeader,
        n: usize,
        host: &mut dyn Host,
        slot: Option<usize>,
    ) -> Result<Option<Completion>, VmError> {
        let base = self.stack.len() - 1 - n;
        match header.kind {
            MethodKind::Native => {
                let native = Native::from_id(header.native)
                    .filter(|nat| nat.arity() == n)
                    .ok_or_else(|| VmError::BadNative(header.native))?;
                self.run_native(native, base, n, host)
            }
            MethodKind::Getter => {
                let receiver = self.stack[base];
                let index = match slot {
                    Some(i) => i,
                    None => self.slot_index(method, receiver, header.slot)?,
                };
                // A contract object (class, metaclass, mixin, method) answers
                // its contract field at that position, as `vm slotOf:at:` does.
                let value = match self.heap.regular_slots(receiver) {
                    Some(slots) => slots.get(index).copied(),
                    None => natives::readable_fields(&self.heap, receiver)
                        .and_then(|f| f.get(index).copied()),
                }
                .ok_or_else(|| self.bad_slot_access(method, receiver))?;
                self.stack.truncate(base);
                self.stack.push(value);
                Ok(None)
            }
            MethodKind::Setter => {
                let receiver = self.stack[base];
                let value = self.stack[base + 1];
                let index = match slot {
                    Some(i) => i,
                    None => self.slot_index(method, receiver, header.slot)?,
                };
                // A frozen object is never written; a value only by its init
                // setters, and only with values (design doc section 5).
                let flags = self.heap.flags(receiver);
                let refusal = if flags & FROZEN != 0 {
                    Some(Reason::WrongKind(0))
                } else if flags & VALUE != 0 && !self.is_value(value) {
                    Some(Reason::WrongKind(1))
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    let failure = self.new_failure(
                        Native::SlotStore,
                        receiver,
                        alloc::vec![value],
                        reason,
                        host,
                    );
                    self.stack.truncate(base);
                    self.stack.push(failure);
                    let signal = self.symbol("signal");
                    return self.ordinary_send(signal, 0, host);
                }
                if !self.store_slot(receiver, index, value) {
                    return Err(self.bad_slot_access(method, receiver));
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
                let (code, literals, mixin) = {
                    let m = self.heap.method(method).expect("method");
                    (m.bytecode().clone(), m.literals(), m.mixin())
                };
                self.frames.push(Frame {
                    serial,
                    method,
                    code,
                    literals,
                    mixin,
                    receiver,
                    bp: base,
                    arg_count: n,
                    lp,
                    ip: 0,
                    activation: None,
                    closure: None,
                });
                if self.jit_threshold > 0 {
                    self.tier_up(method, host);
                }
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
        let outcome = {
            let mut ctx = NativeContext {
                heap: &mut self.heap,
                roles: &self.roles,
                host,
                space: Space::Actor(self.current as u32),
            };
            natives::invoke(
                native,
                receiver,
                &self.stack[base + 1..base + 1 + n],
                &mut ctx,
            )
        };
        if native == Native::MethodHeaderPut {
            self.flush_lookup_cache();
        }
        // Rule Native-Answer, the common case, without copying the arguments.
        if let Outcome::Answer(v) = outcome {
            self.stack.truncate(base);
            self.stack.push(v);
            return Ok(None);
        }
        // A closure's `value`: the closure and its arguments are already in
        // place on the stack.
        if let Outcome::ActivateReceiver = outcome {
            self.activate_closure(receiver, n)?;
            self.tier_up_closure(host);
            return Ok(None);
        }
        let args: Vec<Value> = self.stack[base + 1..base + 1 + n].to_vec();
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
            Outcome::Fail(reason) => self.native_failure(native, receiver, args, reason, host),
            Outcome::Exit(code) => Ok(Some(Completion::Exited(code))),
            Outcome::Frame(_) => unreachable!("frame natives are resolved above"),
            Outcome::ActivateReceiver => unreachable!("closure activation is handled above"),
            Outcome::Spawn(mixin) => {
                // The semantics' create-actor: a fresh actor whose heap has
                // the new class, applying the mixin to Object with a nil
                // enclosing object; the answer is a far reference to it.
                let template = self.roles.get(Role::PlatformTemplate);
                let object = self.roles.get(Role::ObjectClass);
                if template == self.nil() || object == self.nil() {
                    let failure =
                        self.new_failure(native, receiver, args, Reason::Unsupported, host);
                    self.stack.push(failure);
                    let signal = self.symbol("signal");
                    return self.ordinary_send(signal, 0, host);
                }
                let id = self.actors.len() as u32;
                let nil = self.nil();
                let applied = {
                    let mut ctx = NativeContext {
                        heap: &mut self.heap,
                        roles: &self.roles,
                        host,
                        space: Space::Actor(id),
                    };
                    natives::apply_mixin(mixin, object, nil, &mut ctx)
                };
                let class = match applied {
                    Outcome::Answer(class) => class,
                    Outcome::Fail(reason) => {
                        let failure = self.new_failure(native, receiver, args, reason, host);
                        self.stack.push(failure);
                        let signal = self.symbol("signal");
                        return self.ordinary_send(signal, 0, host);
                    }
                    _ => unreachable!("applyMixin answers or fails"),
                };
                let message_loop = self.heap.clone_graph(template, Space::Actor(id));
                self.actors.push(Actor {
                    message_loop,
                    mailbox: VecDeque::new(),
                    wakeup: 0,
                    poked: false,
                });
                let far_class = self.roles.get(Role::FarReferenceClass);
                let r = self.heap.allocate(
                    far_class,
                    Space::Actor(self.current as u32),
                    Body::FarRef {
                        actor: id,
                        target: class,
                    },
                );
                self.heap.add_flags(r, VALUE);
                self.stack.push(r);
                Ok(None)
            }
            Outcome::NewActor => {
                // A new actor runs a clone of the platform template.
                let template = self.roles.get(Role::PlatformTemplate);
                if template == self.nil() {
                    let failure =
                        self.new_failure(native, receiver, args, Reason::Unsupported, host);
                    self.stack.push(failure);
                    let signal = self.symbol("signal");
                    return self.ordinary_send(signal, 0, host);
                }
                let id = self.actors.len();
                let message_loop = self.heap.clone_graph(template, Space::Actor(id as u32));
                self.actors.push(Actor {
                    message_loop,
                    mailbox: VecDeque::new(),
                    wakeup: 0,
                    poked: false,
                });
                let class = self.roles.get(Role::FarReferenceClass);
                let nil = self.nil();
                let handle = self.heap.allocate(
                    class,
                    Space::Actor(self.current as u32),
                    Body::FarRef {
                        actor: id as u32,
                        target: nil,
                    },
                );
                self.heap.add_flags(handle, VALUE);
                self.stack.push(handle);
                Ok(None)
            }
            Outcome::Post {
                packet,
                far_reference,
            } => {
                let Some((actor, _)) = self.heap.far_ref(far_reference) else {
                    unreachable!("checked by the native")
                };
                let to = actor as usize;
                if to >= self.actors.len() {
                    return Err(VmError::Malformed("a far reference to no actor"));
                }
                let elements = self.heap.array_elements(packet).expect("checked").to_vec();
                let from = self.current;
                let translated: Vec<Value> = elements
                    .into_iter()
                    .map(|v| self.remote_representation(v, from, to))
                    .collect();
                let array_class = self.roles.get(Role::ArrayClass);
                let copy = self.heap.allocate(
                    array_class,
                    Space::Actor(to as u32),
                    Body::Array(translated),
                );
                self.actors[to].mailbox.push_back(copy);
                let nil = self.nil();
                self.stack.push(nil);
                Ok(None)
            }
            Outcome::CurrentMessageLoop => {
                let message_loop = self.actors[self.current].message_loop;
                self.stack.push(message_loop);
                Ok(None)
            }
            Outcome::HostCall {
                op,
                receiver: r,
                args: arguments,
                answer,
            } => {
                let mut host_args = Vec::with_capacity(arguments.len());
                for (v, index) in arguments {
                    match self.marshal_out(v, index) {
                        Ok(h) => host_args.push(h),
                        Err(reason) => {
                            return self.native_failure(native, receiver, args, reason, host);
                        }
                    }
                }
                let result = host.host_operation(op, r, &host_args, self);
                match result {
                    Ok(v) => {
                        let v = match answer {
                            Some(a) => a,
                            None => self.marshal_in(v, Space::Actor(self.current as u32)),
                        };
                        self.stack.push(v);
                        Ok(None)
                    }
                    Err(e) => {
                        let reason = match e {
                            HostError::Unsupported => Reason::Unsupported,
                            HostError::Dead => Reason::Dead,
                            HostError::Raised(message) => Reason::Host(self.new_string(&message)),
                        };
                        self.native_failure(native, receiver, args, reason, host)
                    }
                }
            }
            Outcome::Settle {
                record,
                state,
                value,
            } => {
                let settled = self.settle(record, state, value);
                let answer = self
                    .roles
                    .get(if settled { Role::True } else { Role::False });
                self.stack.push(answer);
                Ok(None)
            }
            Outcome::PromiseSend { record, message } => {
                let elements = self.heap.array_elements(message).expect("checked").to_vec();
                let current = self.current;
                let mut entry = Vec::with_capacity(elements.len() + 1);
                entry.push(Value::small_integer(current as i64).expect("small"));
                for v in elements {
                    let n = self.neutral(v);
                    entry.push(n);
                }
                let array_class = self.roles.get(Role::ArrayClass);
                let entry = self.heap.allocate(
                    array_class,
                    Space::Actor(current as u32),
                    Body::Array(entry),
                );
                let last = natives::last_promise(&self.heap, record);
                let Some(p) = self.heap.promise(last) else {
                    unreachable!("a chain of promise records ends in one")
                };
                let (state, value) = (p.state(), p.value());
                if state == PromiseRecord::PENDING {
                    let array_class = self.roles.get(Role::ArrayClass);
                    self.heap.push_promise_queue(last, &[entry], array_class);
                } else {
                    self.deliver(entry, state, value);
                }
                let nil = self.nil();
                self.stack.push(nil);
                Ok(None)
            }
            Outcome::AddWaiter { record, waiter } => {
                let last = natives::last_promise(&self.heap, record);
                let current = self.current as u32;
                let class = self.roles.get(Role::FarReferenceClass);
                let far = self.heap.allocate(
                    class,
                    Space::Actor(current),
                    Body::FarRef {
                        actor: current,
                        target: waiter,
                    },
                );
                self.heap.add_flags(far, VALUE);
                let Some(p) = self.heap.promise(last) else {
                    unreachable!("a chain of promise records ends in one")
                };
                let (state, value) = (p.state(), p.value());
                if state == PromiseRecord::PENDING {
                    let array_class = self.roles.get(Role::ArrayClass);
                    self.heap.push_promise_waiter(last, far, array_class);
                } else {
                    self.notify(far, state, value);
                }
                let nil = self.nil();
                self.stack.push(nil);
                Ok(None)
            }
            Outcome::FinishTurn(wakeup) => {
                // The turn's frames leave the stack; the event loop takes
                // over. If something holds an activation of the turn (a
                // debugger's broken thread, from the handler of last resort),
                // the frames become a stack image of detached activations,
                // as psoup's heap activations outlive their turn (decided
                // 2026-11-18: the actor carries on; M6 design section 9).
                // Otherwise they die.
                if self.frames.iter().any(|f| f.activation.is_some()) {
                    self.detach_all_frames();
                } else {
                    let frames = core::mem::take(&mut self.frames);
                    for f in &frames {
                        self.retire(f);
                    }
                }
                self.stack.clear();
                self.finished_turn = Some(wakeup);
                Ok(None)
            }
            Outcome::ContinueAt { activation, value } => {
                match self.continue_at(activation, value) {
                    Ok(()) => Ok(None),
                    Err(reason) => self.native_failure(native, receiver, args, reason, host),
                }
            }
            Outcome::ResumeActivation(top) => match self.resume_activation(top) {
                Ok(()) => Ok(None),
                Err(reason) => self.native_failure(native, receiver, args, reason, host),
            },
            Outcome::RunNative {
                method,
                receiver: r,
                arguments,
            } => match self.run_native_for_simulator(method, r, arguments, host) {
                Ok(v) => {
                    self.stack.push(v);
                    Ok(None)
                }
                Err(reason) => self.native_failure(native, receiver, args, reason, host),
            },
            Outcome::SaveImage(name) => {
                self.save_pending = Some(name);
                self.checkpoint_pending = true;
                self.safepoint_at = 0;
                self.stack.push(receiver);
                Ok(None)
            }
            Outcome::CheckpointNow => {
                self.checkpoint_pending = true;
                self.safepoint_at = 0;
                let nil = self.nil();
                self.stack.push(nil);
                Ok(None)
            }
            Outcome::CheckpointPolicy(n) => {
                self.set_checkpoint_policy(n);
                self.stack.push(receiver);
                Ok(None)
            }
            Outcome::ProgramVersion => {
                let v = Value::small_integer(self.program_version as i64).expect("small");
                self.stack.push(v);
                Ok(None)
            }
            Outcome::Commit(change_set) => match self.commit(change_set) {
                Ok(()) => {
                    let v = Value::small_integer(self.program_version as i64).expect("small");
                    self.stack.push(v);
                    Ok(None)
                }
                Err(index) => {
                    self.native_failure(native, receiver, args, Reason::WrongKind(index), host)
                }
            },
            Outcome::ClassQuery { target, by_mixin } => {
                // Only live classes: a collection first, with the target
                // rooted on the stack. Classes only, as psoup's weak sets
                // hold: every metaclass's superclass is Class, which is not
                // a subclass relation.
                self.stack.push(target);
                self.collect_garbage();
                let target = self.stack.pop().expect("pushed");
                let found: Vec<Value> = self
                    .heap
                    .objects()
                    .filter(|&v| {
                        self.heap.format(v) == Some(Format::Class)
                            && self.heap.behavior(v).is_some_and(|b| {
                                if by_mixin {
                                    b.mixin() == target
                                } else {
                                    b.superclass() == target
                                }
                            })
                    })
                    .collect();
                // In allocation order, which the arena's order need not be.
                let mut found = found;
                found.sort_by_key(|&v| self.heap.seq(v));
                let array_class = self.roles.get(Role::ArrayClass);
                let answer = self.heap.allocate(
                    array_class,
                    Space::Actor(self.current as u32),
                    Body::Array(found),
                );
                self.stack.push(answer);
                Ok(None)
            }
            Outcome::AllInstances(class) => {
                // Only live instances: a collection first, at this safepoint,
                // with the class rooted on the stack.
                self.stack.push(class);
                self.collect_garbage();
                let class = self.stack.pop().expect("pushed");
                let instances: Vec<Value> = self
                    .heap
                    .objects()
                    .filter(|&v| self.heap.class(v) == Some(class))
                    .collect();
                // In allocation order, which the arena's order need not be.
                let mut instances = instances;
                instances.sort_by_key(|&v| self.heap.seq(v));
                let array_class = self.roles.get(Role::ArrayClass);
                let answer = self.heap.allocate(
                    array_class,
                    Space::Actor(self.current as u32),
                    Body::Array(instances),
                );
                self.stack.push(answer);
                Ok(None)
            }
            Outcome::Collect => {
                // A safepoint: the receiver is back on the stack first.
                self.stack.push(receiver);
                self.collect_garbage();
                Ok(None)
            }
            Outcome::Snapshot => match self.snapshot_writer {
                Some(write) => {
                    let (heap, roles) = self.snapshot_heap();
                    let bytes = write(&heap, &roles, None);
                    let class = self.roles.get(Role::ByteArrayClass);
                    let v = self.heap.allocate(
                        class,
                        Space::Actor(self.current as u32),
                        Body::Bytes(bytes),
                    );
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
                self.tier_up_closure(host);
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
            .ok_or_else(|| VmError::Malformed("closure expected"))?;
        let defining = self
            .heap
            .activation(c.defining())
            .ok_or_else(|| VmError::Malformed("activation expected"))?;
        let (method, receiver) = (defining.method(), defining.receiver());
        let (code, literals, mixin) = {
            let m = self
                .heap
                .method(method)
                .ok_or_else(|| VmError::Malformed("method expected"))?;
            (m.bytecode().clone(), m.literals(), m.mixin())
        };
        if c.initial_ip() as usize >= code.len() {
            return Err(VmError::Malformed("closure body outside its method"));
        }
        let base = self.stack.len() - 1 - n;
        let lp = self.stack.len();
        self.stack.extend(c.copied().iter().copied());
        let serial = self.next_serial;
        self.next_serial += 1;
        self.frames.push(Frame {
            serial,
            method,
            code,
            literals,
            mixin,
            receiver,
            bp: base,
            arg_count: n,
            lp,
            ip: c.initial_ip() as usize,
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
            detached: None,
        };
        let class = self.roles.get(Role::ActivationClass);
        let a = self.heap.allocate(
            class,
            Space::Actor(self.current as u32),
            Body::Activation(record),
        );
        self.frames[index].activation = Some(a);
        a
    }

    /// The index of the live frame an activation handle names.
    fn frame_index(&self, activation: Value) -> Option<usize> {
        let serial = self.heap.activation(activation)?.frame()?;
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
        let Some((method, record_receiver, closure, detached)) =
            self.heap.activation(receiver).map(|a| {
                (
                    a.method(),
                    a.receiver(),
                    a.closure(),
                    a.detached().is_some(),
                )
            })
        else {
            return Outcome::Fail(Reason::WrongKind(0));
        };
        match native {
            Native::ActivationMethod => return Outcome::Answer(method),
            Native::ActivationReceiver => return Outcome::Answer(record_receiver),
            Native::ActivationClosure => return Outcome::Answer(closure),
            _ => {}
        }
        if detached {
            return self.detached_native(native, receiver, args);
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

    /// An activation native on a detached activation (M6 design 4.4): its
    /// fields are the Simulator's to read and write.
    fn detached_native(&mut self, native: Native, receiver: Value, args: &[Value]) -> Outcome {
        let nil = self.nil();
        let is_activation = |vm: &Vm, v: Value| v == nil || vm.heap.activation(v).is_some();
        let arg_ok = match native {
            Native::ActivationSetSender => is_activation(self, args[0]),
            Native::ActivationBciPut => args[0] == nil || args[0].as_small_integer().is_some(),
            Native::ActivationSizePut => args[0]
                .as_small_integer()
                .is_some_and(|n| (0..1 << 16).contains(&n)),
            _ => true,
        };
        if !arg_ok {
            return Outcome::Fail(Reason::WrongKind(1));
        }
        let d = self
            .heap
            .activation(receiver)
            .and_then(|a| a.detached())
            .expect("checked by the caller");
        // A copy: the arms below write the activation (the Simulator's path).
        let (sender, bci, temps) = (d.sender(), d.bci(), d.temps().to_vec());
        let index = |v: Value, len: usize| -> Result<usize, Reason> {
            match v.as_small_integer() {
                Some(i) if i >= 1 && i as usize <= len => Ok(i as usize - 1),
                Some(_) => Err(Reason::OutOfRange(1)),
                None => Err(Reason::WrongKind(1)),
            }
        };
        match native {
            Native::ActivationSender => Outcome::Answer(sender),
            Native::ActivationBci => Outcome::Answer(bci),
            Native::ActivationSize => {
                Outcome::Answer(Value::small_integer(temps.len() as i64).expect("small"))
            }
            Native::ActivationTempAt => match index(args[0], temps.len()) {
                Ok(i) => Outcome::Answer(temps[i]),
                Err(r) => Outcome::Fail(r),
            },
            Native::ActivationTempAtPut => match index(args[0], temps.len()) {
                Ok(i) => {
                    self.heap.set_detached_temp(receiver, i, args[1]);
                    Outcome::Answer(args[1])
                }
                Err(r) => Outcome::Fail(r),
            },
            Native::ActivationSetSender => {
                self.heap.set_detached_sender(receiver, args[0]);
                Outcome::Answer(receiver)
            }
            Native::ActivationBciPut => {
                self.heap.set_detached_bci(receiver, args[0]);
                Outcome::Answer(receiver)
            }
            Native::ActivationSizePut => {
                let n = args[0].as_small_integer().expect("checked") as usize;
                let array_class = self.roles.get(Role::ArrayClass);
                self.heap
                    .resize_detached_temps(receiver, n, nil, array_class);
                Outcome::Answer(receiver)
            }
            Native::ActivationMethodPut => {
                self.heap.set_activation_field(receiver, 0, args[0]);
                Outcome::Answer(receiver)
            }
            Native::ActivationClosurePut => {
                self.heap.set_activation_field(receiver, 2, args[0]);
                Outcome::Answer(receiver)
            }
            Native::ActivationReceiverPut => {
                self.heap.set_activation_field(receiver, 1, args[0]);
                Outcome::Answer(receiver)
            }
            _ => Outcome::Fail(Reason::Unsupported),
        }
    }

    /// Stores `value` in slot `index` (absolute) of `receiver`: a regular
    /// object's slot, an ephemeron's key, value or finalizer, or an unfrozen
    /// method's or mixin's contract field. Answers whether there was one.
    fn store_slot(&mut self, receiver: Value, index: usize, value: Value) -> bool {
        let program = matches!(
            self.heap.format(receiver),
            Some(Format::Method | Format::Mixin)
        );
        let stored = self.store_slot_unflushed(receiver, index, value);
        if stored && program {
            self.flush_lookup_cache();
        }
        stored
    }

    fn store_slot_unflushed(&mut self, receiver: Value, index: usize, value: Value) -> bool {
        if let Some(slots) = self.heap.regular_slots_mut(receiver) {
            return match slots.get_mut(index) {
                Some(slot) => {
                    *slot = value;
                    true
                }
                None => false,
            };
        }
        match self.heap.format(receiver) {
            // The kernel's Ephemeron declares key, value, finalizer.
            Some(Format::Ephemeron) => self.heap.set_ephemeron_field(receiver, index, value),
            // An unfrozen method or mixin built in Newspeak (M6
            // design 4.1): its setters write its contract fields, in
            // `vm slotOf:at:`'s order. A mixin's slot count follows
            // its slot names.
            Some(Format::Method) => self.heap.set_method_field(receiver, index, value),
            Some(Format::Mixin) => {
                let count = self.heap.array_elements(value).map_or(0, <[Value]>::len) as u32;
                self.heap.set_mixin_field(receiver, index, value, count)
            }
            _ => false,
        }
    }

    /// `vm continueAt: a with: v` (M6 design 4.4): the frames above the live
    /// activation `a` leave the stack without unwinding (the library has
    /// unwound), and `a`'s pending send answers `v`. Those frames are not
    /// dead: each becomes a detached activation with its state, the bottom
    /// one's sender `a`, as psoup's heap activations survive a jump, so that
    /// a debugger can still inspect, step or resume the thread they were.
    /// Fails unless `a` is live.
    fn continue_at(&mut self, activation: Value, value: Value) -> Result<(), Reason> {
        let k = self.frame_index(activation).ok_or(Reason::WrongKind(1))?;
        if k + 1 < self.frames.len() {
            self.detach_frames_above(k);
        }
        self.stack.push(value);
        Ok(())
    }

    /// Takes every frame off the stack, each becoming a detached activation
    /// (see [`Vm::detach_frames_above`]); the bottom one has no sender.
    fn detach_all_frames(&mut self) {
        if self.frames.is_empty() {
            return;
        }
        let nil = self.nil();
        self.detach_frames_from(0, nil);
    }

    /// Takes the frames above frame `k` off the stack, each becoming a
    /// detached activation (its handle made if it had none) with its temps
    /// and operands, its bci and its sender's handle.
    fn detach_frames_above(&mut self, k: usize) {
        let below = self.ensure_activation(k);
        self.detach_frames_from(k + 1, below);
    }

    /// Detaches frames `from` and up, the lowest one's sender `below`.
    fn detach_frames_from(&mut self, from: usize, below: Value) {
        for i in from..self.frames.len() {
            self.ensure_activation(i);
        }
        let popped = self.frames.split_off(from);
        let tops: Vec<usize> = popped
            .iter()
            .skip(1)
            .map(|f| f.bp)
            .chain(core::iter::once(self.stack.len()))
            .collect();
        let mut sender = below;
        for (f, top) in popped.iter().zip(tops) {
            let temps = self.stack[f.bp + 1..top].to_vec();
            let handle = f.activation.expect("made above");
            let bci = Value::small_integer(f.ip as i64 + 1).expect("small");
            let array_class = self.roles.get(Role::ArrayClass);
            self.heap
                .detach_activation(handle, sender, bci, temps, array_class);
            sender = handle;
        }
        self.stack.truncate(popped[0].bp);
    }

    /// `vm resumeActivation: top` (M6 design 4.4): the chain of detached
    /// activations from `top` down to the current frame becomes frames, the
    /// bottom one called from the current frame's pending send, and the run
    /// continues at `top`. Each handle keeps its identity and becomes live.
    /// Checked before anything changes; fails if the chain does not end at
    /// the current frame or an activation in it is not runnable.
    fn resume_activation(&mut self, top: Value) -> Result<(), Reason> {
        let current = self
            .frames
            .last()
            .and_then(|f| f.activation)
            .ok_or(Reason::WrongKind(1))?;
        let mut chain = Vec::new();
        let mut a = top;
        loop {
            let record = self.heap.activation(a).ok_or(Reason::WrongKind(1))?;
            let Some(d) = &record.detached() else {
                break;
            };
            if chain.len() > MAX_FRAMES {
                return Err(Reason::WrongKind(1));
            }
            let method = self
                .heap
                .method(record.method())
                .ok_or(Reason::WrongKind(1))?;
            let bci = d.bci().as_small_integer().ok_or(Reason::WrongKind(1))?;
            if bci < 1 || bci as usize > method.bytecode().len() {
                return Err(Reason::WrongKind(1));
            }
            let arity = match self.heap.closure(record.closure()) {
                Some(c) => c.num_args() as usize,
                None => method.header().arg_count as usize,
            };
            if d.temps().len() < arity {
                return Err(Reason::WrongKind(1));
            }
            chain.push(a);
            a = d.sender();
        }
        if a != current || chain.is_empty() {
            return Err(Reason::WrongKind(1));
        }
        if self.frames.len() + chain.len() > MAX_FRAMES {
            return Err(Reason::Exhausted(natives::Resource::Stack));
        }
        for &handle in chain.iter().rev() {
            let serial = self.next_serial;
            self.next_serial += 1;
            let (_, bci, temps) = self
                .heap
                .attach_activation(handle, serial)
                .expect("checked above");
            let record = self.heap.activation(handle).expect("checked above");
            let (method, receiver, closure) =
                (record.method(), record.receiver(), record.closure());
            let nil = self.nil();
            let closure = (closure != nil).then_some(closure);
            let arity = match closure.and_then(|c| self.heap.closure(c)) {
                Some(c) => c.num_args() as usize,
                None => {
                    self.heap
                        .method(method)
                        .expect("checked")
                        .header()
                        .arg_count as usize
                }
            };
            let bp = self.stack.len();
            self.stack.push(closure.unwrap_or(receiver));
            self.stack.extend(temps.iter().copied());
            let (code, literals, mixin) = {
                let m = self.heap.method(method).expect("checked");
                (m.bytecode().clone(), m.literals(), m.mixin())
            };
            self.frames.push(Frame {
                serial,
                method,
                code,
                literals,
                mixin,
                receiver,
                bp,
                arg_count: arity,
                lp: bp + 1 + arity,
                ip: bci.as_small_integer().expect("checked") as usize - 1,
                activation: Some(handle),
                closure,
            });
        }
        Ok(())
    }

    /// `vm runNative:receiver:arguments:` (M6 design 4.4): runs a native or
    /// accessor method for the Simulator, which has no frame for it, and
    /// answers what it did: {0. result}, {1. the failure to signal}, {2.
    /// receiver. selector. arguments} or {3. closure. arguments}.
    fn run_native_for_simulator(
        &mut self,
        method: Value,
        receiver: Value,
        arguments: Value,
        host: &mut dyn Host,
    ) -> Result<Value, Reason> {
        let header = self
            .heap
            .method(method)
            .ok_or(Reason::WrongKind(1))?
            .header();
        let args = self
            .heap
            .array_elements(arguments)
            .ok_or(Reason::WrongKind(3))?
            .to_vec();
        if args.len() != header.arg_count as usize {
            return Err(Reason::WrongKind(3));
        }
        let small = |n: i64| Value::small_integer(n).expect("small");
        let parts: Vec<Value> = match header.kind {
            MethodKind::Ordinary => return Err(Reason::WrongKind(1)),
            MethodKind::ReturnSelf => alloc::vec![small(0), receiver],
            MethodKind::Getter => {
                let index = self
                    .slot_index(method, receiver, header.slot)
                    .map_err(|_| Reason::WrongKind(2))?;
                let value = match self.heap.regular_slots(receiver) {
                    Some(slots) => slots.get(index).copied(),
                    None => natives::readable_fields(&self.heap, receiver)
                        .and_then(|f| f.get(index).copied()),
                }
                .ok_or(Reason::WrongKind(2))?;
                alloc::vec![small(0), value]
            }
            MethodKind::Setter => {
                let index = self
                    .slot_index(method, receiver, header.slot)
                    .map_err(|_| Reason::WrongKind(2))?;
                let flags = self.heap.flags(receiver);
                if flags & FROZEN != 0 || (flags & VALUE != 0 && !self.is_value(args[0])) {
                    let failure = self.new_failure(
                        Native::SlotStore,
                        receiver,
                        args,
                        Reason::WrongKind(0),
                        host,
                    );
                    alloc::vec![small(1), failure]
                } else {
                    if !self.store_slot(receiver, index, args[0]) {
                        return Err(Reason::WrongKind(2));
                    }
                    alloc::vec![small(0), receiver]
                }
            }
            MethodKind::Native => {
                let native = Native::from_id(header.native).ok_or(Reason::WrongKind(1))?;
                let outcome = {
                    let mut ctx = NativeContext {
                        heap: &mut self.heap,
                        roles: &self.roles,
                        host,
                        space: Space::Actor(self.current as u32),
                    };
                    natives::invoke(native, receiver, &args, &mut ctx)
                };
                if native == Native::MethodHeaderPut {
                    self.flush_lookup_cache();
                }
                let outcome = match outcome {
                    Outcome::Frame(op) => self.frame_native(op, receiver, &args),
                    other => other,
                };
                match outcome {
                    Outcome::Answer(v) => alloc::vec![small(0), v],
                    Outcome::Fail(reason) => {
                        let failure = self.new_failure(native, receiver, args, reason, host);
                        alloc::vec![small(1), failure]
                    }
                    Outcome::TailSend {
                        receiver: r,
                        selector,
                        args: a,
                    } => {
                        let selector = self.symbol(selector);
                        let array_class = self.roles.get(Role::ArrayClass);
                        let a = self.heap.allocate(
                            array_class,
                            Space::Actor(self.current as u32),
                            Body::Array(a),
                        );
                        alloc::vec![small(2), r, selector, a]
                    }
                    Outcome::ActivateClosure { closure, args: a } => {
                        let array_class = self.roles.get(Role::ArrayClass);
                        let a = self.heap.allocate(
                            array_class,
                            Space::Actor(self.current as u32),
                            Body::Array(a),
                        );
                        alloc::vec![small(3), closure, a]
                    }
                    Outcome::ActivateReceiver => {
                        let array_class = self.roles.get(Role::ArrayClass);
                        let a = self.heap.allocate(
                            array_class,
                            Space::Actor(self.current as u32),
                            Body::Array(args.clone()),
                        );
                        alloc::vec![small(3), receiver, a]
                    }
                    // A native that changes the VM's own state (turns,
                    // actors, the heap, the program) is not simulated.
                    _ => {
                        let failure =
                            self.new_failure(native, receiver, args, Reason::Unsupported, host);
                        alloc::vec![small(1), failure]
                    }
                }
            }
        };
        let array_class = self.roles.get(Role::ArrayClass);
        Ok(self.heap.allocate(
            array_class,
            Space::Actor(self.current as u32),
            Body::Array(parts),
        ))
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
        if let Some(a) = frame.activation {
            self.heap.set_activation_frame(a, None);
        }
    }

    /// Starts the next turn: round robin, the first actor after the current
    /// one with a packet in its mailbox, which it delivers with
    /// `messageLoop dispatchPacket: packet`; failing that, the actor with the
    /// earliest timer, after waiting for it, with `messageLoop
    /// dispatchMessage: nil port: nil`. With neither, the run is over.
    fn next_turn(&mut self, host: &mut dyn Host) -> Result<Option<Completion>, VmError> {
        let n = self.actors.len();
        for k in 1..=n {
            let a = (self.current + k) % n;
            if let Some(packet) = self.actors[a].mailbox.pop_front() {
                self.current = a;
                let selector = self.symbol("dispatchPacket:");
                return self.dispatch_to_loop(selector, &[packet], host);
            }
        }
        // An actor a callback ran in while idle: its loop drains the sends
        // the callback queued.
        if let Some(a) = (0..n).find(|&a| self.actors[a].poked) {
            self.actors[a].poked = false;
            self.current = a;
            let nil = self.nil();
            return self.dispatch(nil, host);
        }
        let earliest = (0..n)
            .filter(|&a| self.actors[a].wakeup > 0)
            .min_by_key(|&a| (self.actors[a].wakeup, a));
        match earliest {
            Some(a) if !host.blocks() && host.monotonic_nanos() < self.actors[a].wakeup => {
                Ok(Some(Completion::Waiting(self.actors[a].wakeup)))
            }
            Some(a) => {
                host.wait_until(self.actors[a].wakeup);
                self.actors[a].wakeup = 0;
                self.current = a;
                let nil = self.nil();
                self.dispatch(nil, host)
            }
            None => Ok(Some(Completion::Returned(self.nil()))),
        }
    }

    /// Starts a turn: `messageLoop dispatchMessage: message port: nil`.
    fn dispatch(
        &mut self,
        message: Value,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let selector = self.roles.get(Role::DispatchMessageSelector);
        let nil = self.nil();
        self.dispatch_to_loop(selector, &[message, nil], host)
    }

    /// Sends the running actor's message loop `selector` with `args`,
    /// whatever the method's access (the loop's entry points are private).
    fn dispatch_to_loop(
        &mut self,
        selector: Value,
        args: &[Value],
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let receiver = self.actors[self.current].message_loop;
        let sel = self.selector_bytes(selector)?;
        let mut class = Some(self.class_of(receiver));
        while let Some(c) = class {
            if let Some(m) = self.method_at(c, &sel)? {
                self.stack.push(receiver);
                self.stack.extend(args.iter().copied());
                return self.activate(m, args.len(), host);
            }
            class = self.superclass(c)?;
        }
        Err(VmError::MissingUpcall("the message loop's entry point"))
    }

    /// The remote-representation rules (`actor-semantics.tex`, RR-Same to
    /// RR-Promise) for `v` going from actor `from` to actor `to`: a far
    /// reference into `to` becomes its target; any other far reference, any
    /// promise record and any value passes as itself; any other object
    /// becomes a new far reference into `from`, held by `to`.
    fn remote_representation(&mut self, v: Value, from: usize, to: usize) -> Value {
        match self.heap.format(v) {
            None => v,
            Some(Format::FarRef) => {
                let (actor, target) = self.heap.far_ref(v).expect("a far reference");
                if actor as usize == to { target } else { v }
            }
            Some(Format::Promise) => v,
            _ if from == to => v,
            Some(_) if self.is_value(v) => v,
            Some(_) => {
                let class = self.roles.get(Role::FarReferenceClass);
                let r = self.heap.allocate(
                    class,
                    Space::Actor(to as u32),
                    Body::FarRef {
                        actor: from as u32,
                        target: v,
                    },
                );
                self.heap.add_flags(r, VALUE);
                r
            }
        }
    }

    /// Settles promise record `record` (Π) from the running actor: with
    /// `value` (in its terms) as the resolution or the problem, or, for a
    /// resolution that is itself a promise record, by forwarding to it.
    /// Every waiter gets a packet. Answers false, changing nothing, if the
    /// record was not pending.
    fn settle(&mut self, record: Value, state: u8, value: Value) -> bool {
        let pending = self
            .heap
            .promise(record)
            .is_some_and(|p| p.state() == PromiseRecord::PENDING);
        if !pending {
            return false;
        }
        let mut value = self.neutral(value);
        let mut state = state;
        if state == PromiseRecord::FULFILLED && natives::is_promise(&self.heap, value) {
            if natives::last_promise(&self.heap, value) == record {
                state = PromiseRecord::BROKEN;
                value = self.new_string("Cyclic resolution");
            } else {
                state = PromiseRecord::FORWARDED;
            }
        }
        self.heap.set_promise(record, state, value);
        let nil = self.nil();
        let (waiters, queue) = self
            .heap
            .take_promise_lists(record, nil)
            .expect("checked above");
        if state == PromiseRecord::FORWARDED {
            // The queue moves to the end of the chain (or is delivered, if
            // that has settled).
            let last = natives::last_promise(&self.heap, value);
            let Some(q) = self.heap.promise(last) else {
                unreachable!("a chain of promise records ends in one")
            };
            let (s, v) = (q.state(), q.value());
            if s == PromiseRecord::PENDING {
                let array_class = self.roles.get(Role::ArrayClass);
                self.heap.push_promise_queue(last, &queue, array_class);
            } else {
                for entry in queue {
                    self.deliver(entry, s, v);
                }
            }
        } else {
            for entry in queue {
                self.deliver(entry, state, value);
            }
        }
        for w in waiters {
            self.notify(w, state, value);
        }
        true
    }

    /// `v` in no actor's terms: a non-value near object of the running actor
    /// becomes a far reference into it (RR-Near-Far); anything else is itself.
    fn neutral(&mut self, v: Value) -> Value {
        match self.heap.format(v) {
            Some(Format::FarRef | Format::Promise) | None => v,
            Some(_) if self.is_value(v) => v,
            Some(_) => {
                let current = self.current as u32;
                let class = self.roles.get(Role::FarReferenceClass);
                let r = self.heap.allocate(
                    class,
                    Space::Actor(current),
                    Body::FarRef {
                        actor: current,
                        target: v,
                    },
                );
                self.heap.add_flags(r, VALUE);
                r
            }
        }
    }

    /// Delivers a queued eventual send (`{sending actor. selector. result
    /// record. arguments...}`) to a promise record that has settled with
    /// `state` and `value` (Route-Fulfilled, Route-Broken): to a resolution
    /// that is a far reference, as a packet to its actor; to a value, as a
    /// packet to the sender's actor; to a broken promise, by breaking the
    /// result record with the same problem.
    fn deliver(&mut self, entry: Value, state: u8, value: Value) {
        let elements = self.heap.array_elements(entry).expect("an Array").to_vec();
        let sender = elements[0].as_small_integer().expect("an actor number") as u32;
        let result = elements[2];
        if state == PromiseRecord::BROKEN {
            if natives::is_promise(&self.heap, result) {
                self.settle(result, PromiseRecord::BROKEN, value);
            }
            return;
        }
        let to = match self.heap.far_ref(value) {
            Some((actor, _)) => actor,
            None => sender,
        };
        let mut packet = Vec::with_capacity(elements.len());
        packet.push(value);
        packet.extend_from_slice(&elements[1..]);
        let packet: Vec<Value> = packet
            .into_iter()
            .map(|v| natives::localize(&self.heap, v, to))
            .collect();
        let array_class = self.roles.get(Role::ArrayClass);
        let copy = self
            .heap
            .allocate(array_class, Space::Actor(to), Body::Array(packet));
        self.actors[to as usize].mailbox.push_back(copy);
    }

    /// Posts to the library promise that far reference `waiter` reaches
    /// `resolveRemotely: value` (fulfilled, or forwarded to the record
    /// `value`) or `breakRemotely: value`.
    fn notify(&mut self, waiter: Value, state: u8, value: Value) {
        let Some((to, _)) = self.heap.far_ref(waiter) else {
            unreachable!("waiters are far references")
        };
        let selector = if state == PromiseRecord::BROKEN {
            self.symbol("breakRemotely:")
        } else {
            self.symbol("resolveRemotely:")
        };
        let nil = self.nil();
        let packet: Vec<Value> = [waiter, selector, nil, value]
            .into_iter()
            .map(|v| natives::localize(&self.heap, v, to))
            .collect();
        let array_class = self.roles.get(Role::ArrayClass);
        let copy = self
            .heap
            .allocate(array_class, Space::Actor(to), Body::Array(packet));
        self.actors[to as usize].mailbox.push_back(copy);
    }

    /// `vm commit:` (operation list, section 6; M6 design 4.3): validates
    /// the change set, then, all or nothing, reshapes the instances its
    /// layout maps name, forwards each old object's identity to its new one,
    /// freezes the new program objects and what they reach of the program,
    /// and bumps the program version. Running frames keep their methods.
    /// Fails with the index of the malformed part (1 the pairs, 2 a layout
    /// map) before changing anything.
    fn commit(&mut self, change_set: Value) -> Result<(), u32> {
        let parts = self
            .heap
            .array_elements(change_set)
            .filter(|p| p.len() == 3)
            .map(<[Value]>::to_vec)
            .ok_or(1u32)?;
        let olds = self.heap.array_elements(parts[0]).ok_or(1u32)?.to_vec();
        let news = self.heap.array_elements(parts[1]).ok_or(1u32)?.to_vec();
        if olds.len() != news.len() || olds.iter().any(|o| o.as_reference().is_none()) {
            return Err(1);
        }
        let maps = self.heap.array_elements(parts[2]).ok_or(2u32)?.to_vec();
        let mut layouts = Vec::with_capacity(maps.len());
        for m in maps {
            let entry = self
                .heap
                .array_elements(m)
                .filter(|e| e.len() == 3)
                .map(<[Value]>::to_vec)
                .ok_or(2u32)?;
            let remap: Vec<usize> = self
                .heap
                .array_elements(entry[2])
                .ok_or(2u32)?
                .iter()
                .map(|v| v.as_small_integer().filter(|&i| i >= 0).map(|i| i as usize))
                .collect::<Option<Vec<usize>>>()
                .ok_or(2u32)?;
            layouts.push((entry[0], entry[1], remap));
        }
        // Reshape instances: identity, flags and actor are kept. An instance
        // whose size changes moves (the arena does not grow objects in
        // place); references to it are forwarded below with the rest.
        let nil = self.nil();
        let mut moved: Vec<(u64, Value)> = Vec::new();
        for (old_class, new_class, remap) in &layouts {
            let instances: Vec<Value> = self
                .heap
                .objects()
                .filter(|&v| {
                    self.heap.class(v) == Some(*old_class)
                        && self.heap.format(v) == Some(Format::Regular)
                })
                .collect();
            for v in instances {
                let old = self.heap.regular_slots(v).expect("filtered");
                let slots: Vec<Value> = remap
                    .iter()
                    .map(|&i| {
                        if i == 0 {
                            nil
                        } else {
                            old.get(i - 1).copied().unwrap_or(nil)
                        }
                    })
                    .collect();
                let now = self.heap.reshape(v, *new_class, slots);
                if now != v {
                    moved.push((v.as_reference().expect("an object"), now));
                }
            }
        }
        // Forward identities, in the heap and in every root the VM holds.
        let mut map: BTreeMap<u64, Value> = olds
            .iter()
            .zip(news.iter())
            .filter(|(o, n)| o != n)
            .map(|(o, n)| (o.as_reference().expect("checked"), *n))
            .collect();
        map.extend(moved);
        if !map.is_empty() {
            let forward = |v: Value| match v.as_reference() {
                Some(i) => map.get(&i).copied().unwrap_or(v),
                None => v,
            };
            self.heap.forward_identities(&forward);
            // References were rewritten across the heap, old objects
            // included, without the barrier.
            self.heap.require_full_collection();
            for v in &mut self.stack {
                *v = forward(*v);
            }
            for f in &mut self.frames {
                f.receiver = forward(f.receiver);
                f.closure = f.closure.map(forward);
                f.literals = forward(f.literals);
                f.mixin = forward(f.mixin);
            }
            let pairs: Vec<(Role, Value)> = self.roles.pairs().collect();
            for (role, v) in pairs {
                self.roles.set(role, forward(v));
            }
            for a in &mut self.actors {
                a.message_loop = forward(a.message_loop);
                for p in a.mailbox.iter_mut() {
                    *p = forward(*p);
                }
            }
            for e in &mut self.expats {
                e.0 = forward(e.0);
            }
            for v in self.symbols.values_mut() {
                *v = forward(*v);
            }
        }
        // Freeze the new program: each new method, mixin and class, and the
        // methods and method arrays its mixins hold.
        let mut pending: Vec<Value> = news;
        let mut seen = alloc::collections::BTreeSet::new();
        while let Some(v) = pending.pop() {
            if !seen.insert(v.as_reference()) {
                continue;
            }
            let Some(format) = self.heap.format(v) else {
                continue;
            };
            if self.heap.flags(v) & FROZEN != 0 {
                continue;
            }
            match format {
                Format::Method => {
                    let literals = self.heap.method(v).expect("a method").literals();
                    self.heap.add_flags(v, FROZEN);
                    pending.push(literals);
                }
                Format::Mixin => {
                    let m = self.heap.mixin(v).expect("a mixin");
                    let (methods, class_mixin) = (m.methods(), m.class_mixin());
                    self.heap.add_flags(v, FROZEN);
                    pending.push(methods);
                    pending.push(class_mixin);
                }
                Format::Class | Format::Metaclass => {
                    let mixin = self.heap.behavior(v).expect("a class").mixin();
                    self.heap.add_flags(v, FROZEN);
                    pending.push(mixin);
                }
                // A mixin's methods, a method's literals: followed, not
                // frozen, so that only program objects are.
                Format::Array => {
                    let e = self.heap.fields(v).expect("an array").to_vec();
                    pending.extend(e);
                }
                _ => {}
            }
        }
        self.program_version += 1;
        self.flush_lookup_cache();
        Ok(())
    }

    /// Rule Native-Fail: allocates the failure exception and sends it
    /// #signal; its answer is the send's answer.
    fn native_failure(
        &mut self,
        native: Native,
        receiver: Value,
        args: Vec<Value>,
        reason: Reason,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let failure = self.new_failure(native, receiver, args, reason, host);
        self.stack.push(failure);
        let signal = self.symbol("signal");
        self.ordinary_send(signal, 0, host)
    }

    /// `v` as it crosses to the host (operation list, section 10): nil,
    /// Booleans, Integers within 2^53, Floats and Strings by value, Aliens
    /// as their references, closures as new expats owned by the running
    /// actor; anything else fails with `#wrongKind: index`.
    fn marshal_out(&mut self, v: Value, index: u32) -> Result<HostValue, Reason> {
        if v == self.nil() {
            return Ok(HostValue::Null);
        }
        if v == self.roles.get(Role::True) {
            return Ok(HostValue::Bool(true));
        }
        if v == self.roles.get(Role::False) {
            return Ok(HostValue::Bool(false));
        }
        if let Some(n) = v.as_small_integer() {
            if n.abs() > natives::HOST_EXACT {
                return Err(Reason::OutOfRange(index));
            }
            return Ok(HostValue::Number(n as f64));
        }
        if let Some(r) = natives::alien_reference_in(&self.heap, &self.roles, v) {
            return Ok(if r == 0 {
                HostValue::Undefined
            } else {
                HostValue::Ref(r)
            });
        }
        match self.heap.format(v) {
            Some(Format::Float) => Ok(HostValue::Number(self.heap.float(v).expect("a float"))),
            Some(Format::String) => {
                match core::str::from_utf8(self.heap.bytes(v).expect("bytes")) {
                    Ok(s) => Ok(HostValue::String(String::from(s))),
                    Err(_) => Err(Reason::WrongKind(index)),
                }
            }
            Some(Format::Closure) => {
                let id = self.expats.len() as u32;
                self.expats.push((v, self.current as u32));
                Ok(HostValue::Expat(id))
            }
            Some(Format::LargeInteger) => Err(Reason::OutOfRange(index)),
            _ => Err(Reason::WrongKind(index)),
        }
    }

    /// A host value as it arrives in Newspeak, allocated in `space`: `null`
    /// as nil, `undefined` and host objects as Aliens, integral numbers
    /// within 2^53 as Integers.
    fn marshal_in(&mut self, v: HostValue, space: Space) -> Value {
        let alien = |vm: &mut Vm, r: u32| {
            let class = vm.roles.get(Role::AlienClass);
            let reference = Value::small_integer(i64::from(r)).expect("small");
            vm.heap
                .allocate(class, space, Body::Regular(alloc::vec![reference]))
        };
        match v {
            HostValue::Undefined => alien(self, 0),
            HostValue::Null => self.nil(),
            HostValue::Bool(b) => self.roles.get(if b { Role::True } else { Role::False }),
            HostValue::Number(x) => {
                if libm::trunc(x) == x && x.abs() <= natives::HOST_EXACT as f64 {
                    Value::small_integer(x as i64).expect("small")
                } else {
                    let x = if x.is_nan() { f64::NAN } else { x };
                    let class = self.roles.get(Role::FloatClass);
                    self.heap.allocate(class, space, Body::Float(x))
                }
            }
            HostValue::String(s) => {
                let class = self.roles.get(Role::StringClass);
                self.heap
                    .allocate(class, space, Body::String(s.as_bytes().to_vec()))
            }
            HostValue::Ref(r) | HostValue::Expat(r) => alien(self, r),
        }
    }

    /// Runs `closure` with the Array `arguments` as a callback: above the
    /// boundary frame (the callback-dispatch method, role 28), whose
    /// `on: Exception do:` stops exceptions and through which a non-local
    /// return raises `cannotReturn:`; nothing else runs meanwhile. Answers
    /// the boundary's `{true. result}` or `{false. description}`.
    fn run_callback(
        &mut self,
        closure: Value,
        arguments: Value,
        host: &mut dyn Host,
    ) -> Result<(bool, Value), VmError> {
        let method = self.roles.get(Role::CallbackDispatchMethod);
        if method == self.nil() {
            return Err(VmError::MissingUpcall("the callback-dispatch method"));
        }
        let base = self.frames.len();
        self.stack.push(closure);
        self.stack.push(arguments);
        if self.activate(method, 1, host)?.is_some() {
            return Err(VmError::Malformed("a callback exited"));
        }
        while self.frames.len() > base {
            if self.gc_due() {
                self.scheduled_collection();
            }
            if let Step::Done(_) = self.step(host)? {
                return Err(VmError::Malformed("a callback exited"));
            }
        }
        let answer = self
            .stack
            .pop()
            .ok_or_else(|| VmError::Malformed("empty stack"))?;
        let pair = self
            .heap
            .array_elements(answer)
            .filter(|e| e.len() == 2)
            .map(<[Value]>::to_vec)
            .ok_or_else(|| VmError::Malformed("the callback boundary answered no pair"))?;
        Ok((pair[0] == self.roles.get(Role::True), pair[1]))
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
        let method = method.ok_or_else(|| VmError::MissingUpcall(selector.name()))?;
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
            .ok_or_else(|| VmError::Malformed("closure expected"))?
            .defining();
        for _ in 0..MAX_FRAMES {
            let a = self
                .heap
                .activation(home)
                .ok_or_else(|| VmError::Malformed("activation expected"))?;
            if a.closure() == nil {
                return Ok(home);
            }
            home = self
                .heap
                .closure(a.closure())
                .ok_or_else(|| VmError::Malformed("closure expected"))?
                .defining();
        }
        Err(VmError::Malformed("closure chain too deep"))
    }

    /// `^result` inside a closure: return from the home method, following
    /// psoup's protocol. An intervening ensure: frame gets
    /// #aboutToReturn:through:; a dead home, or a simulation-root (role 27,
    /// or a method marked by `vm markSimulationRoot:`) or callback-dispatch
    /// frame in between, gets #cannotReturn:.
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
        let home_serial = self.heap.activation(home).and_then(|a| a.frame());
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
            let marked = self.heap.flags(method) & crate::object::SIMULATION_ROOT != 0;
            if marked || blockers.iter().any(|&b| b == method && b != nil) {
                break;
            }
        }
        self.upcall(Role::CannotReturnSelector, &[result], host)
    }

    /// The failure object of `eq:native-failure-reification`: an instance of
    /// the failure class with slots native name, receiver, arguments, reason
    /// and reason argument. No user code runs.
    /// Whether `v` is a value (see [`NativeContext::is_value`]).
    fn is_value(&self, v: Value) -> bool {
        v == self.nil()
            || v == self.roles.get(Role::True)
            || v == self.roles.get(Role::False)
            || self.heap.is_value(v)
    }

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
        let arguments = self.heap.allocate(
            array_class,
            Space::Actor(self.current as u32),
            Body::Array(args),
        );
        let reason_name = self.new_string(reason.name());
        let reason_argument = {
            let mut ctx = NativeContext {
                heap: &mut self.heap,
                roles: &self.roles,
                host,
                space: Space::Actor(self.current as u32),
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
        self.heap.allocate(
            class,
            Space::Actor(self.current as u32),
            Body::Regular(slots),
        )
    }

    // ----- Execution -----

    fn invalid(&self) -> VmError {
        let f = self.frames.last().expect("a frame");
        VmError::InvalidBytecode {
            selector: self.method_selector(f.method),
            index: f.ip,
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
            .and_then(|b| self.heap.string_bytes(b.name()))
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
            .and_then(|m| self.heap.string_bytes(m.selector()))
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_else(|| String::from("?"))
    }

    /// Literal `i` of the running frame's method.
    fn literal(&self, i: usize) -> Option<Value> {
        let f = self.frames.last()?;
        self.heap.array_elements(f.literals)?.get(i).copied()
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
        let receiver = self.frames.last().expect("a frame").receiver;
        let selector = self.literal(selector_index).ok_or_else(|| self.invalid())?;
        let available = self.stack.len() - self.frames.last().expect("a frame").lp;
        let needed = if kind == SendKind::Ordinary { n + 1 } else { n };
        if available < needed {
            return Err(self.invalid());
        }
        if kind == SendKind::Ordinary {
            return self.ordinary_send(selector, n, host);
        }
        let mixin = self.frames.last().expect("a frame").mixin;
        let rule = match kind {
            SendKind::SelfSend => Rule::SelfSend,
            SendKind::Super => Rule::Super,
            _ => Rule::ImplicitReceiver,
        };
        self.send(rule, selector, n, receiver, mixin, host)
    }

    fn outer_send(
        &mut self,
        selector_index: usize,
        n: usize,
        depth: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        let receiver = self.frames.last().expect("a frame").receiver;
        let selector = self.literal(selector_index).ok_or_else(|| self.invalid())?;
        let mixin = self.frames.last().expect("a frame").mixin;
        self.send(Rule::Outer(depth), selector, n, receiver, mixin, host)
    }

    fn enclosing_object(&self, depth: usize) -> Result<Value, VmError> {
        let f = self.frames.last().expect("a frame");
        let mut receiver = f.receiver;
        let mut mixin = f.mixin;
        for _ in 0..depth {
            let application = self.find_application_of(mixin, self.class_of(receiver))?;
            receiver = self
                .heap
                .behavior(application)
                .expect("class")
                .enclosing_object();
            mixin = self
                .heap
                .mixin(mixin)
                .ok_or_else(|| VmError::Malformed("mixin expected"))?
                .enclosing_mixin();
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

    // ----- Compiled code (newspeak/EURASIA_COMPILER_DESIGN_2026-11-19.md) -----

    /// Compiles a method at its `threshold`th activation (0: never). Only a
    /// host that installs code ([`Host::install_code`]) should set it.
    pub fn set_jit_threshold(&mut self, threshold: u32) {
        self.jit_threshold = i64::from(threshold);
    }

    /// Counts an activation of `method`, and compiles it at the threshold.
    fn tier_up(&mut self, method: Value, host: &mut dyn Host) {
        let Some(word) = self.heap.method_jit(method) else {
            return;
        };
        if word < 0 {
            return;
        }
        if word + 1 < self.jit_threshold {
            self.heap.set_method_jit(method, word + 1);
            return;
        }
        let code = self
            .heap
            .method(method)
            .expect("a method")
            .bytecode()
            .clone();
        let ok: Vec<bool> = (0..SMALL_INTEGER_FAST_PATHS)
            .map(|k| self.small_integer_native_is(k))
            .collect();
        let compiled = crate::compiler::compile(&code, &|k| ok.get(k) == Some(&true));
        match compiled.and_then(|c| host.install_code(&c.module).map(|i| (i, c.entries))) {
            Some((i, entries)) => self.heap.set_method_compiled(method, i, entries.into()),
            None => self.heap.set_method_jit(method, crate::object::JIT_NEVER),
        }
    }

    /// Counts a closure activation (the top frame) against its method.
    fn tier_up_closure(&mut self, host: &mut dyn Host) {
        if self.jit_threshold > 0 {
            let method = self.frames.last().expect("a frame").method;
            self.tier_up(method, host);
        }
    }

    /// The compiled function of the top frame and its block, if the frame
    /// is at an entry of its compiled method (and no step limit counts
    /// instructions).
    fn compiled_entry(&self) -> Option<(u32, u32)> {
        if self.step_limit.is_some() {
            return None;
        }
        let f = self.frames.last()?;
        self.heap.compiled_entry(f.method, &f.code, f.ip)
    }

    /// Continues the run after compiled code answered `status`.
    pub fn continue_after_compiled(
        &mut self,
        status: i32,
        host: &mut dyn Host,
    ) -> Result<Completion, VmError> {
        if status == crate::compiler::STOP {
            match self.jit_stop.take() {
                Some(Err(e)) => return Err(e),
                Some(Ok(Some(c))) => return Ok(c),
                _ => {}
            }
        }
        self.event_loop(host)
    }

    /// For the host: the registers' address, to pass to compiled code.
    pub fn jit_regs(&mut self) -> *mut crate::compiler::Regs {
        &mut *self.jit_regs
    }

    /// For the host: the operand stack, to set the height compiled code left
    /// (it writes Values past the height, within the room it reserved).
    pub fn operand_stack(&mut self) -> &mut Vec<Value> {
        &mut self.stack
    }

    /// Sets the registers for the top frame.
    fn fill_jit_regs(&mut self) {
        self.stack.reserve(crate::compiler::HEADROOM);
        let f = self.frames.last().expect("a frame");
        let (literals, literal_count) = self
            .heap
            .array_elements(f.literals)
            .map_or((0, 0), |e| (e.as_ptr() as usize as u32, e.len() as u32));
        let (bp, lp, argc, receiver) = (f.bp, f.lp, f.arg_count, f.receiver);
        let (nil, t, fa) = (
            self.nil(),
            self.roles.get(Role::True),
            self.roles.get(Role::False),
        );
        let due = self.safepoint_due();
        let r = &mut *self.jit_regs;
        r.base = self.stack.as_ptr() as usize as u32;
        r.sp = self.stack.len() as u32;
        r.bp = bp as u32;
        r.lp = lp as u32;
        r.argc = argc as u32;
        r.due = u32::from(due);
        r.literals = literals;
        r.literal_count = literal_count;
        r.arena = self.heap.words.as_ptr() as usize as u32;
        r.boundary = self.heap.boundary as u32;
        r.receiver = receiver.to_bits();
        r.nil = nil.to_bits();
        r.true_ = t.to_bits();
        r.false_ = fa.to_bits();
        r.depth = self.frames.len() as u32;
    }

    /// Updates the registers after a call that left the top frame as it
    /// was: the stack, the arena (which an allocation may have moved, and
    /// the literals with it), the safepoint.
    fn refresh_jit_regs(&mut self) {
        self.stack.reserve(crate::compiler::HEADROOM);
        let arena = self.heap.words.as_ptr() as usize as u32;
        let due = self.safepoint_due();
        let r = &mut *self.jit_regs;
        r.base = self.stack.as_ptr() as usize as u32;
        r.sp = self.stack.len() as u32;
        r.due = u32::from(due);
        if r.literal_count > 0 {
            r.literals = r.literals.wrapping_sub(r.arena).wrapping_add(arena);
        }
        r.arena = arena;
        r.boundary = self.heap.boundary as u32;
    }

    /// For compiled code: a send from the top frame, whose instruction
    /// continues at `next_ip`; see [`crate::compiler`] for the answer.
    pub fn compiled_send(
        &mut self,
        host: &mut dyn Host,
        rule: i32,
        literal: usize,
        n: usize,
        next_ip: usize,
    ) -> i32 {
        use crate::compiler::{RULE_COMMON, RULE_ORDINARY, RULE_SELF, RULE_SUPER};
        let before = self.before_compiled_call();
        self.jit_regs.ic_kind = 0;
        self.frames.last_mut().expect("a frame").ip = next_ip;
        let cached = match rule {
            RULE_COMMON => None,
            RULE_ORDINARY => self.cached_send(Rule::Ordinary, literal, n, host),
            RULE_SELF => self.cached_send(Rule::SelfSend, literal, n, host),
            RULE_SUPER => None,
            _ => self.cached_send(Rule::ImplicitReceiver, literal, n, host),
        };
        let result = match cached {
            Some(r) => r,
            None if rule == RULE_COMMON => self.common_send(literal, host),
            None => {
                let kind = match rule {
                    RULE_ORDINARY => SendKind::Ordinary,
                    RULE_SELF => SendKind::SelfSend,
                    RULE_SUPER => SendKind::Super,
                    _ => SendKind::ImplicitReceiver,
                };
                self.send_by_literal(kind, literal, n, host)
            }
        };
        self.after_compiled_call(before, result, next_ip, false)
    }

    /// A send of literal `index` by `rule` with `n` arguments from the top
    /// frame through the lookup cache, as `run_simple` makes it: an accessor
    /// read or written here, any other cached method activated. `None` on a
    /// miss, or where the full send must decide.
    fn cached_send(
        &mut self,
        rule: Rule,
        index: usize,
        n: usize,
        host: &mut dyn Host,
    ) -> Option<Result<Option<Completion>, VmError>> {
        let selector = self.literal(index)?;
        let (receiver, mixin, lp) = {
            let f = self.frames.last()?;
            (f.receiver, f.mixin, f.lp)
        };
        let nil = self.nil();
        let len = self.stack.len();
        let ordinary = rule == Rule::Ordinary;
        if len < lp + n + usize::from(ordinary) {
            return None;
        }
        let (key, key_mixin) = if ordinary {
            (self.stack[len - 1 - n], nil)
        } else {
            (receiver, mixin)
        };
        let class = self.class_of(key);
        let e = self.lookup_cache[cache_index(class, selector, key_mixin, rule)]?;
        if e.class != class || e.selector != selector || e.mixin != key_mixin || e.rule != rule {
            return None;
        }
        let object = match e.target.receiver {
            TargetReceiver::Present => key,
            TargetReceiver::Frame => receiver,
            TargetReceiver::Fixed(r) => r,
        };
        // An accessor of the receiver itself: compiled code's inline cache
        // can hold it (keyed by its class; the mixin is the site's).
        let own = matches!(
            (e.target.receiver, ordinary),
            (TargetReceiver::Present, true) | (TargetReceiver::Frame, false)
        );
        match (e.target.header.kind, n, e.target.slot) {
            (MethodKind::Getter, 0, Some(slot)) => {
                let v = *self.heap.regular_slots(object)?.get(slot)?;
                if ordinary {
                    self.stack[len - 1] = v;
                } else {
                    self.stack.push(v);
                }
                if own {
                    self.jit_cache(object, slot);
                }
                Some(Ok(None))
            }
            (MethodKind::Setter, 1, Some(slot)) => {
                if self.heap.flags(object) & (FROZEN | VALUE) != 0 {
                    return None;
                }
                let value = self.stack[len - 1];
                *self.heap.regular_slots_mut(object)?.get_mut(slot)? = value;
                if ordinary {
                    self.stack.pop();
                } else {
                    self.stack[len - 1] = object;
                }
                if own {
                    self.jit_cache(object, slot);
                }
                Some(Ok(None))
            }
            _ => {
                let t = e.target;
                match t.receiver {
                    TargetReceiver::Present => {}
                    TargetReceiver::Frame => self.insert_receiver(receiver, n),
                    TargetReceiver::Fixed(r) => self.insert_receiver(r, n),
                }
                Some(self.activate_with(t.method, t.header, n, host, t.slot))
            }
        }
    }

    /// For compiled code: the top frame's instruction at `ip`, as the
    /// interpreter runs it; see [`crate::compiler`] for the answer.
    pub fn compiled_step(&mut self, host: &mut dyn Host, ip: usize) -> i32 {
        let before = self.before_compiled_call();
        let (expected, returns) = {
            let f = self.frames.last_mut().expect("a frame");
            f.ip = ip;
            let o = f.code.get(ip).copied().unwrap_or(0);
            let len = instruction_length(o);
            let expected = if o == op::PUSH_CLOSURE && ip + 4 <= f.code.len() {
                ip + len + u16::from_le_bytes([f.code[ip + 2], f.code[ip + 3]]) as usize
            } else {
                ip + len
            };
            (expected, (171..=175).contains(&o))
        };
        let result = self.step(host).map(|s| match s {
            Step::Continue => None,
            Step::Done(c) => Some(c),
        });
        self.after_compiled_call(before, result, expected, returns)
    }

    /// What [`Vm::after_compiled_call`] compares with: the frame count, the
    /// top frame's serial and the collection count.
    fn before_compiled_call(&self) -> (usize, u64, u64) {
        let serial = self.frames.last().expect("a frame").serial;
        (self.frames.len(), serial, self.collections())
    }

    /// The answer to compiled code for a call into the runtime that ended
    /// with `result`, the frames as `before` was: the instruction completed
    /// ([`DONE`](crate::compiler::DONE)), the frame returned
    /// ([`RETURNED`](crate::compiler::RETURNED)), a compiled callee's frame
    /// is on top, at an entry ([`CALLEE`](crate::compiler::CALLEE) plus its
    /// function; its block in `jit_block`), or the interpreter continues
    /// ([`INTERPRET`](crate::compiler::INTERPRET)) or the run stops
    /// ([`STOP`](crate::compiler::STOP)). Compiled code holds Values in its
    /// own locals, so after a collection it does not go on: the interpreter
    /// gives the frame back at its next entry. Compiled code goes on only
    /// where it expects: after the instruction, at `expected` (an activation
    /// native can move a frame, or cut frames back), and with a return or
    /// non-local return only after a return instruction (`returns`).
    fn after_compiled_call(
        &mut self,
        (depth, serial, collections): (usize, u64, u64),
        result: Result<Option<Completion>, VmError>,
        expected: usize,
        returns: bool,
    ) -> i32 {
        use crate::compiler::{CALLEE, DONE, INTERPRET, RETURNED, STOP, UNWOUND};
        /// Compiled callees nested in the host's stack, at most.
        const MAX_NESTED: usize = 512;
        match result {
            Err(e) => {
                self.jit_stop = Some(Err(e));
                return STOP;
            }
            Ok(Some(c)) => {
                self.jit_stop = Some(Ok(Some(c)));
                return STOP;
            }
            Ok(None) => {}
        }
        if self.collections() != collections {
            return INTERPRET;
        }
        let len = self.frames.len();
        if len == depth && self.frames[len - 1].serial == serial {
            if self.frames[len - 1].ip != expected {
                return INTERPRET;
            }
            self.refresh_jit_regs();
            DONE
        } else if !returns && len < depth {
            INTERPRET
        } else if len + 1 == depth {
            self.jit_regs.ic_kind = 0;
            if len > 0 {
                self.fill_jit_regs();
            }
            RETURNED
        } else if len == depth + 1
            && self.frames[depth - 1].serial == serial
            && self.frames[depth - 1].ip == expected
            && len.saturating_sub(self.jit_base) < MAX_NESTED
            && !self.safepoint_due()
            && let Some((code, block)) = self.compiled_entry()
        {
            self.jit_regs.block = block;
            self.fill_jit_regs();
            CALLEE + code as i32
        } else if returns && len + 1 < depth && len >= self.jit_base && len > 0 {
            // A non-local return to a frame that compiled code holds (every
            // frame from the one the host entered up is a compiled call).
            self.jit_regs.ic_kind = 0;
            self.fill_jit_regs();
            UNWOUND
        } else {
            INTERPRET
        }
    }

    /// Offers compiled code's inline cache the accessor of `object`'s class
    /// at `slot`.
    fn jit_cache(&mut self, object: Value, slot: usize) {
        let class = self.class_of(object);
        let r = &mut *self.jit_regs;
        r.ic_kind = 1;
        r.ic_slot = slot as u32;
        r.ic_class = class.to_bits();
    }

    /// For compiled code: [`crate::compiler::TICK`] backward jumps of the top
    /// frame, the next at `ip`, counted against the interrupt poll. When
    /// the poll is due, the interpreter's loop takes it.
    pub fn compiled_tick(&mut self, ip: usize) -> i32 {
        let tick = crate::compiler::TICK as u32;
        if self.interrupt_countdown > tick + 1 {
            self.interrupt_countdown -= tick;
            self.refresh_jit_regs();
            crate::compiler::DONE
        } else {
            self.interrupt_countdown = 1;
            self.frames.last_mut().expect("a frame").ip = ip;
            crate::compiler::INTERPRET
        }
    }

    /// For compiled code: the local return of `value` from the top frame.
    pub fn compiled_return(&mut self, value: Value) -> i32 {
        self.local_return(value);
        // The caller's send did not hit an accessor (its callee's last send
        // may have).
        self.jit_regs.ic_kind = 0;
        if !self.frames.is_empty() {
            self.fill_jit_regs();
        }
        crate::compiler::DONE
    }

    /// For compiled code: the interpreter continues the top frame at `ip`.
    pub fn compiled_interpret(&mut self, ip: usize) -> i32 {
        self.frames.last_mut().expect("a frame").ip = ip;
        crate::compiler::INTERPRET
    }

    /// For compiled code: whether the event loop has a safepoint due.
    pub fn safepoint_due(&self) -> bool {
        self.heap.allocation_counter() >= self.safepoint_at
    }

    /// Runs instructions of the current frame, from its instruction, with
    /// the instruction pointer and the code in locals: the simple ones
    /// (pushes, stores, branches, SmallInteger arithmetic) here, and the
    /// first other one by [`Vm::step`], after writing the instruction
    /// pointer back. Anything out of the ordinary in a simple one (an index
    /// out of range, a non-Boolean condition) also goes to `step`, which
    /// does what it always did, errors included.
    ///
    /// The event loop's checks before an instruction still hold for each:
    /// no simple instruction allocates, so no collection or checkpoint can
    /// come due between two of them, and the interrupt countdown is counted
    /// down for every instruction, returning to the event loop before the
    /// one at which it polls the host.
    #[inline(always)]
    fn run_simple(&mut self, host: &mut dyn Host) -> Result<Step, VmError> {
        // The event loop has done its checks for the first instruction.
        let mut first = true;
        loop {
            let (code, mut ip, bp, lp, arg_count, receiver, mixin) = {
                let f = self.frames.last().expect("a frame");
                (
                    f.code.clone(),
                    f.ip,
                    f.bp,
                    f.lp,
                    f.arg_count,
                    f.receiver,
                    f.mixin,
                )
            };
            let (nil, t, fl) = (
                self.nil(),
                self.roles.get(Role::True),
                self.roles.get(Role::False),
            );
            let code_len = code.len();
            let exit = loop {
                if !first {
                    // The event loop's countdown for this instruction; at the
                    // one that polls, let the event loop run it.
                    if self.interrupt_countdown == 1 {
                        break Exit::Polls;
                    }
                    self.interrupt_countdown -= 1;
                }
                first = false;
                let len = self.stack.len();
                if ip >= code_len {
                    break Exit::Step;
                }
                let opcode = code[ip];
                let ilen = instruction_length(opcode);
                if ip + ilen > code_len {
                    break Exit::Step;
                }
                let next = ip + ilen;
                let b1 = if ilen > 1 { code[ip + 1] } else { 0 };
                let b2 = if ilen > 2 { code[ip + 2] } else { 0 };
                let wide = u16::from_le_bytes([b1, b2]) as usize;
                match opcode {
                    0..=15 => match next.checked_sub((opcode & 15) as usize) {
                        Some(target) => ip = target,
                        None => break Exit::Step,
                    },
                    16..=31 => ip = next + (opcode & 15) as usize,
                    op::BRANCH_BACK => match next.checked_sub(wide) {
                        Some(target) => ip = target,
                        None => break Exit::Step,
                    },
                    op::BRANCH => ip = next + wide,
                    32..=63 | op::BRANCH_IF_TRUE | op::BRANCH_IF_FALSE => {
                        if len <= lp {
                            break Exit::Step;
                        }
                        let v = self.stack[len - 1];
                        let condition = matches!(opcode, 32..=47 | op::BRANCH_IF_TRUE);
                        let taken = if v == t {
                            condition
                        } else if v == fl {
                            !condition
                        } else {
                            break Exit::Step;
                        };
                        self.stack.pop();
                        let forward = if opcode < 64 {
                            (opcode & 15) as usize
                        } else {
                            wide
                        };
                        ip = if taken { next + forward } else { next };
                    }
                    112..=119 | op::PUSH_PARAMETER => {
                        let p = if opcode == op::PUSH_PARAMETER {
                            b1 as usize
                        } else {
                            (opcode & 7) as usize
                        };
                        if p == 0 || p > arg_count || bp + 1 + (arg_count - p) >= len {
                            break Exit::Step;
                        }
                        self.stack.push(self.stack[bp + 1 + (arg_count - p)]);
                        ip = next;
                    }
                    120..=127 | op::PUSH_LOCAL => {
                        let i = if opcode == op::PUSH_LOCAL {
                            b1 as usize
                        } else {
                            (opcode & 7) as usize
                        };
                        if lp + i >= len {
                            break Exit::Step;
                        }
                        self.stack.push(self.stack[lp + i]);
                        ip = next;
                    }
                    128..=135 | op::POP_INTO_LOCAL => {
                        let i = if opcode == op::POP_INTO_LOCAL {
                            b1 as usize
                        } else {
                            (opcode & 7) as usize
                        };
                        // `step` checks the slot before popping; a slot at the
                        // top is left to it as well.
                        if lp + i + 1 >= len || len <= lp {
                            break Exit::Step;
                        }
                        let v = self.stack[len - 1];
                        self.stack.pop();
                        self.stack[lp + i] = v;
                        ip = next;
                    }
                    136..=143 | op::STORE_INTO_LOCAL => {
                        let i = if opcode == op::STORE_INTO_LOCAL {
                            b1 as usize
                        } else {
                            (opcode & 7) as usize
                        };
                        if lp + i >= len || len <= lp {
                            break Exit::Step;
                        }
                        self.stack[lp + i] = self.stack[len - 1];
                        ip = next;
                    }
                    144..=151 | op::PUSH_LITERAL => {
                        let i = if opcode == op::PUSH_LITERAL {
                            wide
                        } else {
                            (opcode & 7) as usize
                        };
                        let Some(v) = self.literal(i) else {
                            break Exit::Step;
                        };
                        self.stack.push(v);
                        ip = next;
                    }
                    op::PUSH_NIL => {
                        self.stack.push(nil);
                        ip = next;
                    }
                    op::PUSH_FALSE => {
                        self.stack.push(fl);
                        ip = next;
                    }
                    op::PUSH_TRUE => {
                        self.stack.push(t);
                        ip = next;
                    }
                    op::PUSH_SELF => {
                        self.stack.push(receiver);
                        ip = next;
                    }
                    op::PUSH_MIXIN => {
                        self.stack.push(mixin);
                        ip = next;
                    }
                    op::POP => {
                        if len <= lp {
                            break Exit::Step;
                        }
                        self.stack.pop();
                        ip = next;
                    }
                    op::DUP => {
                        if len <= lp {
                            break Exit::Step;
                        }
                        self.stack.push(self.stack[len - 1]);
                        ip = next;
                    }
                    160..=163 => {
                        self.stack
                            .push(Value::small_integer(i64::from(opcode) - 161).expect("small"));
                        ip = next;
                    }
                    op::PUSH_INTEGER => {
                        let n = i16::from_le_bytes([b1, b2]);
                        self.stack
                            .push(Value::small_integer(n.into()).expect("small"));
                        ip = next;
                    }
                    // A send whose cached method is an accessor with a known
                    // slot: the slot is read or written here, without the
                    // send path (EURASIA_PERFORMANCE_2026-11-19.md section
                    // 12). Anything else, a miss included, goes to `step`.
                    64..=111
                    | 176..=207
                    | op::ORDINARY_SEND
                    | op::SELF_SEND
                    | op::IMPLICIT_RECEIVER_SEND => {
                        // SmallInteger arithmetic and comparisons first.
                        if (176..=189).contains(&opcode)
                            && len >= lp + 2
                            && let (Some(a), Some(b)) = (
                                self.stack[len - 2].as_small_integer(),
                                self.stack[len - 1].as_small_integer(),
                            )
                            && let Some(v) = self.small_integer_op((opcode - 176) as usize, a, b)
                        {
                            self.stack.pop();
                            self.stack[len - 2] = v;
                            ip = next;
                            continue;
                        }
                        let short = |kind: Rule| {
                            (
                                kind,
                                Some((opcode & 7) as usize),
                                ((opcode >> 3) & 1) as usize,
                            )
                        };
                        let (rule, index, n) = match opcode {
                            64..=79 => short(Rule::Ordinary),
                            80..=95 => short(Rule::SelfSend),
                            96..=111 => short(Rule::ImplicitReceiver),
                            176..=207 => (Rule::Ordinary, None, 0),
                            _ => {
                                let rule = match opcode {
                                    op::ORDINARY_SEND => Rule::Ordinary,
                                    op::SELF_SEND => Rule::SelfSend,
                                    _ => Rule::ImplicitReceiver,
                                };
                                (
                                    rule,
                                    Some(b1 as usize | ((b2 as usize & 0x0f) << 8)),
                                    (b2 >> 4) as usize,
                                )
                            }
                        };
                        // A common send's selector and arity are in the table.
                        let (selector, n) = match index {
                            Some(i) => match self.literal(i) {
                                Some(sel) => (sel, n),
                                None => break Exit::Step,
                            },
                            None => {
                                let k = (opcode - 176) as usize;
                                let table = self.roles.get(Role::CommonSelectors);
                                match self.heap.array_elements(table).and_then(|e| {
                                    Some((*e.get(2 * k)?, e.get(2 * k + 1)?.as_small_integer()?))
                                }) {
                                    Some((sel, arity)) if arity >= 0 => (sel, arity as usize),
                                    _ => break Exit::Step,
                                }
                            }
                        };
                        let ordinary = rule == Rule::Ordinary;
                        if len < lp + n + usize::from(ordinary) {
                            break Exit::Step;
                        }
                        let (key, key_mixin) = if ordinary {
                            (self.stack[len - 1 - n], nil)
                        } else {
                            (receiver, mixin)
                        };
                        let class = self.class_of(key);
                        let Some(e) =
                            self.lookup_cache[cache_index(class, selector, key_mixin, rule)]
                        else {
                            break Exit::Step;
                        };
                        if e.class != class
                            || e.selector != selector
                            || e.mixin != key_mixin
                            || e.rule != rule
                        {
                            break Exit::Step;
                        }
                        let slot = e.target.slot;
                        let object = match e.target.receiver {
                            TargetReceiver::Present => key,
                            TargetReceiver::Frame => receiver,
                            TargetReceiver::Fixed(r) => r,
                        };
                        match (e.target.header.kind, n, slot) {
                            (MethodKind::Getter, 0, Some(slot)) => {
                                let Some(&v) =
                                    self.heap.regular_slots(object).and_then(|s| s.get(slot))
                                else {
                                    break Exit::Step;
                                };
                                if ordinary {
                                    self.stack[len - 1] = v;
                                } else {
                                    self.stack.push(v);
                                }
                            }
                            (MethodKind::Setter, 1, Some(slot)) => {
                                // A frozen object or a value: `step`, which refuses
                                // or checks the value.
                                if self.heap.flags(object) & (FROZEN | VALUE) != 0 {
                                    break Exit::Step;
                                }
                                let value = self.stack[len - 1];
                                let Some(s) = self
                                    .heap
                                    .regular_slots_mut(object)
                                    .and_then(|s| s.get_mut(slot))
                                else {
                                    break Exit::Step;
                                };
                                *s = value;
                                // The answer is the receiver.
                                if ordinary {
                                    self.stack.pop();
                                } else {
                                    self.stack[len - 1] = object;
                                }
                            }
                            // Any other method the cache found: the send, from here
                            // (what `step` would do, without looking it up again).
                            _ => {
                                let t = e.target;
                                self.frames.last_mut().expect("a frame").ip = next;
                                match t.receiver {
                                    TargetReceiver::Present => {}
                                    TargetReceiver::Frame => self.insert_receiver(receiver, n),
                                    TargetReceiver::Fixed(r) => self.insert_receiver(r, n),
                                }
                                break Exit::Sent(
                                    self.activate_with(t.method, t.header, n, host, t.slot)?,
                                );
                            }
                        }
                        ip = next;
                    }
                    _ => break Exit::Step,
                }
            };
            match exit {
                Exit::Polls => {
                    self.frames.last_mut().expect("a frame").ip = ip;
                    return Ok(Step::Continue);
                }
                // Any other instruction, by `step`.
                Exit::Step => {
                    self.frames.last_mut().expect("a frame").ip = ip;
                    if let Step::Done(done) = self.step(host)? {
                        return Ok(Step::Done(done));
                    }
                }
                Exit::Sent(Some(done)) => return Ok(Step::Done(done)),
                Exit::Sent(None) => {}
            }
            // The event loop's checks for the next instruction: back to it
            // when it has something to do (no frame left, a collection or
            // checkpoint that can be due, the countdown's poll); otherwise
            // count the instruction down here and go on.
            if self.frames.is_empty()
                || self.heap.allocation_counter() >= self.safepoint_at
                || self.interrupt_countdown == 1
                || (self.jit_threshold > 0
                    && self.callback_depth == 0
                    && self.compiled_entry().is_some())
            {
                return Ok(Step::Continue);
            }
            self.interrupt_countdown -= 1;
            first = true;
        }
    }

    #[inline(always)]
    fn step(&mut self, host: &mut dyn Host) -> Result<Step, VmError> {
        let (ip, bytes) = {
            let f = self.frames.last().expect("a frame");
            let (code, ip) = (&f.code, f.ip);
            let Some(&first) = code.get(ip) else {
                return Err(self.invalid());
            };
            let len = instruction_length(first);
            if ip + len > code.len() {
                return Err(self.invalid());
            }
            let mut b = [0u8; 4];
            b[..len].copy_from_slice(&code[ip..ip + len]);
            (ip, b)
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
                    .literal((opcode & 7) as usize)
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
                let mixin = self.frames.last().expect("a frame").mixin;
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
            176..=207 if let Some(v) = self.small_integer_fast_path((opcode - 176) as usize) => {
                let len = self.stack.len();
                self.stack.truncate(len - 2);
                self.stack.push(v);
                None
            }
            176..=207 => self.common_send((opcode - 176) as usize, host)?,
            op::PUSH_NEW_ARRAY => {
                let nil = self.nil();
                let class = self.roles.get(Role::ArrayClass);
                let a = self.heap.allocate(
                    class,
                    Space::Actor(self.current as u32),
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
                let a = self.heap.allocate(
                    class,
                    Space::Actor(self.current as u32),
                    Body::Array(elements),
                );
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
            op::EVENTUAL_SEND => {
                // `r <-: sel args` is `messageLoop eventualSendTo: r
                // selector: sel arguments: {args}` (V5 spec, opcode 239).
                let index = bytes[1] as usize | (((bytes[2] & 15) as usize) << 8);
                let n = (bytes[2] >> 4) as usize;
                let selector = self.literal(index).ok_or_else(|| self.invalid())?;
                if self.stack.len() < self.frames.last().expect("a frame").lp + n + 1 {
                    return Err(self.invalid());
                }
                let args = self.stack.split_off(self.stack.len() - n);
                let receiver = self.pop()?;
                let array_class = self.roles.get(Role::ArrayClass);
                let array = self.heap.allocate(
                    array_class,
                    Space::Actor(self.current as u32),
                    Body::Array(args),
                );
                // The running actor's own loop.
                let message_loop = self
                    .actors
                    .get(self.current)
                    .map_or_else(|| self.roles.get(Role::MessageLoop), |a| a.message_loop);
                self.stack.extend([message_loop, receiver, selector, array]);
                let send = self.symbol("eventualSendTo:selector:arguments:");
                self.ordinary_send(send, 3, host)?
            }
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
                let frozen = value.is_some() && self.heap.flags(vector) & FROZEN != 0;
                let ok = match self.heap.array_elements_mut(vector) {
                    Some(elements) if index < elements.len() => match value {
                        None => {
                            let v = elements[index];
                            self.stack.push(v);
                            true
                        }
                        Some(_) if frozen => false,
                        Some(v) => {
                            elements[index] = v;
                            true
                        }
                    },
                    _ => false,
                };
                if !ok {
                    return Err(self.invalid());
                }
                None
            }
            op::PUSH_LITERAL => {
                let v = self.literal(wide()).ok_or_else(|| self.invalid())?;
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
                let code_len = self.frames.last().expect("a frame").code.len();
                if self.stack.len() - self.frames.last().expect("a frame").lp < copied_count
                    || next + length > code_len
                {
                    return Err(self.invalid());
                }
                let top = self.frames.len() - 1;
                let defining = self.ensure_activation(top);
                let class = self.roles.get(Role::ClosureClass);
                let from = self.stack.len() - copied_count;
                let closure = self.heap.allocate_closure(
                    class,
                    Space::Actor(self.current as u32),
                    defining,
                    next as u32,
                    num_args,
                    &self.stack[from..],
                );
                self.stack.truncate(from);
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

    /// A send of common selector `k` (SmallInteger fast paths first).
    fn common_send(
        &mut self,
        k: usize,
        host: &mut dyn Host,
    ) -> Result<Option<Completion>, VmError> {
        if let Some(v) = self.small_integer_fast_path(k) {
            let len = self.stack.len();
            self.stack.truncate(len - 2);
            self.stack.push(v);
            return Ok(None);
        }
        let table = self.roles.get(Role::CommonSelectors);
        let entries = self
            .heap
            .array_elements(table)
            .ok_or_else(|| VmError::Malformed("common selectors must be an Array"))?;
        let (Some(&selector), Some(&arity)) = (entries.get(2 * k), entries.get(2 * k + 1)) else {
            return Err(VmError::Malformed("common-selector table too short"));
        };
        let n = arity
            .as_small_integer()
            .and_then(|a| usize::try_from(a).ok())
            .ok_or_else(|| VmError::Malformed("common-selector arity"))?;
        if self.stack.len() - self.frames.last().expect("a frame").lp < n + 1 {
            return Err(self.invalid());
        }
        self.ordinary_send(selector, n, host)
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

impl HostCallbacks for Vm {
    /// A callback (design doc section 4). With a turn on the stack it is a
    /// sub-step of that turn, which only the expat's owning actor may run:
    /// from another actor's turn the call becomes a message to the owner and
    /// the host gets `undefined`. With no turn on the stack it starts a turn
    /// in the owning actor, whose message loop then drains the sends it
    /// queued.
    fn callback(
        &mut self,
        host: &mut dyn Host,
        expat: u32,
        args: &[HostValue],
    ) -> Result<HostValue, HostError> {
        let Some(&(closure, owner)) = self.expats.get(expat as usize) else {
            return Err(HostError::Dead);
        };
        if self.callback_depth >= MAX_CALLBACK_NESTING {
            return Err(HostError::Raised(String::from(
                "callbacks nested too deeply (exhausted: nesting)",
            )));
        }
        let in_turn = !self.frames.is_empty();
        let space = Space::Actor(owner);
        let arity = self
            .heap
            .closure(closure)
            .map_or(0, |c| c.num_args() as usize);
        let mut values: Vec<Value> = args
            .iter()
            .take(arity)
            .map(|a| self.marshal_in(a.clone(), space))
            .collect();
        values.resize(arity, self.nil());
        let array_class = self.roles.get(Role::ArrayClass);
        let arguments = self.heap.allocate(array_class, space, Body::Array(values));
        if in_turn && owner as usize != self.current {
            let selector = self.symbol("valueWithArguments:");
            let nil = self.nil();
            let packet = self.heap.allocate(
                array_class,
                space,
                Body::Array(alloc::vec![closure, selector, nil, arguments]),
            );
            self.actors[owner as usize].mailbox.push_back(packet);
            return Ok(HostValue::Undefined);
        }
        if !in_turn {
            self.current = owner as usize;
        }
        self.callback_depth += 1;
        let outcome = self.run_callback(closure, arguments, host);
        self.callback_depth -= 1;
        if !in_turn {
            self.actors[owner as usize].poked = true;
        }
        match outcome {
            Ok((true, result)) => self.marshal_out(result, 0).map_err(|_| {
                HostError::Raised(String::from(
                    "the callback answered an object that cannot cross to the host",
                ))
            }),
            Ok((false, description)) => {
                let text = self
                    .heap
                    .string_bytes(description)
                    .and_then(|b| core::str::from_utf8(b).ok())
                    .unwrap_or("an exception");
                Err(HostError::Raised(alloc::format!("callback failed: {text}")))
            }
            Err(e) => Err(HostError::Raised(alloc::format!("callback failed: {e}"))),
        }
    }
}
