# Eurasia snapshot format

**Status:** Draft, format version 9. Written before the code (implementation
plan M0); the packager (Newspeak) and `eurasia-snapshot` (Rust) are tested
against each other.

Eurasia snapshots replace psoup's vfuels. The structure follows psoup's
PrimordialFuel (a graph written as nodes, then edges), with two differences.
**An object that has been asked its identity hash is written with its hash
number**, so hashes survive a save and restore exactly
(`kernel-semantics.tex`, `eq:kernel-identity-hash`, as revised: an object is
numbered at the first request, not at allocation). And **the nodes of a
section are in a canonical order** that the object graph determines (section
3), so a heap has one image, whichever VM writes it. psoup groups objects by
class.

Version 9 (2026-11-23) is the contract revision of
`EASTASIA_DESIGN_2026-11-23.md`, section 3, which lets a second VM share the
format. Up to version 8 every object was written with its allocation
sequence number, which was its hash number, and nodes were in sequence
order. A version 9 reader still reads version 8 (section 9).

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
| format version | `u16` | This document: 9 (2026-11-23: version 9 hash numbers in place of sequence numbers, the hash counters, canonical node order and optional sections; 2026-11-19: version 8 a promise record's waiters and queue, and a detached activation's temps, as Arrays, for the arena heap; 2026-11-17: version 2 gave mixins the class-side mixin field, version 3 the value flag bits and the Value role; 2026-11-18: version 4 far references, promise records, the shared bit and roles 35–37; version 5 the simulation-root bit and roles 38–39; version 6 checkpoints: the machine section, live and detached activations, and role 40; version 7 roles 41 and 42, incarnation and lineage) |
| kernel version | `u16` | Must equal the VM's `KERNEL_VERSION` |
| bytecode version | `u16` | Must equal the VM's `BYTECODE_VERSION` |
| word size | `u8` | Always 64 |
| flags | `u8` | Reserved, 0 |
| allocation counter | `uv` | `n_α`: how many objects have been allocated. The clock of safepoints, collections and checkpoints; objects do not carry their numbers. A String the VM makes for itself (a selector it sends, the name an alien send calls) is not counted: whether it is made depends on the VM's caches. |
| hash counter count | `uv` | How many hash counters follow |
| hash counters | `uv` each | Each actor's hash counter, by actor number from 0: how many hash numbers it has given out |
| node count | `uv` | Number of nodes in the file |
| section count | `uv` | Number of sections |

A VM refuses a snapshot whose magic, format version, kernel version,
bytecode version or word size differs from its own (but for an older format
version it still reads), and says which. (psoup
dies with "unimplemented code" when given a snapshot that is too new.)

An optional `#!` interpreter line before the magic is skipped, as in psoup.

## 3. Nodes and references

Nodes are numbered from 1 in file order across all sections; 0 is not a
node. A value's reference field names a node number.

**The canonical order.** A VM writes the nodes of each section in the order
in which a depth-first walk first meets them. The walk starts from the
roles, in role id order, then (in a checkpoint) from the values of the
machine section in the order they are written there. At a node it follows
the values of the node's edge record in order, the class first. A weak
array's elements and an ephemeron's fields are followed like any others:
the order is a property of the file's own edges. A section holds the nodes
of its space in that order. Nodes the walk does not reach (a writer given a
heap it has not collected) follow in each section.

The order is what a VM writes, not a condition a reader checks: an image in
another order is valid, and a VM that loads and saves it writes it in the
canonical one. A loader lays objects out in file order, so the file's order
is the loaded heap's.

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
| 128–255 | optional | Not nodes: the owner is 0, and the node count field holds a **byte length**, followed by that many bytes. A reader that does not know the kind skips them. For what one VM saves for itself and another may ignore (Eastasia's inlining feedback). Before the roles section. |

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
  at the last clearing point, the allocations from it to the next scheduled
  one, the clearing points and checkpoints so far, the program version, the
  checkpoint policy (allocations, 0 for none) and the allocation counter at
  the last checkpoint;
- the timer a turn ended with (optional `uv`), and whether the image runs a
  message loop (flag).

A checkpoint's heap holds everything the VM's roots reach weakly or
strongly: a weak reference is cleared only at a **clearing point**
(`EASTASIA_DESIGN_2026-11-23.md`, section 3.2), so until then the object it
refers to is part of the state. Clearing points are explicit collections
(`vm garbageCollect`) and the first safepoint at which the allocation
counter has advanced by the budget the last one announced; what is cleared
at each is the collector's choice, and a recording logs it. The collection
a checkpoint makes is not a clearing point and does not move this clock.

A WeakArray or Ephemeron always has a hash number, given when it is made:
the log names it by that number. A reader numbers those that have none (in
an image written before this), in file order, after it has read the hash
counters.

Two-phase loading, as in psoup: all nodes of all sections are allocated
first (their sizes are in the node records), then edges are filled, so
cycles need no special handling.

## 5. Node records

| Field | Type | Meaning |
|---|---|---|
| hash | `uv`, then perhaps a `uv` | 0: the object has never been asked its identity hash. Odd, `2c + 1`: it was the `c`-th object (from 0) that an actor numbered; that actor's number follows as a `uv`. Even and not 0, `2s + 2`: its hash number is `s`, the sequence number it had in an image older than version 9. Below. |
| kind | `u8` | Below |
| flags | `u8` | Bit 0: frozen (installed or canonical). Bit 1: a value (deeply immutable; actors may share it). Bit 2: on a class, a value class (its instances are values). Bit 3: shared by every actor though mutable (the kernel module, the VM mirror): cloning a platform does not copy it. Bit 4: on a method, a simulation root (psoup's primitive 142): a non-local return stops at it, and an exception in a simulated thread is routed to it. Others reserved. |
| shape | depends on kind | Below |

**Hash numbers.** An object's identity hash is `h` of its hash number
(`eurasia-core`'s `identity_hash`). An object has no number until its hash
is first asked for; the actor that asks takes the next value `c` of its own
hash counter, and the number is `2^61 + actor · 2^40 + c`. A sequence number
kept from an older image is below `2^61`, so the two never meet. **A class
(kind 11) is numbered when it is made**, by the actor that makes it:
`mixinApplicationsOf:` and `subclassesOf:` answer classes in the order of
their numbers. A writer must give every class a number.

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
| 14 | activation | `u8` state: 0 dead, 1 a live frame's handle (then `uv` the frame's serial number), 2 detached | class value, method, receiver, closure (nil for a method activation); a detached one's sender, bci and temps (an Array, or nil for none) follow |
| 15 | far reference | `uv` actor number | class value, target (in that actor's heap) |
| 16 | promise record | `uv` state (0 pending, 1 fulfilled, 2 broken, 3 forwarded) | class value, value (resolution, problem or record forwarded to; nil while pending), owner (far reference or nil), the waiters (an Array of far references, or nil for none), the queued sends (an Array of Arrays `{sending actor. selector. result record. arguments...}`, or nil for none) |

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
any fails: the header; every hash number in range; every reference names an existing node; node counts match;
every kind and flag is known; method headers are consistent with their kind;
every role is present. Bytecode is checked when a method is first run, not at
load.

## 9. Reading version 8

A version 8 file has no hash counters in its header and no optional
sections, and each node record starts with a **sequence delta** in place of
the hash field: the sequence number minus the previous node's in the same
section (the section's first node's is absolute). Within a section sequence
numbers strictly increase. The reader lays the objects out in sequence
order across all sections, and each object's sequence number becomes its
hash number, so identity hashes, and the collections built on them, are
unchanged. The Newspeak packager (`EurasiaPackager.ns`) still writes
version 8.
