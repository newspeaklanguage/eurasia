// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The baseline compiler: V5 bytecode to a WebAssembly module, one per method
//! (newspeak/EURASIA_COMPILER_DESIGN_2026-11-19.md).
//!
//! Compiled code keeps the interpreter's frames: it runs on the VM's operand
//! stack in linear memory, and every call into the VM's runtime, and every
//! exit, leaves the frame as the interpreter would have it (its instruction
//! index, its stack height), so the interpreter can continue it at any of
//! those points, and compiled code can take it back at any of its *entries*:
//! the starts of its basic blocks. A block starts at the method's start, at
//! each closure body's start (a closure's body is inline in its method's
//! bytecode, so one function runs the method and its closures), at each jump
//! target and after each call into the runtime.
//!
//! Every instruction compiles: the common ones inline, the others as a call
//! to the runtime's `step`, which runs that one instruction as the
//! interpreter does; compiled code goes on after it when the frame is still
//! on top and was not taken over.
//!
//! The module imports the VM's memory and function table and these runtime
//! functions (exported by the web host as `eurasia_rt_*`), each taking the
//! VM's address first:
//! - `send(vm, rule, literal, args, next_ip, sp) -> i32`: makes a send as the
//!   interpreter does (for [`RULE_COMMON`], `literal` is the common
//!   selector's number); answers [`DONE`] when it completed (its result on
//!   the stack), [`INTERPRET`], [`STOP`], or [`CALLEE`] plus the function
//!   index of a compiled callee whose frame it pushed;
//! - `step(vm, ip, sp) -> i32`: runs the instruction at `ip`; answers as
//!   `send` does, or [`RETURNED`] when the frame returned;
//! - `ret(vm, value, sp) -> i32`: the interpreter's local return;
//! - `interpret(vm, ip, sp) -> i32`: leaves the frame at instruction `ip` for
//!   the interpreter;
//! - `tick(vm, ip, sp) -> i32`: counts [`TICK`] backward jumps down against
//!   the interrupt poll; [`DONE`], or [`INTERPRET`] with the frame at `ip`.
//!
//! Everything else compiled code reads from the VM's *registers*
//! ([`Regs`]), which the VM sets before it enters compiled code and after
//! every call that answers [`DONE`], [`RETURNED`] or [`CALLEE`]: the operand
//! stack's address and height, the top frame's layout, receiver and literals,
//! nil, true and false, whether a safepoint is due, and a callee's entry.
//!
//! The function it exports, `run(vm, regs, entry) -> i32`, runs the top frame
//! from its block `entry` and answers [`DONE`] (the frame returned),
//! [`INTERPRET`] or [`STOP`].

use alloc::vec::Vec;

use crate::bytecode::{instruction_length, op};

/// The frame returned; its result is on the caller's stack. (From `send`
/// and `step`: the instruction completed.)
pub const DONE: i32 = 0;
/// The interpreter continues the top frame.
pub const INTERPRET: i32 = 1;
/// The run stops: the VM holds the completion or the error.
pub const STOP: i32 = 2;
/// Added to a function index: a compiled callee's frame was pushed.
pub const CALLEE: i32 = 3;
/// From `step`: the frame returned (a `^` in a method).
pub const RETURNED: i32 = -1;
/// From `step` or a callee: a non-local return unwound to the frame of
/// depth `Regs::depth`, which goes on as after a completed send if it is
/// the caller's; others pass it on.
pub const UNWOUND: i32 = -2;
/// From `call`: the cached callee cannot be entered now; the site makes the
/// general send.
pub const MISS: i32 = -3;
/// Backward jumps between two calls of `tick`.
pub const TICK: i32 = 64;
/// The operand-stack room the VM keeps for compiled code (in Values); a
/// method that could need more is not compiled.
pub const HEADROOM: usize = 2048;

/// The VM's registers for compiled code, at an address that does not change
/// while it runs. Addresses are wasm32's.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Regs {
    /// The operand stack's address, with [`HEADROOM`] Values of room.
    pub base: u32,
    /// The stack's height.
    pub sp: u32,
    /// The top frame's receiver index, first local and argument count.
    pub bp: u32,
    /// See `bp`.
    pub lp: u32,
    /// See `bp`.
    pub argc: u32,
    /// The entry block of a [`CALLEE`].
    pub block: u32,
    /// Whether a safepoint is due (0 or 1).
    pub due: u32,
    /// The address of the top frame's first literal, and how many there are.
    pub literals: u32,
    /// See `literals`.
    pub literal_count: u32,
    /// The heap arena's address: object `v` starts at `arena + (v >> 1) * 8`.
    pub arena: u32,
    /// The nursery's start, in words: older objects need the write barrier.
    pub boundary: u32,
    /// The lookup cache's epoch, changed at every flush (collections
    /// included): an inline cache filled in another epoch is empty.
    pub epoch: u32,
    /// After a send that hit an accessor an inline cache can hold: 1, and
    /// its slot and class.
    pub ic_kind: u32,
    /// See `ic_kind`.
    pub ic_slot: u32,
    /// The number of frames (the top frame's depth).
    pub depth: u32,
    /// The top frame's receiver.
    pub receiver: u64,
    /// nil, true and false.
    pub nil: u64,
    /// See `nil`.
    pub true_: u64,
    /// See `nil`.
    pub false_: u64,
    /// See `ic_kind`.
    pub ic_class: u64,
    /// The byte store's address: a byte object's bytes start at its first
    /// slot (a SmallInteger offset), its size in its meta word.
    pub bytes: u32,
    /// The SmallInteger class, which inline caches hold for SmallInteger
    /// receivers.
    pub small_integer: u64,
    /// See `ic_kind`: for [`IC_OUTER_GETTER`], the object whose slot the
    /// send reads.
    pub ic_object: u64,
    /// The next frame serial: the VM and compiled code take them here.
    pub serial: u64,
    /// The frames' address (each [`FRAME_SIZE`] bytes), their count and
    /// the room for them: compiled code pushes and pops frames itself
    /// within it; the host adopts the count at every call into the VM.
    pub frames: u32,
    /// See `frames`.
    pub frames_len: u32,
    /// See `frames`.
    pub frames_cap: u32,
    /// The inline-cache targets' address (each [`TARGET_SIZE`] bytes).
    pub targets: u32,
    /// The operand stack's room, in Values.
    pub stack_cap: u32,
    /// The frame count when the host last entered compiled code.
    pub jit_base: u32,
    /// Whether a step limit is set (1): compiled code then pushes no frame.
    pub stepping: u32,
    /// The closure targets' address (each [`CTARGET_SIZE`] bytes).
    pub closure_targets: u32,
    /// The arena's length and room, in words: compiled code allocates
    /// within it; the host adopts the length at every call into the VM.
    pub words_len: u32,
    /// See `words_len`.
    pub words_cap: u32,
    /// The allocation counter, and the count at which a safepoint is due:
    /// compiled code allocates only below it, and counts here.
    pub seq: u64,
    /// See `seq`.
    pub safepoint_at: u64,
    /// The Closure and Activation classes, and the meta words of a closure
    /// of no fields and an activation, in the current actor's space.
    pub closure_class: u64,
    /// See `closure_class`.
    pub activation_class: u64,
    /// See `closure_class`.
    pub closure_meta: u64,
    /// See `closure_class`.
    pub activation_meta: u64,
    /// The objects compiled code allocated since the VM last published.
    pub allocated: u32,
    /// The current actor's space, in a meta word's bits.
    pub space: u64,
    /// The Float class.
    pub float_class: u64,
}

const _: () = {
    use core::mem::offset_of;
    assert!(offset_of!(Regs, bytes) == R_BYTES as usize);
    assert!(offset_of!(Regs, small_integer) == R_SMALL_INTEGER as usize);
    assert!(offset_of!(Regs, ic_object) == R_IC_OBJECT as usize);
    assert!(offset_of!(Regs, serial) == R_SERIAL as usize);
    assert!(offset_of!(Regs, frames) == R_FRAMES as usize);
    assert!(offset_of!(Regs, frames_len) == R_FRAMES_LEN as usize);
    assert!(offset_of!(Regs, frames_cap) == R_FRAMES_CAP as usize);
    assert!(offset_of!(Regs, targets) == R_TARGETS as usize);
    assert!(offset_of!(Regs, stack_cap) == R_STACK_CAP as usize);
    assert!(offset_of!(Regs, jit_base) == R_JIT_BASE as usize);
    assert!(offset_of!(Regs, stepping) == R_STEPPING as usize);
    assert!(offset_of!(Regs, closure_targets) == R_CLOSURE_TARGETS as usize);
    assert!(offset_of!(Regs, words_len) == R_WORDS_LEN as usize);
    assert!(offset_of!(Regs, words_cap) == R_WORDS_CAP as usize);
    assert!(offset_of!(Regs, seq) == R_SEQ as usize);
    assert!(offset_of!(Regs, safepoint_at) == R_SAFEPOINT_AT as usize);
    assert!(offset_of!(Regs, closure_class) == R_CLOSURE_CLASS as usize);
    assert!(offset_of!(Regs, activation_class) == R_ACTIVATION_CLASS as usize);
    assert!(offset_of!(Regs, closure_meta) == R_CLOSURE_META as usize);
    assert!(offset_of!(Regs, activation_meta) == R_ACTIVATION_META as usize);
    assert!(offset_of!(Regs, allocated) == R_ALLOCATED as usize);
    assert!(offset_of!(Regs, space) == R_SPACE as usize);
    assert!(offset_of!(Regs, float_class) == R_FLOAT_CLASS as usize);
};

/// A frame's layout on wasm32 (the interpreter's `Frame`, checked there):
/// its size and its fields' offsets.
pub const FRAME_SIZE: u32 = 80;
/// See [`FRAME_SIZE`].
pub const FRAME_SERIAL: u32 = 0;
/// See [`FRAME_SIZE`].
pub const FRAME_METHOD: u32 = 8;
/// See [`FRAME_SIZE`].
pub const FRAME_LITERALS: u32 = 16;
/// See [`FRAME_SIZE`].
pub const FRAME_MIXIN: u32 = 24;
/// See [`FRAME_SIZE`].
pub const FRAME_RECEIVER: u32 = 32;
/// See [`FRAME_SIZE`].
pub const FRAME_ACTIVATION: u32 = 40;
/// See [`FRAME_SIZE`].
pub const FRAME_CLOSURE: u32 = 48;
/// See [`FRAME_SIZE`].
pub const FRAME_CODE: u32 = 56;
/// See [`FRAME_SIZE`].
pub const FRAME_BP: u32 = 60;
/// See [`FRAME_SIZE`].
pub const FRAME_ARG_COUNT: u32 = 64;
/// See [`FRAME_SIZE`].
pub const FRAME_LP: u32 = 68;
/// See [`FRAME_SIZE`].
pub const FRAME_IP: u32 = 72;
/// An inline-cache target's layout (the interpreter's `TargetEntry`,
/// checked there): its size and its fields' offsets.
pub const TARGET_SIZE: u32 = 56;
/// See [`TARGET_SIZE`].
pub const TARGET_METHOD: u32 = 0;
/// See [`TARGET_SIZE`].
pub const TARGET_LITERALS: u32 = 8;
/// See [`TARGET_SIZE`].
pub const TARGET_MIXIN: u32 = 16;
/// See [`TARGET_SIZE`].
pub const TARGET_FIXED: u32 = 24;
/// See [`TARGET_SIZE`].
pub const TARGET_FUNCTION: u32 = 32;
/// See [`TARGET_SIZE`].
pub const TARGET_LITERAL_OFFSET: u32 = 36;
/// See [`TARGET_SIZE`].
pub const TARGET_LITERAL_COUNT: u32 = 40;
/// See [`TARGET_SIZE`].
pub const TARGET_LOCALS: u32 = 44;
/// See [`TARGET_SIZE`].
pub const TARGET_RECEIVER_KIND: u32 = 52;
/// A closure target's layout (the interpreter's `ClosureTarget`, checked
/// there): its size and its fields' offsets.
pub const CTARGET_SIZE: u32 = 56;
/// See [`CTARGET_SIZE`].
pub const CTARGET_METHOD: u32 = 0;
/// See [`CTARGET_SIZE`].
pub const CTARGET_LITERALS: u32 = 8;
/// See [`CTARGET_SIZE`].
pub const CTARGET_MIXIN: u32 = 16;
/// See [`CTARGET_SIZE`].
pub const CTARGET_INITIAL_IP: u32 = 24;
/// See [`CTARGET_SIZE`].
pub const CTARGET_FUNCTION: u32 = 32;
/// See [`CTARGET_SIZE`].
pub const CTARGET_BLOCK: u32 = 36;
/// See [`CTARGET_SIZE`].
pub const CTARGET_LITERAL_OFFSET: u32 = 40;
/// See [`CTARGET_SIZE`].
pub const CTARGET_LITERAL_COUNT: u32 = 44;
/// See [`CTARGET_SIZE`].
pub const CTARGET_ARGS: u32 = 48;
// A closure's contents (after the header): its defining activation, its
// body's first instruction and argument count (SmallIntegers), then the
// copied values; an activation's: its frame, method and receiver first.
const CLOSURE_DEFINING: u32 = SLOTS;
const CLOSURE_INITIAL_IP: u32 = SLOTS + 8;
const CLOSURE_NUM_ARGS: u32 = SLOTS + 16;
const CLOSURE_COPIED: u32 = SLOTS + 24;
const ACTIVATION_FRAME: u32 = SLOTS;
const ACTIVATION_METHOD: u32 = SLOTS + 8;
const ACTIVATION_RECEIVER: u32 = SLOTS + 16;
/// Frames at most (the interpreter's limit).
const MAX_FRAMES: u32 = crate::interpreter::MAX_FRAMES as u32;
/// Compiled callees nested in the host's stack, at most (the VM's limit).
const MAX_NESTED: u32 = 512;

// The registers' offsets.
const R_BASE: u32 = 0;
const R_SP: u32 = 4;
const R_BP: u32 = 8;
const R_LP: u32 = 12;
const R_ARGC: u32 = 16;
const R_BLOCK: u32 = 20;
const R_DUE: u32 = 24;
const R_LITERALS: u32 = 28;
const R_LITERAL_COUNT: u32 = 32;
const R_ARENA: u32 = 36;
const R_BOUNDARY: u32 = 40;
const R_EPOCH: u32 = 44;
const R_IC_KIND: u32 = 48;
const R_IC_SLOT: u32 = 52;
const R_DEPTH: u32 = 56;
const R_RECEIVER: u32 = 64;
const R_NIL: u32 = 72;
const R_TRUE: u32 = 80;
const R_FALSE: u32 = 88;
const R_IC_CLASS: u32 = 96;
const R_BYTES: u32 = 104;
const R_SMALL_INTEGER: u32 = 112;
const R_IC_OBJECT: u32 = 120;
const R_SERIAL: u32 = 128;
const R_FRAMES: u32 = 136;
const R_FRAMES_LEN: u32 = 140;
const R_FRAMES_CAP: u32 = 144;
const R_TARGETS: u32 = 148;
const R_STACK_CAP: u32 = 152;
const R_JIT_BASE: u32 = 156;
const R_STEPPING: u32 = 160;
const R_CLOSURE_TARGETS: u32 = 164;
const R_WORDS_LEN: u32 = 168;
const R_WORDS_CAP: u32 = 172;
const R_SEQ: u32 = 176;
const R_SAFEPOINT_AT: u32 = 184;
const R_CLOSURE_CLASS: u32 = 192;
const R_ACTIVATION_CLASS: u32 = 200;
const R_CLOSURE_META: u32 = 208;
const R_ACTIVATION_META: u32 = 216;
const R_ALLOCATED: u32 = 224;
const R_SPACE: u32 = 232;
const R_FLOAT_CLASS: u32 = 240;

// The object header (newspeak/EURASIA_ARENA_DESIGN): class, sequence number,
// meta (format 5 bits, flags 5, space 21, remembered 1, size 32); then the
// slots.
const META: u32 = 16;
const SLOTS: u32 = 24;
const META_FLAGS_FROZEN_OR_VALUE: i64 = (1 | 2) << 5;
const META_REMEMBERED: i64 = 1 << 31;

/// The globals of an inline-cache entry: the receiver's class (i64), the
/// epoch, the slot (an accessor's) or target (a method's, the VM's index),
/// the kind ([`IC_ACCESSOR`] or [`IC_METHOD`] and the others; 0 empty), and
/// an object (i64; [`IC_OUTER_GETTER`]'s).
const ENTRY_GLOBALS: u32 = 5;
/// The globals of an inline-cache site: two entries, for two classes.
const SITE_GLOBALS: u32 = 2 * ENTRY_GLOBALS;
/// An inline cache of an accessor, read or written in compiled code.
pub const IC_ACCESSOR: u32 = 1;
/// An inline cache of a compiled method, entered through `call`.
pub const IC_METHOD: u32 = 2;
/// Inline caches of the indexed natives, run in compiled code: `size` (of
/// an Array, String or ByteArray), `at:` and `at:put:` (of an Array), and
/// `at:` of a String or ByteArray ([`IC_BYTES_AT`]), by the site's argument
/// count.
pub const IC_SIZE: u32 = 3;
/// See [`IC_SIZE`].
pub const IC_AT: u32 = 4;
/// See [`IC_SIZE`].
pub const IC_AT_PUT: u32 = 5;
/// See [`IC_SIZE`].
pub const IC_BYTES_AT: u32 = 6;
/// An inline cache of `==` (`Object>>==`), compared in compiled code.
pub const IC_IDENTICAL: u32 = 7;
/// Inline caches of SmallInteger's `bitAnd:`, `bitOr:` and `bitXor:` (with
/// a SmallInteger argument), computed in compiled code.
pub const IC_BIT_AND: u32 = 8;
/// See [`IC_BIT_AND`].
pub const IC_BIT_OR: u32 = 9;
/// See [`IC_BIT_AND`].
pub const IC_BIT_XOR: u32 = 10;
/// An inline cache of an implicit-receiver or outer send that reads a slot of an
/// enclosing object (the site's object), read in compiled code.
pub const IC_OUTER_GETTER: u32 = 11;
/// An inline cache of a method that answers its receiver, answered in
/// compiled code.
pub const IC_RETURN_SELF: u32 = 12;
/// An inline cache of a closure's `value` send (an ordinary send to a
/// closure): the slot is a closure target, the VM's index; closures with
/// that target's body have their frames pushed in compiled code.
pub const IC_CLOSURE: u32 = 13;
/// The function of a method target that is a native's: `call` runs it;
/// compiled code pushes no frame for it.
pub const NATIVE_FUNCTION: u32 = u32::MAX;
/// An inline cache of `basicNew` making a Regular object: the slot is its
/// slot count, the object its meta word but for the space; made in
/// compiled code.
pub const IC_BASIC_NEW: u32 = 14;
/// An inline cache of `new:` sent to the Array class: the object is an
/// Array's meta word but for the space and size; made in compiled code.
pub const IC_NEW_ARRAY: u32 = 15;
/// Inline caches of the comparisons `<`, `>`, `<=` and `>=` of numbers
/// (`Number`'s natives), run in compiled code when receiver and argument
/// are SmallIntegers or Floats.
pub const IC_LT: u32 = 16;
/// See [`IC_LT`].
pub const IC_GT: u32 = 17;
/// See [`IC_LT`].
pub const IC_LE: u32 = 18;
/// See [`IC_LT`].
pub const IC_GE: u32 = 19;
/// The largest Array `new:` makes in compiled code.
const MAX_INLINE_ARRAY: i64 = 1 << 16;
/// The format code of an Array.
const CODE_ARRAY: i64 = 3;

/// The send rules `send` takes.
pub const RULE_ORDINARY: i32 = 0;
/// A self send.
pub const RULE_SELF: i32 = 1;
/// A super send.
pub const RULE_SUPER: i32 = 2;
/// An implicit-receiver send.
pub const RULE_IMPLICIT: i32 = 3;
/// A common-selector send.
pub const RULE_COMMON: i32 = 4;
/// An outer send `d` levels out is rule `RULE_OUTER + d`.
pub const RULE_OUTER: i32 = 5;

// Imported functions, in import order.
const F_SEND: u32 = 0;
const F_RET: u32 = 1;
const F_INTERPRET: u32 = 2;
const F_STEP: u32 = 3;
const F_TICK: u32 = 4;
const F_CALL: u32 = 5;
const F_CLOSURE: u32 = 6;
const F_NLR: u32 = 7;
const IMPORTED: u32 = 8;

// Types.
const T_RUN: u32 = 0; // (i32, i32, i32) -> i32: run, interpret, step, tick
const T_SEND: u32 = 1; // (i32 x 6) -> i32
const T_RET: u32 = 2; // (i32, i64, i32) -> i32
const T_CALL: u32 = 3; // (i32 x 4) -> i32
const T_NLR: u32 = 4; // (i32, i64, i32, i32) -> i32

// Locals of `run`: the parameters, then i32s, then i64s.
const L_VM: u32 = 0;
const L_REGS: u32 = 1;
const L_ENTRY: u32 = 2;
const L_BASE: u32 = 3;
const L_SP: u32 = 4;
const L_BP: u32 = 5;
const L_LP: u32 = 6;
const L_ARGC: u32 = 7;
const L_PC: u32 = 8;
const L_R: u32 = 9;
const L_COUNT: u32 = 10;
/// An object's index (in words) and address.
const L_INDEX: u32 = 11;
const L_ADDR: u32 = 12;
/// The frame's depth, to recognize an [`UNWOUND`] to it.
const L_DEPTH: u32 = 13;
const I32_LOCALS: u8 = 11;
const L_A: u32 = 14;
const L_B: u32 = 15;
const L_SELF: u32 = 16;
const L_NIL: u32 = 17;
const L_TRUE: u32 = 18;
const L_FALSE: u32 = 19;
/// The scratch i64 for arithmetic results.
const L_R64: u32 = 20;
const L_META: u32 = 21;
const I64_LOCALS: u8 = 8;
/// The literals: the first's offset in the arena (in bytes; the arena can
/// move at allocations, but the literal array only at collections, after
/// which compiled code leaves its frame), and their count. More i32s.
const L_LITERALS: u32 = 22;
const L_LITERAL_COUNT: u32 = 23;
/// The inline-cache entry a send hit: its kind (0 on a miss) and slot, more
/// i32s; its object, an i64 of its own.
const L_KIND: u32 = 24;
const L_SLOT: u32 = 25;
const MORE_I32_LOCALS: u8 = 4;
const L_OBJECT: u32 = 26;

/// One decoded instruction.
#[derive(Clone, Copy, Debug)]
enum Ins {
    PushParam(u32),
    PushLocal(u32),
    PopIntoLocal(u32),
    StoreIntoLocal(u32),
    PushNil,
    PushTrue,
    PushFalse,
    PushSelf,
    PushSmall(i64),
    PushLiteral(u32),
    Pop,
    Dup,
    /// To the instruction at this index.
    Jump(usize),
    /// Pop; to the target if the value is the Boolean.
    Branch(bool, usize),
    /// Common selector `k` (one of the inline ones) on SmallIntegers.
    Arith(u8),
    Send {
        rule: i32,
        literal: u32,
        args: u32,
    },
    Return(Ret),
    /// Push, pop into or store into element `index` of the indirection
    /// vector in local `local`.
    Indirect {
        op: u8,
        index: u32,
        local: u32,
    },
    /// Any other instruction, by the runtime's `step`; compiled code goes on
    /// at the given index.
    Step(usize),
    /// A closure push, by the runtime's `closure`; compiled code goes on at
    /// the given index, past the closure's body.
    Closure(usize),
    /// A `^` in a closure body: a non-local return, by the runtime's `nlr`.
    NonLocalReturn(Ret),
}

#[derive(Clone, Copy, Debug)]
enum Ret {
    Top,
    Self_,
    Nil,
    True,
    False,
}

/// The common selectors compiled inline: `+ - < > <= >= = & |`.
fn inline_arith(k: u8) -> bool {
    matches!(k, 0 | 1 | 7..=13)
}

/// Decodes `code` into (index, length, instruction), and the closure bodies'
/// starts; `None` if it is malformed.
#[allow(clippy::type_complexity)]
fn decode(code: &[u8]) -> Option<(Vec<(usize, usize, Ins)>, Vec<usize>)> {
    let mut out = Vec::new();
    let mut closures = Vec::new();
    let mut bodies: Vec<core::ops::Range<usize>> = Vec::new();
    let mut ip = 0;
    while ip < code.len() {
        let o = code[ip];
        let len = instruction_length(o);
        if ip + len > code.len() {
            return None;
        }
        let b1 = if len > 1 { code[ip + 1] } else { 0 };
        let b2 = if len > 2 { code[ip + 2] } else { 0 };
        let b3 = if len > 3 { code[ip + 3] } else { 0 };
        let wide = u16::from_le_bytes([b1, b2]) as usize;
        let next = ip + len;
        let ext_sel = u32::from(b1) | (u32::from(b2 & 0x0f) << 8);
        let ext_args = u32::from(b2 >> 4);
        let ins = match o {
            0..=15 => Ins::Jump(next.checked_sub((o & 15) as usize)?),
            16..=31 => Ins::Jump(next + (o & 15) as usize),
            32..=47 => Ins::Branch(true, next + (o & 15) as usize),
            48..=63 => Ins::Branch(false, next + (o & 15) as usize),
            64..=111 => Ins::Send {
                rule: match o {
                    64..=79 => RULE_ORDINARY,
                    80..=95 => RULE_SELF,
                    _ => RULE_IMPLICIT,
                },
                literal: u32::from(o & 7),
                args: u32::from((o >> 3) & 1),
            },
            112..=119 => Ins::PushParam(u32::from(o & 7)),
            120..=127 => Ins::PushLocal(u32::from(o & 7)),
            128..=135 => Ins::PopIntoLocal(u32::from(o & 7)),
            136..=143 => Ins::StoreIntoLocal(u32::from(o & 7)),
            144..=151 => Ins::PushLiteral(u32::from(o & 7)),
            op::PUSH_NIL => Ins::PushNil,
            op::PUSH_FALSE => Ins::PushFalse,
            op::PUSH_TRUE => Ins::PushTrue,
            op::PUSH_SELF => Ins::PushSelf,
            op::POP => Ins::Pop,
            op::DUP => Ins::Dup,
            160..=163 => Ins::PushSmall(i64::from(o) - 161),
            op::RETURN_NIL => Ins::Return(Ret::Nil),
            op::RETURN_FALSE => Ins::Return(Ret::False),
            op::RETURN_TRUE => Ins::Return(Ret::True),
            op::RETURN_SELF => Ins::Return(Ret::Self_),
            op::RETURN_TOP => Ins::Return(Ret::Top),
            176..=207 if inline_arith(o - 176) => Ins::Arith(o - 176),
            176..=207 => Ins::Send {
                rule: RULE_COMMON,
                literal: u32::from(o - 176),
                args: u32::from(crate::bytecode::COMMON_SELECTORS[usize::from(o - 176)].1),
            },
            op::PUSH_PARAMETER => Ins::PushParam(u32::from(b1)),
            op::PUSH_LOCAL => Ins::PushLocal(u32::from(b1)),
            op::POP_INTO_LOCAL => Ins::PopIntoLocal(u32::from(b1)),
            op::STORE_INTO_LOCAL => Ins::StoreIntoLocal(u32::from(b1)),
            op::BRANCH_BACK => Ins::Jump(next.checked_sub(wide)?),
            op::BRANCH => Ins::Jump(next + wide),
            op::BRANCH_IF_TRUE => Ins::Branch(true, next + wide),
            op::BRANCH_IF_FALSE => Ins::Branch(false, next + wide),
            op::PUSH_LITERAL => Ins::PushLiteral(wide as u32),
            op::PUSH_INTEGER => Ins::PushSmall(i64::from(i16::from_le_bytes([b1, b2]))),
            op::ORDINARY_SEND => Ins::Send {
                rule: RULE_ORDINARY,
                literal: ext_sel,
                args: ext_args,
            },
            op::SELF_SEND => Ins::Send {
                rule: RULE_SELF,
                literal: ext_sel,
                args: ext_args,
            },
            op::IMPLICIT_RECEIVER_SEND => Ins::Send {
                rule: RULE_IMPLICIT,
                literal: ext_sel,
                args: ext_args,
            },
            op::OUTER_SEND if i32::from(b3) <= i32::MAX - RULE_OUTER => Ins::Send {
                rule: RULE_OUTER + i32::from(b3),
                literal: ext_sel,
                args: ext_args,
            },
            op::SUPER_SEND => Ins::Send {
                rule: RULE_SUPER,
                literal: ext_sel,
                args: ext_args,
            },
            op::PUSH_INDIRECT_LOCAL
            | op::POP_INTO_INDIRECT_LOCAL
            | op::STORE_INTO_INDIRECT_LOCAL => Ins::Indirect {
                op: o,
                index: u32::from(b1),
                local: u32::from(b2),
            },
            op::PUSH_CLOSURE => {
                // The body follows, `length` bytes; the method goes on after
                // it.
                let length = u16::from_le_bytes([b2, b3]) as usize;
                if next + length > code.len() {
                    return None;
                }
                closures.push(next);
                bodies.push(next..next + length);
                Ins::Closure(next + length)
            }
            // Everything else, the interpreter's errors included.
            _ => Ins::Step(next),
        };
        out.push((ip, len, ins));
        ip = next;
    }
    // A `^` outside every closure body (in an inlined block, say) is in a
    // method's frame, where it is a local return; inside one, a non-local
    // return.
    for entry in &mut out {
        let o = code[entry.0];
        if (171..=175).contains(&o) {
            let r = match o {
                171 => Ret::Nil,
                172 => Ret::False,
                173 => Ret::True,
                174 => Ret::Self_,
                _ => Ret::Top,
            };
            entry.2 = if bodies.iter().any(|b| b.contains(&entry.0)) {
                Ins::NonLocalReturn(r)
            } else {
                Ins::Return(r)
            };
        }
    }
    Some((out, closures))
}

/// A compiled method: its module, and the instruction indices at which its
/// function can be entered (sorted; entry `k` is block `k`).
pub struct Compiled {
    /// The WebAssembly module.
    pub module: Vec<u8>,
    /// The entries.
    pub entries: Vec<u32>,
}

/// Compiles `code` (a method's bytecode) to a module, or answers `None` if
/// it is malformed. `arith_ok(k)` says whether the inline SmallInteger
/// meaning of common selector `k` is SmallInteger's method (as the
/// interpreter's fast path checks); where it is not, the instruction is a
/// send, by `step`.
pub fn compile(code: &[u8], arith_ok: &dyn Fn(usize) -> bool) -> Option<Compiled> {
    let (mut ins, closures) = decode(code)?;
    for entry in &mut ins {
        match entry.2 {
            Ins::Arith(k) if !arith_ok(usize::from(k)) => {
                entry.2 = Ins::Send {
                    rule: RULE_COMMON,
                    literal: u32::from(k),
                    args: 1,
                }
            }
            // Parameters are numbered from 1; others are the interpreter's
            // error. (A closure's parameters are numbered as its method's
            // are, within the closure's own count, which the interpreter
            // checks.)
            Ins::PushParam(0) => entry.2 = Ins::Step(entry.0 + entry.1),
            _ => {}
        }
    }
    // Basic blocks: their first instructions' indices.
    let mut leaders: Vec<usize> = alloc::vec![0];
    leaders.extend_from_slice(&closures);
    for &(ip, len, i) in &ins {
        let next = ip + len;
        match i {
            Ins::Jump(t) | Ins::Branch(_, t) => {
                leaders.push(t);
                leaders.push(next);
            }
            Ins::Return(_) | Ins::NonLocalReturn(_) => leaders.push(next),
            Ins::Send { .. } | Ins::Arith(_) | Ins::Indirect { .. } => leaders.push(next),
            Ins::Step(after) | Ins::Closure(after) => {
                leaders.push(after);
                leaders.push(next);
            }
            _ => {}
        }
    }
    leaders.retain(|&l| l < code.len());
    leaders.sort_unstable();
    leaders.dedup();
    // Every block must start at an instruction.
    for &l in &leaders {
        if ins.binary_search_by_key(&l, |&(ip, _, _)| ip).is_err() {
            return None;
        }
    }
    let block_of = |ip: usize| leaders.binary_search(&ip).ok();
    // Room on the operand stack: no more than every pushing instruction.
    if ins.len() + 8 > HEADROOM {
        return None;
    }

    let mut f = Func::default();
    // Entry: the frame's layout and the constants, from the registers.
    for (local, offset) in [
        (L_BASE, R_BASE),
        (L_SP, R_SP),
        (L_BP, R_BP),
        (L_LP, R_LP),
        (L_ARGC, R_ARGC),
        (L_DEPTH, R_DEPTH),
    ] {
        f.reg32(offset).set(local);
    }
    for (local, offset) in [
        (L_SELF, R_RECEIVER),
        (L_NIL, R_NIL),
        (L_TRUE, R_TRUE),
        (L_FALSE, R_FALSE),
    ] {
        f.reg64(offset).set(local);
    }
    f.reg32(R_LITERALS).reg32(R_ARENA).sub().set(L_LITERALS);
    f.reg32(R_LITERAL_COUNT).set(L_LITERAL_COUNT);
    f.get(L_ENTRY).set(L_PC);
    f.i32c(TICK).set(L_COUNT);
    // loop $dispatch; block x n; br_table; then each block's code after
    // its `end`.
    let n = leaders.len() as u32;
    f.op(0x03).op(0x40); // loop
    for _ in 0..n {
        f.op(0x02).op(0x40); // block
    }
    f.get(L_PC).op(0x0e).uleb(n - 1);
    for k in 0..n - 1 {
        f.uleb(k);
    }
    f.uleb(n - 1); // default
    let mut k = 0usize; // the block being emitted
    for (index, &(ip, len, i)) in ins.iter().enumerate() {
        if block_of(ip) == Some(k) {
            f.op(0x0b); // end of block k: its code follows
        }
        let next = ip + len;
        // The branch depth to the dispatch loop from block k's code: the
        // blocks k+1 .. n-1 still enclose it.
        let to_loop = n - 1 - k as u32;
        // `extra`: the `if`s around the jump.
        let jump = |f: &mut Func, target: usize, extra: u32| {
            f.i32c(block_of(target).expect("a leader") as i32).set(L_PC);
            f.op(0x0c).uleb(to_loop + extra); // br $dispatch
        };
        match i {
            Ins::PushParam(p) => {
                // stack[bp + 1 + (argc - p)]; beyond the count, the
                // interpreter's error.
                f.get(L_ARGC).i32c(p as i32).op(0x48); // argc < p
                f.op(0x04).op(0x40);
                f.interpret(ip);
                f.op(0x0b);
                f.push_addr();
                f.slot_addr(|f| {
                    f.get(L_BP)
                        .i32c(1)
                        .add()
                        .get(L_ARGC)
                        .add()
                        .i32c(p as i32)
                        .sub();
                });
                f.load().store().inc_sp();
            }
            Ins::PushLocal(i) => {
                f.push_addr();
                f.slot_addr(|f| {
                    f.get(L_LP).i32c(i as i32).add();
                });
                f.load().store().inc_sp();
            }
            Ins::PopIntoLocal(i) => {
                f.dec_sp();
                f.slot_addr(|f| {
                    f.get(L_LP).i32c(i as i32).add();
                });
                f.top_value_at_sp();
                f.store();
            }
            Ins::StoreIntoLocal(i) => {
                f.slot_addr(|f| {
                    f.get(L_LP).i32c(i as i32).add();
                });
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                f.load().store();
            }
            Ins::PushNil => {
                f.push_addr().get(L_NIL).store().inc_sp();
            }
            Ins::PushTrue => {
                f.push_addr().get(L_TRUE).store().inc_sp();
            }
            Ins::PushFalse => {
                f.push_addr().get(L_FALSE).store().inc_sp();
            }
            Ins::PushSelf => {
                f.push_addr().get(L_SELF).store().inc_sp();
            }
            Ins::PushSmall(v) => {
                f.push_addr().i64c(v << 1).store().inc_sp();
            }
            Ins::PushLiteral(i) => {
                // At the arena's address plus the literals' offset (the
                // arena can move at allocations, which are calls); beyond
                // the count, the interpreter's error.
                f.get(L_LITERAL_COUNT).i32c(i as i32).op(0x4d); // count <= i
                f.op(0x04).op(0x40);
                f.step(ip);
                f.leave_after_step();
                f.op(0x0b);
                f.push_addr();
                f.reg32(R_ARENA)
                    .get(L_LITERALS)
                    .add()
                    .i32c(8 * i as i32)
                    .add()
                    .load();
                f.store().inc_sp();
            }
            Ins::Pop => {
                f.dec_sp();
            }
            Ins::Dup => {
                f.push_addr();
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                f.load().store().inc_sp();
            }
            Ins::Jump(t) => {
                if t <= ip {
                    // A backward jump: the interrupt poll, every TICK.
                    f.get(L_COUNT).i32c(1).sub().set(L_COUNT);
                    f.get(L_COUNT).op(0x45); // i32.eqz
                    f.op(0x04).op(0x40);
                    f.i32c(TICK).set(L_COUNT);
                    f.get(L_VM).i32c(t as i32).get(L_SP).call(F_TICK).set(L_R);
                    f.get(L_R).op(0x04).op(0x40);
                    f.get(L_R).op(0x0f);
                    f.op(0x0b);
                    f.op(0x0b);
                }
                jump(&mut f, t, 0);
            }
            Ins::Branch(condition, t) => {
                // a = top; not a Boolean: the interpreter's branch (which
                // sends #mustBeBoolean or the like), then the interpreter.
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                f.load().set(L_A);
                f.get(L_A)
                    .get(L_TRUE)
                    .op(0x51)
                    .get(L_A)
                    .get(L_FALSE)
                    .op(0x51)
                    .op(0x72); // eq | eq
                f.op(0x45); // i32.eqz
                f.op(0x04).op(0x40); // if
                f.step(ip);
                f.leave_after_step();
                f.op(0x0b);
                f.dec_sp();
                f.get(L_A)
                    .get(if condition { L_TRUE } else { L_FALSE })
                    .op(0x51); // taken?
                f.op(0x04).op(0x40); // if
                jump(&mut f, t, 1);
                f.op(0x0b);
                // Not taken: on to the next instruction, which starts a block.
                if next < code.len() {
                    jump(&mut f, next, 0);
                }
            }
            Ins::Arith(k) => {
                // a = stack[sp-2], b = stack[sp-1]; both SmallIntegers
                // (low bit 0), else the send, by `step`.
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(2).sub();
                });
                f.load().set(L_A);
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                f.load().set(L_B);
                f.get(L_A)
                    .get(L_B)
                    .op(0x84)
                    .i64c(1)
                    .op(0x83)
                    .op(0x50)
                    .op(0x45); // ((a|b)&1) != 0
                f.op(0x04).op(0x40);
                // Not SmallIntegers: a send, through an inline cache.
                f.common_and_go_on(k, next, true, code.len(), 1, &jump);
                f.op(0x0b);
                // The result, into r64, then into stack[sp-2]; sp -= 1.
                match k {
                    0 | 1 => {
                        f.get(L_A)
                            .get(L_B)
                            .op(if k == 0 { 0x7c } else { 0x7d })
                            .set(L_R64);
                        if k == 0 {
                            // overflow: ((a ^ r) & (b ^ r)) < 0
                            f.get(L_A)
                                .get(L_R64)
                                .op(0x85)
                                .get(L_B)
                                .get(L_R64)
                                .op(0x85)
                                .op(0x83);
                        } else {
                            // overflow: ((a ^ b) & (a ^ r)) < 0
                            f.get(L_A)
                                .get(L_B)
                                .op(0x85)
                                .get(L_A)
                                .get(L_R64)
                                .op(0x85)
                                .op(0x83);
                        }
                        f.i64c(0).op(0x53); // i64.lt_s
                        f.op(0x04).op(0x40);
                        f.common_and_go_on(k, next, false, code.len(), 1, &jump);
                        f.op(0x0b);
                    }
                    7 => {
                        f.get(L_A).get(L_B).op(0x83).set(L_R64);
                    }
                    8 => {
                        f.get(L_A).get(L_B).op(0x84).set(L_R64);
                    }
                    _ => {
                        let cmp = match k {
                            9 => 0x53,  // lt_s
                            10 => 0x55, // gt_s
                            11 => 0x57, // le_s
                            12 => 0x59, // ge_s
                            _ => 0x51,  // eq
                        };
                        f.get(L_TRUE)
                            .get(L_FALSE)
                            .get(L_A)
                            .get(L_B)
                            .op(cmp)
                            .op(0x1b); // select
                        f.set(L_R64);
                    }
                }
                f.slot_addr(|f| {
                    f.get(L_SP).i32c(2).sub();
                });
                f.get(L_R64).store();
                f.dec_sp();
            }
            Ins::Send {
                rule,
                literal,
                args,
            } => {
                f.send(rule, literal, args, next);
            }
            Ins::Indirect {
                op: o,
                index,
                local,
            } => {
                // The vector: an Array with the element; for a store, not
                // frozen, and young or remembered. Else by `step`.
                f.slot_addr(|f| {
                    f.get(L_LP).i32c(local as i32).add();
                });
                f.load().set(L_A);
                f.object_at_a();
                f.get(L_A).i64c(1).op(0x83).op(0x50).op(0x45); // a reference
                f.op(0x04).op(0x7f);
                f.get(L_ADDR).i32c(META as i32).add().load().set(L_META);
                f.get(L_META).i64c(31).op(0x83).i64c(3).op(0x51); // an Array
                f.get(L_META)
                    .i64c(32)
                    .op(0x88)
                    .i64c(i64::from(index))
                    .op(0x56)
                    .op(0x71); // size > index
                if o != op::PUSH_INDIRECT_LOCAL {
                    f.get(L_META).i64c(1 << 5).op(0x83).op(0x50).op(0x71); // not frozen
                    f.get(L_INDEX).reg32(R_BOUNDARY).op(0x4f);
                    f.get(L_META)
                        .i64c(META_REMEMBERED)
                        .op(0x83)
                        .op(0x50)
                        .op(0x45);
                    f.op(0x72).op(0x71);
                }
                f.op(0x05).i32c(0).op(0x0b);
                f.op(0x45).op(0x04).op(0x40); // if not: by step
                f.step(ip);
                f.after_call();
                if next < code.len() {
                    jump(&mut f, next, 1);
                } else {
                    f.interpret(next);
                }
                f.op(0x0b);
                let element = |f: &mut Func| {
                    f.get(L_ADDR).i32c((SLOTS + 8 * index) as i32).add();
                };
                match o {
                    op::PUSH_INDIRECT_LOCAL => {
                        f.push_addr();
                        element(&mut f);
                        f.load().store().inc_sp();
                    }
                    _ => {
                        element(&mut f);
                        f.slot_addr(|f| {
                            f.get(L_SP).i32c(1).sub();
                        });
                        f.load().store();
                        if o == op::POP_INTO_INDIRECT_LOCAL {
                            f.dec_sp();
                        }
                    }
                }
            }
            Ins::Closure(after) => {
                // The closure, made here when there is room (with the
                // frame's activation handle, if it has none yet), else by
                // the runtime's `closure`; on after its body.
                let b1 = code[ip + 1];
                f.make_closure((b1 >> 4).into(), (b1 & 15).into(), ip + 4);
                f.op(0x04).op(0x40);
                if after < code.len() {
                    jump(&mut f, after, 1);
                } else {
                    f.interpret(after);
                }
                f.op(0x0b);
                f.get(L_VM)
                    .i32c(ip as i32)
                    .get(L_SP)
                    .call(F_CLOSURE)
                    .set(L_R);
                f.after_call();
                if after < code.len() {
                    jump(&mut f, after, 0);
                } else {
                    f.interpret(after);
                }
            }
            Ins::Step(after) => {
                f.step(ip);
                f.after_call();
                if after != next {
                    if after < code.len() {
                        jump(&mut f, after, 0);
                    } else {
                        f.interpret(after);
                    }
                }
            }
            Ins::NonLocalReturn(r) => {
                f.ret_value(r);
                f.set(L_R64);
                f.get(L_VM)
                    .get(L_R64)
                    .i32c(next as i32)
                    .get(L_SP)
                    .call(F_NLR)
                    .set(L_R);
                f.after_call();
                // The frame does not go on.
                f.interpret(next);
            }
            Ins::Return(r) => {
                f.ret_value(r);
                f.set(L_R64);
                f.pop_frame();
                f.get(L_VM).get(L_R64);
                f.get(L_SP).call(F_RET).op(0x0f);
            }
        }
        // Falling into the next block: it is the next block's code anyway.
        let at_end = index + 1 == ins.len();
        if !at_end && block_of(ins[index + 1].0).is_some() {
            k += 1;
        }
    }
    // Falling off the end: the interpreter decides.
    f.interpret(code.len());
    f.op(0x0b); // end loop
    f.i32c(INTERPRET);
    Some(Compiled {
        module: module(&f.body, f.sites),
        entries: leaders.iter().map(|&l| l as u32).collect(),
    })
}

/// A function body under construction.
#[derive(Default)]
struct Func {
    body: Vec<u8>,
    /// Inline-cache sites, each [`SITE_GLOBALS`] globals: class (i64),
    /// epoch, slot, kind.
    sites: u32,
}

impl Func {
    fn op(&mut self, b: u8) -> &mut Func {
        self.body.push(b);
        self
    }
    fn uleb(&mut self, mut v: u32) -> &mut Func {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.body.push(byte);
                return self;
            }
            self.body.push(byte | 0x80);
        }
    }
    fn sleb(&mut self, mut v: i64) -> &mut Func {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            let done = (v == 0 && byte & 0x40 == 0) || (v == -1 && byte & 0x40 != 0);
            self.body.push(if done { byte } else { byte | 0x80 });
            if done {
                return self;
            }
        }
    }
    fn get(&mut self, l: u32) -> &mut Func {
        self.op(0x20).uleb(l)
    }
    fn set(&mut self, l: u32) -> &mut Func {
        self.op(0x21).uleb(l)
    }
    fn i32c(&mut self, v: i32) -> &mut Func {
        self.op(0x41).sleb(i64::from(v))
    }
    fn i64c(&mut self, v: i64) -> &mut Func {
        self.op(0x42).sleb(v)
    }
    fn call(&mut self, f: u32) -> &mut Func {
        self.op(0x10).uleb(f)
    }
    fn add(&mut self) -> &mut Func {
        self.op(0x6a)
    }
    fn sub(&mut self) -> &mut Func {
        self.op(0x6b)
    }
    /// base + (index << 3), the index computed by `index`.
    fn slot_addr(&mut self, index: impl FnOnce(&mut Func)) -> &mut Func {
        self.get(L_BASE);
        index(self);
        self.i32c(3).op(0x74).add() // shl, add
    }
    /// The address of the next push: base + sp * 8.
    fn push_addr(&mut self) -> &mut Func {
        self.slot_addr(|f| {
            f.get(L_SP);
        })
    }
    /// The value at stack[sp] (after a decrement: the popped one).
    fn top_value_at_sp(&mut self) -> &mut Func {
        self.slot_addr(|f| {
            f.get(L_SP);
        })
        .load()
    }
    fn load(&mut self) -> &mut Func {
        self.op(0x29).uleb(3).uleb(0) // i64.load align 8
    }
    /// i32.load at the address on the stack plus `offset`.
    fn ld32(&mut self, offset: u32) -> &mut Func {
        self.op(0x28).uleb(2).uleb(offset)
    }
    /// i64.load at the address on the stack plus `offset`.
    fn ld64(&mut self, offset: u32) -> &mut Func {
        self.op(0x29).uleb(3).uleb(offset)
    }
    /// i32.store at the address below the value plus `offset`.
    fn st32(&mut self, offset: u32) -> &mut Func {
        self.op(0x36).uleb(2).uleb(offset)
    }
    /// i64.store at the address below the value plus `offset`.
    fn st64(&mut self, offset: u32) -> &mut Func {
        self.op(0x37).uleb(3).uleb(offset)
    }
    /// Sets the i32 register at `offset` to what `value` pushes.
    fn set_reg32(&mut self, offset: u32, value: impl FnOnce(&mut Func)) -> &mut Func {
        self.get(L_REGS);
        value(self);
        self.st32(offset)
    }
    /// Sets the i64 register at `offset` to what `value` pushes.
    fn set_reg64(&mut self, offset: u32, value: impl FnOnce(&mut Func)) -> &mut Func {
        self.get(L_REGS);
        value(self);
        self.st64(offset)
    }
    /// The address of frame `k` (an i32 on the stack): frames + k * size.
    fn frame_addr(&mut self) -> &mut Func {
        self.i32c(FRAME_SIZE as i32).op(0x6c); // i32.mul
        self.reg32(R_FRAMES).add()
    }
    fn store(&mut self) -> &mut Func {
        self.op(0x37).uleb(3).uleb(0) // i64.store align 8
    }
    fn inc_sp(&mut self) -> &mut Func {
        self.get(L_SP).i32c(1).add().set(L_SP)
    }
    fn dec_sp(&mut self) -> &mut Func {
        self.get(L_SP).i32c(1).sub().set(L_SP)
    }
    /// Runs the instruction at `ip` by the runtime's `step`, into `r`.
    fn step(&mut self, ip: usize) -> &mut Func {
        self.get(L_VM)
            .i32c(ip as i32)
            .get(L_SP)
            .call(F_STEP)
            .set(L_R)
    }
    /// After a `step` whose continuation compiled code does not know: back
    /// to the caller, with the frame as the step left it.
    fn leave_after_step(&mut self) -> &mut Func {
        self.get(L_R).i32c(RETURNED).op(0x46).op(0x04).op(0x40);
        self.i32c(DONE).op(0x0f);
        self.op(0x0b);
        self.get(L_R).i32c(STOP).op(0x46).op(0x04).op(0x40);
        self.i32c(STOP).op(0x0f);
        self.op(0x0b);
        self.i32c(INTERPRET).op(0x0f)
    }
    /// After a `send` or `step` (its answer in `r`) whose instruction goes on
    /// at `next`: runs a compiled callee; returns unless the instruction
    /// completed; takes a due safepoint in the interpreter's loop; reloads
    /// the stack.
    fn after_call(&mut self) -> &mut Func {
        // A compiled callee: run it here, from its entry.
        self.get(L_R).i32c(CALLEE).op(0x4e); // i32.ge_s
        self.op(0x04).op(0x40);
        self.get(L_VM).get(L_REGS).reg32(R_BLOCK);
        self.get(L_R).i32c(CALLEE).sub();
        self.op(0x11).uleb(T_RUN).uleb(0); // call_indirect
        self.set(L_R);
        self.op(0x0b);
        // The frame returned (a `^` by `step`).
        self.get(L_R).i32c(RETURNED).op(0x46).op(0x04).op(0x40);
        self.i32c(DONE).op(0x0f);
        self.op(0x0b);
        // A non-local return to this frame: as a completed send.
        self.get(L_R).i32c(UNWOUND).op(0x46);
        self.reg32(R_DEPTH).get(L_DEPTH).op(0x46).op(0x71);
        self.op(0x04).op(0x40);
        self.i32c(DONE).set(L_R);
        self.op(0x0b);
        // Anything but a completed instruction: leave it to the caller.
        self.get(L_R).op(0x04).op(0x40);
        self.get(L_R).op(0x0f);
        self.op(0x0b);
        // A safepoint due: the interpreter's loop takes it (and gives the
        // frame back at `next`, an entry).
        self.reg32(R_DUE).op(0x04).op(0x40);
        self.i32c(INTERPRET).op(0x0f);
        self.op(0x0b);
        // Reload the stack's address and height.
        self.reg32(R_SP).set(L_SP);
        self.reg32(R_BASE).set(L_BASE)
    }
    fn gget(&mut self, g: u32) -> &mut Func {
        self.op(0x23).uleb(g)
    }
    fn gset(&mut self, g: u32) -> &mut Func {
        self.op(0x24).uleb(g)
    }
    /// A new inline-cache site: its first global.
    fn new_site(&mut self) -> u32 {
        self.sites += 1;
        SITE_GLOBALS * (self.sites - 1)
    }
    /// Looks the send's key up in site `g`: the message receiver below
    /// `args` arguments for an ordinary send, else the frame's. An entry
    /// hits for an object of its class (a SmallInteger, if the class is
    /// SmallInteger) in its epoch; its kind, slot and object are left in
    /// `kind` (0 if neither hits), `slot` and `object`. Leaves the key in
    /// `a`, and for a reference its index and address in `index` and `addr`.
    fn site_hit(&mut self, g: u32, ordinary: bool, args: u32) -> &mut Func {
        if ordinary {
            self.slot_addr(|f| {
                f.get(L_SP).i32c(1 + args as i32).sub();
            });
            self.load().set(L_A);
        } else {
            self.get(L_SELF).set(L_A);
        }
        self.object_at_a();
        // the key's class, in `meta`
        self.get(L_A).i64c(1).op(0x83).op(0x50).op(0x45); // (a & 1) != 0
        self.op(0x04).op(0x7e); // if (result i64)
        self.get(L_ADDR).load();
        self.op(0x05);
        self.reg64(R_SMALL_INTEGER);
        self.op(0x0b).set(L_META);
        self.i32c(0).set(L_KIND);
        for e in [g, g + ENTRY_GLOBALS] {
            self.get(L_META).gget(e).op(0x51); // class == cached
            self.reg32(R_EPOCH).gget(e + 1).op(0x46).op(0x71); // & epoch == cached
            self.op(0x04).op(0x40);
            self.gget(e + 3).set(L_KIND);
            self.gget(e + 2).set(L_SLOT);
            self.gget(e + 4).set(L_OBJECT);
            if e == g {
                self.op(0x05);
            }
        }
        self.op(0x0b).op(0x0b)
    }
    /// Fills the inline-cache entry at global `e` from the registers.
    fn fill_entry(&mut self, e: u32) -> &mut Func {
        self.reg64(R_IC_CLASS).gset(e);
        self.reg32(R_IC_SLOT).gset(e + 2);
        self.reg32(R_EPOCH).gset(e + 1);
        self.reg32(R_IC_KIND).gset(e + 3);
        self.reg64(R_IC_OBJECT).gset(e + 4)
    }
    /// Whether the object at `addr`, of the hit entry's class, holds the
    /// accessor's slot: of the regular format with the slot; for a setter,
    /// neither frozen nor a value, and young or remembered (else the write
    /// barrier must remember it).
    fn accessor_ok(&mut self, setter: bool) -> &mut Func {
        // the meta word: regular (code 0) and the slot within the size
        self.get(L_ADDR).i32c(META as i32).add().load().set(L_META);
        self.get(L_META).i64c(31).op(0x83).op(0x50); // code == 0
        self.get(L_META)
            .i64c(32)
            .op(0x88)
            .get(L_SLOT)
            .op(0xad)
            .op(0x56)
            .op(0x71); // size > slot
        if setter {
            self.get(L_META)
                .i64c(META_FLAGS_FROZEN_OR_VALUE)
                .op(0x83)
                .op(0x50)
                .op(0x71);
            // young, or remembered
            self.get(L_INDEX).reg32(R_BOUNDARY).op(0x4f); // index >= boundary
            self.get(L_META)
                .i64c(META_REMEMBERED)
                .op(0x83)
                .op(0x50)
                .op(0x45);
            self.op(0x72).op(0x71);
        }
        self
    }
    /// Whether the object at `addr` (after [`Func::site_hit`]) is one the
    /// indexed native of inline-cache kind `kind` runs on here: of its
    /// format; for `at:` and `at:put:`, the index a SmallInteger within its
    /// size (left in `b`); for `at:put:`, not frozen, and young or
    /// remembered.
    fn indexed_ok(&mut self, kind: u32) -> &mut Func {
        let args = match kind {
            IC_SIZE => 0,
            IC_AT_PUT => 2,
            _ => 1,
        };
        self.get(L_ADDR).i32c(META as i32).add().load().set(L_META);
        match kind {
            // Bytes, String or Array: (code - 1) <u 3
            IC_SIZE => self
                .get(L_META)
                .i64c(31)
                .op(0x83)
                .i64c(1)
                .op(0x7d)
                .i64c(3)
                .op(0x54),
            // Bytes or String: (code - 1) <u 2
            IC_BYTES_AT => self
                .get(L_META)
                .i64c(31)
                .op(0x83)
                .i64c(1)
                .op(0x7d)
                .i64c(2)
                .op(0x54),
            _ => self.get(L_META).i64c(31).op(0x83).i64c(CODE_ARRAY).op(0x51),
        };
        if args >= 1 {
            // the index, below the value of at:put:
            self.slot_addr(|f| {
                f.get(L_SP).i32c(args).sub();
            });
            self.load().set(L_B);
            self.get(L_B).i64c(1).op(0x83).op(0x50).op(0x71); // a SmallInteger
            // (index - 1) <u size
            self.get(L_B)
                .i64c(1)
                .op(0x87)
                .i64c(1)
                .op(0x7d)
                .get(L_META)
                .i64c(32)
                .op(0x88)
                .op(0x54)
                .op(0x71);
        }
        if args == 2 {
            self.get(L_META).i64c(1 << 5).op(0x83).op(0x50).op(0x71); // not frozen
            self.get(L_INDEX).reg32(R_BOUNDARY).op(0x4f);
            self.get(L_META)
                .i64c(META_REMEMBERED)
                .op(0x83)
                .op(0x50)
                .op(0x45);
            self.op(0x72).op(0x71);
        }
        self
    }
    /// The indexed native of inline-cache kind `kind` (after
    /// [`Func::indexed_ok`]): its answer replaces the receiver and the
    /// arguments (of an ordinary send), or the arguments (else).
    fn indexed(&mut self, kind: u32, ordinary: bool) -> &mut Func {
        let args = match kind {
            IC_SIZE => 0,
            IC_AT_PUT => 2,
            _ => 1,
        };
        // where the answer goes, below sp
        let answer = if ordinary { args + 1 } else { args };
        // the element's address: addr + SLOTS + (index - 1) * 8
        let element = |f: &mut Func| {
            f.get(L_ADDR)
                .i32c(SLOTS as i32)
                .add()
                .get(L_B)
                .i64c(1)
                .op(0x87)
                .i64c(1)
                .op(0x7d)
                .op(0xa7)
                .i32c(3)
                .op(0x74)
                .add();
        };
        let answer_addr = |f: &mut Func| {
            f.slot_addr(|f| {
                f.get(L_SP).i32c(answer).sub();
            });
        };
        match kind {
            IC_SIZE => {
                answer_addr(self);
                self.get(L_META).i64c(32).op(0x88).i64c(1).op(0x86).store();
            }
            IC_BYTES_AT => {
                // bytes + (first slot >> 1) + index - 1, a byte, as a
                // SmallInteger
                answer_addr(self);
                self.reg32(R_BYTES)
                    .get(L_ADDR)
                    .i32c(SLOTS as i32)
                    .add()
                    .load()
                    .i64c(1)
                    .op(0x87)
                    .get(L_B)
                    .i64c(1)
                    .op(0x87)
                    .op(0x7c)
                    .i64c(1)
                    .op(0x7d)
                    .op(0xa7)
                    .add();
                self.op(0x31).uleb(0).uleb(0); // i64.load8_u
                self.i64c(1).op(0x86).store();
            }
            IC_AT => {
                answer_addr(self);
                element(self);
                self.load().store();
            }
            _ => {
                element(self);
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                self.load().store();
                answer_addr(self);
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                self.load().store();
            }
        }
        // sp: less the arguments (and the receiver); plus the answer
        let delta = answer - 1;
        if delta > 0 {
            self.get(L_SP).i32c(delta).sub().set(L_SP);
        } else if delta < 0 {
            self.inc_sp();
        }
        self
    }
    /// The binary operation of inline-cache kind `kind` ([`IC_IDENTICAL`] or
    /// a bit operation) on the key in `a` and the argument on top, when it
    /// applies (a bit operation's argument a SmallInteger), its answer
    /// replacing the receiver and the argument (of an ordinary send) or the
    /// argument (else); `br`s out of the enclosing `if` when it does not.
    fn binary(&mut self, kind: u32, ordinary: bool) -> &mut Func {
        self.slot_addr(|f| {
            f.get(L_SP).i32c(1).sub();
        });
        self.load().set(L_B);
        if (IC_LT..=IC_GE).contains(&kind) {
            // Both numbers this native compares here: else out.
            self.is_number(L_A).is_number(L_B).op(0x71).op(0x45);
            self.op(0x04).op(0x40).op(0x0c).uleb(1).op(0x0b);
            // Two SmallIntegers exactly (tagged, the order kept); else as
            // f64s, as the native does.
            self.get(L_A).get(L_B).op(0x84).i64c(1).op(0x83).op(0x50);
            self.op(0x04).op(0x7f);
            self.get(L_A).get(L_B).op(match kind {
                IC_LT => 0x53,
                IC_GT => 0x55,
                IC_LE => 0x57,
                _ => 0x59,
            });
            self.op(0x05);
            self.as_f64(L_A).as_f64(L_B);
            self.op(match kind {
                IC_LT => 0x63,
                IC_GT => 0x64,
                IC_LE => 0x65,
                _ => 0x66,
            });
            self.op(0x0b);
            self.op(0x04)
                .op(0x7e)
                .get(L_TRUE)
                .op(0x05)
                .get(L_FALSE)
                .op(0x0b);
        } else if kind == IC_IDENTICAL {
            self.get(L_A).get(L_B).op(0x51); // i64.eq
            self.op(0x04)
                .op(0x7e)
                .get(L_TRUE)
                .op(0x05)
                .get(L_FALSE)
                .op(0x0b);
        } else {
            // A SmallInteger argument; the tag (0) survives the operation.
            self.get(L_B).i64c(1).op(0x83).op(0x50).op(0x45); // not a SmallInteger
            self.op(0x04).op(0x40).op(0x0c).uleb(1).op(0x0b); // br: out of the if
            self.get(L_A).get(L_B);
            self.op(match kind {
                IC_BIT_AND => 0x83,
                IC_BIT_OR => 0x84,
                _ => 0x85,
            });
        }
        self.set(L_R64);
        if ordinary {
            self.dec_sp();
        }
        self.slot_addr(|f| {
            f.get(L_SP).i32c(1).sub();
        });
        self.get(L_R64).store()
    }
    /// Whether the Value in local `x` is a SmallInteger or a Float (an i32).
    fn is_number(&mut self, x: u32) -> &mut Func {
        self.get(x).i64c(1).op(0x83).op(0x50); // i64.eqz: a SmallInteger
        self.op(0x04).op(0x7f).i32c(1).op(0x05);
        self.reg32(R_ARENA)
            .get(x)
            .i64c(1)
            .op(0x88)
            .op(0xa7)
            .i32c(3)
            .op(0x74)
            .add()
            .ld64(0)
            .reg64(R_FLOAT_CLASS)
            .op(0x51);
        self.op(0x0b)
    }
    /// The number in local `x` (after [`Func::is_number`]) as an f64.
    fn as_f64(&mut self, x: u32) -> &mut Func {
        self.get(x).i64c(1).op(0x83).op(0x50);
        self.op(0x04).op(0x7c); // if (result f64)
        self.get(x).i64c(1).op(0x87).op(0xb9); // shr_s, f64.convert_i64_s
        self.op(0x05);
        self.reg32(R_ARENA)
            .get(x)
            .i64c(1)
            .op(0x88)
            .op(0xa7)
            .i32c(3)
            .op(0x74)
            .add()
            .ld64(SLOTS)
            .op(0xbf); // f64.reinterpret_i64
        self.op(0x0b)
    }
    /// The hit entry's accessor, read or written (after [`Func::accessor_ok`]).
    fn accessor(&mut self, ordinary: bool, setter: bool) -> &mut Func {
        // the slot's address: addr + SLOTS + slot * 8
        let slot_addr = |f: &mut Func| {
            f.get(L_ADDR)
                .i32c(SLOTS as i32)
                .add()
                .get(L_SLOT)
                .i32c(3)
                .op(0x74)
                .add();
        };
        match (ordinary, setter) {
            (_, false) => {
                // The value replaces the receiver (ordinary) or is pushed.
                if ordinary {
                    self.slot_addr(|f| {
                        f.get(L_SP).i32c(1).sub();
                    });
                } else {
                    self.push_addr();
                }
                slot_addr(self);
                self.load().store();
                if !ordinary {
                    self.inc_sp();
                }
            }
            (_, true) => {
                // slot := top; the answer is the receiver.
                slot_addr(self);
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                self.load().store();
                if ordinary {
                    self.dec_sp();
                } else {
                    self.slot_addr(|f| {
                        f.get(L_SP).i32c(1).sub();
                    });
                    self.get(L_A).store();
                }
            }
        }
        self
    }
    /// The i32 register at `offset`.
    fn reg32(&mut self, offset: u32) -> &mut Func {
        self.get(L_REGS).op(0x28).uleb(2).uleb(offset) // i32.load align 4
    }
    /// The i64 register at `offset`.
    fn reg64(&mut self, offset: u32) -> &mut Func {
        self.get(L_REGS).op(0x29).uleb(3).uleb(offset) // i64.load align 8
    }
    /// index = a >> 1; addr = arena + index * 8 (for `a` a reference).
    fn object_at_a(&mut self) -> &mut Func {
        self.get(L_A).i64c(1).op(0x88).op(0xa7).set(L_INDEX); // i64.shr_u, i32.wrap
        self.reg32(R_ARENA)
            .get(L_INDEX)
            .i32c(3)
            .op(0x74)
            .add()
            .set(L_ADDR)
    }
    /// Whether the method target of the hit entry (its index in `slot`) can
    /// have its frame pushed here: no step limit and no safepoint due, room
    /// for a frame (within the VM's limits) and for its locals and the
    /// headroom on the operand stack. Leaves the target's address in
    /// `index`.
    fn push_frame_ok(&mut self) -> &mut Func {
        self.reg32(R_TARGETS)
            .get(L_SLOT)
            .i32c(TARGET_SIZE as i32)
            .op(0x6c)
            .add()
            .set(L_INDEX);
        self.reg32(R_STEPPING).reg32(R_DUE).op(0x72).op(0x45);
        self.reg32(R_FRAMES_LEN)
            .reg32(R_FRAMES_CAP)
            .op(0x49)
            .op(0x71); // len <u cap
        self.reg32(R_FRAMES_LEN)
            .i32c(MAX_FRAMES as i32 - 1)
            .op(0x49)
            .op(0x71);
        self.reg32(R_FRAMES_LEN)
            .i32c(1)
            .add()
            .reg32(R_JIT_BASE)
            .sub()
            .i32c(MAX_NESTED as i32)
            .op(0x48) // lt_s
            .op(0x71);
        // a method's, not a native's
        self.get(L_INDEX)
            .ld32(TARGET_FUNCTION)
            .i32c(NATIVE_FUNCTION as i32)
            .op(0x47)
            .op(0x71);
        // sp + 1 (a receiver inserted) + locals + headroom <= the room
        self.get(L_SP)
            .i32c(1 + HEADROOM as i32)
            .add()
            .get(L_INDEX)
            .ld32(TARGET_LOCALS)
            .add()
            .reg32(R_STACK_CAP)
            .op(0x4d) // le_u
            .op(0x71)
    }
    /// Pushes the frame of the method target at `index` (after
    /// [`Func::push_frame_ok`]) for a send of `args` arguments that continues
    /// at `next`, as the VM's `enter_target` does: the receiver inserted if
    /// the target's is not the message's, the locals nil, the caller's `ip`
    /// at `next`, the registers set for the callee; `r` is then
    /// [`CALLEE`] plus its function.
    fn push_frame(&mut self, args: u32, ordinary: bool, next: usize) -> &mut Func {
        if !ordinary {
            // The receiver, inserted below the arguments: the frame's (1)
            // or a fixed one (2).
            self.get(L_INDEX)
                .ld32(TARGET_RECEIVER_KIND)
                .op(0x04)
                .op(0x40);
            for i in (0..args).rev() {
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(args as i32 - i as i32 - 1).sub();
                });
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(args as i32 - i as i32).sub();
                });
                self.load().store();
            }
            self.slot_addr(|f| {
                f.get(L_SP).i32c(args as i32).sub();
            });
            self.get(L_INDEX)
                .ld32(TARGET_RECEIVER_KIND)
                .i32c(1)
                .op(0x46);
            self.op(0x04).op(0x7e).get(L_SELF).op(0x05);
            self.get(L_INDEX).ld64(TARGET_FIXED).op(0x0b);
            self.store();
            self.inc_sp();
            self.op(0x0b);
        }
        // bp, in `kind`; the receiver, in `b`
        self.get(L_SP).i32c(1 + args as i32).sub().set(L_KIND);
        self.slot_addr(|f| {
            f.get(L_KIND);
        })
        .load()
        .set(L_B);
        // the caller's ip
        self.reg32(R_FRAMES_LEN).i32c(1).sub().frame_addr();
        self.i32c(next as i32).st32(FRAME_IP);
        // the new frame, at `addr`
        self.reg32(R_FRAMES_LEN).frame_addr().set(L_ADDR);
        self.get(L_ADDR).get(L_SP).st32(FRAME_LP);
        self.get(L_ADDR).get(L_KIND).st32(FRAME_BP);
        self.get(L_ADDR).i32c(args as i32).st32(FRAME_ARG_COUNT);
        self.get(L_ADDR).i32c(0).st32(FRAME_CODE);
        self.get(L_ADDR).i32c(0).st32(FRAME_IP);
        self.get(L_ADDR).reg64(R_SERIAL).st64(FRAME_SERIAL);
        self.set_reg64(R_SERIAL, |f| {
            f.reg64(R_SERIAL).i64c(1).op(0x7c);
        });
        for (from, to) in [
            (TARGET_METHOD, FRAME_METHOD),
            (TARGET_LITERALS, FRAME_LITERALS),
            (TARGET_MIXIN, FRAME_MIXIN),
        ] {
            self.get(L_ADDR).get(L_INDEX).ld64(from).st64(to);
        }
        self.get(L_ADDR).get(L_B).st64(FRAME_RECEIVER);
        self.get(L_ADDR).i64c(0).st64(FRAME_ACTIVATION);
        self.get(L_ADDR).i64c(0).st64(FRAME_CLOSURE);
        // the locals, nil
        self.get(L_INDEX).ld32(TARGET_LOCALS).set(L_SLOT);
        self.op(0x02).op(0x40).op(0x03).op(0x40); // block, loop
        self.get(L_SLOT).op(0x45).op(0x0d).uleb(1); // br_if out when none left
        self.push_addr().get(L_NIL).store();
        self.inc_sp();
        self.get(L_SLOT).i32c(1).sub().set(L_SLOT);
        self.op(0x0c).uleb(0).op(0x0b).op(0x0b); // br loop; end loop; end block
        // the count and the registers
        self.set_reg32(R_FRAMES_LEN, |f| {
            f.reg32(R_FRAMES_LEN).i32c(1).add();
        });
        self.set_reg32(R_DEPTH, |f| {
            f.reg32(R_FRAMES_LEN);
        });
        self.set_reg32(R_SP, |f| {
            f.get(L_SP);
        });
        self.set_reg32(R_BP, |f| {
            f.get(L_KIND);
        });
        self.set_reg32(R_LP, |f| {
            f.get(L_ADDR).ld32(FRAME_LP);
        });
        self.set_reg32(R_ARGC, |f| {
            f.i32c(args as i32);
        });
        self.set_reg32(R_LITERALS, |f| {
            f.reg32(R_ARENA)
                .get(L_INDEX)
                .ld32(TARGET_LITERAL_OFFSET)
                .add();
        });
        self.set_reg32(R_LITERAL_COUNT, |f| {
            f.get(L_INDEX).ld32(TARGET_LITERAL_COUNT);
        });
        self.set_reg64(R_RECEIVER, |f| {
            f.get(L_B);
        });
        self.set_reg32(R_BLOCK, |f| {
            f.i32c(0);
        });
        self.set_reg32(R_IC_KIND, |f| {
            f.i32c(0);
        });
        self.get(L_INDEX)
            .ld32(TARGET_FUNCTION)
            .i32c(CALLEE)
            .add()
            .set(L_R)
    }
    /// Whether the closure in `a` (at `addr`; after [`Func::site_hit`]), the
    /// receiver of a `value` send of `args` arguments, can have its frame
    /// pushed here for the hit entry's closure target (its index in
    /// `slot`): the target's body and method, `args` arguments, no step
    /// limit and no safepoint due, room for a frame and for its copied
    /// values plus the headroom. Leaves the target's address in `index`
    /// and the defining activation's in `kind`.
    fn push_closure_ok(&mut self, args: u32) -> &mut Func {
        self.reg32(R_CLOSURE_TARGETS)
            .get(L_SLOT)
            .i32c(CTARGET_SIZE as i32)
            .op(0x6c)
            .add()
            .set(L_INDEX);
        // the defining activation's address
        self.reg32(R_ARENA)
            .get(L_ADDR)
            .ld64(CLOSURE_DEFINING)
            .i64c(1)
            .op(0x88)
            .op(0xa7)
            .i32c(3)
            .op(0x74)
            .add()
            .set(L_KIND);
        self.get(L_ADDR)
            .ld64(CLOSURE_INITIAL_IP)
            .get(L_INDEX)
            .ld64(CTARGET_INITIAL_IP)
            .op(0x51);
        self.get(L_ADDR)
            .ld64(CLOSURE_NUM_ARGS)
            .i64c(i64::from(args) << 1)
            .op(0x51)
            .op(0x71);
        self.get(L_KIND)
            .ld64(ACTIVATION_METHOD)
            .get(L_INDEX)
            .ld64(CTARGET_METHOD)
            .op(0x51)
            .op(0x71);
        self.reg32(R_STEPPING)
            .reg32(R_DUE)
            .op(0x72)
            .op(0x45)
            .op(0x71);
        self.reg32(R_FRAMES_LEN)
            .reg32(R_FRAMES_CAP)
            .op(0x49)
            .op(0x71);
        self.reg32(R_FRAMES_LEN)
            .i32c(MAX_FRAMES as i32 - 1)
            .op(0x49)
            .op(0x71);
        self.reg32(R_FRAMES_LEN)
            .i32c(1)
            .add()
            .reg32(R_JIT_BASE)
            .sub()
            .i32c(MAX_NESTED as i32)
            .op(0x48)
            .op(0x71);
        // sp + copied (size - 3) + headroom <= the room
        self.get(L_SP)
            .i32c(HEADROOM as i32 - 3)
            .add()
            .get(L_ADDR)
            .ld64(META)
            .i64c(32)
            .op(0x88)
            .op(0xa7)
            .add()
            .reg32(R_STACK_CAP)
            .op(0x4d)
            .op(0x71)
    }
    /// Pushes the frame of the closure in `a` (after
    /// [`Func::push_closure_ok`]) for its `value` send of `args` arguments,
    /// which continues at `next`, as the VM's `direct_closure` does: the
    /// copied values its first locals, the caller's `ip` at `next`, the
    /// registers set for the body's block; `r` is then [`CALLEE`] plus its
    /// function.
    fn push_closure_frame(&mut self, args: u32, next: usize) -> &mut Func {
        // the receiver, the defining activation's, in `b`; bp in `kind`
        self.get(L_KIND).ld64(ACTIVATION_RECEIVER).set(L_B);
        self.get(L_SP).i32c(1 + args as i32).sub().set(L_KIND);
        // the caller's ip
        self.reg32(R_FRAMES_LEN).i32c(1).sub().frame_addr();
        self.i32c(next as i32).st32(FRAME_IP);
        // the new frame, at `slot`
        self.reg32(R_FRAMES_LEN).frame_addr().set(L_SLOT);
        self.get(L_SLOT).get(L_SP).st32(FRAME_LP);
        self.get(L_SLOT).get(L_KIND).st32(FRAME_BP);
        self.get(L_SLOT).i32c(args as i32).st32(FRAME_ARG_COUNT);
        self.get(L_SLOT).i32c(0).st32(FRAME_CODE);
        self.get(L_SLOT)
            .get(L_INDEX)
            .ld64(CTARGET_INITIAL_IP)
            .i64c(1)
            .op(0x87)
            .op(0xa7)
            .st32(FRAME_IP);
        self.get(L_SLOT).reg64(R_SERIAL).st64(FRAME_SERIAL);
        self.set_reg64(R_SERIAL, |f| {
            f.reg64(R_SERIAL).i64c(1).op(0x7c);
        });
        for (from, to) in [
            (CTARGET_METHOD, FRAME_METHOD),
            (CTARGET_LITERALS, FRAME_LITERALS),
            (CTARGET_MIXIN, FRAME_MIXIN),
        ] {
            self.get(L_SLOT).get(L_INDEX).ld64(from).st64(to);
        }
        self.get(L_SLOT).get(L_B).st64(FRAME_RECEIVER);
        self.get(L_SLOT).i64c(0).st64(FRAME_ACTIVATION);
        self.get(L_SLOT).get(L_A).st64(FRAME_CLOSURE);
        // the copied values, pushed: `r` of them, from `addr` on
        self.get(L_ADDR)
            .ld64(META)
            .i64c(32)
            .op(0x88)
            .op(0xa7)
            .i32c(3)
            .sub()
            .set(L_R);
        self.get(L_ADDR)
            .i32c(CLOSURE_COPIED as i32)
            .add()
            .set(L_ADDR);
        self.op(0x02).op(0x40).op(0x03).op(0x40); // block, loop
        self.get(L_R).op(0x45).op(0x0d).uleb(1);
        self.push_addr().get(L_ADDR).load().store();
        self.inc_sp();
        self.get(L_ADDR).i32c(8).add().set(L_ADDR);
        self.get(L_R).i32c(1).sub().set(L_R);
        self.op(0x0c).uleb(0).op(0x0b).op(0x0b);
        // the count and the registers
        self.set_reg32(R_FRAMES_LEN, |f| {
            f.reg32(R_FRAMES_LEN).i32c(1).add();
        });
        self.set_reg32(R_DEPTH, |f| {
            f.reg32(R_FRAMES_LEN);
        });
        self.set_reg32(R_SP, |f| {
            f.get(L_SP);
        });
        self.set_reg32(R_BP, |f| {
            f.get(L_KIND);
        });
        self.set_reg32(R_LP, |f| {
            f.get(L_SLOT).ld32(FRAME_LP);
        });
        self.set_reg32(R_ARGC, |f| {
            f.i32c(args as i32);
        });
        self.set_reg32(R_LITERALS, |f| {
            f.reg32(R_ARENA)
                .get(L_INDEX)
                .ld32(CTARGET_LITERAL_OFFSET)
                .add();
        });
        self.set_reg32(R_LITERAL_COUNT, |f| {
            f.get(L_INDEX).ld32(CTARGET_LITERAL_COUNT);
        });
        self.set_reg64(R_RECEIVER, |f| {
            f.get(L_B);
        });
        self.set_reg32(R_BLOCK, |f| {
            f.get(L_INDEX).ld32(CTARGET_BLOCK);
        });
        self.set_reg32(R_IC_KIND, |f| {
            f.i32c(0);
        });
        self.get(L_INDEX)
            .ld32(CTARGET_FUNCTION)
            .i32c(CALLEE)
            .add()
            .set(L_R)
    }
    /// Allocates `words` words (an i32 on the stack) in the arena: their
    /// first word's offset into `index` and address into `addr`, the length
    /// and the sequence number and count advanced (one object).
    fn bump(&mut self) -> &mut Func {
        self.reg32(R_WORDS_LEN).set(L_INDEX);
        self.reg32(R_ARENA)
            .get(L_INDEX)
            .i32c(3)
            .op(0x74)
            .add()
            .set(L_ADDR);
        // the words count was on the stack: add it to the length
        self.set(L_R);
        self.set_reg32(R_WORDS_LEN, |f| {
            f.get(L_INDEX).get(L_R).add();
        });
        // no hash yet: the hash word is 0 (the allocation still counts)
        self.get(L_ADDR).i64c(0).st64(8);
        self.set_reg64(R_SEQ, |f| {
            f.reg64(R_SEQ).i64c(1).op(0x7c);
        });
        self.set_reg32(R_ALLOCATED, |f| {
            f.reg32(R_ALLOCATED).i32c(1).add();
        })
    }
    /// The `basicNew` (no arguments) or Array `new:` (one) of the hit entry
    /// (after [`Func::site_hit`]: the class in `a`, the slot count in
    /// `slot`, the meta word but for the space and size in `object`), made
    /// here as the native makes it, when there is room in the arena and
    /// below the safepoint (and for `new:`, a SmallInteger size from 0 to
    /// [`MAX_INLINE_ARRAY`]): the object replaces the receiver and the
    /// argument (ordinary) or the argument (else), or is pushed; then `br`
    /// to the send's end. Otherwise nothing.
    fn make_new(&mut self, args: u32, ordinary: bool) -> &mut Func {
        if args == 1 {
            // the size, in `slot`
            self.slot_addr(|f| {
                f.get(L_SP).i32c(1).sub();
            });
            self.load().set(L_B);
            self.get(L_B).i64c(1).op(0x83).op(0x50); // a SmallInteger
            self.get(L_B).i64c(0).op(0x59).op(0x71); // ge_s 0
            self.get(L_B).i64c(MAX_INLINE_ARRAY << 1).op(0x57).op(0x71); // le_s
            self.get(L_B).i64c(1).op(0x87).op(0xa7).set(L_SLOT);
        } else {
            self.i32c(1);
        }
        self.reg32(R_WORDS_LEN)
            .get(L_SLOT)
            .add()
            .i32c(3)
            .add()
            .reg32(R_WORDS_CAP)
            .op(0x4d)
            .op(0x71);
        self.reg64(R_SEQ)
            .i64c(1)
            .op(0x7c)
            .reg64(R_SAFEPOINT_AT)
            .op(0x54)
            .op(0x71);
        self.op(0x04).op(0x40);
        self.get(L_SLOT).i32c(3).add().bump();
        self.get(L_ADDR).get(L_A).st64(0);
        self.get(L_ADDR).get(L_OBJECT).reg64(R_SPACE).op(0x84);
        if args == 1 {
            self.get(L_SLOT).op(0xad).i64c(32).op(0x86).op(0x84);
        }
        self.st64(META);
        // the slots, nil
        self.get(L_ADDR).i32c(SLOTS as i32).add().set(L_ADDR);
        self.op(0x02).op(0x40).op(0x03).op(0x40);
        self.get(L_SLOT).op(0x45).op(0x0d).uleb(1);
        self.get(L_ADDR).get(L_NIL).st64(0);
        self.get(L_ADDR).i32c(8).add().set(L_ADDR);
        self.get(L_SLOT).i32c(1).sub().set(L_SLOT);
        self.op(0x0c).uleb(0).op(0x0b).op(0x0b);
        // the reference, in `r64`, where the answer goes
        self.get(L_INDEX)
            .op(0xad)
            .i64c(1)
            .op(0x86)
            .i64c(1)
            .op(0x84)
            .set(L_R64);
        match (args, ordinary) {
            (0, false) => {
                self.push_addr().get(L_R64).store();
                self.inc_sp();
            }
            (1, true) => {
                self.dec_sp();
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                self.get(L_R64).store();
            }
            _ => {
                self.slot_addr(|f| {
                    f.get(L_SP).i32c(1).sub();
                });
                self.get(L_R64).store();
            }
        }
        self.op(0x0c).uleb(3); // br: the send's end
        self.op(0x0b)
    }
    /// Makes the closure of the `PUSH_CLOSURE` here (`copied` values from
    /// the stack, `num_args` arguments, its body at `initial_ip`) in the
    /// arena, as the VM's `compiled_closure` does, when there is room in the
    /// arena and below the safepoint, the frame's activation handle made
    /// first if it has none: the copied values replaced by the closure,
    /// leaving 1; else 0, nothing done.
    fn make_closure(&mut self, copied: u32, num_args: u32, initial_ip: usize) -> &mut Func {
        // the top frame, in `slot`; its handle, in `meta`
        self.reg32(R_FRAMES_LEN)
            .i32c(1)
            .sub()
            .frame_addr()
            .set(L_SLOT);
        self.get(L_SLOT).ld64(FRAME_ACTIVATION).set(L_META);
        // words: the closure's, and a handle's if need be
        self.get(L_META)
            .op(0x50)
            .op(0x04)
            .op(0x7f)
            .i32c(11)
            .op(0x05)
            .i32c(0)
            .op(0x0b)
            .i32c(6 + copied as i32)
            .add()
            .set(L_KIND);
        self.reg32(R_WORDS_LEN)
            .get(L_KIND)
            .add()
            .reg32(R_WORDS_CAP)
            .op(0x4d); // le_u
        // sequence numbers: 2 with a handle, else 1, below the safepoint
        self.reg64(R_SEQ)
            .i64c(2)
            .op(0x7c)
            .reg64(R_SAFEPOINT_AT)
            .op(0x54) // i64.lt_u: seq + 2 <u safepoint_at
            .op(0x71);
        // the copied values on the frame's stack
        self.get(L_SP)
            .i32c(copied as i32)
            .sub()
            .get(L_LP)
            .op(0x4e) // ge_s
            .op(0x71);
        self.op(0x04).op(0x7f); // if (result i32)
        // the frame's activation handle: frame (serial), method, receiver,
        // closure (or nil), not detached
        self.get(L_META).op(0x50).op(0x04).op(0x40);
        self.i32c(11).bump();
        self.get(L_ADDR).reg64(R_ACTIVATION_CLASS).st64(0);
        self.get(L_ADDR).reg64(R_ACTIVATION_META).st64(META);
        self.get(L_ADDR)
            .get(L_SLOT)
            .ld64(FRAME_SERIAL)
            .i64c(1)
            .op(0x86)
            .st64(SLOTS);
        self.get(L_ADDR)
            .get(L_SLOT)
            .ld64(FRAME_METHOD)
            .st64(SLOTS + 8);
        self.get(L_ADDR)
            .get(L_SLOT)
            .ld64(FRAME_RECEIVER)
            .st64(SLOTS + 16);
        self.get(L_ADDR);
        self.get(L_SLOT).ld64(FRAME_CLOSURE).set(L_R64);
        self.get(L_R64)
            .op(0x50)
            .op(0x04)
            .op(0x7e)
            .get(L_NIL)
            .op(0x05)
            .get(L_R64)
            .op(0x0b);
        self.st64(SLOTS + 24);
        for k in 4..8 {
            self.get(L_ADDR).i64c(0).st64(SLOTS + 8 * k);
        }
        // its reference, into the frame
        self.get(L_INDEX)
            .op(0xad)
            .i64c(1)
            .op(0x86)
            .i64c(1)
            .op(0x84)
            .set(L_META);
        self.get(L_SLOT).get(L_META).st64(FRAME_ACTIVATION);
        self.op(0x0b);
        // the closure: defining handle, first instruction, argument count,
        // the copied values
        self.i32c(6 + copied as i32).bump();
        self.get(L_ADDR).reg64(R_CLOSURE_CLASS).st64(0);
        self.get(L_ADDR)
            .reg64(R_CLOSURE_META)
            .i64c(i64::from(3 + copied) << 32)
            .op(0x7c)
            .st64(META);
        self.get(L_ADDR).get(L_META).st64(SLOTS);
        self.get(L_ADDR)
            .i64c((initial_ip as i64) << 1)
            .st64(SLOTS + 8);
        self.get(L_ADDR)
            .i64c(i64::from(num_args) << 1)
            .st64(SLOTS + 16);
        for k in 0..copied {
            self.get(L_ADDR);
            self.slot_addr(|f| {
                f.get(L_SP).i32c((copied - k) as i32).sub();
            });
            self.load().st64(SLOTS + 24 + 8 * k);
        }
        // the copied values replaced by the closure
        if copied > 0 {
            self.get(L_SP).i32c(copied as i32).sub().set(L_SP);
        }
        self.push_addr()
            .get(L_INDEX)
            .op(0xad)
            .i64c(1)
            .op(0x86)
            .i64c(1)
            .op(0x84)
            .store();
        self.inc_sp();
        self.i32c(1);
        self.op(0x05).i32c(0).op(0x0b)
    }
    /// The local return of the value in `r64`, when a compiled caller in
    /// this wasm stack takes it (the frame is deeper than the one the host
    /// entered, which returns to the host, which must adopt the stack): the
    /// frame popped here, its activation handle (if any) marked dead, as
    /// the VM's `local_return` does, and [`DONE`] returned; else on to the
    /// runtime's `ret`.
    fn pop_frame(&mut self) -> &mut Func {
        self.reg32(R_FRAMES_LEN)
            .i32c(1)
            .sub()
            .frame_addr()
            .set(L_ADDR);
        // deeper than the frame the host entered: a compiled caller below,
        // in this wasm stack, takes the answer
        self.reg32(R_FRAMES_LEN).reg32(R_JIT_BASE).op(0x4b); // gt_u
        self.op(0x04).op(0x40);
        // An activation handle, if any, is dead: its frame field
        // SmallInteger -1 (no barrier: not a reference).
        self.get(L_ADDR).ld64(FRAME_ACTIVATION).set(L_META);
        self.get(L_META).op(0x50).op(0x45); // a handle
        self.op(0x04).op(0x40);
        self.reg32(R_ARENA)
            .get(L_META)
            .i64c(1)
            .op(0x88)
            .op(0xa7)
            .i32c(3)
            .op(0x74)
            .add()
            .i64c(-2)
            .st64(ACTIVATION_FRAME);
        self.op(0x0b);
        self.slot_addr(|f| {
            f.get(L_BP);
        });
        self.get(L_R64).store();
        self.set_reg32(R_SP, |f| {
            f.get(L_BP).i32c(1).add();
        });
        self.set_reg32(R_FRAMES_LEN, |f| {
            f.reg32(R_FRAMES_LEN).i32c(1).sub();
        });
        self.set_reg32(R_DEPTH, |f| {
            f.reg32(R_FRAMES_LEN);
        });
        self.set_reg32(R_IC_KIND, |f| {
            f.i32c(0);
        });
        self.i32c(DONE).op(0x0f);
        self.op(0x0b)
    }
    /// A send of literal `literal` by `rule` with `args` arguments, the
    /// instruction continuing at `next`: through its inline cache
    /// where it has one, else the runtime's `send`; then `after_call`.
    fn send(&mut self, rule: i32, literal: u32, args: u32, next: usize) -> &mut Func {
        // A common send is an ordinary send of its selector (the
        // runtime tries SmallInteger's fast paths first).
        // Every rule: ordinary, self, super, implicit, common, outer.
        let cacheable = rule >= RULE_ORDINARY;
        let site = if cacheable {
            Some(self.new_site())
        } else {
            None
        };
        if let Some(g) = site {
            let ordinary = matches!(rule, RULE_ORDINARY | RULE_COMMON);
            self.op(0x02).op(0x40); // block: the send's end
            self.op(0x02).op(0x40); // block: after the call
            self.site_hit(g, ordinary, args);
            if args <= 1 {
                // An accessor, read or written here.
                self.get(L_KIND).i32c(IC_ACCESSOR as i32).op(0x46);
                self.op(0x04).op(0x7f);
                self.accessor_ok(args == 1);
                self.op(0x05).i32c(0).op(0x0b);
                self.op(0x04).op(0x40);
                self.accessor(ordinary, args == 1);
                self.op(0x0c).uleb(2); // br: the send's end
                self.op(0x0b);
            }
            if ordinary && args <= 3 {
                // A closure's `value`: its frame pushed here when the closure
                // has the body of either entry's target and there is room,
                // else the general send (which fills the cache with this
                // body's target).
                self.get(L_KIND).i32c(IC_CLOSURE as i32).op(0x46);
                self.op(0x04).op(0x40);
                self.push_closure_ok(args);
                self.op(0x04).op(0x40);
                self.push_closure_frame(args, next);
                self.op(0x0c).uleb(2); // br: after the call
                self.op(0x0b);
                // the second entry's target, if it is one
                let e = g + ENTRY_GLOBALS;
                self.gget(e + 3).i32c(IC_CLOSURE as i32).op(0x46);
                self.reg32(R_EPOCH).gget(e + 1).op(0x46).op(0x71);
                self.op(0x04).op(0x40);
                self.gget(e + 2).set(L_SLOT);
                self.push_closure_ok(args);
                self.op(0x04).op(0x40);
                self.push_closure_frame(args, next);
                self.op(0x0c).uleb(3); // br: after the call
                self.op(0x0b);
                self.op(0x0b);
                self.i32c(0).set(L_KIND);
                self.op(0x0b);
            }
            // A compiled method: its frame pushed here when there is room
            // and nothing else to do, else by `call`.
            self.get(L_KIND).i32c(IC_METHOD as i32).op(0x46);
            self.op(0x04).op(0x40);
            self.push_frame_ok();
            self.op(0x04).op(0x40);
            self.push_frame(args, ordinary, next);
            self.op(0x0c).uleb(2); // br: after the call
            self.op(0x0b);
            self.get(L_VM)
                .get(L_SLOT)
                .i32c(next as i32)
                .get(L_SP)
                .call(F_CALL)
                .set(L_R);
            // br_if: after the call, unless a miss (then the general send)
            self.get(L_R).i32c(MISS).op(0x47).op(0x0d).uleb(1);
            self.op(0x0b);
            if args == 0 && (rule == RULE_IMPLICIT || rule >= RULE_OUTER) {
                // An enclosing object's slot, read here.
                self.get(L_KIND).i32c(IC_OUTER_GETTER as i32).op(0x46);
                self.op(0x04).op(0x7f);
                self.get(L_OBJECT).set(L_A);
                self.object_at_a();
                self.accessor_ok(false);
                self.op(0x05).i32c(0).op(0x0b);
                self.op(0x04).op(0x40);
                self.accessor(false, false);
                self.op(0x0c).uleb(2); // br: the send's end
                self.op(0x0b);
            }
            if args == 1 {
                // `==`, SmallInteger's bit operations and the comparisons of
                // numbers, here (the kinds 7 to 19 behind one test).
                self.get(L_KIND)
                    .i32c(IC_IDENTICAL as i32)
                    .sub()
                    .i32c((IC_GE - IC_IDENTICAL + 1) as i32)
                    .op(0x49); // lt_u
                self.op(0x04).op(0x40);
                for kind in [
                    IC_IDENTICAL,
                    IC_BIT_AND,
                    IC_BIT_OR,
                    IC_BIT_XOR,
                    IC_LT,
                    IC_GT,
                    IC_LE,
                    IC_GE,
                ] {
                    self.get(L_KIND).i32c(kind as i32).op(0x46);
                    self.op(0x04).op(0x40);
                    self.binary(kind, ordinary);
                    self.op(0x0c).uleb(3); // br: the send's end
                    self.op(0x0b);
                }
                self.op(0x0b);
            }
            // A method answering its receiver, answered here.
            self.get(L_KIND).i32c(IC_RETURN_SELF as i32).op(0x46);
            self.op(0x04).op(0x40);
            if args > 0 {
                self.get(L_SP).i32c(args as i32).sub().set(L_SP);
            }
            if !ordinary {
                self.push_addr().get(L_A).store();
                self.inc_sp();
            }
            self.op(0x0c).uleb(2); // br: the send's end
            self.op(0x0b);
            if args <= 1 {
                // `basicNew` or `new:` of an Array, made here.
                let kind = if args == 0 {
                    IC_BASIC_NEW
                } else {
                    IC_NEW_ARRAY
                };
                self.get(L_KIND).i32c(kind as i32).op(0x46);
                self.op(0x04).op(0x40);
                self.make_new(args, ordinary);
                self.op(0x0b);
            }
            if args <= 2 {
                // An indexed native, run here.
                let kinds: &[u32] = match args {
                    0 => &[IC_SIZE],
                    1 => &[IC_AT, IC_BYTES_AT],
                    _ => &[IC_AT_PUT],
                };
                for &kind in kinds {
                    self.get(L_KIND).i32c(kind as i32).op(0x46);
                    self.op(0x04).op(0x7f);
                    self.indexed_ok(kind);
                    self.op(0x05).i32c(0).op(0x0b);
                    self.op(0x04).op(0x40);
                    self.indexed(kind, ordinary);
                    self.op(0x0c).uleb(2); // br: the send's end
                    self.op(0x0b);
                }
            }
        }
        self.get(L_VM)
            .i32c(rule)
            .i32c(literal as i32)
            .i32c(args as i32)
            .i32c(next as i32)
            .get(L_SP)
            .call(F_SEND)
            .set(L_R);
        if let Some(g) = site {
            // Fill the cache from what the send found, before a
            // callee runs (its sends change the registers).
            // The first entry if it is empty, stale or of this
            // class and slot, else the second (a closure send's second
            // body, of the same class, goes there).
            self.reg32(R_IC_KIND).op(0x04).op(0x40);
            self.gget(g + 3).op(0x45); // kind == 0
            self.gget(g + 1).reg32(R_EPOCH).op(0x47).op(0x72); // | epoch != cached
            self.gget(g).reg64(R_IC_CLASS).op(0x51); // | class == cached
            self.gget(g + 2).reg32(R_IC_SLOT).op(0x46).op(0x71); // & slot == cached
            self.op(0x72);
            self.op(0x04).op(0x40);
            self.fill_entry(g);
            self.op(0x05);
            self.fill_entry(g + ENTRY_GLOBALS);
            self.op(0x0b);
            self.op(0x0b);
            self.op(0x0b); // end block: after the call
        }
        self.after_call();
        if site.is_some() {
            self.op(0x0b); // end block: the send's end
        }
        self
    }
    /// The slow path of inline common selector `k`: the send
    /// (through an inline cache, if `site`), then on to `next` (from inside
    /// `extra` `if`s).
    fn common_and_go_on(
        &mut self,
        k: u8,
        next: usize,
        site: bool,
        len: usize,
        extra: u32,
        jump: &dyn Fn(&mut Func, usize, u32),
    ) -> &mut Func {
        if site {
            self.send(RULE_COMMON, u32::from(k), 1, next);
        } else {
            self.get(L_VM)
                .i32c(RULE_COMMON)
                .i32c(i32::from(k))
                .i32c(0)
                .i32c(next as i32)
                .get(L_SP)
                .call(F_SEND)
                .set(L_R);
            self.after_call();
        }
        if next < len {
            jump(self, next, extra);
        } else {
            self.interpret(next);
        }
        self
    }
    /// Pushes the value a return answers (popping it, for `^` of the top).
    fn ret_value(&mut self, r: Ret) -> &mut Func {
        match r {
            Ret::Top => self.dec_sp().top_value_at_sp(),
            Ret::Self_ => self.get(L_SELF),
            Ret::Nil => self.get(L_NIL),
            Ret::True => self.get(L_TRUE),
            Ret::False => self.get(L_FALSE),
        }
    }
    /// Leaves the frame at instruction `ip` for the interpreter, and returns.
    fn interpret(&mut self, ip: usize) -> &mut Func {
        self.get(L_VM)
            .i32c(ip as i32)
            .get(L_SP)
            .call(F_INTERPRET)
            .op(0x0f)
    }
}

fn uleb(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn name(out: &mut Vec<u8>, s: &str) {
    uleb(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

/// Names a compiled module's function `name` (a custom `name` section),
/// for profilers and debuggers; the code does not change.
pub fn add_name(module: &mut Vec<u8>, function_name: &str) {
    let mut names = Vec::new();
    uleb(&mut names, 1);
    uleb(&mut names, IMPORTED);
    name(&mut names, function_name);
    let mut content = Vec::new();
    name(&mut content, "name");
    content.push(1); // function names
    uleb(&mut content, names.len() as u32);
    content.extend_from_slice(&names);
    section(module, 0, &content);
}

fn section(out: &mut Vec<u8>, id: u8, content: &[u8]) {
    out.push(id);
    uleb(out, content.len() as u32);
    out.extend_from_slice(content);
}

/// The module around the function body `body` (its locals not included).
fn module(body: &[u8], sites: u32) -> Vec<u8> {
    const I32: u8 = 0x7f;
    const I64: u8 = 0x7e;
    let mut m = alloc::vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    // Types, in the order of the T_ constants.
    let types: [(&[u8], &[u8]); 5] = [
        (&[I32, I32, I32], &[I32]),
        (&[I32, I32, I32, I32, I32, I32], &[I32]),
        (&[I32, I64, I32], &[I32]),
        (&[I32, I32, I32, I32], &[I32]),
        (&[I32, I64, I32, I32], &[I32]),
    ];
    let mut t = Vec::new();
    uleb(&mut t, types.len() as u32);
    for (params, results) in types {
        t.push(0x60);
        uleb(&mut t, params.len() as u32);
        t.extend_from_slice(params);
        uleb(&mut t, results.len() as u32);
        t.extend_from_slice(results);
    }
    section(&mut m, 1, &t);
    // Imports: the functions (in the order of the F_ constants), the
    // memory, the table.
    let functions: [(&str, u32); IMPORTED as usize] = [
        ("send", T_SEND),
        ("ret", T_RET),
        ("interpret", T_RUN),
        ("step", T_RUN),
        ("tick", T_RUN),
        ("call", T_CALL),
        ("closure", T_RUN),
        ("nlr", T_NLR),
    ];
    let mut im = Vec::new();
    uleb(&mut im, functions.len() as u32 + 2);
    for (n, ty) in functions {
        name(&mut im, "env");
        name(&mut im, n);
        im.push(0x00);
        uleb(&mut im, ty);
    }
    name(&mut im, "env");
    name(&mut im, "memory");
    im.extend_from_slice(&[0x02, 0x00, 0x00]); // memory, min 0
    name(&mut im, "env");
    name(&mut im, "table");
    im.extend_from_slice(&[0x01, 0x70, 0x00, 0x00]); // table funcref, min 0
    section(&mut m, 2, &im);
    // The one function, `run`.
    section(&mut m, 3, &[0x01, T_RUN as u8]);
    // Globals: each site's two entries, each a class (i64), epoch, slot
    // and kind (i32), and object (i64), mutable, 0.
    if sites > 0 {
        let mut g = Vec::new();
        uleb(&mut g, SITE_GLOBALS * sites);
        for _ in 0..2 * sites {
            g.extend_from_slice(&[I64, 0x01, 0x42, 0x00, 0x0b]);
            for _ in 1..ENTRY_GLOBALS - 1 {
                g.extend_from_slice(&[I32, 0x01, 0x41, 0x00, 0x0b]);
            }
            g.extend_from_slice(&[I64, 0x01, 0x42, 0x00, 0x0b]);
        }
        section(&mut m, 6, &g);
    }
    let mut ex = Vec::new();
    uleb(&mut ex, 1);
    name(&mut ex, "run");
    ex.push(0x00);
    uleb(&mut ex, IMPORTED);
    section(&mut m, 7, &ex);
    // Code: the locals after the parameters, i32s then i64s.
    let mut code = Vec::new();
    code.extend_from_slice(&[
        0x04,
        I32_LOCALS,
        I32,
        I64_LOCALS,
        I64,
        MORE_I32_LOCALS,
        I32,
        0x01,
        I64,
    ]);
    code.extend_from_slice(body);
    code.push(0x0b); // end of function
    let mut c = Vec::new();
    uleb(&mut c, 1);
    uleb(&mut c, code.len() as u32);
    c.extend_from_slice(&code);
    section(&mut m, 10, &c);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_method_is_not_compiled() {
        // A closure whose body runs past the method's end.
        assert!(compile(&[op::PUSH_CLOSURE, 0, 9, 0], &|_| true).is_none());
        // A jump into the middle of an instruction.
        assert!(compile(&[17, op::PUSH_INTEGER, 1, 0, op::RETURN_TOP], &|_| true).is_none());
    }

    #[test]
    fn a_method_compiles_to_a_module_with_its_entries() {
        // ^self + 1
        let code = [op::PUSH_SELF, 162, 176, op::RETURN_TOP];
        let c = compile(&code, &|_| true).expect("compiles");
        assert_eq!(&c.module[..4], b"\0asm");
        // The start, and after the arithmetic (its slow path is a call).
        assert_eq!(c.entries, [0, 3]);
    }

    #[test]
    fn a_closure_body_is_an_entry() {
        // [^nil] value: push closure (0 copied, 0 args, 1 byte), then send.
        let code = [
            op::PUSH_CLOSURE,
            0,
            1,
            0,
            op::RETURN_NIL,
            64,
            op::RETURN_TOP,
        ];
        let c = compile(&code, &|_| true).expect("compiles");
        assert_eq!(c.entries, [0, 4, 5, 6]);
    }
}
