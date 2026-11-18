// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Whole-image round trips and load-time validation.

use eurasia_core::builder::first_light_image;
use eurasia_core::host::Host;
use eurasia_core::roles::Role;
use eurasia_core::{Completion, Vm};
use eurasia_snapshot::{LoadError, read_image, write_image};

#[derive(Default)]
struct Printer(Vec<String>);

impl Host for Printer {
    fn print(&mut self, text: &str) {
        self.0.push(text.to_string());
    }
    fn monotonic_nanos(&mut self) -> u64 {
        0
    }
    fn utc_nanos(&mut self) -> i64 {
        0
    }
    fn random_bytes(&mut self, buffer: &mut [u8]) {
        buffer.fill(0);
    }
    fn description(&self) -> &str {
        "test"
    }
}

#[test]
fn write_read_write_is_byte_identical() {
    let (heap, roles) = first_light_image();
    let first = write_image(&heap, &roles);
    let (heap2, roles2) = read_image(&first).expect("loads");
    let second = write_image(&heap2, &roles2);
    assert_eq!(first, second);
}

#[test]
fn a_loaded_image_runs() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles);
    let (heap, roles) = read_image(&bytes).expect("loads");
    let mut vm = Vm::new(heap, roles);
    let mut host = Printer::default();
    let result = vm.run(&mut host, &[]).expect("runs");
    assert!(matches!(result, Completion::Returned(_)));
    assert_eq!(host.0, ["7"]);
}

#[test]
fn loading_preserves_sequence_numbers_and_the_allocation_counter() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles);
    let (loaded, loaded_roles) = read_image(&bytes).expect("loads");
    assert_eq!(loaded.allocation_counter(), heap.allocation_counter());
    let original: Vec<u64> = heap.iter().map(|(_, o)| o.header.seq).collect();
    let after: Vec<u64> = loaded.iter().map(|(_, o)| o.header.seq).collect();
    assert_eq!(original, after);
    // Same objects in the same places, so roles map to the same sequence numbers.
    for &role in Role::ALL {
        let a = heap.get(roles.get(role)).map(|o| o.header.seq);
        let b = loaded.get(loaded_roles.get(role)).map(|o| o.header.seq);
        assert_eq!(a, b, "{role:?}");
    }
}

#[test]
fn every_truncation_is_refused_without_panicking() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles);
    for len in 0..bytes.len() {
        assert!(
            read_image(&bytes[..len]).is_err(),
            "prefix of length {len} loaded"
        );
    }
}

#[test]
fn trailing_bytes_are_refused() {
    let (heap, roles) = first_light_image();
    let mut bytes = write_image(&heap, &roles);
    bytes.push(0);
    assert_eq!(read_image(&bytes).unwrap_err(), LoadError::TrailingBytes);
}

#[test]
fn corrupted_bytes_never_panic() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles);
    // Flip each byte after the header in turn; loading may succeed or fail,
    // but must not panic.
    for i in 16..bytes.len() {
        let mut b = bytes.clone();
        b[i] ^= 0x5a;
        if let Ok((heap, roles)) = read_image(&b) {
            // A corrupted image that loads must still run to a result or a
            // VmError: never a panic (design doc, section 9).
            let mut vm = Vm::new(heap, roles);
            vm.set_step_limit(Some(10_000));
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = vm.run(&mut Printer::default(), &[]);
            }));
            assert!(
                outcome.is_ok(),
                "running the image corrupted at byte {i} panicked"
            );
        }
    }
}
