// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The native catalog of kernel version 1 (operation list, section 3; rules
//! Native-Answer, Native-Tail and Native-Fail of `kernel-semantics.tex`).
//!
//! Every native is total: it answers, makes a defined tail send, or fails
//! with a reason. There are no fallback bodies. M1 implements the entries
//! below; the rest of the catalog arrives with M2.

use alloc::string::ToString;
use alloc::vec::Vec;

use crate::host::Host;
use crate::object::{Body, Heap, Space};
use crate::roles::{Role, Roles};
use crate::value::{SmallInteger, Value};

macro_rules! natives {
    ($($name:ident = $id:literal / $arity:literal: $doc:literal,)*) => {
        /// A catalog entry, numbered as in method headers.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u32)]
        pub enum Native {
            $(#[doc = $doc] $name = $id,)*
        }

        impl Native {
            /// The entry with catalog id `id`.
            pub fn from_id(id: u32) -> Option<Native> {
                match id {
                    $($id => Some(Native::$name),)*
                    _ => None,
                }
            }

            /// The number of arguments, not counting the receiver.
            pub fn arity(self) -> usize {
                match self {
                    $(Native::$name => $arity,)*
                }
            }

            /// The entry's name, for diagnostics and failure objects.
            pub fn name(self) -> &'static str {
                match self {
                    $(Native::$name => stringify!($name),)*
                }
            }
        }
    };
}

natives! {
    IntegerAdd = 1 / 1: "`Integer>>+`.",
    IntegerSubtract = 2 / 1: "`Integer>>-`.",
    IntegerMultiply = 3 / 1: "`Integer>>*`.",
    IntegerLess = 4 / 1: "`Number>><`.",
    IntegerGreater = 5 / 1: "`Number>>>`.",
    IntegerLessEqual = 6 / 1: "`Number>><=`.",
    IntegerGreaterEqual = 7 / 1: "`Number>>>=`.",
    NumberEqual = 8 / 1: "`Number>>=`: false for a non-number (decided 2026-11-10).",
    NumberAsString = 9 / 0: "`Number>>asString`.",
    ObjectIdentical = 10 / 1: "`Object>>==`.",
    ObjectClass = 11 / 0: "`Object>>class`.",
    ObjectIdentityHash = 12 / 0: "`Object>>identityHash`: from the allocation sequence number.",
    StringConcatenate = 13 / 1: "`String>>,`.",
    StringSize = 14 / 0: "`String>>size`.",
    VmPrint = 20 / 1: "`vm print:` (output).",
    VmExit = 21 / 1: "`vm exit:`.",
}

/// A resource whose bound can be exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// Heap memory.
    Memory,
    /// Stack depth.
    Stack,
    /// Callback nesting.
    Nesting,
    /// The alien handle table.
    Handles,
}

/// Why a native failed (operation list, section 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Argument `i` (0 = receiver) is of an unacceptable kind.
    WrongKind(u32),
    /// Argument `i` is an index or size out of range.
    OutOfRange(u32),
    /// Division or modulus by zero.
    ZeroDivide,
    /// Wrong number of arguments.
    Arity,
    /// The class cannot be instantiated this way.
    NotInstantiable,
    /// A dead activation, alien or expat.
    Dead,
    /// A resource limit.
    Exhausted(Resource),
    /// The host raised; the value is the host exception.
    Host(Value),
    /// Not provided on this host, or not yet implemented.
    Unsupported,
}

impl Reason {
    /// The reason's name, as stored in failure objects.
    pub fn name(self) -> &'static str {
        match self {
            Reason::WrongKind(_) => "wrongKind",
            Reason::OutOfRange(_) => "outOfRange",
            Reason::ZeroDivide => "zeroDivide",
            Reason::Arity => "arity",
            Reason::NotInstantiable => "notInstantiable",
            Reason::Dead => "dead",
            Reason::Exhausted(_) => "exhausted",
            Reason::Host(_) => "host",
            Reason::Unsupported => "unsupported",
        }
    }

    /// The reason's argument: an index, a resource name, or the host value.
    pub fn argument(self, ctx: &mut NativeContext<'_>) -> Value {
        match self {
            Reason::WrongKind(i) | Reason::OutOfRange(i) => small(i64::from(i)),
            Reason::Exhausted(r) => {
                let name = match r {
                    Resource::Memory => "memory",
                    Resource::Stack => "stack",
                    Resource::Nesting => "nesting",
                    Resource::Handles => "handles",
                };
                ctx.new_string(name)
            }
            Reason::Host(v) => v,
            _ => ctx.roles.get(Role::Nil),
        }
    }
}

/// What a native did (the three outcomes, plus ending the program).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Rule Native-Answer.
    Answer(Value),
    /// Rule Native-Tail: an ordinary send of `selector` to `receiver`.
    TailSend {
        /// The new receiver.
        receiver: Value,
        /// The selector.
        selector: &'static str,
        /// The arguments.
        args: Vec<Value>,
    },
    /// Rule Native-Fail.
    Fail(Reason),
    /// `vm exit:` ended the program.
    Exit(i32),
}

/// What a native may touch.
pub struct NativeContext<'a> {
    /// The heap.
    pub heap: &'a mut Heap,
    /// The roles.
    pub roles: &'a Roles,
    /// The host.
    pub host: &'a mut dyn Host,
}

impl NativeContext<'_> {
    /// Allocates a String.
    pub fn new_string(&mut self, s: &str) -> Value {
        let class = self.roles.get(Role::StringClass);
        self.heap
            .allocate(class, Space::Actor(0), Body::String(s.as_bytes().to_vec()))
    }

    fn boolean(&self, b: bool) -> Value {
        self.roles.get(if b { Role::True } else { Role::False })
    }
}

fn small(n: SmallInteger) -> Value {
    Value::small_integer(n).expect("small enough")
}

/// The integer arithmetic natives on SmallIntegers. A result outside the
/// SmallInteger range needs LargeIntegers, which arrive in M2; until then it
/// fails with `Unsupported` rather than wrapping.
fn arithmetic(
    receiver: Value,
    arg: Value,
    op: fn(i64, i64) -> Option<i64>,
    tail: &'static str,
) -> Outcome {
    let (Some(a), Some(b)) = (receiver.as_small_integer(), arg.as_small_integer()) else {
        if receiver.as_small_integer().is_none() {
            return Outcome::Fail(Reason::Unsupported);
        }
        return Outcome::TailSend {
            receiver: arg,
            selector: tail,
            args: alloc::vec![receiver],
        };
    };
    match op(a, b).and_then(Value::small_integer) {
        Some(v) => Outcome::Answer(v),
        None => Outcome::Fail(Reason::Unsupported),
    }
}

fn comparison(
    receiver: Value,
    arg: Value,
    op: fn(i64, i64) -> bool,
    ctx: &NativeContext<'_>,
) -> Outcome {
    match (receiver.as_small_integer(), arg.as_small_integer()) {
        (Some(a), Some(b)) => Outcome::Answer(ctx.boolean(op(a, b))),
        (Some(_), None) => Outcome::Fail(Reason::WrongKind(1)),
        (None, _) => Outcome::Fail(Reason::Unsupported),
    }
}

/// Runs a native. `args` excludes the receiver and has the native's arity.
pub fn invoke(
    native: Native,
    receiver: Value,
    args: &[Value],
    ctx: &mut NativeContext<'_>,
) -> Outcome {
    debug_assert_eq!(args.len(), native.arity());
    let arg = args.first().copied();
    match native {
        Native::IntegerAdd => {
            arithmetic(receiver, arg.unwrap(), i64::checked_add, "addFromInteger:")
        }
        Native::IntegerSubtract => arithmetic(
            receiver,
            arg.unwrap(),
            i64::checked_sub,
            "subtractFromInteger:",
        ),
        Native::IntegerMultiply => arithmetic(
            receiver,
            arg.unwrap(),
            i64::checked_mul,
            "multiplyFromInteger:",
        ),
        Native::IntegerLess => comparison(receiver, arg.unwrap(), |a, b| a < b, ctx),
        Native::IntegerGreater => comparison(receiver, arg.unwrap(), |a, b| a > b, ctx),
        Native::IntegerLessEqual => comparison(receiver, arg.unwrap(), |a, b| a <= b, ctx),
        Native::IntegerGreaterEqual => comparison(receiver, arg.unwrap(), |a, b| a >= b, ctx),
        Native::NumberEqual => {
            let arg = arg.unwrap();
            match (receiver.as_small_integer(), arg.as_small_integer()) {
                (Some(a), Some(b)) => Outcome::Answer(ctx.boolean(a == b)),
                (Some(_), None) => {
                    let class = ctx.heap.get(arg).map(|o| o.header.class);
                    let numeric = [
                        Role::FloatClass,
                        Role::LargeIntegerClass,
                        Role::MediumIntegerClass,
                    ]
                    .iter()
                    .any(|&r| Some(ctx.roles.get(r)) == class);
                    if numeric {
                        Outcome::Fail(Reason::Unsupported)
                    } else {
                        Outcome::Answer(ctx.boolean(false))
                    }
                }
                (None, _) => Outcome::Fail(Reason::Unsupported),
            }
        }
        Native::NumberAsString => match receiver.as_small_integer() {
            Some(n) => Outcome::Answer(ctx.new_string(&n.to_string())),
            None => Outcome::Fail(Reason::Unsupported),
        },
        Native::ObjectIdentical => Outcome::Answer(ctx.boolean(receiver == arg.unwrap())),
        Native::ObjectClass => Outcome::Answer(match ctx.heap.get(receiver) {
            Some(o) => o.header.class,
            None => ctx.roles.get(Role::SmallIntegerClass),
        }),
        Native::ObjectIdentityHash => match ctx.heap.get(receiver) {
            // eq:kernel-identity-hash with h the identity on SmallIntegers.
            Some(o) => Outcome::Answer(small((o.header.seq & ((1 << 62) - 1)) as i64)),
            None => Outcome::Answer(receiver),
        },
        Native::StringConcatenate => {
            let arg = arg.unwrap();
            let (Some(a), Some(b)) = (ctx.heap.string_bytes(receiver), ctx.heap.string_bytes(arg))
            else {
                return Outcome::Fail(Reason::WrongKind(
                    if ctx.heap.string_bytes(receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let mut joined = a.to_vec();
            joined.extend_from_slice(b);
            let class = ctx.roles.get(Role::StringClass);
            Outcome::Answer(
                ctx.heap
                    .allocate(class, Space::Actor(0), Body::String(joined)),
            )
        }
        Native::StringSize => match ctx.heap.string_bytes(receiver) {
            Some(b) => Outcome::Answer(small(b.len() as i64)),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::VmPrint => match ctx.heap.string_bytes(arg.unwrap()) {
            Some(b) => {
                let text = alloc::string::String::from_utf8_lossy(b).into_owned();
                ctx.host.print(&text);
                Outcome::Answer(receiver)
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmExit => match arg
            .unwrap()
            .as_small_integer()
            .and_then(|n| i32::try_from(n).ok())
        {
            Some(code) => Outcome::Exit(code),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
    }
}
