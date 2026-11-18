#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Genesis (EURASIA_GENESIS_KERNEL_PLAN_2026-11-17.md): package the kernel fork
# with the natives table and the smoke-test application, run the genesis
# image on Eurasia to produce the base image, then run the base image.
# Needs the newspeak and primordialsoup checkouts beside this one (override
# with NEWSPEAK and PRIMORDIALSOUP) and a built psoup with WebCompiler.vfuel.
# Outputs go to target/genesis/.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
OUT=$EURASIA/target/genesis
mkdir -p "$OUT"
# The application's modules: psoup's public kernel, collections and actors,
# and the platform over them.
MODULES="$PRIMORDIALSOUP/newspeak/Kernel.ns $PRIMORDIALSOUP/newspeak/CollectionsForPrimordialSoup.ns $PRIMORDIALSOUP/newspeak/ActorsForPrimordialSoup.ns $NEWSPEAK/EurasiaRuntime.ns"

echo "== building EurasiaPackager.vfuel"
(cd "$PRIMORDIALSOUP" && ./out/ReleaseX64/primordialsoup ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  RuntimeWithMirrorsForPrimordialSoup EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }

echo "== packaging the genesis image"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" "$OUT/EurasiaPackager.vfuel" \
  "$NEWSPEAK/EurasiaKernel.ns" "$EURASIA/kernel/natives.tsv" \
  "$NEWSPEAK/EurasiaKernelSmokeTest.ns" $MODULES "$OUT/genesis.eus"

cargo build -q -p eurasia-native
EURASIA_BIN=$EURASIA/target/debug/eurasia

echo "== genesis: running the genesis image, which saves the base image"
(cd "$OUT" && "$EURASIA_BIN" genesis.eus base.eus)
ls -l "$OUT/base.eus"

echo "== running the base image"
"$EURASIA_BIN" "$OUT/base.eus" | tee "$OUT/output.txt"
expected=$(printf '%s\n' 7 1267650600228229401496703205376 'instance of _Array' abcdef 42 \
  'caught DNU' 'NumberFloorDivide failed: zeroDivide' 'start body ensure' 42 done 'eventually 42')
if [ "$(cat "$OUT/output.txt")" != "$expected" ]; then
  echo "Unexpected output from the base image."; exit 1
fi

echo "== the same under GC stress: a collection after every allocation"
EURASIA_GC_STRESS=1 "$EURASIA_BIN" "$OUT/base.eus" > "$OUT/output-stress.txt" 2> "$OUT/stress.log"
cat "$OUT/stress.log"
cmp "$OUT/output.txt" "$OUT/output-stress.txt" || { echo "GC stress changed the output"; exit 1; }

echo "== an unhandled exception prints a trace and exits 1"
set +e
"$EURASIA_BIN" "$OUT/base.eus" unhandled > "$OUT/unhandled.txt"
status=$?
set -e
cat "$OUT/unhandled.txt"
[ $status -eq 1 ] || { echo "expected exit status 1, got $status"; exit 1; }
grep -q "failDeeper" "$OUT/unhandled.txt" || { echo "the trace lacks failDeeper"; exit 1; }
echo "== reproducibility: building again must give byte-identical images"
mkdir -p "$OUT/again"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" "$OUT/EurasiaPackager.vfuel" \
  "$NEWSPEAK/EurasiaKernel.ns" "$EURASIA/kernel/natives.tsv" \
  "$NEWSPEAK/EurasiaKernelSmokeTest.ns" $MODULES "$OUT/again/genesis.eus" > /dev/null
# A different output path: a snapshot saves only what the roles reach, so
# the arguments genesis ran with are not in the image.
"$EURASIA_BIN" "$OUT/again/genesis.eus" "$OUT/again/base.eus"
cmp "$OUT/genesis.eus" "$OUT/again/genesis.eus" || { echo "the genesis image is not reproducible"; exit 1; }
cmp "$OUT/base.eus" "$OUT/again/base.eus" || { echo "the base image is not reproducible"; exit 1; }
echo "Genesis: base image built, checked and reproducible."
