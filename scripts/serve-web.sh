#!/bin/sh
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# Serves target/web (staged by scripts/web-tests.sh and scripts/ide.sh) to an
# ordinary browser, on port 8093 or the one given:
#   http://localhost:8093/ide.html                         the IDE
#   http://localhost:8093/index.html?snapshot=counter/base.eus   CounterApp
#   http://localhost:8093/index.html                       the base image
# A page recording its session (ide.html?record) can upload its log, which
# lands in target/web/uploads (scripts/serve-web.py). Runs until interrupted.
set -eu
cd "$(dirname "$0")/.."
PORT=${1:-8093}
[ -f target/web/ide.html ] || { echo "nothing staged: run scripts/ide.sh (and scripts/web-tests.sh) first"; exit 1; }
echo "Serving $(pwd)/target/web on http://localhost:$PORT/ (ide.html for the IDE)"
exec python3 scripts/serve-web.py "$PORT" target/web
