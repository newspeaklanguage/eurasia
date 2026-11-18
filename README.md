# Eurasia

A new virtual machine for Newspeak, replacing Primordial Soup. Written in
Rust, for native platforms and the web (wasm32) from one code base.

## Specification

Eurasia implements documents in the Newspeak repository (`../newspeak`):

- `VM_KERNEL_CONTRACT_DESIGN_2026-11-09.md`: the contract between the VM and
  Newspeak, and its decisions;
- `VM_KERNEL_OPERATIONS_2026-11-10.md`: every operation, its failures and
  effect category;
- `formal-semantics/kernel-semantics.tex`: the rules (section 13 of the
  formal semantics), with the native-method rules mechanized in SpecTec;
- `BYTECODES_V5_SPEC_2026-07-13.md`: the instruction set (V5, refined);
- `NEW_VM_IMPLEMENTATION_PLAN_2026-11-17.md`: architecture and milestones.

The snapshot format is [docs/SNAPSHOT_FORMAT.md](docs/SNAPSHOT_FORMAT.md).

## Layout

| Crate | Contents |
|---|---|
| `eurasia-core` | Values, object model, heaps, the V5 interpreter, natives, kernel operations. No I/O; `no_std`. |
| `eurasia-snapshot` | The snapshot format. `no_std`. |
| `eurasia-host` | The host interface: every input and output effect. `no_std`. |
| `eurasia-native` | The `eurasia` command-line VM. |
| `eurasia-web` | The wasm32 build. |

## Building and checking

The toolchain is pinned in `rust-toolchain.toml`. Run

```sh
scripts/check.sh
```

which checks formatting, runs clippy and the tests natively, and builds and
lints the libraries for wasm32. Every change must pass it.

## Status

**M1a (first light, Rust side) done.** The VM loads a snapshot and runs V5
bytecode: ordinary, self, implicit-receiver, super and outer sends with
psoup's lookup and access rules; DNU; natives with their three outcomes
(answer, tail send, failure through the failure class); local returns,
branches, literals, parameters, locals and arrays. Images are built in Rust
(`eurasia_core::builder`) until the packager exists.

```sh
cargo run -p eurasia-native --example write_first_light -- first-light.eus
cargo run -p eurasia-native -- first-light.eus      # prints 7
```

**M1b (first light from source) done.** `EurasiaPackager.ns` (in the
newspeak repository) runs on psoup, compiles a module with the V5 compiler,
and writes a snapshot; Eurasia runs it. `scripts/first-light-from-source.sh`
does the whole round trip with `EurasiaFirstLight.ns` and checks the output.
M1b builds classes statically from the module's nested classes and does not
run the module's initializer; how a module-structured kernel is instantiated
on Eurasia is still open.

**M2 in progress.** Done: closures, non-local return (psoup's protocol,
with `cannotReturn:` and `aboutToReturn:through:`), activation handles,
`nonBooleanReceiver:`, and the value-native catalog: SmallIntegers and
LargeIntegers (in-house arbitrary precision), Floats (bundled `libm`, so
bit-identical on every host), allocation, indexed access, copying, strings
and `perform:withArguments:`. A running program can save the whole heap (`vm snapshot`, `vm writeBytes:toFile:`) and choose the saved image's entry (`vm setRole:to:`); running the saved image continues from the saved state. **Genesis works:** `scripts/genesis.sh` packages the kernel fork (`kernel/fork-kernel.py` derives it from psoup's kernel; `kernel/natives.tsv` maps its methods to natives) with a smoke-test application, runs genesis on Eurasia to save the base image, and checks the base image's output, including psoup's own exception handling and an unhandled-exception trace. Images are build artifacts, always recreatable from source; genesis is reproducible byte for byte, and the script checks it. Next: differential tests against psoup
and packager, and differential tests against psoup
([docs/EXPECTED_DIFFERENCES.md](docs/EXPECTED_DIFFERENCES.md)). M1b is the packager
(Newspeak, in the newspeak repository), which builds images from source.

## License

Copyright 2026-2027, Gilad Bracha. Eurasia is licensed under the same
BSD-style license as the Primordial Soup VM; see [LICENSE](LICENSE). It is a
new implementation and contains no psoup VM code. The kernel and actors fork
scripts (`kernel/fork-kernel.py`, `kernel/fork-actors.py`) contain excerpts
of psoup's Newspeak sources, which are under the Apache License 2.0; the
LICENSE file acknowledges them.
