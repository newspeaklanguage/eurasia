// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The baseline compiler, stage 1 (a spike): V5 bytecode to a WebAssembly
//! module, for a subset of the instructions (newspeak/
//! EURASIA_COMPILER_DESIGN_2026-11-19.md section 7).
//!
//! Compiled code keeps the interpreter's frames: it runs on the VM's operand
//! stack in linear memory, and every call into the VM's runtime, and every
//! exit, leaves the frame as the interpreter would have it (its instruction
//! index, its stack height), so the interpreter can continue it at any of
//! those points.
//!
//! The module imports the VM's memory and function table and these runtime
//! functions (exported by the web host as `eurasia_rt_*`), each taking the
//! VM's address first:
//! - `stack_base(vm, headroom) -> i32`: the operand stack's address, with room
//!   for `headroom` more Values;
//! - `frame(vm, which) -> i32`: the stack height (0), the frame's receiver
//!   index (1), its first local (2) or its argument count (3);
//! - `receiver(vm) -> i64`, `value(vm, which) -> i64` (nil, true, false);
//! - `send(vm, rule, literal, args, next_ip, sp) -> i32`: makes a send as the
//!   interpreter does; answers [`DONE`] when it completed (its result on the
//!   stack), [`INTERPRET`], [`STOP`], or [`CALLEE`] plus the function index of
//!   a compiled method whose fresh frame it pushed;
//! - `ret(vm, value, sp) -> i32`: the interpreter's local return;
//! - `interpret(vm, ip, sp) -> i32`: leaves the frame at instruction `ip` for
//!   the interpreter;
//! - `due(vm) -> i32`: whether a safepoint is due.
//!
//! The function it exports, `run(vm) -> i32`, runs a fresh activation of the
//! method and answers [`DONE`] (the frame returned), [`INTERPRET`] or
//! [`STOP`].

use alloc::vec::Vec;

use crate::bytecode::{instruction_length, op};

/// The frame returned; its result is on the caller's stack.
pub const DONE: i32 = 0;
/// The interpreter continues the top frame.
pub const INTERPRET: i32 = 1;
/// The run stops: the VM holds the completion or the error.
pub const STOP: i32 = 2;
/// Added to a function index: a compiled callee's fresh frame was pushed.
pub const CALLEE: i32 = 3;

/// The send rules `send` takes.
pub const RULE_ORDINARY: i32 = 0;
/// A self send.
pub const RULE_SELF: i32 = 1;
/// An implicit-receiver send.
pub const RULE_IMPLICIT: i32 = 3;

// Imported functions, in import order.
const F_STACK_BASE: u32 = 0;
const F_FRAME: u32 = 1;
const F_RECEIVER: u32 = 2;
const F_VALUE: u32 = 3;
const F_SEND: u32 = 4;
const F_RET: u32 = 5;
const F_INTERPRET: u32 = 6;
const F_DUE: u32 = 7;
const IMPORTED: u32 = 8;

// Types.
const T_RUN: u32 = 0; // (i32) -> i32
const T_I32_I32: u32 = 1; // (i32, i32) -> i32
const T_I32_I64: u32 = 2; // (i32) -> i64
const T_I32I32_I64: u32 = 3; // (i32, i32) -> i64
const T_SEND: u32 = 4; // (i32 x 6) -> i32
const T_RET: u32 = 5; // (i32, i64, i32) -> i32
const T_INTERPRET: u32 = 6; // (i32, i32, i32) -> i32

// Locals of `run`.
const L_VM: u32 = 0;
const L_BASE: u32 = 1;
const L_SP: u32 = 2;
const L_BP: u32 = 3;
const L_LP: u32 = 4;
const L_ARGC: u32 = 5;
const L_PC: u32 = 6;
const L_R: u32 = 7;
const L_A: u32 = 8;
const L_B: u32 = 9;
const L_SELF: u32 = 10;
const L_NIL: u32 = 11;
const L_TRUE: u32 = 12;
const L_FALSE: u32 = 13;

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

/// Decodes `code` into (index, length, instruction), or `None` if it uses an
/// instruction outside the spike's subset.
fn decode(code: &[u8]) -> Option<Vec<(usize, usize, Ins)>> {
    let mut out = Vec::new();
    let mut ip = 0;
    while ip < code.len() {
        let o = code[ip];
        let len = instruction_length(o);
        if ip + len > code.len() {
            return None;
        }
        let b1 = if len > 1 { code[ip + 1] } else { 0 };
        let b2 = if len > 2 { code[ip + 2] } else { 0 };
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
            op::PUSH_PARAMETER => Ins::PushParam(u32::from(b1)),
            op::PUSH_LOCAL => Ins::PushLocal(u32::from(b1)),
            op::POP_INTO_LOCAL => Ins::PopIntoLocal(u32::from(b1)),
            op::STORE_INTO_LOCAL => Ins::StoreIntoLocal(u32::from(b1)),
            op::BRANCH_BACK => Ins::Jump(next.checked_sub(wide)?),
            op::BRANCH => Ins::Jump(next + wide),
            op::BRANCH_IF_TRUE => Ins::Branch(true, next + wide),
            op::BRANCH_IF_FALSE => Ins::Branch(false, next + wide),
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
            _ => return None,
        };
        out.push((ip, len, ins));
        ip = next;
    }
    Some(out)
}

/// Compiles `code` (a method's bytecode) to a module, or answers `None` if
/// it uses an instruction outside the subset. `arith_ok(k)` says whether the
/// inline SmallInteger meaning of common selector `k` is SmallInteger's
/// method (as the interpreter's fast path checks).
pub fn compile(code: &[u8], arg_count: u32, arith_ok: &dyn Fn(usize) -> bool) -> Option<Vec<u8>> {
    let ins = decode(code)?;
    for &(_, _, i) in &ins {
        match i {
            Ins::Arith(k) if !arith_ok(usize::from(k)) => return None,
            // Parameters are numbered from 1 (the interpreter refuses others).
            Ins::PushParam(p) if p == 0 || p > arg_count => return None,
            _ => {}
        }
    }
    // Basic blocks: their first instructions' indices.
    let mut leaders: Vec<usize> = alloc::vec![0];
    for &(ip, len, i) in &ins {
        match i {
            Ins::Jump(t) | Ins::Branch(_, t) => {
                leaders.push(t);
                leaders.push(ip + len);
            }
            Ins::Return(_) => leaders.push(ip + len),
            _ => {}
        }
    }
    leaders.retain(|&l| l < code.len());
    leaders.sort_unstable();
    leaders.dedup();
    // Every jump target must be an instruction's start.
    for &l in &leaders {
        if !ins.iter().any(|&(ip, _, _)| ip == l) {
            return None;
        }
    }
    let block_of = |ip: usize| leaders.binary_search(&ip).ok();
    // Room on the operand stack: no more than every pushing instruction.
    let headroom = ins.len() as i32 + 8;

    let mut f = Func::default();
    // Entry: the frame's layout and the constants.
    f.get(L_VM).i32c(headroom).call(F_STACK_BASE).set(L_BASE);
    for (local, which) in [(L_SP, 0), (L_BP, 1), (L_LP, 2), (L_ARGC, 3)] {
        f.get(L_VM).i32c(which).call(F_FRAME).set(local);
    }
    f.get(L_VM).call(F_RECEIVER).set(L_SELF);
    for (local, which) in [(L_NIL, 0), (L_TRUE, 1), (L_FALSE, 2)] {
        f.get(L_VM).i32c(which).call(F_VALUE).set(local);
    }
    f.i32c(0).set(L_PC);
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
        let to_loop = (n - 1 - k as u32) as u32;
        // `extra`: the `if`s around the jump.
        let jump = |f: &mut Func, target: usize, extra: u32| {
            f.i32c(block_of(target).expect("a leader") as i32).set(L_PC);
            f.op(0x0c).uleb(to_loop + extra); // br $dispatch
        };
        match i {
            Ins::PushParam(p) => {
                // stack[bp + 1 + (argc - p)]
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
            Ins::Jump(t) => jump(&mut f, t, 0),
            Ins::Branch(condition, t) => {
                // a = top; not a Boolean: the interpreter (which sends
                // #nonBooleanReceiver:).
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
                f.interpret(ip);
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
                // (low bit 0), else the interpreter.
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
                f.interpret(ip);
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
                        f.interpret(ip);
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
                f.get(L_VM)
                    .i32c(rule)
                    .i32c(literal as i32)
                    .i32c(args as i32)
                    .i32c(next as i32)
                    .get(L_SP)
                    .call(F_SEND)
                    .set(L_R);
                // A compiled callee: run it here.
                f.get(L_R).i32c(CALLEE).op(0x4e); // i32.ge_s
                f.op(0x04).op(0x40);
                f.get(L_VM).get(L_R).i32c(CALLEE).sub();
                f.op(0x11).uleb(T_RUN).uleb(0); // call_indirect
                f.set(L_R);
                f.op(0x0b);
                // Anything but a completed send: leave it to the caller.
                f.get(L_R).op(0x04).op(0x40);
                f.get(L_R).op(0x0f); // return r
                f.op(0x0b);
                // A safepoint due: the interpreter's loop takes it.
                f.get(L_VM).call(F_DUE).op(0x04).op(0x40);
                f.i32c(INTERPRET).op(0x0f);
                f.op(0x0b);
                // Reload the stack's address and height.
                f.get(L_VM).i32c(headroom).call(F_STACK_BASE).set(L_BASE);
                f.get(L_VM).i32c(0).call(F_FRAME).set(L_SP);
            }
            Ins::Return(r) => {
                f.get(L_VM);
                match r {
                    Ret::Top => {
                        f.dec_sp();
                        f.top_value_at_sp();
                    }
                    Ret::Self_ => {
                        f.get(L_SELF);
                    }
                    Ret::Nil => {
                        f.get(L_NIL);
                    }
                    Ret::True => {
                        f.get(L_TRUE);
                    }
                    Ret::False => {
                        f.get(L_FALSE);
                    }
                }
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
    f.i32c(INTERPRET).op(0x0f);
    f.op(0x0b); // end loop
    f.i32c(INTERPRET);
    Some(module(&f.body))
}

/// The scratch i64 for arithmetic results.
const L_R64: u32 = 14;

/// A function body under construction.
#[derive(Default)]
struct Func {
    body: Vec<u8>,
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
    fn store(&mut self) -> &mut Func {
        self.op(0x37).uleb(3).uleb(0) // i64.store align 8
    }
    fn inc_sp(&mut self) -> &mut Func {
        self.get(L_SP).i32c(1).add().set(L_SP)
    }
    fn dec_sp(&mut self) -> &mut Func {
        self.get(L_SP).i32c(1).sub().set(L_SP)
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

fn section(out: &mut Vec<u8>, id: u8, content: &[u8]) {
    out.push(id);
    uleb(out, content.len() as u32);
    out.extend_from_slice(content);
}

/// The module around the function body `body` (its locals not included).
fn module(body: &[u8]) -> Vec<u8> {
    const I32: u8 = 0x7f;
    const I64: u8 = 0x7e;
    let mut m = alloc::vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    // Types.
    let types: [(&[u8], &[u8]); 7] = [
        (&[I32], &[I32]),
        (&[I32, I32], &[I32]),
        (&[I32], &[I64]),
        (&[I32, I32], &[I64]),
        (&[I32, I32, I32, I32, I32, I32], &[I32]),
        (&[I32, I64, I32], &[I32]),
        (&[I32, I32, I32], &[I32]),
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
    // Imports: the functions, the memory, the table.
    let functions: [(&str, u32); IMPORTED as usize] = [
        ("stack_base", T_I32_I32),
        ("frame", T_I32_I32),
        ("receiver", T_I32_I64),
        ("value", T_I32I32_I64),
        ("send", T_SEND),
        ("ret", T_RET),
        ("interpret", T_INTERPRET),
        ("due", T_RUN),
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
    let mut ex = Vec::new();
    uleb(&mut ex, 1);
    name(&mut ex, "run");
    ex.push(0x00);
    uleb(&mut ex, IMPORTED);
    section(&mut m, 7, &ex);
    // Code: locals 1..=7 i32, 8..=14 i64 (base sp bp lp argc pc r; a b self
    // nil true false r64).
    let mut code = Vec::new();
    code.extend_from_slice(&[0x02, 0x07, I32, 0x07, I64]);
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
    fn a_method_outside_the_subset_is_not_compiled() {
        // push closure: not in the spike's subset.
        assert!(compile(&[op::PUSH_CLOSURE, 0, 0, 0], 0, &|_| true).is_none());
        // push parameter 0 does not exist.
        assert!(compile(&[112, op::RETURN_TOP], 1, &|_| true).is_none());
    }

    #[test]
    fn a_straight_method_compiles_to_a_module() {
        // ^self + 1, as a self send of #foo: then return.
        let code = [op::PUSH_SELF, 162, 176, op::RETURN_TOP];
        let m = compile(&code, 0, &|_| true).expect("compiles");
        assert_eq!(&m[..4], b"\0asm");
    }
}
