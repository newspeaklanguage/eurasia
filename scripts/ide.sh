#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The Newspeak IDE on Eurasia (M6 step 6): generates its genesis entry
# (scripts/generate-ide-entry.py), packages it with every module it names,
# runs genesis on the command line, and stages the image, the page
# (web/ide.html), the VM and the libraries and images psoup's IDE page uses in
# target/web, to be served and opened as ide.html. Needs what
# scripts/genesis.sh needs, and the newspeak checkout's out/ for the libraries.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
PSOUP=$PRIMORDIALSOUP/out/ReleaseX64/primordialsoup
OUT=$EURASIA/target/ide
WEB=$EURASIA/target/web
mkdir -p "$OUT" "$WEB/ide" "$WEB/vendor"

echo "== building EurasiaPackager.vfuel"
(cd "$PRIMORDIALSOUP" && "$PSOUP" ./out/snapshots/WebCompiler.vfuel \
  ./newspeak/*.ns \
  "$NEWSPEAK/Newspeak2V5BytecodeCompilation.ns" \
  "$NEWSPEAK/EurasiaPackager.ns" \
  RuntimeWithMirrorsForPrimordialSoup EurasiaPackager "$OUT/EurasiaPackager.vfuel" > "$OUT/build.log" 2>&1) \
  || { echo "building the packager failed; see $OUT/build.log"; exit 1; }

echo "== the IDE's entry and modules"
python3 scripts/generate-ide-entry.py "$NEWSPEAK" "$PRIMORDIALSOUP" "$OUT/EurasiaIDE-raw.ns" > "$OUT/modules.txt"
(cd "$NEWSPEAK/tool" && ./parse-validate.sh "$OUT/EurasiaIDE-raw.ns" | grep -q "is syntactically valid" \
  && ./pretty-print.sh "$OUT/EurasiaIDE-raw.ns" "$NEWSPEAK/EurasiaIDE.ns" > /dev/null) \
  || { echo "the generated IDE entry does not parse"; exit 1; }
MODULES=$(grep '\.ns$' "$OUT/modules.txt" | tr '\n' ' ')

echo "== packaging and genesis"
# shellcheck disable=SC2086
"$PSOUP" "$OUT/EurasiaPackager.vfuel" "$NEWSPEAK/EurasiaKernel.ns" kernel/natives.tsv \
  "$NEWSPEAK/EurasiaIDE.ns" $MODULES "$OUT/genesis.eus" > "$OUT/package.log" 2>&1 \
  || true
grep -q "^Wrote" "$OUT/package.log" || { head -3 "$OUT/package.log"; echo "packaging the IDE failed; see $OUT/package.log"; exit 1; }
cargo build -q --release -p eurasia-native
cargo build -q --release -p eurasia-web --target wasm32-unknown-unknown
(cd "$OUT" && "$EURASIA/target/release/eurasia" genesis.eus base.eus)
ls -l "$OUT/base.eus"

echo "== staging for a browser"
cp "$OUT/base.eus" "$WEB/ide/"
cp web/ide.html web/eurasia.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm "$WEB/"
cp -R "$NEWSPEAK/out/CodeMirror" "$WEB/"
cp "$NEWSPEAK/out/jszip.min.js" "$WEB/"
for v in buffer-6.0.3.bundle.min.js isomorphic-git-1.38.4.umd.min.js lightning-fs-4.6.2.min.js \
         isomorphic-git-1.38.4-http-web.umd.js diff_match_patch-2019-07.js; do
  cp "$NEWSPEAK/out/vendor/$v" "$WEB/vendor/"
done
cp "$NEWSPEAK"/*.png "$NEWSPEAK"/out/privateImage.png "$NEWSPEAK"/out/protectedImage.png "$NEWSPEAK"/out/publicImage.png "$WEB/"
echo "IDE: serve $WEB and open ide.html."
