#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The Newspeak IDE and the document viewer on Eurasia (M6 step 6): builds both
# from source with the build (scripts/build.sh ide viewer:
# newspeak/EurasiaBuild.ns generates each one's entry and finds its modules,
# the packager packages them, genesis makes their images, stamped by
# BuildInfo), and stages the images, the pages (web/ide.html, web/viewer.html),
# the VM and the libraries and images psoup's IDE page uses in target/web, to
# be served and opened as ide.html and viewer.html?docName=<document>.
#
# It also stages them in the newspeak checkout's out/, beside psoup's
# primordialsoup.html, so that the server that serves out/ (localhost:8080)
# serves Eurasia too: eurasia-ide.html (image ide/base.eus) and
# eurasia-viewer.html?docName=<document> (image viewer/base.eus), with the VM
# (eurasia.mjs, eurasia_web.wasm) and custom-post.js; the libraries and icons
# these pages load are out/'s own. Set EURASIA_NO_OUT=1 to skip it.
#
# Needs what scripts/genesis.sh needs, and the newspeak checkout's out/ for the
# libraries.
set -eu
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=${NEWSPEAK:-$EURASIA/../newspeak}
PRIMORDIALSOUP=${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}
OUT=$EURASIA/target/ide
WEB=$EURASIA/target/web
mkdir -p "$OUT" "$WEB/ide" "$WEB/viewer" "$WEB/vendor"

echo "== the IDE and the viewer, built from source (scripts/build.sh ide viewer)"
scripts/build.sh ide viewer
cp target/build/ide/base.eus target/build/ide.entry.ns target/build/ide.modules.txt "$OUT/"
ls -l "$OUT/base.eus" target/build/viewer/base.eus
echo "== staging for a browser"
cp "$OUT/base.eus" "$WEB/ide/"
cp target/build/viewer/base.eus "$WEB/viewer/"
cp web/ide.html web/viewer.html web/app.html web/eurasia.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm "$WEB/"
# psoup's page helpers (safeDownloadBlob), canonical in primordialsoup.
cp "$PRIMORDIALSOUP/meta/custom-post.js" "$WEB/"
cp -R "$NEWSPEAK/out/CodeMirror" "$WEB/"
cp "$NEWSPEAK/out/jszip.min.js" "$WEB/"
for v in buffer-6.0.3.bundle.min.js isomorphic-git-1.38.4.umd.min.js lightning-fs-4.6.2.min.js \
         isomorphic-git-1.38.4-http-web.umd.js diff_match_patch-2019-07.js; do
  cp "$NEWSPEAK/out/vendor/$v" "$WEB/vendor/"
done
cp "$NEWSPEAK"/*.png "$NEWSPEAK"/out/privateImage.png "$NEWSPEAK"/out/protectedImage.png "$NEWSPEAK"/out/publicImage.png "$WEB/"
echo "IDE: serve $WEB and open ide.html; documents: viewer.html?docName=<document>."

if [ "${EURASIA_NO_OUT:-}" != 1 ]; then
  echo "== staging in $NEWSPEAK/out"
  NSOUT=$NEWSPEAK/out
  mkdir -p "$NSOUT/ide" "$NSOUT/viewer"
  cp "$OUT/base.eus" "$NSOUT/ide/"
  cp target/build/viewer/base.eus "$NSOUT/viewer/"
  cp web/eurasia.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm "$PRIMORDIALSOUP/meta/custom-post.js" "$NSOUT/"
  cp web/ide.html "$NSOUT/eurasia-ide.html"
  cp web/viewer.html "$NSOUT/eurasia-viewer.html"
  echo "Served from out/: eurasia-ide.html, eurasia-viewer.html?docName=<document>."
fi
