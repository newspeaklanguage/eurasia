#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
"""Serves target/web to a browser (scripts/serve-web.sh), on localhost only.

Besides serving files it accepts `PUT /uploads/NAME` and saves the body as
target/web/uploads/NAME: a page recording a session (ide.html?record) can
send its log there, `eurasiaUploadLog()`, instead of downloading it.

Usage: serve-web.py PORT DIRECTORY
"""

import http.server
import os
import re
import sys
from functools import partial


class Handler(http.server.SimpleHTTPRequestHandler):
    def do_PUT(self):
        match = re.fullmatch(r"/uploads/([A-Za-z0-9._-]+)", self.path)
        if not match:
            self.send_error(403, "uploads only")
            return
        length = int(self.headers.get("Content-Length", "0"))
        uploads = os.path.join(self.directory, "uploads")
        os.makedirs(uploads, exist_ok=True)
        with open(os.path.join(uploads, match.group(1)), "wb") as f:
            remaining = length
            while remaining > 0:
                chunk = self.rfile.read(min(remaining, 1 << 20))
                if not chunk:
                    break
                f.write(chunk)
                remaining -= len(chunk)
        self.send_response(201)
        self.end_headers()


def main():
    port, directory = int(sys.argv[1]), sys.argv[2]
    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", port), partial(Handler, directory=directory))
    server.serve_forever()


main()
