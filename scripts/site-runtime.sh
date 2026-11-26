#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# The site runtime (newspeak/EURASIA_WEB_PUBLISHING_DESIGN_2026-11-23.md,
# section 2.1): everything a site of Newspeak documents serves besides the
# documents themselves, built from the repositories and nothing else.
#
#   scripts/site-runtime.sh [--with-ide] OUTDIR
#
# Builds the document viewer (scripts/build.sh viewer) and writes to OUTDIR
#
#   eurasia.mjs, eurasia_web.wasm   the loader and the VM
#   images.mjs                      saved images (Save Image), kept in the browser
#   viewer/base.eus                 the document viewer (AmpleforthViewer)
#   eurasia-viewer.html             the viewer's page: eurasia-viewer.html?docName=<document>
#   deploy-boot.js                  the boot of every document page (newspeak's), and of the viewer's
#   custom-post.js                  page helpers (safeDownloadBlob)
#   CodeMirror/ jszip.min.js vendor/   the libraries (web/libs, see its README)
#   *.png                           the images the viewer loads
#
# With --with-ide, the IDE too (scripts/build.sh viewer ide), for a site that
# offers it:
#
#   ide/base.eus                    the IDE (HopscotchWebIDE)
#   eurasia-ide.html                its page (web/ide.html)
#   app.html                        the page of an application deployed from the IDE
#   *.png                           the images the IDE loads as well
#
# Every file it copies is tracked in a repository: the libraries are web/libs;
# the images are the viewer's image resources (target/build/viewer.images.txt,
# written by the build), each of which must be a tracked newspeak/<name>.png;
# custom-post.js is primordialsoup's, deploy-boot.js newspeak's. Nothing is read from newspeak/out/.
#
# See BUILDING.md for what the build needs.
set -eu
APPS=viewer
if [ "${1:-}" = --with-ide ]; then
  APPS="viewer ide"
  shift
fi
test $# = 1 || { echo "usage: $0 [--with-ide] OUTDIR" >&2; exit 2; }
mkdir -p "$1"
OUTDIR=$(cd "$1" && pwd)
cd "$(dirname "$0")/.."
EURASIA=$(pwd)
NEWSPEAK=$(cd "${NEWSPEAK:-$EURASIA/../newspeak}" && pwd)
PRIMORDIALSOUP=$(cd "${PRIMORDIALSOUP:-$EURASIA/../primordialsoup}" && pwd)
export NEWSPEAK PRIMORDIALSOUP

echo "== built from source (scripts/build.sh $APPS)"
# shellcheck disable=SC2086
scripts/build.sh $APPS
IMAGES=$(for app in $APPS; do cat "target/build/$app.images.txt"; done | sort -u)

echo "== the images"
missing=''
for n in $IMAGES; do
  git -C "$NEWSPEAK" ls-files --error-unmatch "$n.png" > /dev/null 2>&1 || missing="$missing $n.png"
done
test -z "$missing" || { echo "not tracked in $NEWSPEAK:$missing" >&2; exit 1; }

echo "== staging in $OUTDIR"
mkdir -p "$OUTDIR/viewer"
cp target/build/viewer/base.eus "$OUTDIR/viewer/"
cp web/eurasia.mjs web/images.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm "$OUTDIR/"
cp web/viewer.html "$OUTDIR/eurasia-viewer.html"
cp "$PRIMORDIALSOUP/meta/custom-post.js" "$OUTDIR/"
cp "$NEWSPEAK/deploy-boot.js" "$OUTDIR/"
# Everything in web/libs but its README, which must not land on (or replace)
# a README of the target's own.
(cd web/libs && find . -type f ! -name README.md | while read -r f; do
  mkdir -p "$OUTDIR/$(dirname "$f")" && cp "$f" "$OUTDIR/$f"
done)
for n in $IMAGES; do
  cp "$NEWSPEAK/$n.png" "$OUTDIR/"
done
if [ "$APPS" != viewer ]; then
  mkdir -p "$OUTDIR/ide"
  cp target/build/ide/base.eus "$OUTDIR/ide/"
  cp web/ide.html "$OUTDIR/eurasia-ide.html"
  cp web/app.html "$OUTDIR/"
fi
echo "Site runtime in $OUTDIR; documents: eurasia-viewer.html?docName=<document>, beside <document>.zip."
