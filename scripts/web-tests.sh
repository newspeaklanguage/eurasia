#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The web host (M5): builds the wasm32 VM and runs, under Node through the
# browser's loader (web/eurasia.mjs), the images genesis.sh and kernel-tests.sh
# built. The base image must print exactly what the native VM printed, and
# every kernel test must have the same outcome; the kernel tests, which use
# timers, must also have waited for one (the non-blocking path).
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
grep -q "eurasia: [1-9][0-9]* timer waits" "$OUT/kernel-wasm-stats.txt" \
  || { echo "the kernel tests never waited for a timer"; exit 1; }

echo "== the JS FFI and callbacks: JSTesting and EurasiaHostTesting on wasm"
# psoup's native VM cannot run these (a failing JS primitive halts it), so
# they are not in the differential kernel tests: the kernel test runner is
# packaged with JSTesting and EurasiaHostTesting (callbacks), and every JS test must succeed on wasm except
# those docs/EXPECTED_DIFFERENCES.md lists under "JS tests on wasm".
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
P=$PRIMORDIALSOUP/newspeak
JSMODULES="$P/Kernel.ns $P/CollectionsForPrimordialSoup.ns $NEWSPEAK/EurasiaActors.ns $NEWSPEAK/EurasiaRuntime.ns $P/Minitest.ns $P/JSTesting.ns $NEWSPEAK/EurasiaHostTesting.ns"
mkdir -p "$OUT/js"
# shellcheck disable=SC2086
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" target/kerneltests/EurasiaPackager.vfuel \
  "$NEWSPEAK/EurasiaKernel.ns" kernel/natives.tsv "$NEWSPEAK/EurasiaKernelTestRunner.ns" \
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

cp web/index.html web/eurasia.mjs "$WASM" target/genesis/base.eus "$OUT/"
echo "Web host: wasm32 output matches native. For a browser, serve $OUT and open index.html."
