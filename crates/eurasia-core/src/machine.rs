// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! A checkpoint's machine state (M7; snapshot format, section 4, kind 4):
//! everything a VM holds outside its heap and roles, so that a restored VM
//! continues exactly where the checkpoint was taken. Values in it refer to
//! the heap it was saved with.

use alloc::string::String;
use alloc::vec::Vec;

use crate::value::Value;

/// One frame of the stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameState {
    /// Its serial number (activation handles name frames by it).
    pub serial: u64,
    /// The method running.
    pub method: Value,
    /// The method receiver.
    pub receiver: Value,
    /// Index of the message receiver in the operand stack.
    pub bp: u64,
    /// The number of arguments.
    pub arg_count: u64,
    /// Index of the first local.
    pub lp: u64,
    /// The next instruction.
    pub ip: u64,
    /// Its activation handle, if one was made.
    pub activation: Option<Value>,
    /// For a closure frame, the closure.
    pub closure: Option<Value>,
}

/// One actor of the cohort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorState {
    /// Its message loop.
    pub message_loop: Value,
    /// Its packets, oldest first.
    pub mailbox: Vec<Value>,
    /// Its next timer (monotonic nanoseconds; 0: none).
    pub wakeup: u64,
    /// Whether an idle callback ran in it (its loop must run).
    pub poked: bool,
}

/// The machine state of a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineState {
    /// The operand stack, shared by every frame.
    pub stack: Vec<Value>,
    /// The frames, oldest first.
    pub frames: Vec<FrameState>,
    /// The actors.
    pub actors: Vec<ActorState>,
    /// The running actor.
    pub current: u64,
    /// The expat table: each closure with its owning actor.
    pub expats: Vec<(Value, u32)>,
    /// The VM's own selector strings, by name.
    pub symbols: Vec<(String, Value)>,
    /// The next frame serial number.
    pub next_serial: u64,
    /// The allocation counter after the last collection.
    pub gc_counter: u64,
    /// Allocations after which the next collection is due.
    pub gc_budget: u64,
    /// Collections so far.
    pub collections: u64,
    /// A turn ended with `finish:`, naming this timer (0: none).
    pub finished_turn: Option<u64>,
    /// Whether the image runs a message loop.
    pub has_loop: bool,
    /// Commits so far.
    pub program_version: u64,
    /// The checkpoint policy: allocations between checkpoints (0: none).
    pub checkpoint_every: u64,
    /// The allocation counter at the last checkpoint.
    pub last_checkpoint: u64,
}
