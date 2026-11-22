#!/usr/bin/env python3
# Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
# code is governed by a BSD-style license that can be found in the LICENSE file.
# This script contains excerpts of Primordial Soup's Newspeak sources,
# Copyright 2008, the Newspeak project authors, licensed under the Apache
# License, Version 2.0; see the LICENSE file.
"""Derive newspeak/platform/Eurasia/Actors.ns from psoup's platform/PrimordialSoup/Actors.ns (same class name).

The actors on Eurasia are VM actors (EURASIA_ACTORS_DESIGN_2026-11-17.md
section 5): each has its own heap, mailbox and a clone of the platform, and
references cross actors only through the VM, which applies the semantics'
remote-representation rules. psoup's library keeps promises, resolution and
reactors; this fork replaces only what crossed actors inside one heap:

- `Actor named:` creates a VM actor; `seed:` answers a VM far reference;
- an eventual send to a VM far reference posts a packet (target, selector,
  a promise record for the result, the arguments) to the target's actor,
  whose message loop delivers it (dispatchPacket:) with a RemoteResolver that
  settles the record;
- a promise crosses actors as a VM promise record (the semantics' Π and
  RR-Promise): its state is the same in every actor at once. The library
  promise that created it settles it, or follows it (the VM posts the
  settlement to followers); an immediate send to a follower asks the VM;
- arguments cross as `vmWrap` makes them (near objects as themselves,
  resolved promises as their resolution, unresolved ones as their promise
  records, others as their external refs), and the VM turns non-values into
  far references.

Every edit is checked: the script fails if psoup's source has changed under
it. Usage: fork-actors.py <psoup Actors.ns> <output>.
"""

import re
import sys


def replace(text, old, new, count=1):
    found = text.count(old)
    if found != count:
        sys.exit(f"fork-actors: expected {count} of {old[:70]!r}, found {found}")
    return text.replace(old, new)


def main():
    source, output = sys.argv[1], sys.argv[2]
    t = open(source).read()

    # 1. The module's name (the natives table keys its rows by it).
    # Eurasia's platform namespace (PLATFORM_NAMESPACES_DESIGN_2026-11-18.md).
    t = replace(t, "Newspeak3\n'PrimordialSoup'\n", "Newspeak3\n'Eurasia'\n")

    # 2. An Actor is a VM actor; seed: answers a far reference to a value in it.
    t = replace(t, """	public class Actor named: debugName <String> = (
		|
			private (* N.B. Do not leak to subinstances. *) internalActor = InternalActor named: debugName.
		|
	) (""", """	public class Actor named: debugName <String> = (
		(* Eurasia: a VM actor, with its own heap and a clone of the platform. The handle is a far reference to nil in it. *)

		|
			private handle = newActorHandle.
		|
	) (""")
    t = replace(t, """			^createFarReferenceTo: value in: internalActor for: currentActor""",
                """			^farReference: handle to: value""")

    # 3. VM far references are refs of their own kind.
    t = replace(t, """	private asInternalRef: externalRefOrUnboxedNearRef = (
		^(isRef: externalRefOrUnboxedNearRef) ifTrue: [""", """	private asInternalRef: externalRefOrUnboxedNearRef = (
		(isVMPromise: externalRefOrUnboxedNearRef) ifTrue: [
				^internalRefForPromiseRecord: externalRefOrUnboxedNearRef
			].
		(isFarReference: externalRefOrUnboxedNearRef) ifTrue: [
				^InternalVMFarReference wrapping: externalRefOrUnboxedNearRef
			].
		^(isRef: externalRefOrUnboxedNearRef) ifTrue: [""")

    # 4. What crosses actors: vmWrap, beside psoup's wrapFor:.
    t = replace(t, """		public externalRef = (
			^Ref wrapping: self
		)""", """		public externalRef = (
			^Ref wrapping: self
		)
		public vmArgument = (
			(* Eurasia: what an argument that arrived from another actor becomes here. *)

			^externalRef
		)
		public vmPendingRecord = (
			(* Eurasia: the promise record of an unresolved promise of this actor's, for forwarding to; nil for any other ref. *)

			^nil
		)
		public vmWrap = (
			(* Eurasia: what crosses to another actor in place of this ref; the VM turns non-values into far references. *)

			^externalRef
		)""")
    t = replace(t, """		public wrapFor: targetActor = (
			(isValueObject: target) ifTrue: [ ^target ].
			^createFarReferenceTo: target in: currentActor for: targetActor
		)""", """		public wrapFor: targetActor = (
			(isValueObject: target) ifTrue: [ ^target ].
			^createFarReferenceTo: target in: currentActor for: targetActor
		)
		public vmArgument = (
			^target
		)
		public vmWrap = (
			^target
		)""")
    t = replace(t, """		public wrapFor: targetActor = (
			state = #resolved ifTrue: [ ^resolution wrapFor: targetActor ].
			^(InternalRemotePromise target: self targetsActor: currentActor) externalRef
		)""", """		public wrapFor: targetActor = (
			state = #resolved ifTrue: [ ^resolution wrapFor: targetActor ].
			^(InternalRemotePromise target: self targetsActor: currentActor) externalRef
		)
		public vmWrap = (
			state = #resolved ifTrue: [ ^resolution vmWrap ].
			^vmPromiseRecord
		)
		public vmPendingRecord = (
			state = #resolved ifTrue: [ ^resolution vmPendingRecord ].
			state = #unresolved ifTrue: [ ^vmPromiseRecord ].
			^nil
		)
		public vmPromiseRecord = (
			(* Eurasia: the promise record standing for this unresolved promise in other actors. A new one is settled by this promise's resolution. *)

			nil = vmPromise ifTrue: [
					vmPromise:: newPromiseFor: self.
					self addReactor: (RemoteResolver for: vmPromise)
				].
			^vmPromise
		)
		public followVMPromise: record = (
			(* Eurasia: this promise follows a promise record settled in another actor; the VM posts the settlement here. *)

			vmPromise:: record.
			promise: record addWaiter: self
		)
		public resolveRemotely: value = (
			(* Eurasia: the settlement of the promise record this promise follows, posted by the VM. An immediate send may have seen it first (syncWithVM). *)

			state = #unresolved ifFalse: [ ^self ].
			self internalResolve: (asInternalRef: value)
		)
		public breakRemotely: problem = (
			state = #unresolved ifFalse: [ ^self ].
			self internalResolve: (InternalBrokenRef problem: problem)
		)
		public syncWithVM = (
			(* Eurasia: a promise record is settled in every actor at once, before its followers hear of it; an immediate send sees the settlement now. *)

			| s |
			nil = vmPromise ifTrue: [ ^self ].
			s:: promiseState: vmPromise.
			(s at: 1) = 1 ifTrue: [
					^self internalResolve: (asInternalRef: (s at: 2))
				].
			(s at: 1) = 2 ifTrue: [
					^self internalResolve: (InternalBrokenRef problem: (s at: 2))
				]
		)""")
    # The promise record of a crossing promise, and immediate sends that see
    # its settlement.
    t = replace(t, """			resolution <InternalRef>
			reactors
		|""", """			resolution <InternalRef>
			reactors
			vmPromise (* Eurasia: the promise record (Π) standing for this promise in other actors, or nil. *)
		|""")
    # InternalRemotePromise has the same method; edit InternalLocalPromise's.
    start = t.index("\tclass InternalLocalPromise new = InternalRef new (")
    end = t.index("\tclass InternalNearRef wrapping: t = InternalRef new (")
    t = t[:start] + replace(t[start:end], """		public immediateSend: message <Message> ^<Object> = (
			state = #resolved ifTrue: [ ^resolution immediateSend: message ].""", """		public immediateSend: message <Message> ^<Object> = (
			state = #unresolved ifTrue: [ syncWithVM ].
			state = #resolved ifTrue: [ ^resolution immediateSend: message ].""") + t[end:]
    # An eventual send to a promise with a record queues in the record, in
    # the order every actor sends (Route-Pending).
    start = t.index("\tclass InternalLocalPromise new = InternalRef new (")
    end = t.index("\tclass InternalNearRef wrapping: t = InternalRef new (")
    t = t[:start] + replace(t[start:end], """			state = #resolved ifTrue: [
					^resolution eventualSend: selector arguments: arguments
				].
			localPromise:: InternalLocalPromise new.""", """			state = #resolved ifTrue: [
					^resolution eventualSend: selector arguments: arguments
				].
			nil = vmPromise ifFalse: [
					(* Eurasia: the promise has crossed actors, so the send queues in its promise record, in the order every actor sends (Route-Pending). *)

					localPromise:: newFollowerPromise.
					promise: vmPromise
						send: {selector. localPromise vmPromiseRecord. } , (arguments collect: [:each | (asInternalRef: each) vmWrap ]).
					^localPromise externalRef
				].
			localPromise:: InternalLocalPromise new.""") + t[end:]

    # 5. The far reference and the remote resolver.
    anchor = "\tclass InternalLocalPromise new = InternalRef new ("
    t = replace(t, anchor, """	class InternalVMFarReference wrapping: f = InternalRef new (
		(* Eurasia: a VM far reference to an object in another actor. An eventual send posts a packet to that actor: the target, the selector, the promise for the result and the arguments, which the VM translates. *)

		|
			farRef = f.
		|
	) (
		public addReactor: reactor = (
			reactor reactResolved: farRef
		)
		public checkCycle ^<Boolean> = (
			^false
		)
		public eventualSend: selector <Symbol> arguments: arguments <Array> ^<Ref> = (
			| promise |
			promise:: newFollowerPromise.
			post: {farRef. selector. promise vmPromiseRecord. } , (arguments collect: [:each | (asInternalRef: each) vmWrap ])
				to: farRef.
			^promise externalRef
		)
		public externalRef = (
			^farRef
		)
		public immediateSend: message <Message> ^<Object> = (
			^Error signal: 'Cannot immediate-send to an eventual ref: ' , message selector
		)
		public vmWrap = (
			^farRef
		)
		public whenFulfilled: onValue whenBroken: onError ^<Ref> = (
			| localPromise = InternalLocalPromise new. |
			currentActor enqueueReceiver: onValue
				selector: #cull:
				arguments: {farRef. }
				resolver: localPromise.
			^localPromise externalRef
		)
		public wrapFor: targetActor = (
			^farRef
		)
	) : ()
	class RemoteResolver for: r = (
		(* Eurasia: settles the promise record r (Π), which every actor sees at once, with the resolution of a ref of this actor's: the result of a send from another actor, or a promise of this actor's that crossed to another. An unresolved promise of this actor's settles it by forwarding to its own record. *)

		|
			record = r.
		|
	) (
		public internalResolve: internalRef = (
			| forward |
			forward:: internalRef vmPendingRecord.
			nil = forward ifFalse: [ ^resolvePromise: record with: forward ].
			internalRef addReactor: self
		)
		public reactBroken: problem = (
			breakPromise: record with: problem
		)
		public reactMoreResolved: internalRef = (
			internalRef addReactor: self
		)
		public reactResolved: value = (
			resolvePromise: record with: (asInternalRef: value) vmWrap
		)
	) : ()
""" + anchor)

    # 6. The message loop delivers packets posted to its actor.
    t = replace(t, """		public drainQueue = (
			timerHeap drainQueue.""", """		private dispatchPacket: packet = (
			(* Eurasia: the VM delivers a packet posted to this actor: target, selector, the promise record for the result (nil for none), then the arguments, of which promise records become this actor's refs. A target that is a promise of this actor's (reached through a far reference to its ref) gets the message eventually. *)

			| target selector replyTo arguments resolver |
			target:: packet at: 1.
			selector:: packet at: 2.
			replyTo:: packet at: 3.
			arguments:: (packet copyFrom: 4 to: packet size) collect: [:each |
					(isVMPromise: each) ifTrue: [ (asInternalRef: each) vmArgument ] ifFalse: [ each ]
				].
			resolver:: nil = replyTo ifFalse: [ RemoteResolver for: replyTo ].
			(isRef: target)
				ifTrue: [
					| result |
					result:: (asInternalRef: target) eventualSend: selector arguments: arguments.
					nil = resolver ifFalse: [ resolver internalResolve: (asInternalRef: result) ]
				]
				ifFalse: [
					currentActor enqueueReceiver: target
						selector: selector
						arguments: arguments
						resolver: resolver
				].
			finish: drainQueue
		)
		public drainQueue = (
			timerHeap drainQueue.""")

    # 7. The VM operations the module needs; the natives table installs them.
    anchor = "\tpublic buildLoopForApplication: app platform: platform = ("
    t = replace(t, anchor, """	private farReference: handle to: value = (
		(* Eurasia: a native (VmFarReferenceTo), from the natives table. *)

		halt
	)
	private isFarReference: object = (
		(* Eurasia: a native (VmIsFarReference). *)

		halt
	)
	private newActorHandle = (
		(* Eurasia: a native (VmNewActor). *)

		halt
	)
	private post: packet to: farRef = (
		(* Eurasia: a native (VmPost). *)

		halt
	)
	private breakPromise: record with: problem = (
		(* Eurasia: a native (VmBreakPromise). *)

		halt
	)
	private internalRefForPromiseRecord: record = (
		(* Eurasia: a promise record (Π) as this actor sees it: its resolution or problem once settled; while pending, this actor's own promise for it if this actor created it, else a new promise following it. *)

		| s p |
		s:: promiseState: record.
		(s at: 1) = 1 ifTrue: [ ^asInternalRef: (s at: 2) ].
		(s at: 1) = 2 ifTrue: [ ^InternalBrokenRef problem: (s at: 2) ].
		(isVMPromise: (s at: 2)) ifFalse: [ ^s at: 2 ].
		p:: InternalLocalPromise new.
		p followVMPromise: (s at: 2).
		^p
	)
	private isVMPromise: object = (
		(* Eurasia: a native (VmIsPromise). *)

		halt
	)
	private newFollowerPromise = (
		(* Eurasia: a promise for the result of a send that runs in another actor (or whose target is not yet known), following a new promise record which the send's resolver settles. *)

		| p |
		p:: InternalLocalPromise new.
		p followVMPromise: (newPromiseFor: p).
		^p
	)
	private newPromiseFor: localPromise = (
		(* Eurasia: a native (VmNewPromise). *)

		halt
	)
	private promise: record send: message = (
		(* Eurasia: a native (VmPromiseSend). *)

		halt
	)
	private promise: record addWaiter: localPromise = (
		(* Eurasia: a native (VmPromiseAddWaiter). *)

		halt
	)
	private promiseState: record = (
		(* Eurasia: a native (VmPromiseState). *)

		halt
	)
	private resolvePromise: record with: value = (
		(* Eurasia: a native (VmResolvePromise). *)

		halt
	)
""" + anchor)

    # Last: psoup's primitive pragmas go. Eurasia's natives come from its
    # natives table (by class and selector), and no module Eurasia runs carries
    # a primitive (EURASIA_BUILD_AND_DEPLOY_DESIGN_2026-11-21.md, section 6):
    # each becomes a plain comment saying what it was.
    t, count = re.subn(r"\(\* :literalmessage: primitive: (\d+) \*\)",
                       lambda m: f"(* psoup's primitive {m.group(1)}; Eurasia's natives come from its natives table. *)", t)
    print(f"fork-actors: replaced {count} primitive pragmas")
    open(output, "w").write(t)
    print(f"fork-actors: wrote {output}")


main()
