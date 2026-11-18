#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# This script contains excerpts of Primordial Soup's Newspeak sources,
# Copyright 2008, the Newspeak project authors, licensed under the Apache
# License, Version 2.0; see the LICENSE file.
"""Derive EurasiaKernel.ns from psoup's KernelForPrimordialSoup.ns.

Usage: fork-kernel.py PSOUP_KERNEL.ns OUTPUT.ns

Every change from psoup's kernel is here, explicit, and checked: each edit
asserts how many times its pattern occurs, so a change in the source kernel
fails the script instead of producing a silently different fork. See the
newspeak repository's EURASIA_GENESIS_KERNEL_PLAN_2026-11-17.md, section 3.

The output is then pretty-printed and parse-validated like any .ns file.
Once the fork is maintained by hand, this script records how it began.
"""
import re
import sys


def replace(text, old, new, count=1):
    found = text.count(old)
    if found != count:
        sys.exit(f"fork-kernel: expected {count} of {old[:70]!r}, found {found}")
    return text.replace(old, new)


def main():
    source, output = sys.argv[1], sys.argv[2]
    t = open(source).read()

    # 1. The module's name.
    t = replace(t, "class KernelForPrimordialSoup new = (", "class EurasiaKernel new = (")
    t = replace(t, "outer KernelForPrimordialSoup ", "outer EurasiaKernel ", t.count("outer KernelForPrimordialSoup "))

    # 2. Primitive pragmas out: natives are the packager's table, never source
    #    (the V5 compiler rejects them). Each pragma is followed by a line of
    #    indentation the printer emits after a leading comment.
    pragma = re.compile(r"\n\t*\(\* :literalmessage: primitive: \d+ \*\)\n\t*(?=\n)")
    count = len(pragma.findall(t))
    if count != 130:
        sys.exit(f"fork-kernel: expected 130 primitive pragmas, found {count}")
    t = pragma.sub("", t)
    t = re.sub(r"\(\* :literalmessage: primitive: \d+ \*\)", "", t)

    # 3. Module slots the genesis packager fills: the VM mirror, the role
    #    methods (compared by identity instead of psoup's marker primitives),
    #    and the application the genesis entry hands the base image to.
    t = replace(t, """		private messageLoop <MessageLoop>
		private symbolTable <WeakArray[Symbol]>
		private symbolTableUsed
	|""", """		private messageLoop <MessageLoop>
		private symbolTable <WeakArray[Symbol]>
		private symbolTableUsed
		private vm
		private onDoMethod
		private ensureMethod
		private simulationRootMethod
		private dispatchMethod
		private application
	|""")

    # 4. Marker primitives become method identity.
    t = replace(t, "^113 = method primitive", "^ensureMethod == method")
    t = replace(t, "activation method primitive = 142 ifTrue:", "simulationRootMethod == activation method ifTrue:")
    t = replace(t, "activation method primitive = 168 ifTrue:", "dispatchMethod == activation method ifTrue:")
    t = replace(t, "activation method primitive = 116 ifTrue:", "onDoMethod == activation method ifTrue:")

    # 5. Unhandled exceptions: until actors (M4), print a trace and exit.
    t = replace(t, "messageLoop unhandledException: self from: signalActivation sender.",
                "unhandled: self from: signalActivation sender.", 2)

    # 6. terminateTo: — the VM retires the frames in between.
    t = replace(t, """			| currentContext sendingContext |
			(self hasSender: previousContext) ifTrue: [
					currentContext:: sender.
					[ currentContext = previousContext ] whileFalse: [
							sendingContext:: currentContext sender.
							currentContext terminate.
							currentContext:: sendingContext
						]
				].
			sender:: previousContext""", """			(* Eurasia: making previousContext the sender retires the frames in between. *)
			sender:: previousContext""")

    # 7. The privileged helpers go through the VM mirror (decided 2026-11-17).
    helpers = {
        "private classOf: object ^<Class> = (": "^vm classOf: object",
        "private currentActivation ^<Activation> = (": "^vm currentActivation sender",
        "private currentMonotonicNanos = (": "^vm monotonicNanos",
        "private definingActivationOf: closure <Closure> ^<Activation> = (": "^vm definingActivationOf: closure",
        "public garbageCollect = (": "^vm garbageCollect",
        "private identityHashOf: a = (": "^vm identityHashOf: a",
        "private instantiate: klass = (": "^vm instantiate: klass",
        "private is: a identicalTo: b = (": "^vm is: a identicalTo: b",
        "private isCanonical: object = (": "^vm isCanonical: object",
        "private object: rcvr perform: sel with: arguments = (": "^vm object: rcvr perform: sel with: arguments",
        "private setCanonical: object = (": "^vm setCanonical: object",
        "private slotOf: object at: index = (": "^vm slotOf: object at: index",
        "private slotOf: object at: index put: value = (": "^vm slotOf: object at: index put: value",
    }
    for header, body in helpers.items():
        pattern = re.compile(re.escape("\t" + header) + r"\n\t\thalt\n\t\)")
        if len(pattern.findall(t)) != 1:
            sys.exit(f"fork-kernel: helper {header!r} not found as expected")
        t = pattern.sub("\t" + header + "\n\t\t" + body + "\n\t)", t)
    t = replace(t, """	print: message = (
	)""", """	print: message = (
		vm print: message
	)""")

    # 8. Class layout accessors read Eurasia's contract fields (snapshot
    #    format section 5: class = mixin, superclass, enclosing object, name,
    #    library; metaclass = mixin, superclass, enclosing object, this class,
    #    library). Writing a class is not allowed (installed code changes by
    #    commit), so the setters, used only by runtime class creation (not
    #    yet supported on Eurasia), fail through the mirror.
    # (Order matters: methods move off slot 2 before the superclass moves to it.)
    for old, new in [
        ("^self slotOf: behavior at: 2\n", "^self slotOf: (mixinOf: behavior) at: 2\n"),
        ("^self slotOf: behavior at: 1\n", "^self slotOf: behavior at: 2\n"),
        ("^self slotOf: behavior at: 4\n", "^self slotOf: behavior at: 1\n"),
        ("^self slotOf: klass at: 7\n", "^self slotOf: klass at: 4\n"),
        ("^self slotOf: metaclass at: 7\n", "^self slotOf: metaclass at: 4\n"),
    ]:
        t = replace(t, old, new)

    # 9. The genesis entry replaces psoup's object-store builder.
    start = t.index("\tpublic buildObjectStoreWithApplication: app platform: platform symbols: symbols = (")
    end = t.index("\n\t)\n", start) + len("\n\t)\n")
    t = t[:start] + """	public main: v args: args = (
		(* The genesis entry (Eurasia). The genesis packager built this module's classes and filled its slots; Eurasia runs this once, makes the application the entry of the image it saves, and saves the base image to the path given as the first argument. *)

		vm setRole: 32 to: application.
		vm writeBytes: vm snapshot toFile: (args at: 1)
	)
	unhandled: exception from: activation = (
		(* Until actors (M4): print the exception and a stack trace, then exit. *)

		| a |
		vm print: 'Unhandled exception: ' , exception printString.
		a:: activation.
		[ nil = a ] whileFalse: [
				vm print: '  ' , (traceLineFor: a).
				a:: a sender
			].
		vm exit: 1
	)
	traceLineFor: activation = (
		| method |
		method:: activation method.
		nil = method ifTrue: [ ^'?' ].
		^(nameOrQuestionMark: (vm slotOf: (vm classOf: activation receiver) at: 4)) , '>>' , (vm slotOf: method at: 3)
	)
	nameOrQuestionMark: name = (
		^nil = name ifTrue: [ '?' ] ifFalse: [ name ]
	)
""" + t[end:]

    # 10. The native failure class: an Exception whose own five slots the VM
    #     fills (rule Native-Fail), and the VM-mirror class whose methods the
    #     packager replaces with natives.
    anchor = "\tpublic class Fraction reducedNumerator: num denominator: denom = Number new ("
    t = replace(t, anchor, """	public class NativeFailure new = Exception new (
		(* Signalled when a native fails (rule Native-Fail). The VM fills the five slots below. *)

		|
			public nativeName
			public failedReceiver
			public arguments
			public reason
			public reasonArgument
		|
	) (
		public messageText = (
			^nativeName , ' failed: ' , reason , (nil = reasonArgument ifTrue: [ '' ] ifFalse: [ ' ' , reasonArgument printString ])
		)
	) : ()
""" + anchor)
    anchor = "\tpublic class WeakArray _cannotInstantiate = Collection new ("
    t = replace(t, anchor, """	public class VMMirror _cannotInstantiate = (
		(* The VM mirror. Every method here is replaced by a native from the genesis packager's table; the bodies are never run. *)

	) (
		public classOf: object = (
		)
		public currentActivation = (
		)
		public definingActivationOf: closure = (
		)
		public exit: code = (
		)
		public garbageCollect = (
		)
		public identityHashOf: object = (
		)
		public instantiate: klass = (
		)
		public is: a identicalTo: b = (
		)
		public isCanonical: object = (
		)
		public monotonicNanos = (
		)
		public object: receiver perform: selector with: arguments = (
		)
		public print: text = (
		)
		public readFile: path = (
		)
		public setCanonical: object = (
		)
		public setRole: id to: value = (
		)
		public slotOf: object at: index = (
		)
		public slotOf: object at: index put: value = (
		)
		public snapshot = (
		)
		public writeBytes: bytes toFile: path = (
		)
	) : ()
""" + anchor)

    # 11. Classes, metaclasses, methods and mixins are Eurasia contract
    #     objects, not psoup-layout objects. Their slots are declared in the
    #     contract order (snapshot format section 5), so a slot getter reads
    #     the contract field at that position (Behavior: mixin, superclass,
    #     enclosing object; Class: name, library; Metaclass: this class,
    #     library; Method: literals, mixin, selector, source, library; mixins:
    #     name, methods, enclosing mixin, slot names, library). psoup-only
    #     fields become accessors answering neutral values: they serve runtime
    #     class creation and reflective method decoding, which arrive with the
    #     mirrors (M6).
    t = replace(t, """			private superclass <Behavior> (* Must be slot 1, known to the VM. *)
			private methods <Array[Method]> (* Must be slot 2, known to the VM. *)
			private enclosingObject <Object> (* Must be slot 3, known to the VM. *)
			private mixin <AbstractMixin> (* Must be slot 4, known to the VM. *)
			private classIndex <SmallInteger> (* Must be slot 5, known to the VM. *)
			private format <SmallInteger> (* Must be slot 6, known to the VM. *)""",
"""			private mixin <AbstractMixin> (* Eurasia contract field 1. *)
			private superclass <Behavior> (* Eurasia contract field 2. *)
			private enclosingObject <Object> (* Eurasia contract field 3. *)""")
    t = replace(t, """			private nameS <Symbol> (* Must be slot 7, known to the VM for debug printing only. *)
			private subclasses <WeakArray>""",
"""			private nameS <Symbol> (* Eurasia contract field 4. *)
			private library (* Eurasia contract field 5: the mirror library's. *)""")
    t = replace(t, """			private thisClass <Class>
		|""", """			private thisClass <Class> (* Eurasia contract field 4. *)
			private library (* Eurasia contract field 5: the mirror library's. *)
		|""")
    t = replace(t, """			public header <Integer> (* Must be slot 1, known to the VM. *)
			public literals <Array> (* Must be slot 2, known to the VM. *)
			public bytecode <ByteArray> (* Must be slot 3, known to the VM. *)
			public mixin <AbstractMixin> (* Must be slot 4, known to the VM. *)
			public selector <Symbol> (* Must be slot 5, known to the VM. *)
			public metadata <String | MethodMetadata | DebugInfo | Nil>
		|
	) (""", """			public literals <Array> (* Eurasia contract field 1. *)
			public mixin <AbstractMixin> (* Eurasia contract field 2. *)
			public selector <Symbol> (* Eurasia contract field 3. *)
			public metadata <String | MethodMetadata | DebugInfo | Nil> (* Eurasia contract field 4: source. *)
			public library (* Eurasia contract field 5: the mirror library's. *)
		|
	) (
		public header ^<Integer> = (
			(* Eurasia keeps method headers in the VM; decoding them through a mirror arrives with M6. Until then every method reads as public, with no arguments, temps or primitive. *)

			^0
		)
		public bytecode = (
			^nil
		)""")
    t = replace(t, """			public _instanceMixin <InstanceMixin>
			public _methods <Array[Method]> (* Must be slot 2, known to the VM. *)
			public _enclosingMixin <InstanceMixin | nil> (* Must be slot 3, known to the VM. *)
		|
	) (
		public _name ^<String> = (
			^_instanceMixin _name , ' class'
		)""", """			public _name <Symbol> (* Eurasia contract field 1. *)
			public _methods <Array[Method]> (* Eurasia contract field 2. *)
			public _enclosingMixin <InstanceMixin | nil> (* Eurasia contract field 3. *)
			public _slotNames (* Eurasia contract field 4. *)
			public _library (* Eurasia contract field 5: the mirror library's. *)
		|
	) (""")
    t = replace(t, """			public _slots <Array[{Symbol. Boolean. Symbol}]>
			public _nestedMixins <Array[InstanceMixin]>
			public _applications <WeakArray>
			public _classMixin <ClassMixin>
			public _accessModifier <Symbol>
			public _primaryFactorySelector <Symbol>
			public _headerSource <String>
		|
	) (""", """			public _slots (* Eurasia contract field 4: slot names. *)
			public _library (* Eurasia contract field 5: the mirror library's. *)
		|
	) (
		public _accessModifier = (
			^#public
		)
		public _applications = (
			^nil
		)
		public _classMixin = (
			^nil
		)
		public _headerSource = (
			^nil
		)
		public _nestedMixins = (
			^{}
		)
		public _primaryFactorySelector = (
			^nil
		)""")

    open(output, "w").write(t)
    print(f"fork-kernel: wrote {output}")


main()
