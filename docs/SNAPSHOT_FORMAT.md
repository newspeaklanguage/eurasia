# Eurasia snapshot format

**Status:** Draft, format version 7. Written before the code (implementation
plan M0); the packager (Newspeak) and `eurasia-snapshot` (Rust) are tested
against each other.

Eurasia snapshots replace psoup's vfuels. The structure follows psoup's
PrimordialFuel (a graph written as nodes, then edges), with one essential
difference: **every object is written with its allocation sequence number**,
and objects are written in sequence order within each section. Allocation order is the identity order of the formal
semantics, and an object's sequence number is its identity hash
(`kernel-semantics.tex`, equations `eq:kernel-fresh` and
`eq:kernel-identity-hash`), so both must survive a save and restore exactly.
psoup groups objects by class instead, which loses that order.

## 1. Encoding conventions

- `u8`, `u16`, `u32`: fixed width, **little-endian**.
- `uv`: unsigned LEB128 (7 bits per byte, low group first, high bit set on
  every byte but the last). At most 10 bytes.
- `sv`: signed LEB128 of the zigzag encoding (`(n << 1) ^ (n >> 63)`).
- `bytes(n)`: `n` raw bytes.
- A **value** is a `uv` holding the 64-bit tagged word of the VM
  (`eurasia-core::Value`): bit 0 = 0 is a SmallInteger shifted left by one;
  bit 0 = 1 is a reference, `ref << 1 | 1`, where `ref` is a **node number**
  (section 3), not a memory address. SmallIntegers are written zigzagged
  first, so small negative numbers stay short: the written `uv` is
  `zigzag(n) << 1` for a SmallInteger `n`.

## 2. Header

| Field | Type | Meaning |
|---|---|---|
| magic | `bytes(4)` | `EURA` (0x45 0x55 0x52 0x41) |
| format version | `u16` | This document: 7 (2026-11-17: version 2 gave mixins the class-side mixin field, version 3 the value flag bits and the Value role; 2026-11-18: version 4 far references, promise records, the shared bit and roles 35–37; version 5 the simulation-root bit and roles 38–39; version 6 checkpoints: the machine section, live and detached activations, and role 40; version 7 roles 41 and 42, incarnation and lineage) |
| kernel version | `u16` | Must equal the VM's `KERNEL_VERSION` |
| bytecode version | `u16` | Must equal the VM's `BYTECODE_VERSION` |
| word size | `u8` | Always 64 |
| flags | `u8` | Reserved, 0 |
| allocation counter | `uv` | `n_α`: the next sequence number to allocate |
| node count | `uv` | Number of nodes in the file |
| section count | `uv` | Number of sections |

A VM refuses a snapshot whose magic, format version, kernel version,
bytecode version or word size differs from its own, and says which. (psoup
dies with "unimplemented code" when given a snapshot that is too new.)

An optional `#!` interpreter line before the magic is skipped, as in psoup.

## 3. Nodes and references

Nodes are numbered from 1 in file order across all sections; 0 is not a
node. A value's reference field names a node number. Within a section,
sequence numbers strictly increase. Sections hold different spaces, whose
objects interleave in allocation order, so the loader orders all objects by
sequence number; sequence numbers are unique across the file.

## 4. Sections

Each section is a kind, then its nodes, then its edges:

| Field | Type |
|---|---|
| section kind | `u8` |
| owner | `uv` (0 for the program section; the actor number for a heap) |
| node count | `uv` |
| nodes | node records |
| edges | edge records, one per node, in the same order |

Section kinds:

| Kind | Name | Contents |
|---|---|---|
| 1 | roles | No nodes. Edges: `uv` count, then pairs (`uv` role id, value). Role ids are fixed by the kernel version (section 7). Must be the last section. |
| 2 | program | Installed methods and mixins: frozen, shared by every actor of the cohort. |
| 3 | actor heap | One actor's objects, including its class objects. |
| 4 | machine | A checkpoint's machine state (M7, below). No nodes; its body follows the node count. At most one, before the roles section. |

A file with a machine section is a **checkpoint**: restoring it continues
where it was taken (`Vm::restore`, then `Vm::continue_run`). A file without
one is a **snapshot**, which starts afresh (`vm snapshot`, genesis). The
machine section's body, in order (values are node references, as in
edges; a flag is a `u8` 0 or 1, and an optional value is a flag then, if
1, the value):

- the operand stack: `uv` length, then the values;
- the frames, oldest first: `uv` count, then for each its serial number
  (`uv`), method, receiver, operand-stack indices of its message receiver,
  argument count, first local and next instruction (four `uv`), its
  activation handle (optional) and closure (optional);
- the actors: `uv` count, then for each its message loop, mailbox (`uv`
  length, then the packets), next timer (`uv` monotonic nanoseconds, 0 for
  none) and whether an idle callback poked it (flag);
- the running actor (`uv`);
- the expats: `uv` count, then each closure and its owning actor (`uv`);
- the VM's selector strings: `uv` count, then each name (`uv` length and
  UTF-8 bytes) and its String;
- seven `uv` counters: the next frame serial number, the allocation counter
  after the last collection, the collection budget, the collections so far,
  the program version, the checkpoint policy (allocations, 0 for none) and
  the allocation counter at the last checkpoint;
- the timer a turn ended with (optional `uv`), and whether the image runs a
  message loop (flag).

A checkpoint's heap holds everything the VM's roots reach weakly or
strongly, so that weak references clear at the same collections in a
restored VM as in the running one (M7 design, section 3.2).

Two-phase loading, as in psoup: all nodes of all sections are allocated
first (their sizes are in the node records), then edges are filled, so
cycles need no special handling.

## 5. Node records

| Field | Type | Meaning |
|---|---|---|
| sequence delta | `uv` | Sequence number minus the previous node's in the same section (the section's first node's is absolute). Gaps are objects in other spaces, or collected before the save. |
| kind | `u8` | Below |
| flags | `u8` | Bit 0: frozen (installed or canonical). Bit 1: a value (deeply immutable; actors may share it). Bit 2: on a class, a value class (its instances are values). Bit 3: shared by every actor though mutable (the kernel module, the VM mirror): cloning a platform does not copy it. Bit 4: on a method, a simulation root (psoup's primitive 142): a non-local return stops at it, and an exception in a simulated thread is routed to it. Others reserved. |
| shape | depends on kind | Below |

| Kind | Name | Shape in the node record | Edge record |
|---|---|---|---|
| 1 | regular | `uv` slot count | class value, then one value per slot |
| 2 | bytes | `uv` length, `bytes(length)` | class value |
| 3 | string | `uv` length, `bytes(length)` (UTF-8) | class value |
| 4 | array | `uv` length | class value, then the elements |
| 5 | weak array | `uv` length | class value, then the elements |
| 6 | ephemeron | none | class value, key, value, finalizer |
| 7 | float | `bytes(8)`, IEEE 754 binary64, little-endian, NaN canonical | class value |
| 8 | large integer | `u8` sign (0 or 1), `uv` length, `bytes(length)` magnitude, little-endian | class value |
| 9 | method | header (section 6), `uv` bytecode length, `bytes(length)` | class value, literals array, mixin, selector, source, library field |
| 10 | mixin | `uv` slot-declaration count | class value, name, methods array, enclosing mixin, slot-declaration names array, class-side mixin (nil for a class-side mixin), library field |
| 11 | class | none | class value (its metaclass), mixin, superclass, enclosing object, name, library field |
| 12 | metaclass | none | class value, mixin, superclass, enclosing object, this class, library field |
| 13 | closure | `uv` initial bytecode index, `uv` argument count, `uv` copied count | class value, defining activation, then the copied values |
| 14 | activation | `u8` state: 0 dead, 1 a live frame's handle (then `uv` the frame's serial number), 2 detached (then `uv` its temp count) | class value, method, receiver, closure (nil for a method activation); a detached one's sender, bci and temps follow |
| 15 | far reference | `uv` actor number | class value, target (in that actor's heap) |
| 16 | promise record | `uv` state (0 pending, 1 fulfilled, 2 broken, 3 forwarded), `uv` waiter count, `uv` queue length | class value, value (resolution, problem or record forwarded to; nil while pending), owner (far reference or nil), the waiters (far references), the queued sends (Arrays `{sending actor. selector. result record. arguments...}`) |

Kinds 15 and 16 (`EURASIA_ACTORS_DESIGN_2026-11-17.md`) are hidden too:
the semantics' tables Φ and Π. A far reference is the only place a
reference into another actor's heap may be held. A promise record holds
nothing in any actor's terms: values, far references and records only. Both
use the FarReference class (role 35). A snapshot saves only what the roles
reach, so only the actor whose message loop is the role is saved; a
checkpoint saves every actor of the cohort, with its mailbox, in the machine
section.

Kinds 13 and 14 are hidden layouts: their encoding belongs to the format,
not to a layout the image may read. In a snapshot, a frame's activation
handle is saved **dead**: a snapshot holds only what the roles reach (the VM
collects into a copy before writing, M3), and starts with no frames. In a
checkpoint it is saved live, naming its frame in the machine section by
serial number. A detached activation (one the debugger's Simulator made,
`Activation new`, or one cut off by `vm continueAt:with:` or at the end of
a turn) is saved with its state in both.

Kinds 9–12 are the contract layouts of the implementation plan (section 4
of the operation list): the VM reads those fields and no others. Everything
the VM derives (class ids, final layouts, lookup caches) is rebuilt at load
and never written.

## 6. Method header

| Field | Type | Meaning |
|---|---|---|
| kind | `u8` | 0 ordinary, 1 native, 2 getter, 3 setter, 4 return-self |
| access | `u8` | 0 public, 1 protected, 2 private |
| argument count | `uv` | |
| local count | `uv` | |
| native id | `uv` | Kind 1 only: the catalog entry, valid for this kernel version |
| slot index | `uv` | Kinds 2 and 3 only: the index among the slot declarations of the method's **own mixin**, from 0. The VM adds the number of slots of the superclasses of that mixin's application in the receiver's class, so one method serves every application (the application-qualified slots `(C, j)` of the formal semantics). |

A native method has bytecode length 0.

## 7. Roles

Role ids for kernel version 1 (the operation list, section 2):

| Id | Role | Id | Role |
|---|---|---|---|
| 1 | nil | 18 | `doesNotUnderstand:` selector |
| 2 | true | 19 | `nonBooleanReceiver:` selector |
| 3 | false | 20 | `cannotReturn:` selector |
| 4 | SmallInteger class | 21 | `aboutToReturnThrough:` selector |
| 5 | MediumInteger class | 22 | `dispatchMessage:port:` selector |
| 6 | LargeInteger class | 23 | `dispatchSignal` selector |
| 7 | Float class | 24 | failure exception class |
| 8 | String class | 25 | `ensure:` method |
| 9 | ByteArray class | 26 | `on:do:` method |
| 10 | Array class | 27 | simulation-root method |
| 11 | WeakArray class | 28 | callback-dispatch method |
| 12 | Ephemeron class | 29 | common-selector array (V5 opcodes 176–207) |
| 13 | Closure class | 30 | Message class |
| 14 | Activation class | 31 | VM mirror (its class carries the kernel operations) |
| 15 | Method class | 32 | entry object (sent `main:args:` with the VM mirror and an Array of argument Strings when the message loop is nil) |
| 16 | Expat class | 33 | Alien class |
| 17 | message loop (when not nil, the VM starts the image by sending it `dispatchMessage:port:`) | 34 | Value class (its subclasses are value classes) |
| | | 35 | FarReference class (of far references and promise records) |
| | | 36 | platform template: the message loop a new actor's platform is cloned from |
| | | 37 | Object class (`vm spawn:` applies its mixin to it) |
| | | 38 | InstanceMixin class (`basicNew` of it answers an unfrozen mixin) |
| | | 39 | ClassMixin class (likewise, a class-side mixin) |
| | | 40 | restore handler: sent `restored`, in a turn of its own, after a checkpoint is restored into a new host; or nil |
| | | 41 | incarnation: the running incarnation's id (a ByteArray of 16 bytes); in a saved image, the id of the incarnation that saved it; nil in an image no incarnation saved (genesis) |
| | | 42 | lineage: the id of the incarnation that saved the image the running one started from, or nil |

Every start (a run from a snapshot, or a checkpoint restored into a new
host) mints a fresh incarnation id from host entropy, and the id the image
was saved with becomes the lineage (contract design, section 6). A replay
continuing from a checkpoint is the same incarnation and keeps both.

Every role must be present. Unknown role ids are an error.

## 8. Validation at load

The loader checks, and refuses the snapshot with a message naming the node if
any fails: the header; strictly increasing sequence numbers within each
section, unique across the file, and below the allocation counter; every reference names an existing node; node counts match;
every kind and flag is known; method headers are consistent with their kind;
every role is present. Bytecode is checked when a method is first run, not at
load.
