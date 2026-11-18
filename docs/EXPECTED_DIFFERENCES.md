# Expected differences from psoup

Differential tests run the same Newspeak source on psoup (V4) and Eurasia
(V5) and compare output. Every difference must be explained by an entry here:
a decided semantic change, a psoup bug, or a representation detail. Anything
else is a bug.

## Per-test differences

`scripts/kernel-tests.sh` runs psoup's `KernelTests`, `AccessModifierTesting`,
`KernelWeakTests` and `ActorsTesting` on both VMs, asynchronously as psoup's
`TestRunner` does, and compares each test's outcome
(success, failure or error). A test whose outcome may differ is listed here
as a line `- `Context>>selector`: reason`; the script fails on any other
difference, and also reports listed tests that agree again.

None at present (2026-11-17): all 445 tests have the same outcome on both
VMs, also under GC stress (`GC_STRESS=64`). 429 succeed on both; the 16 that
do not fail identically on both, with the same message:
`FloatTests>>testFloatRaisedTo` (the test computes `8 ** 1 / 3`, which is
`(8 ** 1) / 3`) and 15 `SingleActorTests`, which immediate-send `isPromise`
or `isResolved` to an eventual reference, which the actors library refuses.
Both runners install a handler of last resort, so a turn's unhandled
exception (`TimerTests>>testOneShotTimerInvalidCallback` causes one) is
reported and the suite goes on, as on psoup.

Not yet compared: ports and spawning another VM (no suite uses them), the mirror suites and `MinitestTests` (mirrors, M6; Minitest's own
discovery and runner use mirrors and promises, so on Eurasia the runner
finds tests from the packager's catalog). psoup's `CollectionsTesting` does
not run on psoup itself (it imports an `OrderedMap` that
`CollectionsForPrimordialSoup` lacks).

## Semantic and representation differences

| Area | psoup | Eurasia | Why |
|---|---|---|---|
| `Number>>=` with a non-number | fails (`ArgumentError`) | answers false | Decided 2026-11-10. |
| `<`, `>`, `<=`, `>=` with a non-number argument | `ArgumentError`, except a Fraction argument | tail-sends the reversed comparison to the argument (`a < b` sends `b > a`), so a non-number receiver of the reversal gets DNU | No fallback bodies (design doc section 3); the argument decides. |
| Comparisons with a Fraction argument | **psoup bug**: `a < b` answers `b >= a` (and likewise for the others), which is wrong when the values are equal | correct reversal | `KernelForPrimordialSoup.ns`, `Number>><` and siblings. |
| `Integer>>/` | primitive, then Fraction in the fallback | an ordinary Newspeak method | The fallback was real code (operation list, section 3.1). |
| A closure called with the wrong number of arguments | `ArgumentError` (the primitive fails and `Closure>>value` reports it) | `NativeFailure`, `#arity` | No fallback bodies: a failing native signals (operation list, section 0). Both are Errors, so `on: Error` catches either; printed messages differ. |
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
