#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Regenerate newspeak/EurasiaKernel.ns from psoup's KernelForPrimordialSoup.ns
# (kernel/fork-kernel.py), pretty-printed and parse-validated as the newspeak
# repository requires. The output replaces the file only if it parses.
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
