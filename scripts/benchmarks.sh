#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# BenchmarkRunner's benchmarks on Eurasia (newspeak/EurasiaBenchmarkApp.ns),
# measured as BenchmarkRunner>>report does, in runs per second; with PSOUP=1,
# psoup's own BenchmarkRunner too, for comparison. Arguments name the
# benchmarks to run (all when none). Results: newspeak/
# EURASIA_PERFORMANCE_2026-11-19.md. Needs what scripts/genesis.sh needs.
# Outputs go to target/benchmarks/.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
PSOUP_BIN=$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup
P=$PRIMORDIALSOUP/newspeak
OUT=$EURASIA/target/benchmarks
mkdir -p "$OUT"

echo "== building EurasiaPackager.vfuel"
(cd "$PRIMORDIALSOUP" && "$PSOUP_BIN" ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  ./newspeak/platform/PrimordialSoup/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  PrimordialSoup/RuntimeWithMirrors EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }
echo "== packaging and genesis"
MODULES="$P/Kernel.ns $P/platform/PrimordialSoup/Collections.ns $NEWSPEAK/platform/Eurasia/Actors.ns $NEWSPEAK/platform/Eurasia/Runtime.ns"
for b in ClosureDefFibonacci ClosureFibonacci DeltaBlue MethodFibonacci NLRImmediate NLRLoop ParserCombinators Richards SlotRead SlotWrite Splay; do
  MODULES="$MODULES $P/$b.ns"
done
# shellcheck disable=SC2086
"$PSOUP_BIN" "$OUT/EurasiaPackager.vfuel" --entry-in-kernel "$NEWSPEAK/platform/Eurasia/InternalKernel.ns" kernel/natives.tsv \
  "$NEWSPEAK/EurasiaBenchmarkApp.ns" $MODULES "$OUT/genesis.eus" > "$OUT/package.log"
cargo build -q --release -p eurasia-native
(cd "$OUT" && "$EURASIA/target/release/eurasia" genesis.eus base.eus)
echo "== Eurasia (native)"
"$EURASIA/target/release/eurasia" "$OUT/base.eus" "$@" | tee "$OUT/eurasia.txt"
if [ "${PSOUP:-}" = 1 ]; then
  echo "== psoup (native)"
  "$PSOUP_BIN" "$PRIMORDIALSOUP/out/snapshots/BenchmarkRunner.vfuel" | tee "$OUT/psoup.txt"
fi
