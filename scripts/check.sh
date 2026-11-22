#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The local CI: everything must pass before a change is reported done.
# Runs from any directory; operates on the repository containing this script.
set -eu
cd "$(dirname "$0")/.."

echo "== format"
cargo fmt --all -- --check
echo "== clippy (native)"
cargo clippy --workspace --all-targets -- -D warnings
echo "== tests (native)"
cargo test --workspace
echo "== natives module (newspeak/platform/Eurasia/EurasiaNatives.ns)"
python3 scripts/natives-module.py --check
echo "== Eurasia's forks of the command-line applications (kernel/fork-apps.py)"
python3 kernel/fork-apps.py --check
echo "== build (wasm32)"
cargo build --workspace --exclude eurasia-native --target wasm32-unknown-unknown
echo "== clippy (wasm32)"
cargo clippy --workspace --exclude eurasia-native --target wasm32-unknown-unknown -- -D warnings
echo "All checks passed."
