// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Eurasia VM core.
//!
//! The specification is the Newspeak repository's
//! `VM_KERNEL_CONTRACT_DESIGN_2026-11-09.md`, `VM_KERNEL_OPERATIONS_2026-11-10.md`
//! and `formal-semantics/kernel-semantics.tex`. This crate has no I/O and no
//! host types, so it builds unchanged for native targets and wasm32.

#![no_std]

extern crate alloc;

pub mod builder;
pub mod bytecode;
pub mod host;
pub mod interpreter;
pub mod natives;
pub mod object;
pub mod roles;
pub mod value;
pub mod version;

pub use interpreter::{Completion, Vm, VmError};
pub use value::{SmallInteger, Value};
pub use version::{BYTECODE_VERSION, KERNEL_VERSION};
