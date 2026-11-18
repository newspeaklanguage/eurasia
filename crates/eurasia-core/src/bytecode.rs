// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The instruction set: upstream V5 (`BYTECODES_V5_SPEC_2026-07-13.md`) with
//! Eurasia's refinements (implementation plan, section 2.4). Every
//! instruction's length follows from its first byte.
//!
//! The assembler half of this module builds bytecode for tests and for the
//! Rust-side image builder; the packager's code generator is the real source
//! of bytecode.

use alloc::vec::Vec;

/// The length of the instruction whose first byte is `op`.
pub fn instruction_length(op: u8) -> usize {
    match op {
        0..=215 => 1,
        216..=237 => 2,
        238..=253 => 3,
        254..=255 => 4,
    }
}

/// Opcodes with fixed meanings.
pub mod op {
    /// Push nil.
    pub const PUSH_NIL: u8 = 152;
    /// Push false.
    pub const PUSH_FALSE: u8 = 153;
    /// Push true.
    pub const PUSH_TRUE: u8 = 154;
    /// Push the method receiver.
    pub const PUSH_SELF: u8 = 155;
    /// Push the method's mixin.
    pub const PUSH_MIXIN: u8 = 156;
    /// Drop the top of stack.
    pub const POP: u8 = 158;
    /// Duplicate the top of stack.
    pub const DUP: u8 = 159;
    /// Local return of nil.
    pub const RETURN_NIL: u8 = 166;
    /// Local return of false.
    pub const RETURN_FALSE: u8 = 167;
    /// Local return of true.
    pub const RETURN_TRUE: u8 = 168;
    /// Local return of the receiver.
    pub const RETURN_SELF: u8 = 169;
    /// Local return of the top of stack.
    pub const RETURN_TOP: u8 = 170;
    /// Fresh nil-filled Array; one-byte size operand.
    pub const PUSH_NEW_ARRAY: u8 = 222;
    /// Array of the top `n` elements; one-byte size operand.
    pub const PUSH_NEW_ARRAY_WITH_ELEMENTS: u8 = 223;
    /// Push parameter, one-byte operand.
    pub const PUSH_PARAMETER: u8 = 228;
    /// Push local, one-byte operand.
    pub const PUSH_LOCAL: u8 = 229;
    /// Pop into local, one-byte operand.
    pub const POP_INTO_LOCAL: u8 = 230;
    /// Store into local, one-byte operand.
    pub const STORE_INTO_LOCAL: u8 = 231;
    /// Push the enclosing object at a depth.
    pub const PUSH_ENCLOSING_OBJECT: u8 = 233;
    /// Eventual send.
    pub const EVENTUAL_SEND: u8 = 239;
    /// Backward branch, 16-bit delta.
    pub const BRANCH_BACK: u8 = 240;
    /// Forward branch, 16-bit delta.
    pub const BRANCH: u8 = 241;
    /// Branch if true, 16-bit delta.
    pub const BRANCH_IF_TRUE: u8 = 242;
    /// Branch if false, 16-bit delta.
    pub const BRANCH_IF_FALSE: u8 = 243;
    /// Push indirect local.
    pub const PUSH_INDIRECT_LOCAL: u8 = 245;
    /// Pop into indirect local.
    pub const POP_INTO_INDIRECT_LOCAL: u8 = 246;
    /// Store into indirect local.
    pub const STORE_INTO_INDIRECT_LOCAL: u8 = 247;
    /// Push literal, 16-bit index.
    pub const PUSH_LITERAL: u8 = 248;
    /// Push a signed 16-bit integer.
    pub const PUSH_INTEGER: u8 = 249;
    /// Ordinary send, extended.
    pub const ORDINARY_SEND: u8 = 250;
    /// Self send, extended.
    pub const SELF_SEND: u8 = 251;
    /// Super send, extended.
    pub const SUPER_SEND: u8 = 252;
    /// Implicit-receiver send, extended.
    pub const IMPLICIT_RECEIVER_SEND: u8 = 253;
    /// Outer send.
    pub const OUTER_SEND: u8 = 254;
    /// Push a closure.
    pub const PUSH_CLOSURE: u8 = 255;
}

/// The kinds of send that name a selector by literal index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendKind {
    /// Receiver on the stack; public methods only.
    Ordinary,
    /// To self, lexically.
    SelfSend,
    /// Implicit receiver: lexically outward, then to self.
    ImplicitReceiver,
    /// To super.
    Super,
}

/// A small assembler for V5 bytecode.
#[derive(Default, Clone, Debug)]
pub struct Assembler {
    code: Vec<u8>,
}

impl Assembler {
    /// An empty assembler.
    pub fn new() -> Assembler {
        Assembler::default()
    }

    /// The bytecode.
    pub fn finish(self) -> Vec<u8> {
        self.code
    }

    /// Appends a raw byte.
    pub fn byte(&mut self, b: u8) -> &mut Assembler {
        self.code.push(b);
        self
    }

    /// Push literal `i`.
    pub fn push_literal(&mut self, i: u16) -> &mut Assembler {
        if i < 8 {
            self.byte(144 + i as u8)
        } else {
            let [lo, hi] = i.to_le_bytes();
            self.byte(op::PUSH_LITERAL).byte(lo).byte(hi)
        }
    }

    /// Push an integer in -32768..=32767.
    pub fn push_integer(&mut self, n: i16) -> &mut Assembler {
        match n {
            -1..=2 => self.byte((161 + n) as u8),
            _ => {
                let [lo, hi] = n.to_le_bytes();
                self.byte(op::PUSH_INTEGER).byte(lo).byte(hi)
            }
        }
    }

    /// Push parameter `i` in the reversed, one-based numbering: for a method of
    /// `n` arguments, the first parameter is `n`.
    pub fn push_parameter(&mut self, i: u8) -> &mut Assembler {
        if i < 8 {
            self.byte(112 + i)
        } else {
            self.byte(op::PUSH_PARAMETER).byte(i)
        }
    }

    /// Push local `i` (zero-based).
    pub fn push_local(&mut self, i: u8) -> &mut Assembler {
        if i < 8 {
            self.byte(120 + i)
        } else {
            self.byte(op::PUSH_LOCAL).byte(i)
        }
    }

    /// Pop into local `i`.
    pub fn pop_into_local(&mut self, i: u8) -> &mut Assembler {
        if i < 8 {
            self.byte(128 + i)
        } else {
            self.byte(op::POP_INTO_LOCAL).byte(i)
        }
    }

    /// A send of literal selector `selector` with `args` arguments.
    pub fn send(&mut self, kind: SendKind, selector: u16, args: u8) -> &mut Assembler {
        let short_base = match kind {
            SendKind::Ordinary => Some(64),
            SendKind::SelfSend => Some(80),
            SendKind::ImplicitReceiver => Some(96),
            SendKind::Super => None,
        };
        if let Some(base) = short_base
            && selector < 8
            && args < 2
        {
            return self.byte(base + (args << 3) + selector as u8);
        }
        let opcode = match kind {
            SendKind::Ordinary => op::ORDINARY_SEND,
            SendKind::SelfSend => op::SELF_SEND,
            SendKind::Super => op::SUPER_SEND,
            SendKind::ImplicitReceiver => op::IMPLICIT_RECEIVER_SEND,
        };
        assert!(selector <= 4095 && args <= 15, "send operand out of range");
        self.byte(opcode)
            .byte((selector & 0xff) as u8)
            .byte((args << 4) | (selector >> 8) as u8)
    }

    /// A common send (opcodes 176-207): entry `k` of the common-selector table.
    pub fn common_send(&mut self, k: u8) -> &mut Assembler {
        assert!(k < 32, "common-selector index out of range");
        self.byte(176 + k)
    }

    /// Return the top of stack.
    pub fn return_top(&mut self) -> &mut Assembler {
        self.byte(op::RETURN_TOP)
    }

    /// Non-local return of the top of stack (from inside a closure).
    pub fn non_local_return_top(&mut self) -> &mut Assembler {
        self.byte(175)
    }

    /// Push a closure with `copied` copied values (popped from the stack)
    /// and `args` parameters, whose body is `body`.
    pub fn push_closure(&mut self, copied: u8, args: u8, body: &[u8]) -> &mut Assembler {
        assert!(copied <= 15 && args <= 15, "closure operand out of range");
        let [lo, hi] = u16::try_from(body.len())
            .expect("closure body too long")
            .to_le_bytes();
        self.byte(op::PUSH_CLOSURE)
            .byte((copied << 4) | args)
            .byte(lo)
            .byte(hi);
        self.code.extend_from_slice(body);
        self
    }

    /// Branch forward by `delta` bytes if the popped value is false.
    pub fn branch_if_false(&mut self, delta: u16) -> &mut Assembler {
        let [lo, hi] = delta.to_le_bytes();
        self.byte(op::BRANCH_IF_FALSE).byte(lo).byte(hi)
    }
}

/// The common-selector table of V5 (opcodes 176-207), as (selector, arity).
pub const COMMON_SELECTORS: [(&str, u8); 32] = [
    ("+", 1),
    ("-", 1),
    ("*", 1),
    ("//", 1),
    ("\\\\", 1),
    ("<<", 1),
    (">>", 1),
    ("&", 1),
    ("|", 1),
    ("<", 1),
    (">", 1),
    ("<=", 1),
    (">=", 1),
    ("=", 1),
    ("new", 0),
    ("new:", 1),
    ("at:", 1),
    ("at:put:", 2),
    ("size", 0),
    ("value", 0),
    ("value:", 1),
    ("value:value:", 2),
    ("name", 0),
    ("name:", 1),
    ("add:", 1),
    ("hash", 0),
    ("yourself", 0),
    (",", 1),
    ("and:", 1),
    ("or:", 1),
    ("not", 0),
    ("basicNew", 0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_follow_the_first_byte() {
        assert_eq!(instruction_length(0), 1);
        assert_eq!(instruction_length(215), 1);
        assert_eq!(instruction_length(222), 2);
        assert_eq!(instruction_length(250), 3);
        assert_eq!(instruction_length(254), 4);
        assert_eq!(instruction_length(255), 4);
    }

    #[test]
    fn sends_use_short_forms_when_they_fit() {
        let mut a = Assembler::new();
        a.send(SendKind::Ordinary, 3, 1)
            .send(SendKind::SelfSend, 0, 0)
            .send(SendKind::Ordinary, 9, 2)
            .send(SendKind::Super, 1, 0);
        assert_eq!(a.finish(), [64 + 8 + 3, 80, 250, 9, 0x20, 252, 1, 0]);
    }

    #[test]
    fn extended_send_packs_selector_and_arity() {
        let mut a = Assembler::new();
        a.send(SendKind::Ordinary, 0x234, 15);
        assert_eq!(a.finish(), [250, 0x34, 0xf2]);
    }

    #[test]
    fn integers_use_quick_forms() {
        let mut a = Assembler::new();
        a.push_integer(-1)
            .push_integer(2)
            .push_integer(3)
            .push_integer(-300);
        assert_eq!(a.finish(), [160, 163, 249, 3, 0, 249, 0xd4, 0xfe]);
    }

    #[test]
    fn common_selector_table_matches_the_spec() {
        assert_eq!(COMMON_SELECTORS[0], ("+", 1));
        assert_eq!(COMMON_SELECTORS[13], ("=", 1));
        assert_eq!(COMMON_SELECTORS[27], (",", 1));
        assert_eq!(COMMON_SELECTORS[31], ("basicNew", 0));
    }
}
