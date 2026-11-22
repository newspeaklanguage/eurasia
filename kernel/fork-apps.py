#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
"""Eurasia's forks of the command-line applications that psoup and Eurasia
share (EURASIA_BUILD_AND_DEPLOY_DESIGN_2026-11-21.md, section 6: no module
Eurasia runs carries a primitive).

The genesis packager and psoup's tools read and write files with psoup's
primitive pragmas (130 reads, 128 writes). Each fork, in
newspeak/platform/Eurasia/, is the same module with those pragmas replaced by a
plain comment: its methods readFileAsBytes: and writeBytes:toFileNamed: are
Eurasia's natives VmReadFile and VmWriteFile by the natives table
(kernel/natives.tsv), as the mirrors' natives are.

  kernel/fork-apps.py           writes the forks, pretty-printed
  kernel/fork-apps.py --check   fails when a fork is not its module's
"""

import os
import re
import subprocess
import sys
import tempfile

EURASIA = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NEWSPEAK = os.environ.get("NEWSPEAK", os.path.join(EURASIA, "..", "newspeak"))
FORKS = os.path.join(NEWSPEAK, "platform", "Eurasia")

# The applications forked, from the newspeak checkout.
APPS = ["EurasiaPackager", "NewspeakParser", "NewspeakTestRunner", "NewspeakPrettyPrintApp",
        "NewspeakRoundTripCheckApp", "NewspeakAttachmentDiffApp"]

NATIVES = {"130": "VmReadFile", "128": "VmWriteFile"}


def forked(app):
    with open(os.path.join(NEWSPEAK, app + ".ns"), encoding="utf-8") as f:
        text = f.read()

    def comment(m):
        number = m.group(1)
        if number not in NATIVES:
            sys.exit(f"fork-apps: {app} has psoup's primitive {number}, which Eurasia does not have")
        return f"(* Eurasia: a native ({NATIVES[number]}), by the natives table; psoup's primitive {number}. *)"

    text, count = re.subn(r"\(\* :literalmessage: primitive: (\d+) \*\)", comment, text)
    if count == 0:
        sys.exit(f"fork-apps: {app} has no primitive to fork away")
    lines = text.split("\n", 2)
    if lines[0] != "Newspeak3":
        sys.exit(f"fork-apps: {app} does not start with Newspeak3")
    return "\n".join([lines[0], "'Eurasia'", lines[2]])


def printed(app):
    tool = os.path.join(NEWSPEAK, "tool")
    with tempfile.TemporaryDirectory() as d:
        raw = os.path.join(d, app + "-raw.ns")
        out = os.path.join(d, app + ".ns")
        with open(raw, "w", encoding="utf-8") as f:
            f.write(forked(app))
        subprocess.run(["./pretty-print.sh", raw, out], cwd=tool, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with open(out, encoding="utf-8") as f:
            return f.read()


def main():
    check = sys.argv[1:] == ["--check"]
    stale = []
    for app in APPS:
        text = printed(app)
        path = os.path.join(FORKS, app + ".ns")
        if check:
            if not os.path.exists(path) or open(path, encoding="utf-8").read() != text:
                stale.append(app)
        else:
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            print(f"fork-apps: wrote {path}")
    if stale:
        sys.exit(f"fork-apps: stale forks ({', '.join(stale)}): run kernel/fork-apps.py")
    if check:
        print(f"fork-apps: {len(APPS)} forks match their modules")


if __name__ == "__main__":
    main()
