#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# This script contains excerpts of Primordial Soup's Newspeak sources,
# Copyright 2008, the Newspeak project authors, licensed under the Apache
# License, Version 2.0; see the LICENSE file.
"""Derive newspeak/EurasiaMirrors.ns from psoup's MirrorsForPrimordialSoup.ns.

The mirrors on Eurasia (newspeak/EURASIA_M6_REFLECTION_DESIGN_2026-11-18.md
section 4.2): psoup's mirror library, with what knows psoup's object layout
changed to Eurasia's contract fields. The kernel's mixins already answer
psoup's mixin fields (kernel/fork-kernel.py), so the changes are few:

- the module's name (the natives table keys its rows by it);
- a class's contract fields are mixin, superclass, enclosing object, name and
  library, and its methods are its mixin's (accessors included, since
  Eurasia's builders synthesize them per mixin, not per application);
- an instance's size is the slots its class's mixins declare;
- the compiler is the V5 one (Newspeak2V5BytecodeCompilation over
  NewspeakBytecodeCompilation, which the namespace provides composed);
- the primitives become natives, by the natives table.

Every edit is checked: the script fails if psoup's source has changed under
it. Usage: fork-mirrors.py <MirrorsForPrimordialSoup.ns> <output>.
"""

import sys


def replace(text, old, new, count=1):
    found = text.count(old)
    if found != count:
        sys.exit(f"fork-mirrors: expected {count} of {old[:70]!r}, found {found}")
    return text.replace(old, new)


def main():
    source, output = sys.argv[1], sys.argv[2]
    t = open(source).read()

    # 1. The module's name.
    t = replace(t, "class MirrorsForPrimordialSoup usingPlatform: platform internalKernel: ik namespace: ns",
                "class EurasiaMirrors usingPlatform: platform internalKernel: ik namespace: ns")

    # 2. Classes: Eurasia's contract fields (mixin, superclass, enclosing
    #    object, name, library). The enclosing object is field 3 on both.
    t = replace(t, """	private mixinOf: behavior = (
		^self slotOf: behavior at: 4
	)""", """	private mixinOf: behavior = (
		(* Eurasia: contract field 1. *)

		^self slotOf: behavior at: 1
	)""")
    t = replace(t, """	private superclassOf: behavior = (
		^self slotOf: behavior at: 1
	)""", """	private superclassOf: behavior = (
		(* Eurasia: contract field 2. *)

		^self slotOf: behavior at: 2
	)""")
    t = replace(t, """	private methodsOf: behavior = (
		^self slotOf: behavior at: 2
	)""", """	private methodsOf: behavior = (
		(* Eurasia: a class has no methods of its own; its mixin's include the slot accessors, which are relative to the mixin. *)

		^(mixinOf: behavior) _methods
	)""")
    t = replace(t, """	private formatOf: behavior = (
		^self slotOf: behavior at: 6
	)""", """	private formatOf: behavior = (
		(* Eurasia: the number of instance slots, the slots every mixin along the superclass chain declares (psoup keeps it in the class). *)

		| cls n |
		cls:: behavior.
		n:: 0.
		[ nil = cls ] whileFalse: [
				n:: n + (mixinOf: cls) _slots size.
				cls:: superclassOf: cls
			].
		^n
	)""")

    # 3. The V5 compiler.
    t = replace(t, """		compilation:: namespace Newspeak2PrimordialSoupCompilation usingPlatform: cachedPlatform""",
                """		compilation:: namespace Newspeak2V5BytecodeCompilation usingPlatform: cachedPlatform""")

    open(output, "w").write(t)
    print(f"fork-mirrors: wrote {output}")


main()
