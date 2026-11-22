#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The Newspeak IDE on Eurasia (M6 step 6): builds it from source with the
# build (scripts/build.sh ide: newspeak/EurasiaBuild.ns generates its entry
# and finds its modules, the packager packages them, genesis makes the
# image, stamped by BuildInfo), and stages the image, the page
# (web/ide.html), the VM and the libraries and images psoup's IDE page uses in
# target/web, to be served and opened as ide.html. Needs what
# scripts/genesis.sh needs, and the newspeak checkout's out/ for the libraries.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
OUT=$EURASIA/target/ide
WEB=$EURASIA/target/web
mkdir -p "$OUT" "$WEB/ide" "$WEB/vendor"

echo "== the IDE, built from source (scripts/build.sh ide)"
scripts/build.sh ide
mkdir -p "$OUT"
cp target/build/ide/base.eus target/build/ide.entry.ns target/build/ide.modules.txt "$OUT/"
ls -l "$OUT/base.eus"
echo "== staging for a browser"
cp "$OUT/base.eus" "$WEB/ide/"
cp web/ide.html web/app.html web/eurasia.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm "$WEB/"
# psoup's page helpers (safeDownloadBlob), canonical in primordialsoup.
cp "$PRIMORDIALSOUP/meta/custom-post.js" "$WEB/"
cp -R "$NEWSPEAK/out/CodeMirror" "$WEB/"
cp "$NEWSPEAK/out/jszip.min.js" "$WEB/"
for v in buffer-6.0.3.bundle.min.js isomorphic-git-1.38.4.umd.min.js lightning-fs-4.6.2.min.js \
         isomorphic-git-1.38.4-http-web.umd.js diff_match_patch-2019-07.js; do
  cp "$NEWSPEAK/out/vendor/$v" "$WEB/vendor/"
done
cp "$NEWSPEAK"/*.png "$NEWSPEAK"/out/privateImage.png "$NEWSPEAK"/out/protectedImage.png "$NEWSPEAK"/out/publicImage.png "$WEB/"
echo "IDE: serve $WEB and open ide.html."
