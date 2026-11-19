#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# M1b end to end: compile EurasiaFirstLight.ns with the V5 compiler on psoup,
# package it with EurasiaPackager, run the image on Eurasia, and check the
# output. Needs the newspeak and primordialsoup checkouts beside this one
# (override with NEWSPEAK and PRIMORDIALSOUP) and a built psoup with
# WebCompiler.vfuel. Outputs go to target/first-light/.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
OUT=$EURASIA/target/first-light
mkdir -p "$OUT"

echo "== building EurasiaPackager.vfuel"
(cd "$PRIMORDIALSOUP" && ./out/ReleaseX64/primordialsoup ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  ./newspeak/platform/PrimordialSoup/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  PrimordialSoup/RuntimeWithMirrors EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }

echo "== packaging EurasiaFirstLight.ns"
"$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup" "$OUT/EurasiaPackager.vfuel" \
  "$NEWSPEAK/EurasiaFirstLight.ns" Main "$OUT/first-light.eus"

echo "== running on Eurasia"
cargo run -q -p eurasia-native -- "$OUT/first-light.eus" > "$OUT/output.txt"
expected=$(printf '7\nwrongKind\naddFromInteger:\n20')
if [ "$(cat "$OUT/output.txt")" = "$expected" ]; then
  cat "$OUT/output.txt"
  echo "First light from source: output as expected."
else
  echo "Unexpected output:"; cat "$OUT/output.txt"; exit 1
fi
