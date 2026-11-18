// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! kernel/natives.tsv, the genesis packager's natives table, must agree with
//! the catalog: every name exists with the same id, and every selector takes
//! as many arguments as its native.

use eurasia_core::natives::Native;

fn selector_arity(selector: &str) -> usize {
    match selector.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => selector.matches(':').count(),
        Some(_) => 1,
        None => 0,
    }
}

#[test]
fn the_natives_table_agrees_with_the_catalog() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../kernel/natives.tsv");
    let text = std::fs::read_to_string(path).expect("kernel/natives.tsv");
    let mut seen = std::collections::BTreeSet::new();
    let mut rows = 0;
    for (n, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 5, "line {}: {line}", n + 1);
        let (class, side, selector, name, id) =
            (fields[0], fields[1], fields[2], fields[3], fields[4]);
        assert!(
            side == "instance" || side == "class",
            "line {}: side {side}",
            n + 1
        );
        let id: u32 = id.parse().expect("numeric id");
        let native =
            Native::from_id(id).unwrap_or_else(|| panic!("line {}: no native {id}", n + 1));
        assert_eq!(
            native.name(),
            name,
            "line {}: id {id} is {}",
            n + 1,
            native.name()
        );
        assert_eq!(
            native.arity(),
            selector_arity(selector),
            "line {}: {class}>>{selector} takes {} arguments, {name} {}",
            n + 1,
            selector_arity(selector),
            native.arity()
        );
        assert!(
            seen.insert((class, side, selector)),
            "line {}: duplicate {class} {side} {selector}",
            n + 1
        );
        rows += 1;
    }
    assert!(rows > 100, "the table has {rows} rows");
}
