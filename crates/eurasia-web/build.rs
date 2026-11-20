// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! On wasm32, exports the function table and lets it grow, so that the host
//! can add compiled code to it (newspeak/EURASIA_COMPILER_DESIGN_2026-11-19.md).

fn main() {
    if std::env::var("TARGET").is_ok_and(|t| t.starts_with("wasm32")) {
        println!("cargo:rustc-cdylib-link-arg=--export-table");
        println!("cargo:rustc-cdylib-link-arg=--growable-table");
    }
}
