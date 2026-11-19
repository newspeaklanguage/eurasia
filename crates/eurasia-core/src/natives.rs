// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

//! The native catalog of kernel version 1 (operation list, section 3; rules
//! Native-Answer, Native-Tail and Native-Fail of `kernel-semantics.tex`).
//!
//! Every native is total: it answers, makes a defined tail send, or fails
//! with a reason. There are no fallback bodies. Integers are SmallIntegers
//! (63-bit immediates) or LargeIntegers (heap objects); a result that fits
//! a SmallInteger is always one. Floats are IEEE 754 binary64 with one
//! canonical NaN, and every function on them comes from the bundled `libm`,
//! so results are bit-identical on every host (decided 2026-11-10).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::bigint::BigInt;
use crate::host::{HOST_GLOBAL, Host, HostOp};
use crate::object::{
    Access, Behavior, Body, FROZEN, Heap, Method, MethodHeader, MethodKind, Mixin, PromiseRecord,
    Space, VALUE, VALUE_CLASS,
};
use crate::roles::{Role, Roles};
use crate::value::{SmallInteger, Value};

macro_rules! natives {
    ($($name:ident = $id:literal / $arity:literal: $doc:literal,)*) => {
        /// A catalog entry, numbered as in method headers.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u32)]
        pub enum Native {
            $(#[doc = $doc] $name = $id,)*
        }

        impl Native {
            /// Every entry, in id order.
            pub const ALL: &'static [Native] = &[$(Native::$name,)*];

            /// The entry with catalog id `id`.
            pub fn from_id(id: u32) -> Option<Native> {
                match id {
                    $($id => Some(Native::$name),)*
                    _ => None,
                }
            }

            /// The number of arguments, not counting the receiver.
            pub fn arity(self) -> usize {
                match self {
                    $(Native::$name => $arity,)*
                }
            }

            /// The entry's name, for diagnostics and failure objects.
            pub fn name(self) -> &'static str {
                match self {
                    $(Native::$name => stringify!($name),)*
                }
            }
        }
    };
}

natives! {
    IntegerAdd = 1 / 1: "`Integer>>+`; a non-integer argument tail-sends `addFromInteger:`.",
    IntegerSubtract = 2 / 1: "`Integer>>-`; otherwise `subtractFromInteger:`.",
    IntegerMultiply = 3 / 1: "`Integer>>*`; otherwise `multiplyFromInteger:`.",
    IntegerLess = 4 / 1: "`Number>><` on integers and floats; otherwise the reversed comparison `>`.",
    IntegerGreater = 5 / 1: "`Number>>>`; otherwise `<`.",
    IntegerLessEqual = 6 / 1: "`Number>><=`; otherwise `>=`.",
    IntegerGreaterEqual = 7 / 1: "`Number>>>=`; otherwise `<=`.",
    NumberEqual = 8 / 1: "`Number>>=`: false for a non-number (decided 2026-11-10).",
    NumberAsString = 9 / 0: "`Number>>asString`.",
    ObjectIdentical = 10 / 1: "`Object>>==`.",
    ObjectClass = 11 / 0: "`Object>>class`.",
    ObjectIdentityHash = 12 / 0: "`Object>>identityHash`: from the allocation sequence number.",
    StringConcatenate = 13 / 1: "`String>>,`.",
    StringSize = 14 / 0: "`String>>size`, in bytes.",
    NumberFloorDivide = 15 / 1: "`Number>>//`: floored.",
    NumberFloorModulo = 16 / 1: "`Number>>\\\\`: floored; the sign of the divisor.",
    NumberQuo = 17 / 1: "`Number>>quo:`: truncated.",
    NumberRem = 18 / 1: "`Number>>rem:`: truncated; the sign of the dividend.",
    VmPrint = 20 / 1: "`vm print:` (output).",
    VmExit = 21 / 1: "`vm exit:`.",
    IntegerBitAnd = 22 / 1: "`Integer>>bitAnd:` and `&`.",
    IntegerBitOr = 23 / 1: "`Integer>>bitOr:` and `|`.",
    IntegerBitXor = 24 / 1: "`Integer>>bitXor:`.",
    IntegerShiftLeft = 25 / 1: "`Integer>><<`.",
    IntegerShiftRight = 26 / 1: "`Integer>>>>`: arithmetic.",
    NumberAsFloat = 27 / 0: "`Number>>asFloat`.",
    NumberAsInteger = 28 / 0: "`Number>>asInteger`: truncated.",
    ClosureValue = 30 / 0: "`Closure>>value`.",
    ClosureValue1 = 31 / 1: "`Closure>>value:`.",
    ClosureValue2 = 32 / 2: "`Closure>>value:value:`.",
    ClosureValue3 = 33 / 3: "`Closure>>value:value:value:`.",
    ClosureValueWithArguments = 34 / 1: "`Closure>>valueWithArguments:`.",
    ClosureNumArgs = 35 / 0: "`Closure>>numArgs`.",
    FloatAdd = 40 / 1: "`Float>>+`; otherwise `addFromFloat:`.",
    FloatSubtract = 41 / 1: "`Float>>-`; otherwise `subtractFromFloat:`.",
    FloatMultiply = 42 / 1: "`Float>>*`; otherwise `multiplyFromFloat:`.",
    FloatDivide = 43 / 1: "`Float>>/`; otherwise `divideFromNumber:`, as psoup's `Number>>/`.",
    FloatFloor = 44 / 0: "`Float>>floor`.",
    FloatCeiling = 45 / 0: "`Float>>ceiling`.",
    FloatRounded = 46 / 0: "`Float>>rounded`: half away from zero.",
    FloatSqrt = 48 / 0: "`Float>>sqrt`.",
    FloatSin = 49 / 0: "`Float>>sin`.",
    FloatCos = 50 / 0: "`Float>>cos`.",
    FloatTan = 51 / 0: "`Float>>tan`.",
    FloatExp = 52 / 0: "`Float>>exp`.",
    FloatLn = 53 / 0: "`Float>>ln`.",
    FloatLog = 54 / 0: "`Float>>log`: base 10.",
    FloatPow = 55 / 1: "`Float>>pow:`.",
    FloatArcSin = 56 / 0: "`Float>>arcSin`.",
    FloatArcCos = 57 / 0: "`Float>>arcCos`.",
    FloatArcTan = 58 / 0: "`Float>>arcTan`.",
    FloatArcTan2 = 59 / 1: "`Float>>arcTan:`.",
    FloatAsStringFixed = 60 / 1: "`Float>>asStringFixed:`.",
    FloatAsStringExponential = 61 / 1: "`Float>>asStringExponential:`.",
    FloatAsStringPrecision = 62 / 1: "`Float>>asStringPrecision:`.",
    FloatParse = 63 / 1: "`Float class>>parse:`.",
    ClassBasicNew = 70 / 0: "`Class>>basicNew`.",
    IndexedNew = 71 / 1: "`new:` on the Array, WeakArray and ByteArray classes.",
    IndexedAt = 72 / 1: "`at:` on Array, WeakArray, ByteArray and String (a byte).",
    IndexedAtPut = 73 / 2: "`at:put:` on Array, WeakArray and ByteArray.",
    IndexedSize = 74 / 0: "`size` on Array, WeakArray and ByteArray.",
    CopyFromTo = 75 / 2: "`copyFrom:to:` on Array, ByteArray and String: the same kind.",
    CopyStringFromTo = 76 / 2: "`copyStringFrom:to:` on ByteArray and String.",
    CopyByteArrayFromTo = 77 / 2: "`copyByteArrayFrom:to:` on ByteArray and String.",
    ReplaceFromToWithStartingAt = 78 / 4: "`replaceFrom:to:with:startingAt:` on Array and ByteArray.",
    ByteArrayWithAll = 79 / 1: "`ByteArray class>>withAll:` from a ByteArray or String.",
    StringWithAll = 80 / 1: "`String class>>withAll:` from a ByteArray or String.",
    StringWith = 81 / 1: "`String class>>with:` a byte.",
    StringHash = 82 / 0: "`String>>hash`: a function of the bytes.",
    StringEquals = 83 / 1: "`String>>=`: false for a non-String.",
    StartsWith = 84 / 1: "`startsWith:` on String and ByteArray.",
    EndsWith = 85 / 1: "`endsWith:` on String and ByteArray.",
    IndexOfStartingAt = 86 / 2: "`indexOf:startingAt:`: 0 if absent.",
    LastIndexOfStartingAt = 87 / 2: "`lastIndexOf:startingAt:`: 0 if absent.",
    ObjectPerformWithArguments = 89 / 2: "`Object>>perform:withArguments:`: an ordinary send.",
    VmSnapshot = 90 / 0: "`vm snapshot`: the heap, saved without frames, as a ByteArray.",
    VmWriteFile = 91 / 2: "`vm writeBytes:toFile:` (output; interim host operation).",
    VmReadFile = 92 / 1: "`vm readFile:` (input; interim host operation).",
    VmSetRole = 93 / 2: "`vm setRole:to:`: assign a role by its snapshot id.",
    CurrentActivation = 100 / 0: "The activation of the frame that sent this message.",
    ActivationSender = 101 / 0: "`Activation>>sender`: the frame below, or nil.",
    ActivationSetSender = 102 / 1: "`Activation>>sender:`, restricted to cutting back to a live frame below.",
    ActivationMethod = 103 / 0: "`Activation>>method`.",
    ActivationReceiver = 104 / 0: "`Activation>>receiver`.",
    ActivationClosure = 105 / 0: "`Activation>>closure`: nil for a method activation.",
    ActivationTempAt = 106 / 1: "`Activation>>tempAt:`: arguments, then locals, then operands.",
    ActivationTempAtPut = 107 / 2: "`Activation>>tempAt:put:`.",
    ActivationSize = 108 / 0: "`Activation>>size`: the number of temps and operands.",
    ActivationBci = 109 / 0: "`Activation>>bci`: one-based, or nil once dead.",
    DefiningActivationOf = 110 / 1: "The activation that created a closure.",
    VmSlotOfAt = 111 / 2: "`vm slotOf:at:`: a slot of a regular object or ephemeron, or a contract field (format order).",
    VmSlotOfAtPut = 112 / 3: "`vm slotOf:at:put:`: regular objects and ephemerons only, not frozen.",
    VmClassOf = 113 / 1: "`vm classOf:`: the real class, whatever the object answers to `class`.",
    VmIsIdenticalTo = 114 / 2: "`vm is:identicalTo:`.",
    VmIdentityHashOf = 115 / 1: "`vm identityHashOf:`.",
    VmInstantiate = 116 / 1: "`vm instantiate:`: `basicNew` of the argument.",
    VmIsCanonical = 117 / 1: "`vm isCanonical:`: whether the object is frozen.",
    VmSetCanonical = 118 / 1: "`vm setCanonical:`: freeze the object.",
    VmPerformWith = 119 / 3: "`vm object:perform:with:`: an ordinary send, whatever the receiver overrides.",
    VmGarbageCollect = 120 / 0: "`vm garbageCollect`: a full collection; answers the receiver.",
    VmMonotonicNanos = 121 / 0: "`vm monotonicNanos` (input).",
    VmClone = 126 / 1: "`vm clone: root`: a copy of the mutable graph reachable from root (values, program objects, shared objects and their classes are not copied).",
    VmNewActor = 127 / 0: "`vm newActor`: a new actor running a clone of the platform template; answers a far reference to nil in it, its handle.",
    VmFarReferenceTo = 128 / 2: "`vm farReference: handle to: value`: a far reference to a value in the handle's actor.",
    VmPost = 129 / 2: "`vm post: array to: farReference`: queue the array, its elements translated by the remote-representation rules, in the far reference's actor, whose message loop gets dispatchPacket:.",
    VmIsFarReference = 130 / 1: "`vm isFarReference:`.",
    VmCurrentMessageLoop = 131 / 0: "`vm currentMessageLoop`: the running actor's message loop.",
    VmNewPromise = 132 / 1: "`vm newPromiseFor: localPromise`: a pending promise record (Π) whose owner is a far reference to the argument in the running actor.",
    VmResolvePromise = 133 / 2: "`vm resolvePromise: record with: value`: fulfil a pending record, or forward it if the value is a record; its waiters are sent resolveRemotely:. Answers whether it was pending.",
    VmBreakPromise = 134 / 2: "`vm breakPromise: record with: problem`: break a pending record; its waiters are sent breakRemotely:. Answers whether it was pending.",
    VmPromiseState = 135 / 1: "`vm promiseState: record`: `{state. payload}` at the end of its forwarding chain, in the running actor's terms: 0 and the owner's promise if that is in the running actor, else the last record; 1 and the resolution; 2 and the problem.",
    VmPromiseAddWaiter = 136 / 2: "`vm promise: record addWaiter: localPromise`: the argument is sent resolveRemotely: or breakRemotely: (in a packet) when the record, at the end of its chain, settles; at once if it has.",
    VmIsPromise = 137 / 1: "`vm isPromise:`: whether the object is a promise record.",
    VmHostGlobal = 140 / 0: "`vm hostGlobal`: an Alien for the host's global object (operation list, section 10).",
    AlienAt = 141 / 1: "`Alien>>at:`: `this[key]`.",
    AlienAtPut = 142 / 2: "`Alien>>at:put:`: `this[key] = value`; answers the value.",
    AlienRemoveKey = 143 / 1: "`Alien>>removeKey:`: `delete this[key]`.",
    AlienIncludesKey = 144 / 1: "`Alien>>includesKey:`: `key in this`.",
    AlienCall = 145 / 2: "`Alien>>call:withArguments:` (psoup's `perform:withArguments:`): `this[key](...)`.",
    AlienApply = 146 / 1: "`Alien>>callWithArguments:`: the alien is a function; `this(...)`.",
    AlienNew = 147 / 1: "`Alien>>newWithArguments:`: `new this(...)`.",
    AlienInstanceOf = 148 / 1: "`Alien>>isInstanceOf:` (psoup's `instanceOf:`): `this instanceof constructor`.",
    MethodHeader = 149 / 0: "`Method>>header`: psoup's header word: access << 28, primitive << 16, temps << 8, arguments; primitive 256 + i is a getter of the mixin's slot i, 512 + i its setter, 1023 answers the receiver, any other nonzero one a Eurasia native.",
    MethodHeaderPut = 150 / 1: "`Method>>header:` on an unfrozen method: sets its kind, access, argument and local counts from psoup's header word (see `Method>>header`).",
    MethodBytecode = 151 / 0: "`Method>>bytecode`: a copy of its V5 bytecode, a ByteArray.",
    MethodBytecodePut = 152 / 1: "`Method>>bytecode:` on an unfrozen method: a ByteArray of well-formed V5 instructions.",
    VmAllInstancesOf = 153 / 1: "`vm allInstancesOf: c`: an Array of the live instances of exactly `c`, cohort-wide, in allocation order (after a full collection).",
    ClosureCopiedAt = 154 / 2: "`copiedOf: closure at: i`: its ith copied value, from 1.",
    ClosureCopiedAtPut = 155 / 3: "`copiedOf: closure at: i put: v`.",
    ClosureInitialPc = 156 / 1: "`initialBCIOf: closure`: the bytecode position of its body, from 1.",
    ClosureNumCopied = 157 / 1: "`numCopiedOf: closure`: the number of its copied values.",
    VmCommit = 158 / 1: "`vm commit: {old objects. new objects. layout maps}`: installs a change set atomically. Each old program object's references go to its new one, which is frozen with what it reaches of the program; each layout map {old class. new class. for each new slot, the old slot index or 0} reshapes every instance of the old class in place. Answers the new program version.",
    VmMixinApplicationsOf = 159 / 1: "`vm mixinApplicationsOf: m`: an Array of the live classes whose mixin is `m`, cohort-wide, in allocation order (after a full collection).",
    VmSubclassesOf = 160 / 1: "`vm subclassesOf: c`: an Array of the live classes whose superclass is `c`, cohort-wide, in allocation order (after a full collection).",
    VmProgramVersion = 161 / 0: "`vm programVersion`: the number of commits so far.",
    ActivationNew = 162 / 0: "`Activation class>>new`: a detached activation (M6 design 4.4), with nil method, receiver, closure, sender and bci, and no temps.",
    ActivationMethodPut = 163 / 1: "`Activation>>method:`, on a detached activation.",
    ActivationClosurePut = 164 / 1: "`Activation>>closure:`, on a detached activation.",
    ActivationReceiverPut = 165 / 1: "`Activation>>receiver:`, on a detached activation.",
    ActivationBciPut = 166 / 1: "`Activation>>bci:`, on a detached activation.",
    ActivationSizePut = 167 / 1: "`Activation>>size:`, on a detached activation: new temps are nil.",
    VmMarkSimulationRoot = 168 / 1: "`vm markSimulationRoot: method`: marks a simulation root (psoup's primitive 142): a non-local return stops at it.",
    VmIsSimulationRoot = 169 / 1: "`vm isSimulationRoot: method`.",
    VmContinueAt = 170 / 2: "`vm continueAt: a with: v`: discards the frames above the live activation `a`, whose pending send answers `v` (psoup's jump to a live activation).",
    VmResumeActivation = 171 / 1: "`vm resumeActivation: top`: turns the chain of detached activations from `top` down to the sender, which must be the current frame, into frames, and continues at `top` (psoup's jump to a simulated activation). The handles keep their identity and become live.",
    VmRunNative = 172 / 3: "`vm runNative: method receiver: r arguments: args`, for the Simulator: runs a native or accessor method and answers {0. result}, {1. the failure to signal}, {2. receiver. selector. arguments} (a send to make instead) or {3. closure. arguments} (a closure to activate).",
    VmCheckpointNow = 174 / 0: "`vm checkpointNow`: a checkpoint at the next safepoint with no host operation under way, handed to the host (M7). Answers nil.",
    VmCheckpointPolicy = 175 / 1: "`vm checkpointPolicy: n`: a checkpoint at the first safepoint after every `n` allocations; 0 is none. Answers the receiver.",
    VmLogPosition = 176 / 0: "`vm logPosition`: the position in the host's input log (recording or replaying), or nil when there is none.",
    VmIncarnation = 177 / 0: "`vm incarnation`: a copy of the running incarnation's id, 16 bytes minted from host entropy at every start (contract design, section 6).",
    VmLineage = 178 / 0: "`vm lineage`: a copy of the id of the incarnation that saved the image this one started from, or nil (an image from genesis).",
    VmSaveImage = 179 / 1: "`saveImage: name`: a checkpoint at the next safepoint with no host operation under way, handed to the host to save under `name` (a String); the restore handler (role 40) is then sent `imageSaved`, in a turn of its own. Answers the receiver.",
    ClosureIn = 173 / 4: "`newClosureIn: activation initialBCI: bci numArgs: n numCopied: c`, for the Simulator (psoup's primitive 69): a closure defined by the activation, its body at bytecode position `bci` (from 1), with `c` copied values, nil until set.",
    VmSpawn = 139 / 1: "`vm spawn: mixin`: a new actor running a clone of the platform template, with the mixin applied to Object (role 37) in its heap, enclosing object nil; answers a far reference to the new class (`actor-semantics.tex`, eq. `create-actor`).",
    VmPromiseSend = 138 / 2: "`vm promise: record send: {selector. resultRecord. arguments...}`: an eventual send to the promise (Route-Pending): queued in the record, at the end of its chain, until it settles; delivered at once if it has.",
    VmIsValue = 125 / 1: "`vm isValue:`: whether the object is a value (immediates, nil, booleans, strings, numbers, instances of value classes, classes with a value enclosing object).",
    SlotStore = 124 / 1: "Not a native: the failure of a slot setter, for a frozen receiver or a non-value stored into a value.",
    VmFinishTurn = 123 / 1: "`MessageLoop>>finish:`: ends the turn; the argument is the next timer's monotonic time in nanoseconds, or 0 for none.",
    VmApplyMixin = 122 / 3: "`vm applyMixin:superclass:enclosingObject:`: a new class and its metaclass.",
}

/// `vm applyMixin: m superclass: s enclosingObject: eo` (operation list,
/// section 6; formal semantics, mixin application): a new class applying
/// the instance-side mixin `m` to `s`, named after `m`, and its metaclass
/// applying `m`'s class-side mixin (a contract field of `m`). The metaclass's superclass is that of `s`'s metaclass and its class
/// is `s`'s metaclass's class, so the new pair follows the image's own
/// convention (in the genesis kernel, every metaclass's superclass is
/// `Class`). Nothing is installed: the pair is ordinary heap objects, and
/// instances get their layout from the mixins along the chain.
pub fn apply_mixin(m: Value, s: Value, eo: Value, ctx: &mut NativeContext) -> Outcome {
    // An instance-side mixin: one whose class-side mixin is a mixin.
    let Some((name, cm)) = ctx.heap.mixin(m).map(|x| (x.name, x.class_mixin)) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    if ctx.heap.mixin(cm).is_none() {
        return Outcome::Fail(Reason::WrongKind(1));
    }
    // A nil superclass makes a root class, as psoup's apply does; its
    // metaclass follows the image's root's (`Class` its superclass).
    let nil = ctx.nil();
    let reference = if s == nil {
        ctx.roles.get(Role::ObjectClass)
    } else {
        s
    };
    let Some(Body::Class(_)) = ctx.heap.get(reference).map(|o| &o.body) else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let super_meta = ctx.heap.get(reference).expect("class").header.class;
    let Some(super_meta_object) = ctx.heap.get(super_meta) else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let metaclass_class = super_meta_object.header.class;
    let Body::Metaclass(meta_behavior) = &super_meta_object.body else {
        return Outcome::Fail(Reason::WrongKind(2));
    };
    let meta_super = meta_behavior.superclass;
    // A value class: the superclass is one (its chain includes Value). Its
    // enclosing object must be a value, and its mixin may declare no mutable
    // slot (a setter other than the private init` ones) (design doc section
    // 5: the checks happen at mixin application until the spec lets the
    // compiler know value classes).
    let value_class = s != nil
        && (ctx.heap.get(s).expect("class").header.flags & VALUE_CLASS != 0
            || s == ctx.roles.get(Role::ValueClass));
    if value_class {
        if !ctx.is_value(eo) {
            return Outcome::Fail(Reason::WrongKind(3));
        }
        if has_mutable_slot(ctx.heap, m) {
            return Outcome::Fail(Reason::WrongKind(1));
        }
    }
    let class_flags =
        (if ctx.is_value(eo) { VALUE } else { 0 }) | (if value_class { VALUE_CLASS } else { 0 });
    let metaclass = ctx.heap.allocate(
        metaclass_class,
        ctx.space,
        Body::Metaclass(Behavior {
            mixin: cm,
            superclass: meta_super,
            enclosing_object: eo,
            name: nil,
            this_class: nil,
            library: nil,
        }),
    );
    let class = ctx.heap.allocate(
        metaclass,
        ctx.space,
        Body::Class(Behavior {
            mixin: m,
            superclass: s,
            enclosing_object: eo,
            name,
            this_class: nil,
            library: nil,
        }),
    );
    if let Some(Body::Metaclass(b)) = ctx.heap.get_mut(metaclass).map(|o| &mut o.body) {
        b.this_class = class;
    }
    // A class (and its metaclass) whose enclosing object is a value is one.
    ctx.heap.get_mut(class).expect("class").header.flags |= class_flags;
    ctx.heap.get_mut(metaclass).expect("metaclass").header.flags |= class_flags & VALUE;
    Outcome::Answer(class)
}

/// A far reference to `target` in `actor`'s heap, allocated in the running
/// actor's. Far references are immutable capabilities, so they are values.
pub fn new_far_reference(ctx: &mut NativeContext<'_>, actor: u32, target: Value) -> Value {
    let class = ctx.roles.get(Role::FarReferenceClass);
    let space = ctx.space;
    let r = ctx
        .heap
        .allocate(class, space, Body::FarRef { actor, target });
    ctx.heap.get_mut(r).expect("new").header.flags |= VALUE;
    r
}

/// The largest magnitude a host number holds exactly (2^53).
pub const HOST_EXACT: i64 = 1 << 53;

/// An Alien (role 33: one slot, the host reference) for host reference `r`.
pub fn new_alien(ctx: &mut NativeContext<'_>, r: u32) -> Value {
    let class = ctx.roles.get(Role::AlienClass);
    let space = ctx.space;
    ctx.heap.allocate(
        class,
        space,
        Body::Regular(alloc::vec![small(i64::from(r))]),
    )
}

/// The host reference of an Alien.
fn alien_reference(ctx: &NativeContext<'_>, v: Value) -> Option<u32> {
    alien_reference_in(ctx.heap, ctx.roles, v)
}

/// The host reference of an Alien, if `v` is one.
pub fn alien_reference_in(heap: &Heap, roles: &Roles, v: Value) -> Option<u32> {
    let o = heap.get(v)?;
    if o.header.class != roles.get(Role::AlienClass) {
        return None;
    }
    match &o.body {
        Body::Regular(slots) if slots.len() == 1 => slots[0]
            .as_small_integer()
            .and_then(|r| u32::try_from(r).ok()),
        _ => None,
    }
}

/// Whether `v` is an Alien that did not survive a restore into a new host
/// ([`crate::Vm::restored_into_new_host`] gives it the reference -1).
pub fn is_dead_alien(heap: &Heap, roles: &Roles, v: Value) -> bool {
    heap.get(v).is_some_and(|o| {
        o.header.class == roles.get(Role::AlienClass)
            && matches!(&o.body, Body::Regular(s) if s.len() == 1 && s[0].as_small_integer() == Some(-1))
    })
}

/// The arguments of a host call: the `fixed` ones (indices 1, 2, ...), then
/// the elements of an Array argument at `spread`'s index; each with the
/// index a failure names. The interpreter marshals them.
fn host_call(
    ctx: &NativeContext<'_>,
    op: HostOp,
    receiver: Value,
    fixed: &[Value],
    spread: Option<(Value, u32)>,
    answer: Option<Value>,
) -> Outcome {
    if is_dead_alien(ctx.heap, ctx.roles, receiver) {
        return Outcome::Fail(Reason::Dead);
    }
    let Some(r) = alien_reference(ctx, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let mut args: Vec<(Value, u32)> = fixed
        .iter()
        .enumerate()
        .map(|(i, &v)| (v, i as u32 + 1))
        .collect();
    if let Some((array, index)) = spread {
        let Some(elements) = ctx.heap.array_elements(array) else {
            return Outcome::Fail(Reason::WrongKind(index));
        };
        args.extend(elements.iter().map(|&v| (v, index)));
    }
    Outcome::HostCall {
        op,
        receiver: r,
        args,
        answer,
    }
}

/// A method's header as psoup's header word: access << 28, primitive << 16,
/// temps (arguments and locals) << 8, arguments. The primitive encodes the
/// kind: 0 ordinary, 256 + i a getter and 512 + i a setter of the method's
/// own mixin's slot i, 1023 answers the receiver, anything else a native.
pub fn psoup_header(h: &MethodHeader) -> i64 {
    let access = match h.access {
        Access::Public => 0,
        Access::Protected => 1,
        Access::Private => 2,
    };
    let primitive = match h.kind {
        MethodKind::Ordinary => 0,
        MethodKind::Native => i64::from(h.native),
        MethodKind::Getter => 256 + i64::from(h.slot),
        MethodKind::Setter => 512 + i64::from(h.slot),
        MethodKind::ReturnSelf => 1023,
    };
    let temps = i64::from(h.arg_count + h.local_count);
    (access << 28) | (primitive << 16) | (temps << 8) | i64::from(h.arg_count)
}

/// The header psoup's header word describes (see [`psoup_header`]), if it
/// is well formed: a known access modifier, at least as many temps as
/// arguments for an ordinary method, a native of the right arity, and
/// accessors of the right arity.
pub fn header_from_psoup(word: i64) -> Option<MethodHeader> {
    if !(0..1 << 30).contains(&word) {
        return None;
    }
    let access = match word >> 28 {
        0 => Access::Public,
        1 => Access::Protected,
        2 => Access::Private,
        _ => return None,
    };
    let primitive = ((word >> 16) & 0xFFF) as u32;
    let temps = ((word >> 8) & 0xFF) as u32;
    let arg_count = (word & 0xFF) as u32;
    // psoup's accessor headers leave the temps 0; only an ordinary method's
    // count its arguments.
    if primitive == 0 && temps < arg_count {
        return None;
    }
    let mut header = MethodHeader {
        kind: MethodKind::Ordinary,
        access,
        arg_count,
        local_count: temps.saturating_sub(arg_count),
        native: 0,
        slot: 0,
    };
    match primitive {
        0 => {}
        256..=511 if arg_count == 0 => {
            header.kind = MethodKind::Getter;
            header.slot = primitive - 256;
        }
        512..=1022 if arg_count == 1 => {
            header.kind = MethodKind::Setter;
            header.slot = primitive - 512;
        }
        1023 => header.kind = MethodKind::ReturnSelf,
        n => {
            let native = Native::from_id(n)?;
            if native.arity() != arg_count as usize {
                return None;
            }
            header.kind = MethodKind::Native;
            header.native = n;
        }
    }
    if header.kind != MethodKind::Ordinary {
        header.local_count = 0;
    }
    Some(header)
}

/// Whether `bytes` is a sequence of whole V5 instructions.
fn well_formed(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        let n = crate::bytecode::instruction_length(bytes[i]);
        if n == 0 {
            return false;
        }
        i += n;
    }
    i == bytes.len()
}

/// The method `v`, if it is one and not frozen (installed).
fn unfrozen_method<'h>(ctx: &'h mut NativeContext<'_>, v: Value) -> Option<&'h mut Method> {
    let o = ctx.heap.get_mut(v)?;
    if o.header.flags & FROZEN != 0 {
        return None;
    }
    match &mut o.body {
        Body::Method(m) => Some(m),
        _ => None,
    }
}

/// Whether `v` is a promise record.
pub fn is_promise(heap: &Heap, v: Value) -> bool {
    matches!(heap.get(v).map(|o| &o.body), Some(Body::Promise(_)))
}

/// The promise record at the end of `record`'s forwarding chain.
pub fn last_promise(heap: &Heap, record: Value) -> Value {
    let mut r = record;
    while let Some(Body::Promise(p)) = heap.get(r).map(|o| &o.body)
        && p.state == PromiseRecord::FORWARDED
    {
        r = p.value;
    }
    r
}

/// `v`, in no actor's terms, as actor `actor` sees it: a far reference into
/// it is its target (RR-Far-Home); anything else is itself.
pub fn localize(heap: &Heap, v: Value, actor: u32) -> Value {
    match heap.get(v).map(|o| &o.body) {
        Some(Body::FarRef { actor: a, target }) if *a == actor => *target,
        _ => v,
    }
}

/// Whether mixin `m` declares a mutable slot: it has a setter other than the
/// private `init` ones that initializers use, and the nested-class cache
/// setters (`` X`slot: ``), which memoize a class; that class is a value when
/// its enclosing object, the value being checked, is one.
fn has_mutable_slot(heap: &Heap, m: Value) -> bool {
    let Some(methods) = heap.mixin(m).and_then(|x| heap.array_elements(x.methods)) else {
        return false;
    };
    methods.iter().any(|&method| {
        heap.method(method).is_some_and(|record| {
            record.header.kind == crate::object::MethodKind::Setter
                && !heap
                    .string_bytes(record.selector)
                    .is_some_and(|s| s.starts_with(b"init`") || s.ends_with(b"`slot:"))
        })
    })
}

/// The largest shift `<<` performs; beyond it the result would exhaust
/// memory, and the native fails with `#exhausted: #memory`.
pub const MAX_SHIFT: i64 = 1 << 24;

/// A resource whose bound can be exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// Heap memory.
    Memory,
    /// Stack depth.
    Stack,
    /// Callback nesting.
    Nesting,
    /// The alien handle table.
    Handles,
}

/// Why a native failed (operation list, section 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Argument `i` (0 = receiver) is of an unacceptable kind.
    WrongKind(u32),
    /// Argument `i` is an index or size out of range.
    OutOfRange(u32),
    /// Division or modulus by zero.
    ZeroDivide,
    /// Wrong number of arguments.
    Arity,
    /// The class cannot be instantiated this way.
    NotInstantiable,
    /// A dead activation, alien or expat.
    Dead,
    /// A resource limit.
    Exhausted(Resource),
    /// The host raised; the value is the host exception.
    Host(Value),
    /// Not provided on this host, or not yet implemented.
    Unsupported,
}

impl Reason {
    /// The reason's name, as stored in failure objects.
    pub fn name(self) -> &'static str {
        match self {
            Reason::WrongKind(_) => "wrongKind",
            Reason::OutOfRange(_) => "outOfRange",
            Reason::ZeroDivide => "zeroDivide",
            Reason::Arity => "arity",
            Reason::NotInstantiable => "notInstantiable",
            Reason::Dead => "dead",
            Reason::Exhausted(_) => "exhausted",
            Reason::Host(_) => "host",
            Reason::Unsupported => "unsupported",
        }
    }

    /// The reason's argument: an index, a resource name, or the host value.
    pub fn argument(self, ctx: &mut NativeContext<'_>) -> Value {
        match self {
            Reason::WrongKind(i) | Reason::OutOfRange(i) => small(i64::from(i)),
            Reason::Exhausted(r) => {
                let name = match r {
                    Resource::Memory => "memory",
                    Resource::Stack => "stack",
                    Resource::Nesting => "nesting",
                    Resource::Handles => "handles",
                };
                ctx.new_string(name)
            }
            Reason::Host(v) => v,
            _ => ctx.roles.get(Role::Nil),
        }
    }
}

/// What a native did (the three outcomes, plus control transfers the
/// interpreter performs).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Rule Native-Answer.
    Answer(Value),
    /// Rule Native-Tail: an ordinary send of `selector` to `receiver`.
    TailSend {
        /// The new receiver.
        receiver: Value,
        /// The selector.
        selector: &'static str,
        /// The arguments.
        args: Vec<Value>,
    },
    /// Rule Native-Fail.
    Fail(Reason),
    /// `vm exit:` ended the program.
    Exit(i32),
    /// A closure's `value` family: run `closure` with `args` in a new frame.
    /// The arity has been checked.
    ActivateClosure {
        /// The closure.
        closure: Value,
        /// Its arguments.
        args: Vec<Value>,
    },
    /// A closure's `value` to `value:value:value:`: run the receiver, a
    /// closure, with the native's own arguments, which are still on the
    /// stack, so nothing is copied. The arity has been checked.
    ActivateReceiver,
    /// `vm snapshot`: the interpreter serializes the heap and answers it.
    Snapshot,
    /// Collect garbage now (`vm garbageCollect`), then answer the receiver.
    Collect,
    /// `vm newActor`: the interpreter, which owns the actors, creates one.
    NewActor,
    /// `vm post:to:`: the interpreter translates and queues the packet.
    Post {
        /// The packet (an Array in the sending actor's heap).
        packet: Value,
        /// The destination.
        far_reference: Value,
    },
    /// `vm currentMessageLoop`.
    CurrentMessageLoop,
    /// An Alien operation: the interpreter marshals the arguments (closures
    /// become expats) and performs it, with itself as the host's way to call
    /// back.
    HostCall {
        /// The operation.
        op: HostOp,
        /// The receiver's host reference.
        receiver: u32,
        /// The arguments, each with the argument index a failure names.
        args: Vec<(Value, u32)>,
        /// The answer instead of the host's result (`at:put:` answers the
        /// value).
        answer: Option<Value>,
    },
    /// `vm spawn:`: the interpreter creates the actor and applies the mixin
    /// in its heap.
    Spawn(Value),
    /// `vm allInstancesOf:`: the interpreter collects, then answers them.
    AllInstances(Value),
    /// `vm commit:`: the interpreter applies the change set.
    Commit(Value),
    /// `vm mixinApplicationsOf:` (by mixin) or `vm subclassesOf:`: the
    /// interpreter collects, so that no dead class answers, then scans.
    ClassQuery {
        /// The mixin or the superclass.
        target: Value,
        /// Whether `target` is a mixin.
        by_mixin: bool,
    },
    /// `vm programVersion`.
    ProgramVersion,
    /// `vm checkpointNow`.
    CheckpointNow,
    /// `vm checkpointPolicy:`.
    CheckpointPolicy(u64),
    /// `vm continueAt:with:`: the interpreter cuts back to the activation.
    ContinueAt {
        /// A live activation.
        activation: Value,
        /// What its pending send answers.
        value: Value,
    },
    /// `vm resumeActivation:`: the interpreter makes frames of a detached
    /// chain.
    ResumeActivation(Value),
    /// `vm runNative:receiver:arguments:`: the interpreter runs the method.
    RunNative {
        /// A native or accessor method.
        method: Value,
        /// The receiver.
        receiver: Value,
        /// An Array of the arguments.
        arguments: Value,
    },
    /// `vm resolvePromise:with:` and `vm breakPromise:with:`: the
    /// interpreter settles the record and notifies its waiters.
    Settle {
        /// The promise record.
        record: Value,
        /// [`PromiseRecord::FULFILLED`] or [`PromiseRecord::BROKEN`].
        state: u8,
        /// The resolution or problem, in the running actor's terms.
        value: Value,
    },
    /// `vm promise:send:`: the interpreter queues the message in the record
    /// or delivers it.
    PromiseSend {
        /// The promise record.
        record: Value,
        /// `{selector. result record or nil. arguments...}`, in the running
        /// actor's terms.
        message: Value,
    },
    /// `vm promise:addWaiter:`: the interpreter registers the waiter, or
    /// notifies it at once.
    AddWaiter {
        /// The promise record.
        record: Value,
        /// A library promise in the running actor.
        waiter: Value,
    },
    /// End the turn (`finish:`): the stack unwinds and the VM's event loop
    /// waits until the given monotonic time in nanoseconds (0: no timer).
    FinishTurn(u64),
    /// An activation native: the interpreter, which owns the frames, carries
    /// it out and answers or fails.
    Frame(Native),
    /// `saveImage:`: the interpreter takes a checkpoint and hands it to
    /// the host under this name.
    SaveImage(String),
    /// `vm setRole:to:`: the interpreter assigns the role.
    SetRole {
        /// The role.
        role: Role,
        /// Its new value.
        value: Value,
    },
    /// `perform:withArguments:`: an ordinary send whose selector is a
    /// String object. The arity has been checked.
    Perform {
        /// The receiver.
        receiver: Value,
        /// The selector (a String).
        selector: Value,
        /// The arguments.
        args: Vec<Value>,
    },
}

/// What a native may touch.
pub struct NativeContext<'a> {
    /// The heap.
    pub heap: &'a mut Heap,
    /// The roles.
    pub roles: &'a Roles,
    /// The host.
    pub host: &'a mut dyn Host,
    /// The running actor's heap, where allocations go.
    pub space: Space,
}

impl NativeContext<'_> {
    /// Allocates a String.
    pub fn new_string(&mut self, s: &str) -> Value {
        self.new_bytes_object(Role::StringClass, Body::String(s.as_bytes().to_vec()))
    }

    fn new_bytes_object(&mut self, role: Role, body: Body) -> Value {
        let class = self.roles.get(role);
        self.heap.allocate(class, self.space, body)
    }

    fn boolean(&self, b: bool) -> Value {
        self.roles.get(if b { Role::True } else { Role::False })
    }

    /// Whether `v` is a value: an immediate, nil, true, false, or an object
    /// with the value bit.
    pub fn is_value(&self, v: Value) -> bool {
        v == self.nil()
            || v == self.roles.get(Role::True)
            || v == self.roles.get(Role::False)
            || self.heap.is_value(v)
    }

    fn nil(&self) -> Value {
        self.roles.get(Role::Nil)
    }

    /// An integer value: a SmallInteger when it fits, else a LargeInteger.
    fn integer(&mut self, n: BigInt) -> Value {
        if let Some(v) = n.to_i64().and_then(Value::small_integer) {
            return v;
        }
        let (negative, magnitude) = n.to_bytes();
        let class = self.roles.get(Role::LargeIntegerClass);
        self.heap.allocate(
            class,
            self.space,
            Body::LargeInteger {
                negative,
                magnitude,
            },
        )
    }

    /// A Float value, with NaN canonical.
    fn float(&mut self, f: f64) -> Value {
        let f = if f.is_nan() { f64::NAN } else { f };
        let class = self.roles.get(Role::FloatClass);
        self.heap.allocate(class, self.space, Body::Float(f))
    }
}

fn small(n: SmallInteger) -> Value {
    Value::small_integer(n).expect("small enough")
}

/// A number, as natives see it.
#[derive(Clone, Debug)]
enum Num {
    Int(BigInt),
    Float(f64),
}

fn integer_of(heap: &Heap, v: Value) -> Option<BigInt> {
    if let Some(n) = v.as_small_integer() {
        return Some(BigInt::from_i64(n));
    }
    match &heap.get(v)?.body {
        Body::LargeInteger {
            negative,
            magnitude,
        } => Some(BigInt::from_bytes(*negative, magnitude)),
        _ => None,
    }
}

fn float_of(heap: &Heap, v: Value) -> Option<f64> {
    match &heap.get(v)?.body {
        Body::Float(f) => Some(*f),
        _ => None,
    }
}

fn number_of(heap: &Heap, v: Value) -> Option<Num> {
    integer_of(heap, v)
        .map(Num::Int)
        .or_else(|| float_of(heap, v).map(Num::Float))
}

fn as_f64(n: &Num) -> f64 {
    match n {
        Num::Int(i) => i.to_f64(),
        Num::Float(f) => *f,
    }
}

fn small_index(v: Value) -> Option<i64> {
    v.as_small_integer()
}

/// Integer arithmetic: an integer argument computes; anything else
/// tail-sends the double-dispatch selector to the argument.
fn integer_arithmetic(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(&BigInt, &BigInt) -> BigInt,
    tail: &'static str,
) -> Outcome {
    // Two SmallIntegers whose result is one: no BigInt.
    if let (Some(x), Some(y)) = (receiver.as_small_integer(), arg.as_small_integer()) {
        let small = match tail {
            "addFromInteger:" => x.checked_add(y),
            "subtractFromInteger:" => x.checked_sub(y),
            "multiplyFromInteger:" => x.checked_mul(y),
            _ => None,
        };
        if let Some(v) = small.and_then(Value::small_integer) {
            return Outcome::Answer(v);
        }
    }
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match integer_of(ctx.heap, arg) {
        Some(b) => Outcome::Answer(ctx.integer(op(&a, &b))),
        None => Outcome::TailSend {
            receiver: arg,
            selector: tail,
            args: alloc::vec![receiver],
        },
    }
}

/// Float arithmetic: an integer or float argument computes; anything else
/// tail-sends the double-dispatch selector.
fn float_arithmetic(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(f64, f64) -> f64,
    tail: &'static str,
) -> Outcome {
    let Some(a) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match number_of(ctx.heap, arg) {
        Some(b) => {
            // IEEE 754: a zero divisor gives an infinity or NaN, as in psoup.
            Outcome::Answer(ctx.float(op(a, as_f64(&b))))
        }
        None => Outcome::TailSend {
            receiver: arg,
            selector: tail,
            args: alloc::vec![receiver],
        },
    }
}

fn compare(a: &Num, b: &Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(y)),
        _ => as_f64(a).partial_cmp(&as_f64(b)),
    }
}

/// A comparison. A non-number argument tail-sends the reversed comparison
/// (`a < b` becomes `b > a`), so other kinds of number can answer.
fn comparison(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    test: fn(Ordering) -> bool,
    reversed: &'static str,
) -> Outcome {
    if let (Some(x), Some(y)) = (receiver.as_small_integer(), arg.as_small_integer()) {
        return Outcome::Answer(ctx.boolean(test(x.cmp(&y))));
    }
    let Some(a) = number_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match number_of(ctx.heap, arg) {
        Some(b) => Outcome::Answer(ctx.boolean(compare(&a, &b).is_some_and(test))),
        None => Outcome::TailSend {
            receiver: arg,
            selector: reversed,
            args: alloc::vec![receiver],
        },
    }
}

#[derive(Clone, Copy)]
enum Division {
    FloorDivide,
    FloorModulo,
    Quo,
    Rem,
}

fn division(ctx: &mut NativeContext<'_>, receiver: Value, arg: Value, kind: Division) -> Outcome {
    // Two SmallIntegers, a non-zero divisor and a result in range: no BigInt.
    if let (Some(x), Some(y)) = (receiver.as_small_integer(), arg.as_small_integer())
        && y != 0
    {
        let v = match kind {
            Division::FloorDivide => x.checked_div(y).map(|q| {
                if x % y != 0 && ((x < 0) != (y < 0)) {
                    q - 1
                } else {
                    q
                }
            }),
            Division::FloorModulo => x.checked_rem(y).map(|r| {
                if r != 0 && ((r < 0) != (y < 0)) {
                    r + y
                } else {
                    r
                }
            }),
            Division::Quo => x.checked_div(y),
            Division::Rem => x.checked_rem(y),
        };
        if let Some(v) = v.and_then(Value::small_integer) {
            return Outcome::Answer(v);
        }
    }
    let Some(a) = number_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(b) = number_of(ctx.heap, arg) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    if let (Num::Int(x), Num::Int(y)) = (&a, &b) {
        let result = match kind {
            Division::FloorDivide | Division::FloorModulo => x.div_mod(y),
            Division::Quo | Division::Rem => x.quo_rem(y),
        };
        let Some((q, r)) = result else {
            return Outcome::Fail(Reason::ZeroDivide);
        };
        let v = match kind {
            Division::FloorDivide | Division::Quo => q,
            Division::FloorModulo | Division::Rem => r,
        };
        return Outcome::Answer(ctx.integer(v));
    }
    let (x, y) = (as_f64(&a), as_f64(&b));
    if y == 0.0 {
        return Outcome::Fail(Reason::ZeroDivide);
    }
    let quotient = match kind {
        Division::FloorDivide | Division::FloorModulo => libm::floor(x / y),
        Division::Quo | Division::Rem => libm::trunc(x / y),
    };
    match kind {
        Division::FloorDivide | Division::Quo => match BigInt::from_f64_integral(quotient) {
            Some(q) => Outcome::Answer(ctx.integer(q)),
            None => Outcome::Fail(Reason::OutOfRange(0)),
        },
        Division::FloorModulo | Division::Rem => Outcome::Answer(ctx.float(x - y * quotient)),
    }
}

fn integer_bits(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    op: fn(&BigInt, &BigInt) -> BigInt,
    small: fn(i64, i64) -> i64,
) -> Outcome {
    // Two SmallIntegers: and, or and xor of two are one.
    if let (Some(x), Some(y)) = (receiver.as_small_integer(), arg.as_small_integer())
        && let Some(v) = Value::small_integer(small(x, y))
    {
        return Outcome::Answer(v);
    }
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let Some(b) = integer_of(ctx.heap, arg) else {
        return Outcome::Fail(Reason::WrongKind(1));
    };
    Outcome::Answer(ctx.integer(op(&a, &b)))
}

fn shift(ctx: &mut NativeContext<'_>, receiver: Value, arg: Value, left: bool) -> Outcome {
    let Some(a) = integer_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    let k = match arg.as_small_integer() {
        Some(k) => k,
        // A LargeInteger count: every bit shifts out to the right; to the
        // left the result could not be represented.
        None => match integer_of(ctx.heap, arg) {
            Some(big) if big.is_negative() => return Outcome::Fail(Reason::OutOfRange(1)),
            Some(_) => i64::MAX,
            None => return Outcome::Fail(Reason::WrongKind(1)),
        },
    };
    if k < 0 {
        return Outcome::Fail(Reason::OutOfRange(1));
    }
    if left {
        if k > MAX_SHIFT && !a.is_zero() {
            return Outcome::Fail(Reason::Exhausted(Resource::Memory));
        }
        Outcome::Answer(ctx.integer(a.shl(k.min(MAX_SHIFT) as usize)))
    } else {
        let k = usize::try_from(k)
            .unwrap_or(usize::MAX)
            .min(a.bit_length() + 1);
        Outcome::Answer(ctx.integer(a.shr(k)))
    }
}

/// The shortest decimal that reads back as the same Float, printed as psoup
/// prints it (double-conversion's shortest form with JavaScript's limits):
/// decimal notation, with `.0` on integral values, when the decimal
/// exponent is from -6 to 20, otherwise `d.ddde+X`; and `NaN`, `Infinity`,
/// `-Infinity`.
pub fn float_to_string(f: f64) -> String {
    if f.is_nan() {
        return String::from("NaN");
    }
    if f.is_infinite() {
        return String::from(if f > 0.0 { "Infinity" } else { "-Infinity" });
    }
    let sign = if f.is_sign_negative() { "-" } else { "" };
    // Rust's exponential form has the shortest round-tripping digits.
    let e = format!("{:e}", f.abs());
    let (mantissa, exponent) = e.split_once('e').expect("exponential form");
    let exponent: i32 = exponent.parse().expect("exponent");
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let n = digits.len() as i32;
    if (-6..21).contains(&exponent) {
        if exponent < 0 {
            let zeros = "0".repeat((-exponent - 1) as usize);
            format!("{sign}0.{zeros}{digits}")
        } else if n <= exponent + 1 {
            let zeros = "0".repeat((exponent + 1 - n) as usize);
            format!("{sign}{digits}{zeros}.0")
        } else {
            let (int, frac) = digits.split_at(exponent as usize + 1);
            format!("{sign}{int}.{frac}")
        }
    } else {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if exponent < 0 { "-" } else { "+" };
        format!(
            "{sign}{first}{point}{rest}e{exponent_sign}{}",
            exponent.abs()
        )
    }
}

/// `Float parse:`: decimal and exponential notation, `NaN`, `Infinity` and
/// `-Infinity`, as psoup accepts them; not Rust's other spellings
/// (`inf`, `nan`, any case).
fn parse_float(s: &str) -> Option<f64> {
    match s {
        "NaN" => return Some(f64::NAN),
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    if s.bytes()
        .any(|b| !(b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E')))
    {
        return None;
    }
    s.parse().ok()
}

fn float_unary(ctx: &mut NativeContext<'_>, receiver: Value, f: fn(f64) -> f64) -> Outcome {
    match float_of(ctx.heap, receiver) {
        Some(x) => Outcome::Answer(ctx.float(f(x))),
        None => Outcome::Fail(Reason::WrongKind(0)),
    }
}

fn float_to_integer(ctx: &mut NativeContext<'_>, receiver: Value, f: fn(f64) -> f64) -> Outcome {
    let Some(x) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    match BigInt::from_f64_integral(f(x)) {
        Some(n) => Outcome::Answer(ctx.integer(n)),
        None => Outcome::Fail(Reason::OutOfRange(0)),
    }
}

/// `asStringFixed:`, `asStringExponential:` and `asStringPrecision:`, as
/// JavaScript's `toFixed`, `toExponential` and `toPrecision` (psoup's
/// behaviour): the digit count must be in `range`.
fn float_format(
    ctx: &mut NativeContext<'_>,
    receiver: Value,
    arg: Value,
    range: core::ops::RangeInclusive<i64>,
    large_is_shortest: bool,
    format: fn(f64, usize) -> String,
) -> Outcome {
    let Some(x) = float_of(ctx.heap, receiver) else {
        return Outcome::Fail(Reason::WrongKind(0));
    };
    // Operation list 3.1: any unusable digit count is #outOfRange: 1.
    let Some(digits) = arg.as_small_integer().filter(|d| range.contains(d)) else {
        return Outcome::Fail(Reason::OutOfRange(1));
    };
    // JavaScript prints non-finite values, and toFixed values of 1e21 and
    // beyond, as their shortest form.
    if !x.is_finite() || (large_is_shortest && x.abs() >= 1e21) {
        let s = float_to_string(x);
        return Outcome::Answer(ctx.new_string(&s));
    }
    let s = format(x, digits as usize);
    Outcome::Answer(ctx.new_string(&s))
}

/// JavaScript's exponent: `3e0` becomes `3e+0`; `3e-1` stays.
fn signed_exponent(s: String) -> String {
    match s.find('e') {
        Some(i) if !s[i + 1..].starts_with('-') => format!("{}e+{}", &s[..i], &s[i + 1..]),
        _ => s,
    }
}

/// The bytes of a String or ByteArray.
fn bytes_of(heap: &Heap, v: Value) -> Option<&[u8]> {
    match &heap.get(v)?.body {
        Body::String(b) | Body::Bytes(b) => Some(b),
        _ => None,
    }
}

/// The fields `vm slotOf:at:` reads: a regular object's slots, an
/// ephemeron's key, value and finalizer, or a contract object's fields in the
/// snapshot format's edge order (format section 5).
pub(crate) fn readable_fields(body: &Body) -> Option<Vec<Value>> {
    Some(match body {
        Body::Regular(slots) => slots.clone(),
        Body::Ephemeron {
            key,
            value,
            finalizer,
        } => alloc::vec![*key, *value, *finalizer],
        Body::Class(b) => alloc::vec![b.mixin, b.superclass, b.enclosing_object, b.name, b.library],
        Body::Metaclass(b) => alloc::vec![
            b.mixin,
            b.superclass,
            b.enclosing_object,
            b.this_class,
            b.library
        ],
        Body::Mixin(m) => alloc::vec![
            m.name,
            m.methods,
            m.enclosing_mixin,
            m.slot_names,
            m.class_mixin,
            m.library
        ],
        Body::Method(m) => alloc::vec![m.literals, m.mixin, m.selector, m.source, m.library],
        _ => return None,
    })
}

/// The number of instance slots of `class`: every mixin's slot declarations
/// along the superclass chain.
fn instance_slots(heap: &Heap, roles: &Roles, class: Value) -> Option<usize> {
    let nil = roles.get(Role::Nil);
    let mut total = 0usize;
    let mut c = class;
    for _ in 0..100_000 {
        if c == nil {
            return Some(total);
        }
        let b = heap.behavior(c)?;
        total += heap.mixin(b.mixin)?.slot_count as usize;
        c = b.superclass;
    }
    None
}

/// A one-based index into a sequence of `len` elements.
fn index(v: Value, len: usize) -> Result<usize, Reason> {
    match small_index(v) {
        None => Err(Reason::WrongKind(1)),
        Some(i) if i >= 1 && (i as u64) <= len as u64 => Ok(i as usize - 1),
        Some(_) => Err(Reason::OutOfRange(1)),
    }
}

/// A one-based inclusive range `start..=stop` in a sequence of `len`
/// elements; `stop = start - 1` is empty. Arguments are numbered from
/// `first` for failure reasons.
fn range(
    start: Value,
    stop: Value,
    len: usize,
    first: u32,
) -> Result<core::ops::Range<usize>, Reason> {
    let (Some(a), Some(b)) = (small_index(start), small_index(stop)) else {
        return Err(Reason::WrongKind(if small_index(start).is_none() {
            first
        } else {
            first + 1
        }));
    };
    if a < 1 || b < a - 1 || b > len as i64 {
        return Err(Reason::OutOfRange(if a < 1 { first } else { first + 1 }));
    }
    Ok(a as usize - 1..b as usize)
}

/// The one-based position of `needle` in `haystack`, searching forward from,
/// or (`last`) backward from, the zero-based position `from`; 0 when absent.
/// An empty needle is found at `from` itself, as in psoup.
fn find(haystack: &[u8], needle: &[u8], from: usize, last: bool) -> usize {
    if needle.is_empty() {
        return from + 1;
    }
    if needle.len() > haystack.len() {
        return 0;
    }
    let positions = 0..=haystack.len() - needle.len();
    let hit = |&i: &usize| haystack[i..i + needle.len()] == *needle;
    let found = if last {
        positions.filter(|&i| i <= from).rev().find(hit)
    } else {
        positions.filter(|&i| i >= from).find(hit)
    };
    found.map_or(0, |i| i + 1)
}

/// The number of arguments a selector takes.
fn selector_arity(selector: &[u8]) -> usize {
    match selector.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == b'_' => {
            selector.iter().filter(|&&b| b == b':').count()
        }
        Some(_) => 1,
        None => 0,
    }
}

fn activate_closure(receiver: Value, args: Vec<Value>, ctx: &NativeContext<'_>) -> Outcome {
    match ctx.heap.closure(receiver) {
        Some(c) if c.num_args as usize == args.len() => Outcome::ActivateClosure {
            closure: receiver,
            args,
        },
        Some(_) => Outcome::Fail(Reason::Arity),
        None => Outcome::Fail(Reason::WrongKind(0)),
    }
}

/// Runs a native. `args` excludes the receiver and has the native's arity.
pub fn invoke(
    native: Native,
    receiver: Value,
    args: &[Value],
    ctx: &mut NativeContext<'_>,
) -> Outcome {
    debug_assert_eq!(args.len(), native.arity());
    let arg = |i: usize| args[i];
    match native {
        Native::IntegerAdd => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::add, "addFromInteger:")
        }
        Native::IntegerSubtract => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::sub, "subtractFromInteger:")
        }
        Native::IntegerMultiply => {
            integer_arithmetic(ctx, receiver, arg(0), BigInt::mul, "multiplyFromInteger:")
        }
        Native::IntegerLess => comparison(ctx, receiver, arg(0), Ordering::is_lt, ">"),
        Native::IntegerGreater => comparison(ctx, receiver, arg(0), Ordering::is_gt, "<"),
        Native::IntegerLessEqual => comparison(ctx, receiver, arg(0), Ordering::is_le, ">="),
        Native::IntegerGreaterEqual => comparison(ctx, receiver, arg(0), Ordering::is_ge, "<="),
        Native::NumberEqual => {
            if let (Some(x), Some(y)) = (receiver.as_small_integer(), arg(0).as_small_integer()) {
                return Outcome::Answer(ctx.boolean(x == y));
            }
            let Some(a) = number_of(ctx.heap, receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            let equal = number_of(ctx.heap, arg(0))
                .is_some_and(|b| compare(&a, &b) == Some(Ordering::Equal));
            Outcome::Answer(ctx.boolean(equal))
        }
        Native::NumberAsString => match number_of(ctx.heap, receiver) {
            Some(Num::Int(n)) => Outcome::Answer(ctx.new_string(&n.to_decimal())),
            Some(Num::Float(f)) => Outcome::Answer(ctx.new_string(&float_to_string(f))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::NumberFloorDivide => division(ctx, receiver, arg(0), Division::FloorDivide),
        Native::NumberFloorModulo => division(ctx, receiver, arg(0), Division::FloorModulo),
        Native::NumberQuo => division(ctx, receiver, arg(0), Division::Quo),
        Native::NumberRem => division(ctx, receiver, arg(0), Division::Rem),
        Native::IntegerBitAnd => integer_bits(ctx, receiver, arg(0), BigInt::and, |x, y| x & y),
        Native::IntegerBitOr => integer_bits(ctx, receiver, arg(0), BigInt::or, |x, y| x | y),
        Native::IntegerBitXor => integer_bits(ctx, receiver, arg(0), BigInt::xor, |x, y| x ^ y),
        Native::IntegerShiftLeft => shift(ctx, receiver, arg(0), true),
        Native::IntegerShiftRight => shift(ctx, receiver, arg(0), false),
        Native::NumberAsFloat => match number_of(ctx.heap, receiver) {
            Some(n) => Outcome::Answer(ctx.float(as_f64(&n))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::NumberAsInteger => match number_of(ctx.heap, receiver) {
            Some(Num::Int(_)) => Outcome::Answer(receiver),
            Some(Num::Float(_)) => float_to_integer(ctx, receiver, libm::trunc),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::ObjectIdentical => Outcome::Answer(ctx.boolean(receiver == arg(0))),
        Native::ObjectClass => Outcome::Answer(match ctx.heap.get(receiver) {
            Some(o) => o.header.class,
            None => ctx.roles.get(Role::SmallIntegerClass),
        }),
        Native::ObjectIdentityHash => match ctx.heap.get(receiver) {
            // eq:kernel-identity-hash with h the identity on SmallIntegers.
            Some(o) => Outcome::Answer(small((o.header.seq & ((1 << 62) - 1)) as i64)),
            None => Outcome::Answer(receiver),
        },
        Native::StringConcatenate => {
            let (Some(a), Some(b)) = (
                ctx.heap.string_bytes(receiver),
                ctx.heap.string_bytes(arg(0)),
            ) else {
                return Outcome::Fail(Reason::WrongKind(
                    if ctx.heap.string_bytes(receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let mut joined = a.to_vec();
            joined.extend_from_slice(b);
            Outcome::Answer(ctx.new_bytes_object(Role::StringClass, Body::String(joined)))
        }
        Native::StringSize => match ctx.heap.string_bytes(receiver) {
            Some(b) => Outcome::Answer(small(b.len() as i64)),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::VmPrint => match ctx.heap.string_bytes(arg(0)) {
            Some(b) => {
                let text = String::from_utf8_lossy(b).into_owned();
                ctx.host.print(&text);
                Outcome::Answer(receiver)
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmExit => match arg(0)
            .as_small_integer()
            .and_then(|n| i32::try_from(n).ok())
        {
            Some(code) => Outcome::Exit(code),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureValue
        | Native::ClosureValue1
        | Native::ClosureValue2
        | Native::ClosureValue3 => match ctx.heap.closure(receiver) {
            Some(c) if c.num_args as usize == args.len() => Outcome::ActivateReceiver,
            Some(_) => Outcome::Fail(Reason::Arity),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::ClosureValueWithArguments => match ctx.heap.array_elements(arg(0)) {
            Some(elements) => {
                let elements = elements.to_vec();
                activate_closure(receiver, elements, ctx)
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureNumArgs => match ctx.heap.closure(receiver) {
            Some(c) => Outcome::Answer(small(i64::from(c.num_args))),
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::FloatAdd => float_arithmetic(ctx, receiver, arg(0), |a, b| a + b, "addFromFloat:"),
        Native::FloatSubtract => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a - b, "subtractFromFloat:")
        }
        Native::FloatMultiply => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a * b, "multiplyFromFloat:")
        }
        Native::FloatDivide => {
            float_arithmetic(ctx, receiver, arg(0), |a, b| a / b, "divideFromNumber:")
        }
        Native::FloatFloor => float_to_integer(ctx, receiver, libm::floor),
        Native::FloatCeiling => float_to_integer(ctx, receiver, libm::ceil),
        Native::FloatRounded => float_to_integer(ctx, receiver, libm::round),
        Native::FloatSqrt => float_unary(ctx, receiver, libm::sqrt),
        Native::FloatSin => float_unary(ctx, receiver, libm::sin),
        Native::FloatCos => float_unary(ctx, receiver, libm::cos),
        Native::FloatTan => float_unary(ctx, receiver, libm::tan),
        Native::FloatExp => float_unary(ctx, receiver, libm::exp),
        Native::FloatLn => float_unary(ctx, receiver, libm::log),
        Native::FloatLog => float_unary(ctx, receiver, libm::log10),
        Native::FloatArcSin => float_unary(ctx, receiver, libm::asin),
        Native::FloatArcCos => float_unary(ctx, receiver, libm::acos),
        Native::FloatArcTan => float_unary(ctx, receiver, libm::atan),
        Native::FloatPow | Native::FloatArcTan2 => {
            let Some(x) = float_of(ctx.heap, receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            let Some(y) = number_of(ctx.heap, arg(0)).map(|n| as_f64(&n)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let r = if native == Native::FloatPow {
                libm::pow(x, y)
            } else {
                libm::atan2(x, y)
            };
            Outcome::Answer(ctx.float(r))
        }
        Native::FloatAsStringFixed => float_format(ctx, receiver, arg(0), 0..=20, true, |x, d| {
            format!("{x:.d$}")
        }),
        Native::FloatAsStringExponential => {
            float_format(ctx, receiver, arg(0), 0..=20, false, |x, d| {
                signed_exponent(format!("{x:.d$e}"))
            })
        }
        Native::FloatAsStringPrecision => {
            float_format(ctx, receiver, arg(0), 1..=21, false, |x, d| {
                let d = d.max(1);
                let e = format!("{x:.p$e}", p = d - 1);
                let exponent: i32 = e
                    .rsplit('e')
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if exponent < -6 || exponent >= d as i32 {
                    signed_exponent(e)
                } else {
                    let decimals = (d as i32 - 1 - exponent).max(0) as usize;
                    format!("{x:.decimals$}")
                }
            })
        }
        Native::FloatParse => {
            let Some(b) = ctx.heap.string_bytes(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match core::str::from_utf8(b)
                .ok()
                .and_then(|s| parse_float(s.trim()))
            {
                Some(f) => Outcome::Answer(ctx.float(f)),
                None => Outcome::Fail(Reason::WrongKind(1)),
            }
        }
        Native::ClassBasicNew => {
            let special = [
                Role::ArrayClass,
                Role::WeakArrayClass,
                Role::ByteArrayClass,
                Role::StringClass,
                Role::FloatClass,
                Role::LargeIntegerClass,
                Role::SmallIntegerClass,
                Role::ClosureClass,
                Role::ActivationClass,
                Role::EphemeronClass,
            ];
            if receiver == ctx.roles.get(Role::EphemeronClass) {
                let nil = ctx.nil();
                return Outcome::Answer(ctx.heap.allocate(
                    receiver,
                    ctx.space,
                    Body::Ephemeron {
                        key: nil,
                        value: nil,
                        finalizer: nil,
                    },
                ));
            }
            if special.iter().any(|&r| ctx.roles.get(r) == receiver) {
                return Outcome::Fail(Reason::NotInstantiable);
            }
            // Program objects built in Newspeak (M6 design 4.1): real
            // methods and mixins, unfrozen until installed.
            let nil = ctx.nil();
            if receiver == ctx.roles.get(Role::MethodClass) && receiver != nil {
                return Outcome::Answer(ctx.heap.allocate(
                    receiver,
                    ctx.space,
                    Body::Method(Method {
                        header: MethodHeader {
                            kind: MethodKind::Ordinary,
                            access: Access::Public,
                            arg_count: 0,
                            local_count: 0,
                            native: 0,
                            slot: 0,
                        },
                        bytecode: alloc::rc::Rc::from([]),
                        literals: nil,
                        mixin: nil,
                        selector: nil,
                        source: nil,
                        library: nil,
                    }),
                ));
            }
            if (receiver == ctx.roles.get(Role::InstanceMixinClass)
                || receiver == ctx.roles.get(Role::ClassMixinClass))
                && receiver != nil
            {
                return Outcome::Answer(ctx.heap.allocate(
                    receiver,
                    ctx.space,
                    Body::Mixin(Mixin {
                        name: nil,
                        methods: nil,
                        enclosing_mixin: nil,
                        slot_names: nil,
                        slot_count: 0,
                        class_mixin: nil,
                        library: nil,
                    }),
                ));
            }
            match instance_slots(ctx.heap, ctx.roles, receiver) {
                Some(n) => {
                    let nil = ctx.nil();
                    let instance =
                        ctx.heap
                            .allocate(receiver, ctx.space, Body::Regular(alloc::vec![nil; n]));
                    // An instance of a value class is a value from allocation;
                    // its init setters check what it is given (the end of
                    // construction is not marked yet: design doc section 5).
                    if ctx
                        .heap
                        .get(receiver)
                        .map(|o| o.header.flags & VALUE_CLASS != 0)
                        == Some(true)
                    {
                        ctx.heap.get_mut(instance).expect("new").header.flags |= VALUE;
                    }
                    Outcome::Answer(instance)
                }
                None => Outcome::Fail(Reason::NotInstantiable),
            }
        }
        Native::IndexedNew => {
            let Some(n) = small_index(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            if !(0..=(1 << 28)).contains(&n) {
                return Outcome::Fail(if n < 0 {
                    Reason::OutOfRange(1)
                } else {
                    Reason::Exhausted(Resource::Memory)
                });
            }
            let n = n as usize;
            let nil = ctx.nil();
            let body = if receiver == ctx.roles.get(Role::ArrayClass) {
                Body::Array(alloc::vec![nil; n])
            } else if receiver == ctx.roles.get(Role::WeakArrayClass) {
                Body::WeakArray(alloc::vec![nil; n])
            } else if receiver == ctx.roles.get(Role::ByteArrayClass) {
                Body::Bytes(alloc::vec![0; n])
            } else {
                return Outcome::Fail(Reason::NotInstantiable);
            };
            Outcome::Answer(ctx.heap.allocate(receiver, ctx.space, body))
        }
        Native::IndexedAt => {
            let result = match ctx.heap.get(receiver).map(|o| &o.body) {
                Some(Body::Array(e)) | Some(Body::WeakArray(e)) => {
                    index(arg(0), e.len()).map(|i| e[i])
                }
                Some(Body::Bytes(b)) | Some(Body::String(b)) => {
                    index(arg(0), b.len()).map(|i| small(i64::from(b[i])))
                }
                _ => Err(Reason::WrongKind(0)),
            };
            match result {
                Ok(v) => Outcome::Answer(v),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::IndexedAtPut => {
            let value = arg(1);
            let Some(o) = ctx.heap.get_mut(receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            if o.header.flags & FROZEN != 0 {
                return Outcome::Fail(Reason::WrongKind(0));
            }
            let result = match &mut o.body {
                Body::Array(e) | Body::WeakArray(e) => index(arg(0), e.len()).map(|i| e[i] = value),
                Body::Bytes(b) => match value.as_small_integer() {
                    Some(byte @ 0..=255) => index(arg(0), b.len()).map(|i| b[i] = byte as u8),
                    Some(_) => Err(Reason::OutOfRange(2)),
                    None => Err(Reason::WrongKind(2)),
                },
                _ => Err(Reason::WrongKind(0)),
            };
            match result {
                Ok(()) => Outcome::Answer(value),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::IndexedSize => match ctx.heap.get(receiver).map(|o| &o.body) {
            Some(Body::Array(e)) | Some(Body::WeakArray(e)) => {
                Outcome::Answer(small(e.len() as i64))
            }
            Some(Body::Bytes(b)) | Some(Body::String(b)) => Outcome::Answer(small(b.len() as i64)),
            _ => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::CopyFromTo | Native::CopyStringFromTo | Native::CopyByteArrayFromTo => {
            let class = ctx.heap.get(receiver).map(|o| o.header.class);
            let copy = match ctx.heap.get(receiver).map(|o| &o.body) {
                Some(Body::Array(e)) if native == Native::CopyFromTo => {
                    range(arg(0), arg(1), e.len(), 1).map(|r| Body::Array(e[r].to_vec()))
                }
                Some(Body::Bytes(b)) | Some(Body::String(b)) => range(arg(0), arg(1), b.len(), 1)
                    .map(|r| {
                        let bytes = b[r].to_vec();
                        match native {
                            Native::CopyStringFromTo => Body::String(bytes),
                            Native::CopyByteArrayFromTo => Body::Bytes(bytes),
                            _ => match ctx.heap.get(receiver).map(|o| &o.body) {
                                Some(Body::String(_)) => Body::String(bytes),
                                _ => Body::Bytes(bytes),
                            },
                        }
                    }),
                _ => Err(Reason::WrongKind(0)),
            };
            match copy {
                Ok(body) => {
                    let class = match (&body, native) {
                        (Body::String(_), Native::CopyStringFromTo) => {
                            ctx.roles.get(Role::StringClass)
                        }
                        (Body::Bytes(_), Native::CopyByteArrayFromTo) => {
                            ctx.roles.get(Role::ByteArrayClass)
                        }
                        _ => class.expect("receiver"),
                    };
                    Outcome::Answer(ctx.heap.allocate(class, ctx.space, body))
                }
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::ReplaceFromToWithStartingAt => {
            let (start, stop, source, source_start) = (arg(0), arg(1), arg(2), arg(3));
            enum Src {
                Values(Vec<Value>),
                Bytes(Vec<u8>),
            }
            let src = match ctx.heap.get(source).map(|o| &o.body) {
                Some(Body::Array(e)) | Some(Body::WeakArray(e)) => Src::Values(e.clone()),
                Some(Body::Bytes(b)) | Some(Body::String(b)) => Src::Bytes(b.clone()),
                _ => return Outcome::Fail(Reason::WrongKind(3)),
            };
            let Some(o) = ctx.heap.get_mut(receiver) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            if o.header.flags & FROZEN != 0 {
                return Outcome::Fail(Reason::WrongKind(0));
            }
            let src_len = match &src {
                Src::Values(v) => v.len(),
                Src::Bytes(b) => b.len(),
            };
            let result = (|| -> Result<(), Reason> {
                let len = match &o.body {
                    Body::Array(e) | Body::WeakArray(e) => e.len(),
                    Body::Bytes(b) => b.len(),
                    _ => return Err(Reason::WrongKind(0)),
                };
                let target = range(start, stop, len, 1)?;
                let Some(s0) = small_index(source_start) else {
                    return Err(Reason::WrongKind(4));
                };
                if s0 < 1 || (s0 as u64 - 1) + target.len() as u64 > src_len as u64 {
                    return Err(Reason::OutOfRange(4));
                }
                let s0 = s0 as usize - 1;
                match (&mut o.body, &src) {
                    (Body::Array(e) | Body::WeakArray(e), Src::Values(v)) => {
                        e[target.clone()].copy_from_slice(&v[s0..s0 + target.len()]);
                    }
                    (Body::Bytes(b), Src::Bytes(s)) => {
                        b[target.clone()].copy_from_slice(&s[s0..s0 + target.len()]);
                    }
                    _ => return Err(Reason::WrongKind(3)),
                }
                Ok(())
            })();
            match result {
                Ok(()) => Outcome::Answer(receiver),
                Err(r) => Outcome::Fail(r),
            }
        }
        Native::ByteArrayWithAll | Native::StringWithAll => match bytes_of(ctx.heap, arg(0)) {
            Some(b) => {
                let b = b.to_vec();
                Outcome::Answer(if native == Native::StringWithAll {
                    ctx.new_bytes_object(Role::StringClass, Body::String(b))
                } else {
                    ctx.new_bytes_object(Role::ByteArrayClass, Body::Bytes(b))
                })
            }
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::StringWith => match arg(0).as_small_integer() {
            Some(byte @ 0..=255) => Outcome::Answer(
                ctx.new_bytes_object(Role::StringClass, Body::String(alloc::vec![byte as u8])),
            ),
            Some(_) => Outcome::Fail(Reason::OutOfRange(1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::StringHash => match ctx.heap.string_bytes(receiver) {
            Some(b) => {
                // FNV-1a, 64-bit, folded into the SmallInteger range.
                let mut h: u64 = 0xcbf2_9ce4_8422_2325;
                for &byte in b {
                    h ^= u64::from(byte);
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
                Outcome::Answer(small((h >> 2) as i64))
            }
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::StringEquals => match ctx.heap.string_bytes(receiver) {
            Some(a) => {
                let equal = ctx.heap.string_bytes(arg(0)) == Some(a);
                Outcome::Answer(ctx.boolean(equal))
            }
            None => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::StartsWith | Native::EndsWith => {
            let (Some(a), Some(b)) = (bytes_of(ctx.heap, receiver), bytes_of(ctx.heap, arg(0)))
            else {
                return Outcome::Fail(Reason::WrongKind(
                    if bytes_of(ctx.heap, receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let r = if native == Native::StartsWith {
                a.starts_with(b)
            } else {
                a.ends_with(b)
            };
            Outcome::Answer(ctx.boolean(r))
        }
        Native::IndexOfStartingAt | Native::LastIndexOfStartingAt => {
            let (Some(a), Some(b)) = (bytes_of(ctx.heap, receiver), bytes_of(ctx.heap, arg(0)))
            else {
                return Outcome::Fail(Reason::WrongKind(
                    if bytes_of(ctx.heap, receiver).is_none() {
                        0
                    } else {
                        1
                    },
                ));
            };
            let Some(from) = small_index(arg(1)) else {
                return Outcome::Fail(Reason::WrongKind(2));
            };
            if from < 1 || from > a.len() as i64 + 1 {
                return Outcome::Fail(Reason::OutOfRange(2));
            }
            let i = find(
                a,
                b,
                from as usize - 1,
                native == Native::LastIndexOfStartingAt,
            );
            Outcome::Answer(small(i as i64))
        }
        Native::VmSnapshot => Outcome::Snapshot,
        Native::CurrentActivation
        | Native::ActivationSender
        | Native::ActivationSetSender
        | Native::ActivationMethod
        | Native::ActivationReceiver
        | Native::ActivationClosure
        | Native::ActivationTempAt
        | Native::ActivationTempAtPut
        | Native::ActivationSize
        | Native::ActivationBci
        | Native::ActivationMethodPut
        | Native::ActivationClosurePut
        | Native::ActivationReceiverPut
        | Native::ActivationBciPut
        | Native::ActivationSizePut => Outcome::Frame(native),
        Native::ActivationNew => {
            let class = ctx.roles.get(Role::ActivationClass);
            let nil = ctx.nil();
            let space = ctx.space;
            Outcome::Answer(ctx.heap.allocate(
                class,
                space,
                Body::Activation(crate::object::Activation {
                    frame: None,
                    method: nil,
                    receiver: nil,
                    closure: nil,
                    detached: Some(crate::object::Detached {
                        sender: nil,
                        bci: nil,
                        temps: Vec::new(),
                    }),
                }),
            ))
        }
        Native::VmMarkSimulationRoot => match ctx.heap.get_mut(arg(0)) {
            Some(o) if matches!(o.body, Body::Method(_)) => {
                o.header.flags |= crate::object::SIMULATION_ROOT;
                Outcome::Answer(arg(0))
            }
            _ => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmIsSimulationRoot => Outcome::Answer(
            ctx.boolean(
                ctx.heap
                    .get(arg(0))
                    .is_some_and(|o| o.header.flags & crate::object::SIMULATION_ROOT != 0),
            ),
        ),
        Native::VmContinueAt => Outcome::ContinueAt {
            activation: arg(0),
            value: arg(1),
        },
        Native::VmResumeActivation => Outcome::ResumeActivation(arg(0)),
        Native::VmRunNative => Outcome::RunNative {
            method: arg(0),
            receiver: arg(1),
            arguments: arg(2),
        },
        Native::DefiningActivationOf => match ctx.heap.closure(arg(0)) {
            Some(c) => Outcome::Answer(c.defining),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmAllInstancesOf => Outcome::AllInstances(arg(0)),
        Native::ClosureIn => {
            if ctx.heap.activation(arg(0)).is_none() {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            let (Some(bci), Some(n), Some(c)) = (
                small_index(arg(1)).filter(|&b| b >= 1),
                small_index(arg(2)).filter(|&n| (0..=15).contains(&n)),
                small_index(arg(3)).filter(|&c| (0..=15).contains(&c)),
            ) else {
                return Outcome::Fail(Reason::OutOfRange(2));
            };
            let class = ctx.roles.get(Role::ClosureClass);
            let nil = ctx.nil();
            let space = ctx.space;
            Outcome::Answer(ctx.heap.allocate(
                class,
                space,
                Body::Closure(crate::object::Closure {
                    defining: arg(0),
                    initial_ip: (bci - 1) as u32,
                    num_args: n as u32,
                    copied: alloc::vec![nil; c as usize],
                }),
            ))
        }
        Native::VmCommit => Outcome::Commit(arg(0)),
        Native::VmProgramVersion => Outcome::ProgramVersion,
        Native::VmCheckpointNow => Outcome::CheckpointNow,
        Native::VmSaveImage => match ctx.heap.string_bytes(arg(0)) {
            Some(name) => Outcome::SaveImage(String::from_utf8_lossy(name).into_owned()),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmCheckpointPolicy => match arg(0).as_small_integer() {
            Some(n) if n >= 0 => Outcome::CheckpointPolicy(n as u64),
            Some(_) => Outcome::Fail(Reason::OutOfRange(1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmIncarnation | Native::VmLineage => {
            let role = if native == Native::VmIncarnation {
                Role::Incarnation
            } else {
                Role::Lineage
            };
            match ctx.heap.get(ctx.roles.get(role)).map(|o| &o.body) {
                Some(Body::Bytes(b)) => {
                    let copy = b.clone();
                    Outcome::Answer(ctx.new_bytes_object(Role::ByteArrayClass, Body::Bytes(copy)))
                }
                _ => Outcome::Answer(ctx.roles.get(Role::Nil)),
            }
        }
        Native::VmLogPosition => match ctx.host.log_position() {
            Some(p) => Outcome::Answer(ctx.integer(BigInt::from_i128(i128::from(p)))),
            None => Outcome::Answer(ctx.roles.get(Role::Nil)),
        },
        Native::VmMixinApplicationsOf => Outcome::ClassQuery {
            target: arg(0),
            by_mixin: true,
        },
        Native::VmSubclassesOf => Outcome::ClassQuery {
            target: arg(0),
            by_mixin: false,
        },
        Native::ClosureNumCopied => match ctx.heap.closure(arg(0)) {
            Some(c) => Outcome::Answer(small(c.copied.len() as i64)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureInitialPc => match ctx.heap.closure(arg(0)) {
            Some(c) => Outcome::Answer(small(i64::from(c.initial_ip) + 1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::ClosureCopiedAt => {
            let Some(c) = ctx.heap.closure(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match small_index(arg(1)).filter(|&i| i >= 1 && i as usize <= c.copied.len()) {
                Some(i) => Outcome::Answer(c.copied[i as usize - 1]),
                None => Outcome::Fail(Reason::OutOfRange(2)),
            }
        }
        Native::ClosureCopiedAtPut => {
            let value = arg(2);
            let Some(Body::Closure(c)) = ctx.heap.get_mut(arg(0)).map(|o| &mut o.body) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match small_index(arg(1)).filter(|&i| i >= 1 && i as usize <= c.copied.len()) {
                Some(i) => {
                    c.copied[i as usize - 1] = value;
                    Outcome::Answer(value)
                }
                None => Outcome::Fail(Reason::OutOfRange(2)),
            }
        }
        Native::VmWriteFile => {
            let (Some(bytes), Some(path)) =
                (bytes_of(ctx.heap, arg(0)), ctx.heap.string_bytes(arg(1)))
            else {
                return Outcome::Fail(Reason::WrongKind(if bytes_of(ctx.heap, arg(0)).is_none() {
                    1
                } else {
                    2
                }));
            };
            let (bytes, path) = (bytes.to_vec(), String::from_utf8_lossy(path).into_owned());
            match ctx.host.write_file(&path, &bytes) {
                Ok(()) => Outcome::Answer(receiver),
                Err(message) => {
                    let m = ctx.new_string(&message);
                    Outcome::Fail(Reason::Host(m))
                }
            }
        }
        Native::VmReadFile => {
            let Some(path) = ctx.heap.string_bytes(arg(0)) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let path = String::from_utf8_lossy(path).into_owned();
            match ctx.host.read_file(&path) {
                Ok(bytes) => {
                    Outcome::Answer(ctx.new_bytes_object(Role::ByteArrayClass, Body::Bytes(bytes)))
                }
                Err(message) => {
                    let m = ctx.new_string(&message);
                    Outcome::Fail(Reason::Host(m))
                }
            }
        }
        Native::VmSetRole => {
            let role = arg(0)
                .as_small_integer()
                .and_then(|id| u64::try_from(id).ok())
                .and_then(Role::from_id);
            match role {
                Some(role) => Outcome::SetRole {
                    role,
                    value: arg(1),
                },
                None => Outcome::Fail(Reason::OutOfRange(1)),
            }
        }
        Native::VmSlotOfAt => {
            let (object, i) = (arg(0), arg(1));
            match ctx.heap.get(object).and_then(|o| readable_fields(&o.body)) {
                Some(fields) => match index(i, fields.len()) {
                    Ok(k) => Outcome::Answer(fields[k]),
                    Err(Reason::WrongKind(_)) => Outcome::Fail(Reason::WrongKind(2)),
                    Err(_) => Outcome::Fail(Reason::OutOfRange(2)),
                },
                None => Outcome::Fail(Reason::WrongKind(1)),
            }
        }
        Native::VmSlotOfAtPut => {
            let (object, i, value) = (arg(0), arg(1), arg(2));
            let Some(o) = ctx.heap.get_mut(object) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            if o.header.flags & (FROZEN | VALUE) != 0 {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            let slot = match &mut o.body {
                Body::Regular(slots) => index(i, slots.len()).map(|k| &mut slots[k]),
                Body::Ephemeron {
                    key,
                    value: v,
                    finalizer,
                } => match i.as_small_integer() {
                    Some(1) => Ok(key),
                    Some(2) => Ok(v),
                    Some(3) => Ok(finalizer),
                    Some(_) => Err(Reason::OutOfRange(2)),
                    None => Err(Reason::WrongKind(2)),
                },
                _ => Err(Reason::WrongKind(1)),
            };
            match slot {
                Ok(s) => {
                    *s = value;
                    Outcome::Answer(value)
                }
                Err(Reason::WrongKind(1)) => Outcome::Fail(Reason::WrongKind(1)),
                Err(Reason::WrongKind(_)) => Outcome::Fail(Reason::WrongKind(2)),
                Err(_) => Outcome::Fail(Reason::OutOfRange(2)),
            }
        }
        Native::VmClassOf => Outcome::Answer(match ctx.heap.get(arg(0)) {
            Some(o) => o.header.class,
            None => ctx.roles.get(Role::SmallIntegerClass),
        }),
        Native::VmIsIdenticalTo => Outcome::Answer(ctx.boolean(arg(0) == arg(1))),
        Native::VmIdentityHashOf => match ctx.heap.get(arg(0)) {
            Some(o) => Outcome::Answer(small((o.header.seq & ((1 << 62) - 1)) as i64)),
            None => Outcome::Answer(arg(0)),
        },
        Native::VmInstantiate => invoke(Native::ClassBasicNew, arg(0), &[], ctx),
        Native::VmApplyMixin => apply_mixin(arg(0), arg(1), arg(2), ctx),
        Native::VmIsCanonical => {
            let frozen = match ctx.heap.get(arg(0)) {
                Some(o) => o.header.flags & FROZEN != 0,
                // Immediates are values.
                None => true,
            };
            Outcome::Answer(ctx.boolean(frozen))
        }
        Native::VmSetCanonical => {
            if let Some(o) = ctx.heap.get_mut(arg(0)) {
                o.header.flags |= FROZEN;
            }
            Outcome::Answer(arg(0))
        }
        Native::VmPerformWith => {
            match invoke(
                Native::ObjectPerformWithArguments,
                arg(0),
                &[arg(1), arg(2)],
                ctx,
            ) {
                // Shift argument numbers: the object is argument 1 here.
                Outcome::Fail(Reason::WrongKind(i)) => Outcome::Fail(Reason::WrongKind(i + 1)),
                other => other,
            }
        }
        Native::VmGarbageCollect => Outcome::Collect,
        Native::VmIsValue => Outcome::Answer(ctx.boolean(ctx.is_value(arg(0)))),
        Native::VmClone => {
            let space = ctx.space;
            Outcome::Answer(ctx.heap.clone_graph(arg(0), space))
        }
        Native::VmNewActor => Outcome::NewActor,
        Native::VmSpawn => Outcome::Spawn(arg(0)),
        Native::MethodHeader => match ctx.heap.get(receiver).map(|o| &o.body) {
            Some(Body::Method(m)) => Outcome::Answer(small(psoup_header(&m.header))),
            _ => Outcome::Fail(Reason::WrongKind(0)),
        },
        Native::MethodHeaderPut => {
            let Some(word) = arg(0).as_small_integer() else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let Some(header) = header_from_psoup(word) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            match unfrozen_method(ctx, receiver) {
                Some(m) => {
                    m.header = header;
                    Outcome::Answer(receiver)
                }
                None => Outcome::Fail(Reason::WrongKind(0)),
            }
        }
        Native::MethodBytecode => {
            let Some(Body::Method(m)) = ctx.heap.get(receiver).map(|o| &o.body) else {
                return Outcome::Fail(Reason::WrongKind(0));
            };
            let bytes = m.bytecode.to_vec();
            let class = ctx.roles.get(Role::ByteArrayClass);
            let space = ctx.space;
            Outcome::Answer(ctx.heap.allocate(class, space, Body::Bytes(bytes)))
        }
        Native::MethodBytecodePut => {
            let Some(Body::Bytes(bytes)) = ctx.heap.get(arg(0)).map(|o| &o.body) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let bytes = bytes.clone();
            if !well_formed(&bytes) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            match unfrozen_method(ctx, receiver) {
                Some(m) => {
                    m.bytecode = bytes.into();
                    Outcome::Answer(receiver)
                }
                None => Outcome::Fail(Reason::WrongKind(0)),
            }
        }
        Native::VmHostGlobal => Outcome::Answer(new_alien(ctx, HOST_GLOBAL)),
        Native::AlienAt => host_call(ctx, HostOp::Get, receiver, &[arg(0)], None, None),
        Native::AlienAtPut => host_call(
            ctx,
            HostOp::Set,
            receiver,
            &[arg(0), arg(1)],
            None,
            Some(arg(1)),
        ),
        Native::AlienRemoveKey => host_call(ctx, HostOp::Delete, receiver, &[arg(0)], None, None),
        Native::AlienIncludesKey => host_call(ctx, HostOp::Has, receiver, &[arg(0)], None, None),
        Native::AlienCall => host_call(
            ctx,
            HostOp::Call,
            receiver,
            &[arg(0)],
            Some((arg(1), 2)),
            None,
        ),
        Native::AlienApply => host_call(ctx, HostOp::Apply, receiver, &[], Some((arg(0), 1)), None),
        Native::AlienNew => host_call(ctx, HostOp::New, receiver, &[], Some((arg(0), 1)), None),
        Native::AlienInstanceOf => {
            host_call(ctx, HostOp::InstanceOf, receiver, &[arg(0)], None, None)
        }
        Native::VmFarReferenceTo => {
            let Some(Body::FarRef { actor, .. }) = ctx.heap.get(arg(0)).map(|o| &o.body) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let actor = *actor;
            if !ctx.is_value(arg(1)) {
                return Outcome::Fail(Reason::WrongKind(2));
            }
            Outcome::Answer(new_far_reference(ctx, actor, arg(1)))
        }
        Native::VmPost => {
            if !matches!(ctx.heap.get(arg(0)).map(|o| &o.body), Some(Body::Array(_))) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            if !matches!(
                ctx.heap.get(arg(1)).map(|o| &o.body),
                Some(Body::FarRef { .. })
            ) {
                return Outcome::Fail(Reason::WrongKind(2));
            }
            Outcome::Post {
                packet: arg(0),
                far_reference: arg(1),
            }
        }
        Native::VmIsFarReference => Outcome::Answer(ctx.boolean(matches!(
            ctx.heap.get(arg(0)).map(|o| &o.body),
            Some(Body::FarRef { .. })
        ))),
        Native::VmCurrentMessageLoop => Outcome::CurrentMessageLoop,
        Native::VmNewPromise => {
            let Space::Actor(current) = ctx.space else {
                return Outcome::Fail(Reason::Unsupported);
            };
            let owner = new_far_reference(ctx, current, arg(0));
            let class = ctx.roles.get(Role::FarReferenceClass);
            let space = ctx.space;
            Outcome::Answer(ctx.heap.allocate(
                class,
                space,
                Body::Promise(PromiseRecord {
                    state: PromiseRecord::PENDING,
                    value: ctx.nil(),
                    owner,
                    waiters: Vec::new(),
                    queue: Vec::new(),
                }),
            ))
        }
        Native::VmResolvePromise | Native::VmBreakPromise => {
            if !is_promise(ctx.heap, arg(0)) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            Outcome::Settle {
                record: arg(0),
                state: if native == Native::VmResolvePromise {
                    PromiseRecord::FULFILLED
                } else {
                    PromiseRecord::BROKEN
                },
                value: arg(1),
            }
        }
        Native::VmPromiseState => {
            if !is_promise(ctx.heap, arg(0)) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            let Space::Actor(current) = ctx.space else {
                return Outcome::Fail(Reason::Unsupported);
            };
            let last = last_promise(ctx.heap, arg(0));
            let Some(Body::Promise(p)) = ctx.heap.get(last).map(|o| &o.body) else {
                unreachable!("a chain of promise records ends in one")
            };
            let (state, value, owner) = (p.state, p.value, p.owner);
            let payload = if state == PromiseRecord::PENDING {
                match ctx.heap.get(owner).map(|o| &o.body) {
                    Some(Body::FarRef { actor, target }) if *actor == current => *target,
                    _ => last,
                }
            } else {
                localize(ctx.heap, value, current)
            };
            let array_class = ctx.roles.get(Role::ArrayClass);
            let space = ctx.space;
            let state = Value::small_integer(state.into()).expect("small");
            Outcome::Answer(ctx.heap.allocate(
                array_class,
                space,
                Body::Array(alloc::vec![state, payload]),
            ))
        }
        Native::VmPromiseAddWaiter => {
            if !is_promise(ctx.heap, arg(0)) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            Outcome::AddWaiter {
                record: arg(0),
                waiter: arg(1),
            }
        }
        Native::VmIsPromise => Outcome::Answer(ctx.boolean(is_promise(ctx.heap, arg(0)))),
        Native::VmPromiseSend => {
            if !is_promise(ctx.heap, arg(0)) {
                return Outcome::Fail(Reason::WrongKind(1));
            }
            match ctx.heap.get(arg(1)).map(|o| &o.body) {
                Some(Body::Array(e)) if e.len() >= 2 => Outcome::PromiseSend {
                    record: arg(0),
                    message: arg(1),
                },
                _ => Outcome::Fail(Reason::WrongKind(2)),
            }
        }
        // Never installed: the interpreter reports refused slot stores with it.
        Native::SlotStore => Outcome::Fail(Reason::WrongKind(0)),
        Native::VmFinishTurn => match arg(0).as_small_integer() {
            Some(t) if t >= 0 => Outcome::FinishTurn(t as u64),
            Some(_) => Outcome::Fail(Reason::OutOfRange(1)),
            None => Outcome::Fail(Reason::WrongKind(1)),
        },
        Native::VmMonotonicNanos => {
            let t = ctx.host.monotonic_nanos();
            Outcome::Answer(ctx.integer(BigInt::from_i128(i128::from(t))))
        }
        Native::ObjectPerformWithArguments => {
            let selector = arg(0);
            let Some(sel) = ctx.heap.string_bytes(selector) else {
                return Outcome::Fail(Reason::WrongKind(1));
            };
            let arity = selector_arity(sel);
            let Some(arguments) = ctx.heap.array_elements(arg(1)) else {
                return Outcome::Fail(Reason::WrongKind(2));
            };
            if arguments.len() != arity {
                return Outcome::Fail(Reason::Arity);
            }
            Outcome::Perform {
                receiver,
                selector,
                args: arguments.to_vec(),
            }
        }
    }
}

/// A description of every catalog entry, for documentation and the
/// packager's natives table: (id, name, arity).
pub fn catalog() -> Vec<(u32, &'static str, usize)> {
    Native::ALL
        .iter()
        .map(|&n| (n as u32, n.name(), n.arity()))
        .collect()
}

/// The decimal string of an integer value, if it is one (for tests and
/// diagnostics).
pub fn integer_to_string(heap: &Heap, v: Value) -> Option<String> {
    integer_of(heap, v).map(|n| n.to_decimal())
}
