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
- the primitives become natives, by the natives table;
- installing is vm commit:, with layout maps;
- the V4 Simulator becomes the V5 one: V5BytecodeSimulation's decoder and
  upstream's Simulator (rmacnak/primordialsoup Mirrors.ns at 338f0bb3).

Every edit is checked: the script fails if psoup's source has changed under
it. Usage: fork-mirrors.py <MirrorsForPrimordialSoup.ns>
<V5BytecodeSimulation.ns> <upstream-V5Simulator-338f0bb3.ns.txt> <output>.
"""

import re
import sys


def replace(text, old, new, count=1):
    """Replaces exactly `count` occurrences of `old`. A whitespace-only line
    in `old` matches a whitespace-only line in the text, however indented:
    psoup's printer indents blank lines, editors often strip them."""
    pattern = re.compile(
        "\n".join(r"[ \t]*" if line.strip() == "" else re.escape(line) for line in old.split("\n")))
    found = len(pattern.findall(text))
    if found != count:
        sys.exit(f"fork-mirrors: expected {count} of {old[:70]!r}, found {found}")
    return pattern.sub(lambda m: new, text)


def main():
    source, v5_source, upstream_source, output = sys.argv[1:5]
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

    # 4. Installing (M6 design 4.3): vm commit: instead of a become. The
    #    VM reshapes instances in place from layout maps, so their identity
    #    and actor stay, and forwards the old program objects' identities.
    t = replace(t, """			private updateInstances <IdentityMap[OldObject, NewObject]>""",
                """			private updateInstances <IdentityMap[OldObject, NewObject]> (* Eurasia: unused; instances are reshaped by the VM from layoutMaps. *)
			private layoutMaps <List[{Class. Class. Array[Integer]}]>""")
    t = replace(t, """		private installAll = (
			| updateCount oldObjects newObjects index |
			updateCount:: updateMixinsAndClasses size + updateInstances size.
			oldObjects:: Array new: updateCount.
			newObjects:: Array new: updateCount.
			index: 1.
			updateInstances keysAndValuesDo: [:old :new |
					oldObjects at: index put: old.
					newObjects at: index put: new.
					index: index + 1
				].

			(* Put updated mixins and classes last, so A -> C follows B -> C (see class comment). *)
			updateMixinsAndClasses keysAndValuesDo: [:old :new |
					oldObjects at: index put: old.
					newObjects at: index put: (updateInstances at: old ifAbsent: [ new ]).

					(* Follow B to avoid B -> C, A -> B. *)
					index: index + 1
				].
			updateCount = 0 ifTrue: [ ^self ].
			elementsOf: oldObjects forwardIdentityToElementsOf: newObjects
		)""", """		private installAll = (
			(* Eurasia: one vm commit: with the forwarding pairs (old mixins and classes to new) and the layout maps, which the VM applies to the instances in place. *)

			| oldObjects newObjects index |
			oldObjects:: Array new: updateMixinsAndClasses size.
			newObjects:: Array new: updateMixinsAndClasses size.
			index:: 1.
			updateMixinsAndClasses keysAndValuesDo: [:old :new |
					oldObjects at: index put: old.
					newObjects at: index put: new.
					index:: index + 1
				].
			(index = 1 and: [ layoutMaps isEmpty ]) ifTrue: [ ^self ].
			commit: {oldObjects. newObjects. layoutMaps asArray. }
		)""")
    t = replace(t, """			(allInstancesOf: oldClass) do: [:oldInstance |

					(* Avoid A -> D (see class comment). *)
					(updateMixinsAndClasses includesKey: oldInstance) ifFalse: [
							| newInstance = allocate: newClass. |

							(* Copy state from oldInstance to newInstance. *)
							1 to: newSlotCount
								do: [:newIndex |
									| oldIndex value |
									oldIndex:: remapIndices at: newIndex.
									0 = oldIndex ifFalse: [
											value:: slotOf: oldInstance at: oldIndex.
											slotOf: newInstance at: newIndex put: value
										]
								].
							updateInstances at: oldInstance put: newInstance
						]
				]""", """			(* Eurasia: the VM reshapes every instance in place (vm commit:'s layout maps). *)
			layoutMaps add: {oldClass. newClass. remapIndices asArray. }""")
    t = replace(t, """			updateInstances:: IdentityMap new.
			existingClasses:: IdentityMap new.""", """			updateInstances:: IdentityMap new.
			layoutMaps:: List new.
			existingClasses:: IdentityMap new.""")
    t = replace(t, """			updateInstances: nil.
			existingClasses: nil.""", """			updateInstances: nil.
			layoutMaps: nil.
			existingClasses: nil.""")
    # Slot accessors are synthesized per mixin, with indices relative to it
    # (psoup's are placeholders its kernel rebuilds for each application).
    t = replace(t, """		private methodDictionaryFor: mixin <AbstractMixin> from: imixin <IntermediateMixin> ^<MethodDictionary> = (
			| cmethods |
			cmethods:: imixin assembleMethods collect: [:imethod <IntermediateMethod> |
					imethod methodMixin: mixin.
					imethod compiledMethod
				].
			^cmethods asArray
		)""", """		private methodDictionaryFor: mixin <AbstractMixin> from: imixin <IntermediateMixin> ^<MethodDictionary> = (
			(* Eurasia: the slot accessors are made here, with indices relative to the mixin, as the genesis packager makes them; psoup's assembleMethods has placeholders, which psoup's kernel rebuilds for each application. *)

			| slots accessors cmethods |
			slots:: imixin assembleSlots.
			accessors:: List new.
			1 to: slots size
				do: [:i |
					| slot = slots at: i. |
					accessors add: (internalKernel Method newGetterForSlot: slot name
							at: i
							accessModifier: slot accessModifier).
					accessors add: (internalKernel Method newInitializerForSlot: slot name at: i).
					slot isMutable ifTrue: [
							accessors add: (internalKernel Method newSetterForSlot: slot name
									at: i
									accessModifier: slot accessModifier)
						]
				].
			accessors do: [:accessor | accessor mixin: mixin ].
			cmethods:: List new.
			imixin assembleMethods do: [:imethod <IntermediateMethod> |
					| method |
					imethod methodMixin: mixin.
					method:: imethod compiledMethod.
					(accessors anySatisfy: [:accessor | accessor selector = method selector ]) ifFalse: [
							cmethods add: method
						]
				].
			cmethods addAll: accessors.
			^cmethods asArray
		)""")
    t = replace(t, """	private subclassesOf: klass = (
		^self slotOf: klass at: 8
	)""", """	private commit: changeSet = (
		(* Eurasia: a native (VmCommit). *)

		halt
	)
	private subclassesOf: klass = (
		(* Eurasia: a native (VmSubclassesOf); psoup keeps a weak set in the class. *)

		halt
	)""")

    # 5. The V5 Simulator (M6 design 4.2, decision 7.2): the decoder is
    #    V5BytecodeSimulation's InstructionStream (upstream's, with the
    #    short popJumpFalse: fixed); the Simulator is upstream's
    #    (rmacnak/primordialsoup Mirrors.ns at 338f0bb3, kept as
    #    newspeak/upstream-V5Simulator-338f0bb3.ns.txt), adapted to Eurasia's
    #    natives and markers. It replaces psoup's V4 Simulator.
    v5 = open(v5_source).read()
    instruction_stream = v5[v5.index("\tpublic class InstructionStream new = ("):v5.index("\tpublic class SelectorCollector overBytecode: b literals: l = InstructionStream new (")]
    selector_collector = v5[v5.index("\tpublic class SelectorCollector overBytecode: b literals: l = InstructionStream new ("):]
    selector_collector = selector_collector[:selector_collector.index("\n\t) : ()\n") + len("\n\t) : ()\n")]
    instruction_stream = instruction_stream.replace("\tpublic class InstructionStream new = (", "\tprivate class InstructionStream new = (", 1)
    selector_collector = selector_collector.replace("\tpublic class SelectorCollector", "\tprivate class SelectorCollector", 1)
    up = open(upstream_source).read()
    simulator = up[up.index("private class Simulator = InstructionStream ("):up.index("private class SlotDeclarationMirror")]
    simulator = replace(simulator, "private class Simulator = InstructionStream (", "private class Simulator new = InstructionStream new (")
    simulator = simulator.replace("panic", "halt")
    simulator = replace(simulator, "(tryPrimitive: newMethod primitive receiver: newReceiver arguments: arguments)",
                        "(tryPrimitive: newMethod receiver: newReceiver arguments: arguments)")
    simulator = replace(simulator, "	loop:: internalKernel messageLoop.", "	loop:: currentMessageLoop.")
    simulator = replace(simulator, "		unwind method primitive = 162 ifTrue:", "		(isEnsureMethod: unwind method) ifTrue:")
    simulator = replace(simulator, "		unwind method primitive = 163 ifTrue:", "		(isSimulationBoundary: unwind method) ifTrue:")
    simulator = replace(simulator, "	error: 'Recursive doesNotUnderstand:'", "	Error signal: 'Recursive doesNotUnderstand:'")
    start = simulator.index("tryPrimitive: primitive receiver: newReceiver arguments: arguments = (")
    end = simulator.index(") : (\n", start)
    simulator = simulator[:start] + """tryPrimitive: newMethod receiver: newReceiver arguments: arguments = (
	(* Eurasia: a method whose header names no primitive runs as bytecode. A few natives are simulated, as upstream simulates its primitives: the current activation (native 100), jumps (170, 171), closure activation (30-34) and perform (89, 119). Any other native or accessor runs in the VM (vm runNative:receiver:arguments:): its result is pushed; its failure is signalled, as the VM signals it; a tail send or closure activation it asks for is simulated. *)
	| primitive result |
	primitive:: newMethod primitive.
	0 = primitive ifTrue: [^false].
	100 = primitive ifTrue: [activation push: activation. ^true].
	(* Jumps: continuing at an activation (vm continueAt:with:, 170) or resuming a chain (vm resumeActivation:, 171) switches the simulated activation, as upstream's jump (165). *)
	170 = primitive ifTrue:
		[(arguments at: 1) push: (arguments at: 2).
		 activation:: arguments at: 1.
		 ^true].
	171 = primitive ifTrue: [activation:: arguments at: 1. ^true].
	(30 <= primitive and: [primitive <= 33]) ifTrue:
		[newReceiver numArgs = arguments size ifFalse: [^false].
		 activateClosure: newReceiver arguments: arguments.
		 ^true].
	34 = primitive ifTrue:
		[ | closureArgs = arguments at: 1. |
		 Array = (classOf: closureArgs) ifFalse: [^false].
		 closureArgs size = newReceiver numArgs ifFalse: [^false].
		 activateClosure: newReceiver arguments: closureArgs.
		 ^true].
	89 = primitive ifTrue:
		[ | selector = arguments at: 1. performArguments = arguments at: 2. |
		 (String = (classOf: selector)) ifFalse: [^false].
		 (Array = (classOf: performArguments)) ifFalse: [^false].
		 activation push: newReceiver.
		 performArguments do: [:arg | activation push: arg].
		 ordinarySend: selector numArgs: performArguments size.
		 ^true].
	119 = primitive ifTrue:
		[ | object = arguments at: 1. selector = arguments at: 2. performArguments = arguments at: 3. |
		 (String = (classOf: selector)) ifFalse: [^false].
		 (Array = (classOf: performArguments)) ifFalse: [^false].
		 activation push: object.
		 performArguments do: [:arg | activation push: arg].
		 ordinarySend: selector numArgs: performArguments size.
		 ^true].
	result:: runNative: newMethod receiver: newReceiver arguments: arguments.
	0 = (result at: 1) ifTrue: [activation push: (result at: 2). ^true].
	1 = (result at: 1) ifTrue:
		[activation push: (result at: 2).
		 ordinarySend: #signal numArgs: 0.
		 ^true].
	2 = (result at: 1) ifTrue:
		[activation push: (result at: 2).
		 (result at: 4) do: [:arg | activation push: arg].
		 ordinarySend: (result at: 3) numArgs: (result at: 4) size.
		 ^true].
	activateClosure: (result at: 2) arguments: (result at: 3).
	^true
)
""" + simulator[end:]
    # V5 instruction sizes on the class side, as the VM decodes them.
    simulator = replace(simulator, """sizeOf: bytecode = (
	bytecode <= 215 ifTrue: [^1].
	bytecode <= 237 ifTrue: [^2].
	bytecode <= 253 ifTrue: [^3].
	^4
)""", """sizeOf: bytecode = (
	bytecode <= 215 ifTrue: [^1].
	bytecode <= 237 ifTrue: [^2].
	bytecode <= 253 ifTrue: [^3].
	^4
)""")
    start = t.index("\tprivate class Simulator new = (")
    end = t.index("\tprivate class SlotDeclarationMirror")
    t = t[:start] + instruction_stream + selector_collector + "\t" + simulator.rstrip() + "\n" + t[end:]

    # The V4 selector scan of a method becomes the V5 one.
    start = t.index("\t\tpublic selectors ^<List[Symbol]> = (\n\t\t\t| pc ::= 1. extA ::= 0.")
    end = t.index("\t\tpublic source ^<String> = (", start)
    t = t[:start] + """		public selectors ^<List[Symbol]> = (
			(* Eurasia: the V5 instruction set, by V5BytecodeSimulation's SelectorCollector. *)

			^(SelectorCollector overBytecode: method bytecode literals: method literals) selectors
		)
""" + t[end:]

    # 6. Threads: simulation roots are marked methods (psoup's primitive
    #    142); the bottom of a turn has no sender on Eurasia; resuming makes
    #    frames of the suspended chain (vm resumeActivation:).
    t = replace(t, """			nil = activation sender ifTrue: [ ^true ].
			142 = activation sender method primitive ifTrue: [ ^true ].
			^168 = activation sender method primitive""", """			nil = activation sender ifTrue: [ ^true ].
			^isSimulationRoot: activation sender method""")
    t = replace(t, """			nil = suspendedActivation ifTrue: [ ^Error signal: 'Nothing to resume' ].
			^resumeImpl""", """			nil = suspendedActivation ifTrue: [ ^Error signal: 'Nothing to resume' ].
			markSimulationRoots.
			^resumeImpl""")
    t = replace(t, """		private simulatorRootMethod = (
			| methods = methodsOf: self class. |""", """		private markSimulationRoots = (
			(* Eurasia: resumeImpl and simulatorRoot are simulation roots (psoup marks them with primitive 142): a non-local return stops at them, and the kernel routes a simulated thread's exceptions to them. *)

			(methodsOf: (classOf: self)) do: [:m |
					(#resumeImpl = m selector or: [ #simulatorRoot = m selector ]) ifTrue: [
							markSimulationRoot: m
						]
				]
		)
		private simulatorRootMethod = (
			| methods = methodsOf: (classOf: self). |
			markSimulationRoots.""")

    t = replace(t, "			rawSender method primitive = 142 ifTrue: [ ^nil ].",
                "			(isSimulationRoot: rawSender method) ifTrue: [ ^nil ].")

    # 7. The module-level natives and helpers these use.
    t = replace(t, """	private commit: changeSet = (""", """	private currentMessageLoop = (
		(* Eurasia: a native (VmCurrentMessageLoop). *)

		halt
	)
	private isEnsureMethod: m = (
		(* Eurasia: whether m is the kernel's Closure>>ensure: (psoup's primitive 162). *)

		^#ensure: = m selector and: [ (mixinOf: Closure) = m mixin ]
	)
	private isSimulationBoundary: m = (
		(* Eurasia: a simulation root, or the callback boundary (psoup's primitive 163), which a non-local return may not cross. *)

		(isSimulationRoot: m) ifTrue: [ ^true ].
		^#dispatchCallbackWith: = m selector and: [ (mixinOf: Closure) = m mixin ]
	)
	private isSimulationRoot: m = (
		(* Eurasia: a native (VmIsSimulationRoot). *)

		halt
	)
	private markSimulationRoot: m = (
		(* Eurasia: a native (VmMarkSimulationRoot). *)

		halt
	)
	private runNative: method receiver: receiver arguments: arguments = (
		(* Eurasia: a native (VmRunNative). *)

		halt
	)
	private commit: changeSet = (""")

    open(output, "w").write(t)
    print(f"fork-mirrors: wrote {output}")


main()
