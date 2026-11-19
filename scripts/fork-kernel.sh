#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Regenerate the Eurasia forks of psoup's modules: newspeak/platform/Eurasia/InternalKernel.ns
# from psoup's platform/PrimordialSoup/InternalKernel.ns (kernel/fork-kernel.py) and
# newspeak/platform/Eurasia/Actors.ns from psoup's Actors.ns
# (kernel/fork-actors.py) and newspeak/platform/Eurasia/Mirrors.ns from
# psoup's Mirrors.ns (kernel/fork-mirrors.py), pretty-printed and
# parse-validated as the newspeak
# repository requires. Each output replaces its file only if it parses.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
TMP=$EURASIA/target/fork-kernel
mkdir -p "$TMP"
python3 kernel/fork-kernel.py "$PRIMORDIALSOUP/newspeak/platform/PrimordialSoup/InternalKernel.ns" "$TMP/raw.ns"
cd "$NEWSPEAK/tool"
# parse-validate exits 0 even on failure: gate on its output.
./parse-validate.sh "$TMP/raw.ns" | grep -q "is syntactically valid" \
  || { echo "the fork does not parse"; ./parse-validate.sh "$TMP/raw.ns"; exit 1; }
./pretty-print.sh "$TMP/raw.ns" "$TMP/printed.ns" > /dev/null
./parse-validate.sh "$TMP/printed.ns" | grep -q "is syntactically valid" \
  || { echo "the printed fork does not parse"; exit 1; }
cp "$TMP/printed.ns" "$NEWSPEAK/platform/Eurasia/InternalKernel.ns"
echo "fork-kernel: wrote $NEWSPEAK/platform/Eurasia/InternalKernel.ns"

# The actors module (kernel/fork-actors.py), the same way.
python3 "$EURASIA/kernel/fork-actors.py" "$PRIMORDIALSOUP/newspeak/platform/PrimordialSoup/Actors.ns" "$TMP/actors-raw.ns"
./parse-validate.sh "$TMP/actors-raw.ns" | grep -q "is syntactically valid" \
  || { echo "the actors fork does not parse"; ./parse-validate.sh "$TMP/actors-raw.ns"; exit 1; }
./pretty-print.sh "$TMP/actors-raw.ns" "$TMP/actors-printed.ns" > /dev/null
./parse-validate.sh "$TMP/actors-printed.ns" | grep -q "is syntactically valid" \
  || { echo "the printed actors fork does not parse"; exit 1; }
cp "$TMP/actors-printed.ns" "$NEWSPEAK/platform/Eurasia/Actors.ns"
echo "fork-kernel: wrote $NEWSPEAK/platform/Eurasia/Actors.ns"

# The mirrors module (kernel/fork-mirrors.py), the same way.
python3 "$EURASIA/kernel/fork-mirrors.py" "$PRIMORDIALSOUP/newspeak/platform/PrimordialSoup/Mirrors.ns" \
  "$NEWSPEAK/V5BytecodeSimulation.ns" "$NEWSPEAK/upstream-V5Simulator-338f0bb3.ns.txt" "$TMP/mirrors-raw.ns"
./parse-validate.sh "$TMP/mirrors-raw.ns" | grep -q "is syntactically valid" \
  || { echo "the mirrors fork does not parse"; ./parse-validate.sh "$TMP/mirrors-raw.ns"; exit 1; }
./pretty-print.sh "$TMP/mirrors-raw.ns" "$TMP/mirrors-printed.ns" > /dev/null
./parse-validate.sh "$TMP/mirrors-printed.ns" | grep -q "is syntactically valid" \
  || { echo "the printed mirrors fork does not parse"; exit 1; }
cp "$TMP/mirrors-printed.ns" "$NEWSPEAK/platform/Eurasia/Mirrors.ns"
echo "fork-kernel: wrote $NEWSPEAK/platform/Eurasia/Mirrors.ns"
