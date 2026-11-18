# Expected differences from psoup

Differential tests run the same Newspeak source on psoup (V4) and Eurasia
(V5) and compare output. Every difference must be explained by an entry here:
a decided semantic change, a psoup bug, or a representation detail. Anything
else is a bug.

## Per-test differences

`scripts/kernel-tests.sh` runs psoup's `KernelTests`, `AccessModifierTesting`,
`KernelWeakTests`, `ActorsTesting`, `MirrorTesting`, `MirrorBuilderTesting` and
`ActivationMirrorTesting` on both VMs, asynchronously as psoup's
`TestRunner` does, and compares each test's outcome
(success, failure or error). A test whose outcome may differ is listed here
as a line `- `Context>>selector`: reason`; the script fails on any other
difference, and also reports listed tests that agree again.

- `TwoActorTests>>testPipelining`: the test sends its `Promise` (the actors module's `PromiseFactories` instance) to another actor, which immediate-sends `when:fulfilled:` to it. psoup's library treats `Promise` as a value object, so it crosses as itself; on Eurasia every actor has its own platform, `Promise` is a mutable object of the sender's, and it crosses as a far reference, which refuses the immediate send. Decided 2026-11-18 (`newspeak/EURASIA_ACTORS_DESIGN_2026-11-17.md` section 8); platform correspondence (an object of the platform template crossing as the receiver's clone of it) would remove it.

The other 619 tests have the same outcome on both VMs (2026-11-18), including
all of psoup's `MirrorTesting`, `MirrorBuilderTesting` and
`ActivationMirrorTesting`; four of `ActivationMirrorTesting`'s fail on both,
because they expect `[^2]` where the pretty-printed test source now has
`[ ^2 ]`. Of the 444 outside the mirror suites, 428 succeed on both; the 16 that
do not fail identically on both, with the same message:
`FloatTests>>testFloatRaisedTo` (the test computes `8 ** 1 / 3`, which is
`(8 ** 1) / 3`) and 15 `SingleActorTests`, which immediate-send `isPromise`
or `isResolved` to an eventual reference, which the actors library refuses.
Both runners install a handler of last resort, so a turn's unhandled
exception (`TimerTests>>testOneShotTimerInvalidCallback` causes one) is
reported and the suite goes on, as on psoup.

Not yet compared: ports and spawning another VM (no suite uses them), and
`MinitestTests` (Minitest's own discovery and runner use mirrors and
promises; on Eurasia the runner finds tests from the packager's catalog). psoup's `CollectionsTesting` does
not run on psoup itself (it imports an `OrderedMap` that
`CollectionsForPrimordialSoup` lacks).

## JS tests on wasm

`scripts/web-tests.sh` runs psoup's `JSTesting` on the wasm32 VM under
Node. psoup's native VM cannot run them (a failing JS primitive halts it),
so they are not compared with psoup: every JS test must succeed, except
those listed here as `- `JSTests>>selector`: reason`.

- `JSTests>>testRawPushRejectsNonBilingualReceiver`: the test checks that passing a non-bilingual object to JavaScript raises an Error whose text mentions `rawPush:`, the name of psoup's internal marshalling primitive. Eurasia refuses the argument too, but as a native failure of the operation: `AlienCall failed: wrongKind 2` (operation list, section 10).

## Semantic and representation differences

| Area | psoup | Eurasia | Why |
|---|---|---|---|
| `Number>>=` with a non-number | fails (`ArgumentError`) | answers false | Decided 2026-11-10. |
| `<`, `>`, `<=`, `>=` with a non-number argument | `ArgumentError`, except a Fraction argument | tail-sends the reversed comparison to the argument (`a < b` sends `b > a`), so a non-number receiver of the reversal gets DNU | No fallback bodies (design doc section 3); the argument decides. |
| Comparisons with a Fraction argument | **psoup bug**: `a < b` answers `b >= a` (and likewise for the others), which is wrong when the values are equal | correct reversal | `KernelForPrimordialSoup.ns`, `Number>><` and siblings. |
| `Integer>>/` | primitive, then Fraction in the fallback | an ordinary Newspeak method | The fallback was real code (operation list, section 3.1). |
| A closure called with the wrong number of arguments | `ArgumentError` (the primitive fails and `Closure>>value` reports it) | `NativeFailure`, `#arity` | No fallback bodies: a failing native signals (operation list, section 0). Both are Errors, so `on: Error` catches either; printed messages differ. |
| JavaScript `undefined` | an Alien (`isUndefined`) | the same: an Alien for reference 0; `null` arrives as nil | psoup's API, which `JSTesting` and Hopscotch use (operation list, section 10; decided 2026-11-18). |
| Host aliens and identity | a new Alien (and table entry) for every host object answered | the same | The host table only grows on both; reclaiming entries is open. |
| Callback arguments | extra JS arguments raise `ArgumentError` | dropped; missing ones are nil | Decided 2026-11-10. |
| Float printing | double-conversion's shortest form | the same digits and spelling (`3.0`, `1.7976931348623157e+308`, `1.5e-7`), from Rust's shortest round-trip digits | Aligned 2026-11-17 by the kernel tests. |
| `asStringFixed:`, `asStringExponential:`, `asStringPrecision:` | JavaScript's `toFixed`, `toExponential`, `toPrecision` | the same (`e+` exponents, digit ranges 0-20, 0-20 and 1-21) | Aligned 2026-11-17; any unusable digit count fails with `#outOfRange: 1` (operation list 3.1). |
| Float division by zero | an infinity or NaN | the same | IEEE 754; aligned 2026-11-17 (Eurasia had failed with `#zeroDivide`). |
| `String>>hash` | psoup's function | FNV-1a | Hash values are not portable between VMs; only equal strings hashing equally is guaranteed. |
| `identityHash` | address-derived | the allocation sequence number | Decided: deterministic. Collection keeps sequence numbers, so hashes never change. |
| When collection happens | psoup's heap policy (scavenges, mark-sweeps) | a full collection after as many allocations as there are live objects, and at `vm garbageCollect` | Permitted nondeterminism (`gc-semantics.tex`); only weak containers can observe it. Eurasia's timing depends only on the allocation count. |
| Ephemeron finalizers | not run (psoup clears the field) | not run (cleared) | Same; finalization is not part of the platform yet. |
| Transcendental functions | the host's `libm` | bundled `libm` crate (musl port) | Decided: bit-identical on every host. Results may differ from psoup in the last bit. |
| SmallInteger range | word-sized (31 bits on wasm32, 63 natively) | 63 bits everywhere | Decided 2026-11-17. |
