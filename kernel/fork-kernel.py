#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# This script contains excerpts of Primordial Soup's Newspeak sources,
# Copyright 2008, the Newspeak project authors, licensed under the Apache
# License, Version 2.0; see the LICENSE file.
"""Derive platform/Eurasia/InternalKernel.ns from psoup's KernelForPrimordialSoup.ns.

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
    # Eurasia's platform namespace (PLATFORM_NAMESPACES_DESIGN_2026-11-18.md).
    t = replace(t, "Newspeak3\n'Root'\n", "Newspeak3\n'Eurasia'\n")
    t = replace(t, "class KernelForPrimordialSoup new = (", "class InternalKernel new = (")
    t = replace(t, "outer KernelForPrimordialSoup ", "outer InternalKernel ", t.count("outer KernelForPrimordialSoup "))

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
		private reportingUnhandled
	|""")

    # 4. Marker primitives become method identity.
    t = replace(t, "^113 = method primitive", "^ensureMethod == method")
    t = replace(t, "activation method primitive = 142 ifTrue:", "(vm isSimulationRoot: activation method) ifTrue:")
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
		(* The genesis entry (Eurasia). The genesis packager built this module's classes and filled its slots; Eurasia runs this once and saves the base image to the path given as the first argument. The application builds its platform over this kernel (platformOver:); its actors module builds the message loop, which becomes the image's MessageLoop role, so the base image starts as psoup's images do: the VM delivers the arguments as the startup message, and the loop sends the application main: platform args:. *)

		| platform loop |
		vm setRole: 32 to: application.
		platform:: application platformOver: self.
		loop:: platform actors buildLoopForApplication: application platform: platform.
		vm setRole: 17 to: loop.
		(* The platform template, from which every further actor's platform is cloned: a copy made before the loop runs anything (EURASIA_ACTORS_DESIGN_2026-11-17.md section 5). *)
		vm setRole: 36 to: (vm clone: loop).
		vm writeBytes: vm snapshot toFile: (args at: 1)
	)
	unhandled: exception from: activation = (
		(* An image with a message loop hands the exception to it, as psoup's kernel does (MessageLoop>>unhandledException:from:: the application's handler of last resort, if it set one, and the turn ends; otherwise a trace and exit). Without one (genesis), print the exception and a stack trace, then exit. Pausing the actor, the decided semantics, arrives with the debugger (M6). *)

		| a |
		nil = vm currentMessageLoop ifFalse: [
				^vm currentMessageLoop unhandledException: exception from: activation
			].
		nil = reportingUnhandled ifFalse: [
				vm print: 'Unhandled exception while reporting an unhandled exception'.
				vm exit: 2
			].
		reportingUnhandled:: true.
		vm print: 'Unhandled exception: ' , exception printString.
		a:: activation.
		[ nil = a ] whileFalse: [
				vm print: '  ' , (traceLineFor: a).
				a:: a sender
			].
		vm exit: 1
	)
	traceLineFor: activation = (
		(* Contract field 4 is a class's name, or a metaclass's class. *)

		| method name |
		method:: activation method.
		nil = method ifTrue: [ ^'?' ].
		name:: vm slotOf: (vm classOf: activation receiver) at: 4.
		(nil = name or: [ name isKindOfString ]) ifFalse: [
				name:: (nameOrQuestionMark: (vm slotOf: name at: 4)) , ' class'
			].
		^(nameOrQuestionMark: name) , '>>' , (vm slotOf: method at: 3)
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
		public applyMixin: mixin superclass: superclass enclosingObject: enclosingObject = (
		)
		public isValue: object = (
		)
		public clone: root = (
		)
		public newActor = (
		)
		public farReference: handle to: value = (
		)
		public post: packet to: farReference = (
		)
		public isFarReference: object = (
		)
		public currentMessageLoop = (
		)
		public spawn: mixin = (
		)
		public hostGlobal = (
		)
		public markSimulationRoot: method = (
		)
		public isSimulationRoot: method = (
		)
		public continueAt: activation with: value = (
		)
		public resumeActivation: top = (
		)
		public runNative: method receiver: receiver arguments: arguments = (
		)
		public commit: changeSet = (
		)
		public mixinApplicationsOf: mixin = (
		)
		public subclassesOf: class = (
		)
		public programVersion = (
		)
		public checkpointNow = (
		)
		public checkpointPolicy: allocations = (
		)
		public logPosition = (
		)
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
    #     name, methods, enclosing mixin, slot names, class-side mixin,
    #     library). psoup-only fields become accessors answering neutral
    #     values until the mirrors (M6), except those runtime class creation
    #     needs (step 12).
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
			(* Eurasia keeps method headers in the VM; a native (MethodHeader) answers psoup's header word: access << 28, primitive << 16, temps << 8, arguments. Primitive 256 + i is a getter of the mixin's own slot i, 512 + i its setter, 1023 answers the receiver, any other nonzero one is a Eurasia native. *)

			halt
		)
		public header: word <Integer> = (
			(* A native (MethodHeaderPut): sets an unfrozen method's header from psoup's header word. *)

			halt
		)
		public bytecode = (
			(* A native (MethodBytecode): a copy of the V5 bytecode. *)

			halt
		)
		public bytecode: bytes <ByteArray> = (
			(* A native (MethodBytecodePut), on an unfrozen method. *)

			halt
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
			public _classMixin (* Eurasia contract field 5: nil, this being a class-side mixin. *)
			public _library (* Eurasia contract field 6: the mirror library's, here {the instance-side mixin}. *)
		|
	) (
		public _instanceMixin ^<InstanceMixin> = (
			^nil = _library ifTrue: [ nil ] ifFalse: [ _library at: 1 ]
		)
		public _instanceMixin: m <InstanceMixin> = (
			(* Eurasia: the class-side mixin's name is a contract field; psoup derives it, as 'X class'. *)

			_library: {m. }.
			(nil = _name and: [ nil = m _name ]) ifFalse: [ _name: m _name , ' class' ]
		)""")
    t = replace(t, """			public _slots <Array[{Symbol. Boolean. Symbol}]>
			public _nestedMixins <Array[InstanceMixin]>
			public _applications <WeakArray>
			public _classMixin <ClassMixin>
			public _accessModifier <Symbol>
			public _primaryFactorySelector <Symbol>
			public _headerSource <String>
		|
	) (""", """			public _slots <Array[{Symbol. Boolean. Symbol}]> (* Eurasia contract field 4: psoup's slot descriptions {name. isMutable. accessModifier}; the VM only counts them. *)
			public _classMixin <ClassMixin> (* Eurasia contract field 5: the paired class-side mixin. *)
			public _library (* Eurasia contract field 6: the mirror library's, here {nested mixins. access modifier. primary factory selector. header source}. *)
		|
	) (
		public _accessModifier = (
			^nil = _library ifTrue: [ #public ] ifFalse: [ _library at: 2 ]
		)
		public _accessModifier: a <Symbol> = (
			library at: 2 put: a
		)
		public _applications = (
			(* Eurasia: the classes applying this mixin, from the VM (psoup keeps a weak set). *)

			^vm mixinApplicationsOf: self
		)
		public _headerSource = (
			^(nil = _library or: [ _library size < 4 ]) ifTrue: [ nil ] ifFalse: [ _library at: 4 ]
		)
		public _headerSource: s <String> = (
			library at: 4 put: s
		)
		public _nestedMixins = (
			^nil = _library ifTrue: [ {} ] ifFalse: [ _library at: 1 ]
		)
		public _nestedMixins: mixins <Array[InstanceMixin]> = (
			library at: 1 put: mixins
		)
		public _primaryFactorySelector = (
			^nil = _library ifTrue: [ nil ] ifFalse: [ _library at: 3 ]
		)
		public _primaryFactorySelector: s <Symbol> = (
			library at: 3 put: s
		)
		library = (
			(* Eurasia: the library field of a mixin being built, made on first use. *)

			nil = _library ifTrue: [ _library: {{}. #public. nil. nil. } ].
			^_library
		)""")

    # 12. Runtime class creation (nested-class accessors, mixin
    #     applications): the VM creates the class and its metaclass from the
    #     instance mixin and its class-side mixin, a contract field
    #     (`vm applyMixin:superclass:enclosingObject:`). The genesis packager
    #     keeps the nested mixins, the access modifier and the factory
    #     selector in the instance mixin's library field ({nested. access.
    #     factory}). Accessors
    #     need no per-application copies: their slot indices are relative to
    #     their mixin. psoup's bookkeeping of applications and subclasses
    #     (weak sets) is dropped until the mirrors (M6). As in psoup, the
    #     classes whose instances are not regular objects cannot be
    #     subclassed. Natives go only on the `...Again:` methods that psoup
    #     keeps for the primitive alone, since a failing Eurasia native
    #     signals instead of running the method body.
    start = t.index("\t\tpublic apply: superclass <Class> format: f withName: name <Symbol> enclosingObject: eo <Object> = (")
    end = t.index("\n\t\t)\n", start) + len("\n\t\t)\n")
    t = t[:start] + """		public apply: superclass <Class> format: f withName: name <Symbol> enclosingObject: eo <Object> = (
			^vm applyMixin: self superclass: superclass enclosingObject: eo
		)
""" + t[end:]
    t = replace(t, """			cid:: slotOf: object at: 5.
			nil = cid ifTrue: [ ^true ].
			^(cid between: 3 and: 13) not""", """			(* Eurasia: psoup's class ids 3-13, the classes whose instances are not regular objects. *)
			^({_Array. ByteArray. String. WeakArray. Ephemeron. Float. SmallInteger. MediumInteger. LargeInteger. Closure. Activation. } includes: object) not""")
    t = replace(t, """			^self apply: superclass
				format: (formatOf: superclass)""", """			^self apply: superclass
				format: 0""")
    t = replace(t, """	public main: v args: args = (""", """	public eurasiaInternalKernel = (
		(* The kernel module, for the application, which is packaged as if nested here: it builds the platform's public modules over it. *)

		^self
	)
	public main: v args: args = (""")

    # 13. Float division. psoup's Number>>/ primitive divided floats, so its
    #     Float has no / and no divideFromNumber:. Eurasia's FloatDivide
    #     native goes on a Float>>/ that, like Number>>/, falls back to the
    #     argument's divideFromNumber:; an Integer divided by a Float reaches
    #     Float>>divideFromNumber: through Integer>>/.
    t = replace(t, """			^x subtractFromFloat: self
		)
""", """			^x subtractFromFloat: self
		)
		public / other <Number> ^<Number> = (
			(* Eurasia: the FloatDivide native; this body is not run. *)

			^other divideFromNumber: self
		)
		public divideFromNumber: left <Number> ^<Float> = (
			^left asFloat / self
		)
""")

    # 16. Host objects (role 33; operation list, section 10): the call-based
    #     FFI. The VM makes an Alien for each host value that is not
    #     bilingual; its operations are natives, and the messages it does not
    #     understand go to the host object as psoup's Aliens' do.
    anchor = "\tpublic class ArgumentError value: v = Exception new ("
    t = replace(t, anchor, """	public class Alien _cannotInstantiate = (
		(* A host object (Eurasia; operation list, section 10). The VM makes one for each host value that is not bilingual; it holds a reference into the host's table, 0 for the host's undefined. Its operations are natives. A message it does not understand goes to the host object, as with psoup's Aliens: a keyword selector names the method by its first keyword, and new constructs. *)

		|
			private reference
		|
	) (
		private asJSSelector: nsSelector <String> ^<String> = (
			| position = nsSelector indexOf: ':'. |
			0 = position ifTrue: [ ^nsSelector ].
			^nsSelector copyFrom: 1 to: position - 1
		)
		public at: key = (
			halt
		)
		public at: key put: value = (
			halt
		)
		public call: key withArguments: arguments <Array> = (
			halt
		)
		public callWithArguments: arguments <Array> = (
			halt
		)
		doesNotUnderstand: message = (
			| jsSelector |
			0 = message arguments size ifTrue: [
					(message selector startsWith: 'isKindOf') ifTrue: [ ^false ]
				].
			jsSelector:: asJSSelector: message selector.
			^'new' = jsSelector ifTrue: [ self newWithArguments: message arguments ]
				ifFalse: [ self perform: jsSelector withArguments: message arguments ]
		)
		public includesKey: key ^<Boolean> = (
			halt
		)
		public instanceOf: constructor ^<Boolean> = (
			halt
		)
		public isInstanceOf: constructor ^<Boolean> = (
			halt
		)
		public isKindOfClosure ^<Boolean> = (
			^false
		)
		public isKindOfDOMNode ^<Boolean> = (
			^instanceOf: (vm hostGlobal at: 'Node')
		)
		public isKindOfImage ^<Boolean> = (
			^instanceOf: (vm hostGlobal at: 'HTMLImageElement')
		)
		public isKindOfJSAlien ^<Boolean> = (
			^true
		)
		public isUndefined ^<Boolean> = (
			^0 = reference
		)
		public newWithArguments: arguments <Array> = (
			halt
		)
		public perform: selector <String> withArguments: arguments <Array> = (
			halt
		)
		public printString ^<String> = (
			self isUndefined ifTrue: [ ^'undefined' ].
			^self perform: 'toString' withArguments: {}
		)
		public propertyAt: key = (
			^at: key
		)
		public propertyAt: key put: value = (
			^at: key put: value
		)
		public removeKey: key = (
			halt
		)
	) : (
		public global = (
			(* The host's global object. *)

			^vm hostGlobal
		)
	)
""" + anchor)

    # 17. The callback boundary (role 28; design doc section 4): the VM runs
    #     a callback from the host as this method, so that an exception in
    #     it stops here and a non-local return through it raises
    #     cannotReturn: (the VM's non-local return stops at role 28).
    t = replace(t, """		public ensure: onUnwind <[]> = (
""", """		public dispatchCallbackWith: arguments <Array> = (
			(* The boundary of a callback from the host (Eurasia; design doc section 4, role 28). The VM runs a callback as this method: an exception in it stops here, and a non-local return through it raises cannotReturn:. Answers {true. result} or {false. a description of the exception}, from which the host caller gets a host exception. *)

			^[ {true. self valueWithArguments: arguments. } ] on: Exception
				do: [:e |
					| text |
					text:: e messageText.
					{false. nil = text ifTrue: [ 'an exception' ] ifFalse: [ text ]. }
				]
		)
		public ensure: onUnwind <[]> = (
""")

    # 18. Returning to a simulation root (M6 design 4.4): psoup pushes the
    #     exception on the root and jumps to it; Eurasia's root is a live
    #     frame, whose pending send answers the exception.
    t = replace(t, """			simulationRoot push: self.
			jumpTo: simulationRoot""", """			vm continueAt: simulationRoot with: self""")

    # 15. Far references (role 35): opaque VM objects (Φ). An ordinary send
    #     to one fails as an immediate send to psoup's eventual refs does.
    anchor = "\tpublic class Fraction reducedNumerator: num denominator: denom = Number new ("
    t = replace(t, anchor, """	public class FarReference _cannotInstantiate = (
		(* A far reference (Eurasia): a reference to an object in another actor's heap. Only eventual sends reach its target; the VM translates their arguments by the remote-representation rules. *)

	) (
		doesNotUnderstand: message <Message> = (
			0 = message arguments size ifTrue: [
					(message selector startsWith: 'isKindOf') ifTrue: [ ^false ]
				].
			^Error signal: 'Cannot immediate-send to an eventual ref: ' , message selector
		)
		public isKindOfFarReference = (
			^true
		)
		public printString = (
			^'a far reference'
		)
	) : ()
""" + anchor)

    # 14. Values (EURASIA_ACTORS_DESIGN_2026-11-17.md section 5): the Value
    #     class of the language spec, the superclass of all values, which the
    #     VM knows as a role; and an accessor on _Object so that any code can
    #     name it, as psoup's kernel exposes Array.
    anchor = "\tpublic class WeakArray _cannotInstantiate = Collection new ("
    t = replace(t, anchor, """	public class Value new = (
		(* The superclass of all values (language spec): a class whose superclass chain includes Value is a value class, whose instances the VM keeps deeply immutable (Eurasia, EURASIA_ACTORS_DESIGN_2026-11-17.md section 5). *)

	) (
	) : ()
""" + anchor)
    t = replace(t, """			^outer InternalKernel _Array
		)
""", """			^outer InternalKernel _Array
		)
		Value ^<Value class> = (
			^outer InternalKernel Value
		)
""")

    open(output, "w").write(t)
    print(f"fork-kernel: wrote {output}")


main()
