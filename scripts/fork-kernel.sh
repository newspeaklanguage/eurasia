#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Regenerate the Eurasia forks of psoup's modules: newspeak/EurasiaKernel.ns
# from KernelForPrimordialSoup.ns (kernel/fork-kernel.py) and
# newspeak/EurasiaActors.ns from ActorsForPrimordialSoup.ns
# (kernel/fork-actors.py), pretty-printed and parse-validated as the newspeak
# repository requires. Each output replaces its file only if it parses.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
TMP=$EURASIA/target/fork-kernel
mkdir -p "$TMP"
python3 kernel/fork-kernel.py "$PRIMORDIALSOUP/newspeak/KernelForPrimordialSoup.ns" "$TMP/raw.ns"
cd "$NEWSPEAK/tool"
# parse-validate exits 0 even on failure: gate on its output.
./parse-validate.sh "$TMP/raw.ns" | grep -q "is syntactically valid" \
  || { echo "the fork does not parse"; ./parse-validate.sh "$TMP/raw.ns"; exit 1; }
./pretty-print.sh "$TMP/raw.ns" "$TMP/printed.ns" > /dev/null
./parse-validate.sh "$TMP/printed.ns" | grep -q "is syntactically valid" \
  || { echo "the printed fork does not parse"; exit 1; }
cp "$TMP/printed.ns" "$NEWSPEAK/EurasiaKernel.ns"
echo "fork-kernel: wrote $NEWSPEAK/EurasiaKernel.ns"

# The actors module (kernel/fork-actors.py), the same way.
python3 "$EURASIA/kernel/fork-actors.py" "$PRIMORDIALSOUP/newspeak/ActorsForPrimordialSoup.ns" "$TMP/actors-raw.ns"
./parse-validate.sh "$TMP/actors-raw.ns" | grep -q "is syntactically valid" \
  || { echo "the actors fork does not parse"; ./parse-validate.sh "$TMP/actors-raw.ns"; exit 1; }
./pretty-print.sh "$TMP/actors-raw.ns" "$TMP/actors-printed.ns" > /dev/null
./parse-validate.sh "$TMP/actors-printed.ns" | grep -q "is syntactically valid" \
  || { echo "the printed actors fork does not parse"; exit 1; }
cp "$TMP/actors-printed.ns" "$NEWSPEAK/EurasiaActors.ns"
echo "fork-kernel: wrote $NEWSPEAK/EurasiaActors.ns"
