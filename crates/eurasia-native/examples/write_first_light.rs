// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Writes the M1 first-light image to the path given as the only argument.
//! `cargo run -p eurasia-native --example write_first_light -- first-light.eus`

use eurasia_core::builder::first_light_image;
use eurasia_snapshot::write_image;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: write_first_light PATH");
    let (heap, roles) = first_light_image();
    std::fs::write(&path, write_image(&heap, &roles)).expect("write the snapshot");
}
