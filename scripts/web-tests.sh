#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The web host (M5): builds the wasm32 VM and runs, under Node through the
# browser's loader (web/eurasia.mjs), the images genesis.sh and kernel-tests.sh
# built. The base image must print exactly what the native VM printed, and
# every kernel test must have the same outcome; the kernel tests, which use
# timers, must also have waited for one (the non-blocking path). Compiled:
# the kernel tests with every method compiled must have the same outcomes,
# and compiled code must not go on after a collection
# (EurasiaCompiledGCProbe) nor trap in a callback with a checkpoint pending
# (EurasiaCallbackSafepointProbe); STRESS=1 adds the kernel tests compiled under GC
# stress.
# Run genesis.sh and kernel-tests.sh first. Also copies the build and the base
# image into target/web/ beside the page, for a browser (web/index.html).
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
OUT=$EURASIA/target/web
mkdir -p "$OUT"

cargo build -q --release -p eurasia-web --target wasm32-unknown-unknown
WASM=$EURASIA/target/wasm32-unknown-unknown/release/eurasia_web.wasm
ls -l "$WASM"

echo "== the base image on wasm"
[ -f target/genesis/base.eus ] || { echo "run scripts/genesis.sh first"; exit 1; }
node web/run-node.mjs "$WASM" target/genesis/base.eus > "$OUT/base-output.txt"
cmp "$OUT/base-output.txt" target/genesis/output.txt \
  || { echo "the base image prints differently on wasm"; exit 1; }

echo "== the kernel tests on wasm"
[ -f target/kerneltests/base.eus ] || { echo "run scripts/kernel-tests.sh first"; exit 1; }
target/release/eurasia target/kerneltests/base.eus > "$OUT/kernel-native.txt" 2>&1 || true
EURASIA_WEB_STATS=1 node web/run-node.mjs "$WASM" target/kerneltests/base.eus \
  > "$OUT/kernel-wasm.txt" 2> "$OUT/kernel-wasm-stats.txt" || true
# Outcomes, not raw output: how often a timer test's callback fails depends
# on wall-clock timing, on either VM.
grep '>>' "$OUT/kernel-native.txt" | sort > "$OUT/kernel-native-outcomes.txt"
grep '>>' "$OUT/kernel-wasm.txt" | sort > "$OUT/kernel-wasm-outcomes.txt"
[ "$(wc -l < "$OUT/kernel-wasm-outcomes.txt")" -gt 400 ] || { echo "the kernel tests did not run on wasm"; exit 1; }
cmp "$OUT/kernel-native-outcomes.txt" "$OUT/kernel-wasm-outcomes.txt" \
  || { echo "the kernel tests have different outcomes on wasm"; exit 1; }
tail -1 "$OUT/kernel-wasm.txt" | grep -qx done || { echo "the kernel tests did not finish on wasm"; exit 1; }
cat "$OUT/kernel-wasm-stats.txt"

echo "== the kernel tests on wasm, every method compiled"
# The baseline compiler (newspeak/EURASIA_COMPILER_DESIGN_2026-11-19.md):
# compiled at its first activation, every method the tests run must give the
# interpreter's outcomes; with STRESS=1, also with a collection after every
# 64 allocations.
EURASIA_JIT=1 EURASIA_WEB_STATS=1 node web/run-node.mjs "$WASM" target/kerneltests/base.eus \
  > "$OUT/kernel-jit.txt" 2> "$OUT/kernel-jit-stats.txt" || true
grep '>>' "$OUT/kernel-jit.txt" | sort > "$OUT/kernel-jit-outcomes.txt"
cmp "$OUT/kernel-native-outcomes.txt" "$OUT/kernel-jit-outcomes.txt" \
  || { echo "the kernel tests have different outcomes compiled"; exit 1; }
tail -1 "$OUT/kernel-jit.txt" | grep -qx done || { echo "the kernel tests did not finish compiled"; exit 1; }
grep -q "eurasia: .* [1-9][0-9]* methods compiled" "$OUT/kernel-jit-stats.txt" \
  || { echo "no method was compiled"; cat "$OUT/kernel-jit-stats.txt"; exit 1; }
cat "$OUT/kernel-jit-stats.txt"
if [ "${STRESS:-}" = 1 ]; then
  EURASIA_JIT=1 EURASIA_GC_STRESS=64 node web/run-node.mjs "$WASM" target/kerneltests/base.eus \
    > "$OUT/kernel-jit-stress.txt" 2>&1 || true
  grep '>>' "$OUT/kernel-jit-stress.txt" | sort > "$OUT/kernel-jit-stress-outcomes.txt"
  cmp "$OUT/kernel-native-outcomes.txt" "$OUT/kernel-jit-stress-outcomes.txt" \
    || { echo "the kernel tests have different outcomes compiled, under GC stress"; exit 1; }
  echo "the same outcomes compiled under GC stress"
fi
grep -q "eurasia: [1-9][0-9]* timer waits" "$OUT/kernel-wasm-stats.txt" \
  || { echo "the kernel tests never waited for a timer"; exit 1; }

echo "== compiled code across collections (EurasiaCompiledGCProbe)"
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
P=$PRIMORDIALSOUP/newspeak
mkdir -p "$OUT/gcprobe"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel --entry-in-kernel \
  "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaCompiledGCProbe.ns" \
  "$P/Kernel.ns" "$P/platform/PrimordialSoup/Collections.ns" "$NEWSPEAK/platform/Eurasia/Actors.ns" "$NEWSPEAK/platform/Eurasia/Runtime.ns" \
  "$OUT/gcprobe/genesis.eus" > "$OUT/gcprobe/package.log"
(cd "$OUT/gcprobe" && "$EURASIA/target/release/eurasia" genesis.eus base.eus > /dev/null)
EURASIA_JIT=1 node web/run-node.mjs "$WASM" "$OUT/gcprobe/base.eus" > "$OUT/gcprobe/output.txt"
cat "$OUT/gcprobe/output.txt"
grep -qx "compiled code across collections: 0 of 20 rounds failed" "$OUT/gcprobe/output.txt" \
  || { echo "compiled code went on after a collection"; exit 1; }

echo "== compiled code in a callback with a checkpoint pending (EurasiaCallbackSafepointProbe)"
# Save Image from a DOM event used to trap: see the probe's comment. A host
# timer's callback asks for a checkpoint and makes three million sends,
# compiled; it must run to its end.
mkdir -p "$OUT/cbprobe"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel --entry-in-kernel \
  "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaCallbackSafepointProbe.ns" \
  "$P/Kernel.ns" "$P/platform/PrimordialSoup/Collections.ns" "$NEWSPEAK/platform/Eurasia/Actors.ns" "$NEWSPEAK/platform/Eurasia/Runtime.ns" \
  "$OUT/cbprobe/genesis.eus" > "$OUT/cbprobe/package.log"
(cd "$OUT/cbprobe" && "$EURASIA/target/release/eurasia" genesis.eus base.eus > /dev/null)
EURASIA_JIT=1 node web/run-node.mjs "$WASM" "$OUT/cbprobe/base.eus" > "$OUT/cbprobe/output.txt" 2>&1 || true
cat "$OUT/cbprobe/output.txt"
grep -qx "sends in a callback with a checkpoint pending: 3000000 sends, sum 9000003" "$OUT/cbprobe/output.txt" \
  || { echo "compiled code in a callback with a checkpoint pending did not run to its end"; exit 1; }

echo "== the JS FFI and callbacks: JSTesting and EurasiaHostTesting on wasm"
# psoup's native VM cannot run these (a failing JS primitive halts it), so
# they are not in the differential kernel tests: the kernel test runner is
# packaged with JSTesting and EurasiaHostTesting (callbacks), and every JS test must succeed on wasm except
# those docs/EXPECTED_DIFFERENCES.md lists under "JS tests on wasm".
JSMODULES="$P/Kernel.ns $P/platform/PrimordialSoup/Collections.ns $NEWSPEAK/platform/Eurasia/Actors.ns $NEWSPEAK/platform/Eurasia/Runtime.ns $P/Minitest.ns $P/JSTesting.ns $NEWSPEAK/EurasiaHostTesting.ns"
mkdir -p "$OUT/js"
# shellcheck disable=SC2086
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel --entry-in-kernel \
  "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaKernelTestRunner.ns" \
  $JSMODULES "$OUT/js/genesis.eus" > "$OUT/js/package.log"
(cd "$OUT/js" && "$EURASIA/target/release/eurasia" genesis.eus base.eus)
node web/run-node.mjs "$WASM" "$OUT/js/base.eus" > "$OUT/js/output.txt" || true
cat "$OUT/js/output.txt"
tail -1 "$OUT/js/output.txt" | grep -qx done || { echo "the JS tests did not finish on wasm"; exit 1; }
grep -E '^(JSTests|CallbackTests)>>' "$OUT/js/output.txt" | grep -v ' success$' | awk '{ print $1 }' | sort > "$OUT/js/failing.txt" || true
sed -n '/^## JS tests on wasm/,/^## /p' docs/EXPECTED_DIFFERENCES.md | grep -o '^- `[^`]*`' | tr -d '`-' | tr -d ' ' | sort > "$OUT/js/expected.txt" || true
[ "$(grep -cE '^(JSTests|CallbackTests)>>' "$OUT/js/output.txt")" -ge 16 ] || { echo "the JS tests did not run"; exit 1; }
if ! cmp -s "$OUT/js/failing.txt" "$OUT/js/expected.txt"; then
  echo "JS tests not succeeding, against those expected:"; diff "$OUT/js/failing.txt" "$OUT/js/expected.txt" || true; exit 1
fi

echo "== CounterApp: packaged with Hopscotch, genesis on the command line"
# Hopscotch needs a DOM, so CounterApp runs only in a browser: open
# index.html?snapshot=counter/base.eus from a server for target/web.
COUNTERMODULES="$P/Kernel.ns $P/platform/PrimordialSoup/Collections.ns $NEWSPEAK/platform/Eurasia/Actors.ns $NEWSPEAK/platform/Eurasia/Runtime.ns $NEWSPEAK/Ducts.ns $NEWSPEAK/GraphicsForHTML5.ns $NEWSPEAK/FontsForHTML5.ns $NEWSPEAK/TextModule.ns $NEWSPEAK/HopscotchForHTML5.ns $NEWSPEAK/VoiceController.ns $NEWSPEAK/AIAccess.ns $NEWSPEAK/platform/Eurasia/WebRuntime.ns $NEWSPEAK/CounterUI.ns"
mkdir -p "$OUT/counter"
# shellcheck disable=SC2086
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel \
  "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaCounterApp.ns" \
  $COUNTERMODULES "$OUT/counter/genesis.eus" > "$OUT/counter/package.log"
(cd "$OUT/counter" && "$EURASIA/target/release/eurasia" genesis.eus base.eus)
ls -l "$OUT/counter/base.eus"
cp "$NEWSPEAK"/*.png "$OUT/"

cp web/index.html web/eurasia.mjs "$WASM" target/genesis/base.eus "$OUT/"
# psoup's page helpers (safeDownloadBlob), canonical in primordialsoup.
cp "$PRIMORDIALSOUP/meta/custom-post.js" "$OUT/"
echo "Web host: wasm32 output matches native. For a browser, serve $OUT and open index.html (?snapshot=counter/base.eus for CounterApp)."
