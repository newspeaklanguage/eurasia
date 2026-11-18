#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Differential kernel tests (implementation plan section 5, item 3): run
# psoup's KernelTests on psoup with Minitest (EurasiaKernelTestReference) and
# on Eurasia from a genesis image (EurasiaKernelTestRunner), then compare the
# outcome of every test. A test whose outcome differs must be listed, with
# the reason, in docs/EXPECTED_DIFFERENCES.md; any other difference fails.
# Needs what scripts/genesis.sh needs. Outputs go to target/kerneltests/.
# With GC_STRESS=n, Eurasia collects at the first safepoint after every n
# allocations (implementation plan M3's stress test; GC_STRESS=64 takes a
# few minutes, 1 far longer).
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
PSOUP=$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup
OUT=$EURASIA/target/kerneltests
mkdir -p "$OUT"
MODULES="$PRIMORDIALSOUP/newspeak/Kernel.ns $PRIMORDIALSOUP/newspeak/CollectionsForPrimordialSoup.ns $PRIMORDIALSOUP/newspeak/Minitest.ns $PRIMORDIALSOUP/newspeak/KernelTests.ns $PRIMORDIALSOUP/newspeak/AccessModifierTesting.ns $PRIMORDIALSOUP/newspeak/KernelWeakTests.ns"

echo "== building EurasiaPackager.vfuel and EurasiaKernelTestReference.vfuel"
(cd "$PRIMORDIALSOUP" && "$PSOUP" ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  RuntimeWithMirrorsForPrimordialSoup EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }
(cd "$PRIMORDIALSOUP" && "$PSOUP" ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns "$NEWSPEAK/EurasiaKernelTestReference.ns" \
  RuntimeWithMirrorsForPrimordialSoup EurasiaKernelTestReference "$OUT/Reference.vfuel" > "$OUT/build-reference.log" 2>&1) \
  || { echo "building the reference failed; see $OUT/build-reference.log"; exit 1; }

echo "== psoup"
"$PSOUP" "$OUT/Reference.vfuel" > "$OUT/psoup-raw.txt" 2>&1 || true
grep '>>' "$OUT/psoup-raw.txt" | sort > "$OUT/psoup.txt" || true

echo "== Eurasia: packaging, genesis, then the tests"
# shellcheck disable=SC2086
"$PSOUP" "$OUT/EurasiaPackager.vfuel" "$NEWSPEAK/EurasiaKernel.ns" "$EURASIA/kernel/natives.tsv" \
  "$NEWSPEAK/EurasiaKernelTestRunner.ns" $MODULES "$OUT/genesis.eus"
cargo build -q --release -p eurasia-native
EURASIA_BIN=$EURASIA/target/release/eurasia
(cd "$OUT" && "$EURASIA_BIN" genesis.eus base.eus)
EURASIA_GC_STRESS=${GC_STRESS:-0} "$EURASIA_BIN" "$OUT/base.eus" > "$OUT/eurasia-raw.txt" 2>&1 || true
grep '>>' "$OUT/eurasia-raw.txt" | sort > "$OUT/eurasia.txt" || true

echo "== comparing outcomes"
outcomes() { awk '{ print $1, $2 }' "$1"; }
outcomes "$OUT/psoup.txt" > "$OUT/psoup-outcomes.txt"
outcomes "$OUT/eurasia.txt" > "$OUT/eurasia-outcomes.txt"
echo "psoup:   $(wc -l < "$OUT/psoup-outcomes.txt") tests, $(grep -c ' success$' "$OUT/psoup-outcomes.txt" || true) successes"
echo "Eurasia: $(wc -l < "$OUT/eurasia-outcomes.txt") tests, $(grep -c ' success$' "$OUT/eurasia-outcomes.txt" || true) successes"
diff "$OUT/psoup-outcomes.txt" "$OUT/eurasia-outcomes.txt" > "$OUT/differences.txt" || true
# Expected differences: lines '- `Context>>selector`' in the document.
grep -o '^- `[^`]*>>[^`]*`' "$EURASIA/docs/EXPECTED_DIFFERENCES.md" 2>/dev/null | tr -d '`-' | tr -d ' ' | sort -u > "$OUT/expected.txt" || true
grep '^[<>]' "$OUT/differences.txt" | awk '{ print $2 }' | sort -u > "$OUT/differing.txt" || true
unexpected=$(comm -23 "$OUT/differing.txt" "$OUT/expected.txt")
stale=$(comm -13 "$OUT/differing.txt" "$OUT/expected.txt")
if [ -n "$stale" ]; then
  echo "Listed as expected differences but now agreeing (remove them from docs/EXPECTED_DIFFERENCES.md):"
  echo "$stale"
fi
if [ -n "$unexpected" ]; then
  echo "Unexpected differences:"
  for t in $unexpected; do
    grep -F "$t " "$OUT/psoup.txt" | sed 's/^/  psoup:   /' || echo "  psoup:   (missing) $t"
    grep -F "$t " "$OUT/eurasia.txt" | sed 's/^/  Eurasia: /' || echo "  Eurasia: (missing) $t"
  done
  exit 1
fi
echo "Kernel tests: every difference from psoup is expected."
