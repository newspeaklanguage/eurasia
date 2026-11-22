// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Versions that a snapshot must match (design doc section 7).

/// The kernel version: the native catalog and the kernel operations.
/// A snapshot built for another kernel version is refused. Version 2
/// scrambles identity hashes (`natives::identity_hash`): an image saved with
/// version 1 holds identity-keyed tables laid out by the old hash.
pub const KERNEL_VERSION: u16 = 2;

/// The bytecode version: Eurasia's refinement of upstream V5.
/// Version 1 is upstream V5 with the refinements of the implementation plan
/// (section 2.4): method kinds and native identities in method headers, a
/// four-bit closure arity, and branch-false decoding for the short range.
pub const BYTECODE_VERSION: u16 = 1;
