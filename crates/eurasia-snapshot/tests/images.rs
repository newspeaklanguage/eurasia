// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! Whole-image round trips and load-time validation.

use eurasia_core::builder::first_light_image;
use eurasia_core::host::Host;
use eurasia_core::roles::Role;
use eurasia_core::{Completion, Value, Vm};
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
    let first = write_image(&heap, &roles, None);
    let (heap2, roles2) = read_image(&first).expect("loads");
    let second = write_image(&heap2, &roles2, None);
    assert_eq!(first, second);
}

#[test]
fn a_loaded_image_runs() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles, None);
    let (heap, roles) = read_image(&bytes).expect("loads");
    let mut vm = Vm::new(heap, roles);
    let mut host = Printer::default();
    let result = vm.run(&mut host, &[]).expect("runs");
    assert!(matches!(result, Completion::Returned(_)));
    assert_eq!(host.0, ["7"]);
}

#[test]
fn loading_preserves_hash_numbers_and_the_counters() {
    let (mut heap, roles) = first_light_image();
    // Some objects have been asked their hash, by two actors.
    let all: Vec<Value> = heap.objects().collect();
    heap.hash_number(all[3], 0);
    heap.hash_number(all[1], 2);
    let bytes = write_image(&heap, &roles, None);
    let (loaded, loaded_roles) = read_image(&bytes).expect("loads");
    assert_eq!(loaded.allocation_counter(), heap.allocation_counter());
    assert_eq!(loaded.hash_counters(), heap.hash_counters());
    assert_eq!(loaded.len(), heap.len());
    let hashes = |h: &eurasia_core::object::Heap| -> Vec<u64> {
        let mut all: Vec<u64> = h.objects().map(|v| h.hash_word(v).unwrap()).collect();
        all.sort_unstable();
        all
    };
    assert_eq!(hashes(&heap), hashes(&loaded));
    for &role in Role::ALL {
        let a = heap.hash_word(roles.get(role));
        let b = loaded.hash_word(loaded_roles.get(role));
        assert_eq!(a, b, "{role:?}");
    }
}

#[test]
fn an_image_is_canonical_whatever_order_the_heap_lies_in() {
    // The loaded heap lies in file order, not in the order the builder
    // allocated; writing it again gives the same bytes.
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles, None);
    let (loaded, loaded_roles) = read_image(&bytes).expect("loads");
    assert_eq!(write_image(&loaded, &loaded_roles, None), bytes);
    // The walk starts at the first role, nil: it is the first object of
    // its space.
    let nil = loaded_roles.get(Role::Nil);
    let first = loaded
        .objects()
        .find(|&v| loaded.space(v) == loaded.space(nil))
        .unwrap();
    assert_eq!(first, nil);
}

#[test]
fn an_optional_section_is_skipped() {
    let (heap, roles) = first_light_image();
    let plain = write_image(&heap, &roles, None);
    let bytes = eurasia_snapshot::write_image_with(&heap, &roles, None, &[(200, b"feedback")]);
    assert!(bytes.len() > plain.len());
    let (loaded, loaded_roles) = read_image(&bytes).expect("loads");
    assert_eq!(write_image(&loaded, &loaded_roles, None), plain);
}

#[test]
fn every_truncation_is_refused_without_panicking() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles, None);
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
    let mut bytes = write_image(&heap, &roles, None);
    bytes.push(0);
    assert_eq!(read_image(&bytes).unwrap_err(), LoadError::TrailingBytes);
}

#[test]
fn corrupted_bytes_never_panic() {
    let (heap, roles) = first_light_image();
    let bytes = write_image(&heap, &roles, None);
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

#[test]
fn closures_and_dead_activations_round_trip() {
    use eurasia_core::Value;
    use eurasia_core::builder::ImageBuilder;
    use eurasia_core::object::{Activation, Body, Closure, Space};

    let mut b = ImageBuilder::new();
    let activation_class = b.role(Role::ActivationClass).unwrap();
    let closure_class = b.role(Role::ClosureClass).unwrap();
    let nil = b.nil;
    let activation = b.heap.allocate(
        activation_class,
        Space::Actor(0),
        Body::Activation(Activation {
            frame: None,
            method: nil,
            receiver: nil,
            closure: nil,
            detached: None,
        }),
    );
    let closure = b.heap.allocate(
        closure_class,
        Space::Actor(0),
        Body::Closure(Closure {
            defining: activation,
            initial_ip: 4,
            num_args: 1,
            copied: vec![Value::small_integer(7).unwrap(), nil],
        }),
    );
    let (heap, roles) = b.finish();
    let first = write_image(&heap, &roles, None);
    let (loaded, roles2) = read_image(&first).expect("loads");
    assert_eq!(write_image(&loaded, &roles2, None), first);
    let c = loaded
        .closure(closure)
        .expect("the closure, at the same index");
    assert_eq!((c.initial_ip(), c.num_args(), c.copied().len()), (4, 1, 2));
    assert_eq!(c.copied()[0], Value::small_integer(7).unwrap());
    assert!(
        loaded
            .activation(c.defining())
            .expect("activation")
            .frame()
            .is_none()
    );
}

#[test]
fn checkpoints_keep_frames_and_detached_activations() {
    use eurasia_core::Value;
    use eurasia_core::builder::ImageBuilder;
    use eurasia_core::machine::{ActorState, FrameState, MachineState};
    use eurasia_core::object::{Activation, Body, Detached, Space};
    use eurasia_snapshot::read_checkpoint;

    let mut b = ImageBuilder::new();
    let activation_class = b.role(Role::ActivationClass).unwrap();
    let nil = b.nil;
    let seven = Value::small_integer(7).unwrap();
    let live = b.heap.allocate(
        activation_class,
        Space::Actor(0),
        Body::Activation(Activation {
            frame: Some(3),
            method: nil,
            receiver: seven,
            closure: nil,
            detached: None,
        }),
    );
    let array_class = b.role(Role::ArrayClass).unwrap();
    let temps = b.heap.allocate(
        array_class,
        Space::Actor(0),
        Body::Array(vec![seven, nil, live]),
    );
    let detached = b.heap.allocate(
        activation_class,
        Space::Actor(0),
        Body::Activation(Activation {
            frame: None,
            method: nil,
            receiver: nil,
            closure: nil,
            detached: Some(Detached {
                sender: live,
                bci: seven,
                temps,
            }),
        }),
    );
    let (heap, roles) = b.finish();
    let state = MachineState {
        stack: vec![nil, seven, detached],
        frames: vec![FrameState {
            serial: 3,
            method: nil,
            receiver: seven,
            bp: 0,
            arg_count: 1,
            lp: 2,
            ip: 9,
            activation: Some(live),
            closure: None,
        }],
        actors: vec![ActorState {
            message_loop: nil,
            mailbox: vec![detached],
            wakeup: 12,
            poked: true,
        }],
        current: 0,
        expats: vec![(detached, 0)],
        symbols: vec![("restored".to_string(), detached)],
        next_serial: 4,
        gc_counter: 5,
        gc_budget: 6,
        collections: 1,
        finished_turn: Some(8),
        has_loop: true,
        program_version: 2,
        checkpoint_every: 100,
        last_checkpoint: 50,
    };
    let bytes = write_image(&heap, &roles, Some(&state));
    // Loading lays the heap out in the image's canonical order, so the
    // loaded state names the objects by their new places; from then on a
    // round trip changes nothing.
    let (heap, roles, state) = {
        let (h, r, m) = read_checkpoint(&bytes).expect("loads");
        (h, r, m.expect("a checkpoint"))
    };
    let (nil, live, detached) = (
        roles.get(Role::Nil),
        state.frames[0].activation.unwrap(),
        state.stack[2],
    );
    assert_eq!(state.stack, [nil, seven, detached]);
    assert_eq!(write_image(&heap, &roles, Some(&state)), bytes);
    let (loaded, roles2, machine) = read_checkpoint(&bytes).expect("loads");
    assert_eq!(machine.as_ref(), Some(&state));
    assert_eq!(write_image(&loaded, &roles2, machine.as_ref()), bytes);
    assert_eq!(loaded.activation(live).unwrap().frame(), Some(3));
    let d = loaded.activation(detached).unwrap().detached().unwrap();
    assert_eq!(
        (d.sender(), d.bci(), d.temps().to_vec()),
        (live, seven, vec![seven, nil, live])
    );
    // A snapshot (no machine state) saves a frame's handle dead.
    let (plain, _) = read_image(&write_image(&heap, &roles, None)).expect("loads");
    assert_eq!(plain.activation(live).unwrap().frame(), None);
    assert!(plain.activation(detached).unwrap().detached().is_some());
}
