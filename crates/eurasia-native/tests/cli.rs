// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The `eurasia` binary on real snapshot files.

use std::process::Command;

use eurasia_core::builder::first_light_image;
use eurasia_snapshot::write_image;

fn eurasia() -> Command {
    Command::new(env!("CARGO_BIN_EXE_eurasia"))
}

fn temp_snapshot(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("eurasia-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn first_light_prints_seven() {
    let (heap, roles) = first_light_image();
    let path = temp_snapshot("first-light.eus", &write_image(&heap, &roles, None));
    let out = eurasia().arg(&path).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "7\n");
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn a_bad_snapshot_exits_65_with_a_message() {
    let path = temp_snapshot("bad.eus", b"not a snapshot");
    let out = eurasia().arg(&path).output().unwrap();
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad magic"));
}

#[test]
fn version_names_the_supported_versions() {
    let out = eurasia().arg("--version").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("snapshot format 9"));
    assert!(text.contains("kernel 2"));
}
