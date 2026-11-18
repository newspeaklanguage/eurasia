// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! `eurasia`: the command-line Eurasia VM.
//!
//! M0 checks a snapshot's header and reports the versions; it does not run
//! anything yet. Usage: `eurasia SNAPSHOT [ARGS...]`, or `eurasia --version`.

use std::process::ExitCode;

use eurasia_core::{BYTECODE_VERSION, KERNEL_VERSION};
use eurasia_snapshot::{FORMAT_VERSION, Header, Reader};

fn version() -> String {
    format!(
        "eurasia {} (snapshot format {FORMAT_VERSION}, kernel {KERNEL_VERSION}, bytecode {BYTECODE_VERSION})",
        env!("CARGO_PKG_VERSION")
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        None => {
            eprintln!("usage: eurasia SNAPSHOT [ARGS...]");
            ExitCode::from(2)
        }
        Some("--version") => {
            println!("{}", version());
            ExitCode::SUCCESS
        }
        Some(path) => {
            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("eurasia: cannot read {path}: {e}");
                    return ExitCode::from(1);
                }
            };
            match Header::read(&mut Reader::new(&bytes)) {
                Ok(h) => {
                    eprintln!(
                        "eurasia: {path}: {} nodes in {} sections; running snapshots arrives in M1",
                        h.node_count, h.section_count
                    );
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("eurasia: {path}: {e}");
                    ExitCode::from(1)
                }
            }
        }
    }
}
