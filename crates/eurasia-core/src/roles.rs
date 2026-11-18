// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Roles: the objects the VM must recognise (snapshot format, section 7).

use alloc::vec::Vec;

use crate::value::Value;

macro_rules! roles {
    ($($name:ident = $id:literal: $doc:literal,)*) => {
        /// A role, numbered as in the snapshot format.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u16)]
        pub enum Role {
            $(#[doc = $doc] $name = $id,)*
        }

        impl Role {
            /// Every role, in id order.
            pub const ALL: &'static [Role] = &[$(Role::$name,)*];

            /// The role with snapshot id `id`.
            pub fn from_id(id: u64) -> Option<Role> {
                match id {
                    $($id => Some(Role::$name),)*
                    _ => None,
                }
            }

            /// The role's name, for diagnostics.
            pub fn name(self) -> &'static str {
                match self {
                    $(Role::$name => stringify!($name),)*
                }
            }
        }
    };
}

roles! {
    Nil = 1: "nil",
    True = 2: "true",
    False = 3: "false",
    SmallIntegerClass = 4: "The SmallInteger class.",
    MediumIntegerClass = 5: "The MediumInteger class.",
    LargeIntegerClass = 6: "The LargeInteger class.",
    FloatClass = 7: "The Float class.",
    StringClass = 8: "The String class.",
    ByteArrayClass = 9: "The ByteArray class.",
    ArrayClass = 10: "The Array class.",
    WeakArrayClass = 11: "The WeakArray class.",
    EphemeronClass = 12: "The Ephemeron class.",
    ClosureClass = 13: "The Closure class.",
    ActivationClass = 14: "The Activation class.",
    MethodClass = 15: "The Method class.",
    ExpatClass = 16: "The Expat class.",
    MessageLoop = 17: "The message loop.",
    DoesNotUnderstandSelector = 18: "The doesNotUnderstand: selector.",
    NonBooleanReceiverSelector = 19: "The nonBooleanReceiver: selector.",
    CannotReturnSelector = 20: "The cannotReturn: selector.",
    AboutToReturnThroughSelector = 21: "The aboutToReturnThrough: selector.",
    DispatchMessageSelector = 22: "The dispatchMessage:port: selector.",
    DispatchSignalSelector = 23: "The dispatchSignal selector.",
    FailureClass = 24: "The failure exception class (rule Native-Fail).",
    EnsureMethod = 25: "The ensure: method.",
    OnDoMethod = 26: "The on:do: method.",
    SimulationRootMethod = 27: "The simulation-root method.",
    CallbackDispatchMethod = 28: "The callback-dispatch method.",
    CommonSelectors = 29: "The common-selector array (V5 opcodes 176-207).",
    MessageClass = 30: "The Message class.",
    VmMirror = 31: "The VM mirror: its class carries the kernel operations.",
    Entry = 32: "The entry object; M1 sends it main:args: (until the message loop, M4).",
    AlienClass = 33: "The Alien class.",
    ValueClass = 34: "The Value class: classes whose chain includes it are value classes.",
}

/// A complete assignment of values to roles.
#[derive(Clone, Debug)]
pub struct Roles {
    values: Vec<Value>,
}

/// Why a role table was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RolesError {
    /// A role id the kernel version does not define.
    UnknownRole(u64),
    /// A role given twice.
    DuplicateRole(Role),
    /// A role not given.
    MissingRole(Role),
}

impl Roles {
    /// Roles from (role, value) pairs. Every role must appear exactly once.
    pub fn new(pairs: &[(Role, Value)]) -> Result<Roles, RolesError> {
        let mut slots: Vec<Option<Value>> = alloc::vec![None; Role::ALL.len()];
        for &(role, value) in pairs {
            let slot = &mut slots[role as usize - 1];
            if slot.is_some() {
                return Err(RolesError::DuplicateRole(role));
            }
            *slot = Some(value);
        }
        let mut values = Vec::with_capacity(slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            values.push(slot.ok_or(RolesError::MissingRole(Role::ALL[i]))?);
        }
        Ok(Roles { values })
    }

    /// The value playing `role`.
    pub fn get(&self, role: Role) -> Value {
        self.values[role as usize - 1]
    }

    /// Assigns `value` to `role` (the kernel operation `vm setRole:to:`).
    pub fn set(&mut self, role: Role, value: Value) {
        self.values[role as usize - 1] = value;
    }

    /// Every (role, value) pair, in id order.
    pub fn pairs(&self) -> impl Iterator<Item = (Role, Value)> + '_ {
        Role::ALL.iter().map(move |&r| (r, self.get(r)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(v: Value) -> Vec<(Role, Value)> {
        Role::ALL.iter().map(|&r| (r, v)).collect()
    }

    #[test]
    fn ids_are_dense_and_round_trip() {
        for (i, &r) in Role::ALL.iter().enumerate() {
            assert_eq!(r as usize, i + 1);
            assert_eq!(Role::from_id(r as u64), Some(r));
        }
        assert_eq!(Role::from_id(0), None);
        assert_eq!(Role::from_id(Role::ALL.len() as u64 + 1), None);
    }

    #[test]
    fn every_role_is_required_once() {
        let v = Value::small_integer(1).unwrap();
        assert!(Roles::new(&all(v)).is_ok());
        let mut missing = all(v);
        missing.retain(|&(r, _)| r != Role::FailureClass);
        assert_eq!(
            Roles::new(&missing).unwrap_err(),
            RolesError::MissingRole(Role::FailureClass)
        );
        let mut twice = all(v);
        twice.push((Role::Nil, v));
        assert_eq!(
            Roles::new(&twice).unwrap_err(),
            RolesError::DuplicateRole(Role::Nil)
        );
    }
}
