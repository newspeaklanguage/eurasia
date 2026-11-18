# Expected differences from psoup

Differential tests run the same Newspeak source on psoup (V4) and Eurasia
(V5) and compare output. Every difference must be explained by an entry here:
a decided semantic change, a psoup bug, or a representation detail. Anything
else is a bug.

| Area | psoup | Eurasia | Why |
|---|---|---|---|
| `Number>>=` with a non-number | fails (`ArgumentError`) | answers false | Decided 2026-11-10. |
| `<`, `>`, `<=`, `>=` with a non-number argument | `ArgumentError`, except a Fraction argument | tail-sends the reversed comparison to the argument (`a < b` sends `b > a`), so a non-number receiver of the reversal gets DNU | No fallback bodies (design doc section 3); the argument decides. |
| Comparisons with a Fraction argument | **psoup bug**: `a < b` answers `b >= a` (and likewise for the others), which is wrong when the values are equal | correct reversal | `KernelForPrimordialSoup.ns`, `Number>><` and siblings. |
| `Integer>>/` | primitive, then Fraction in the fallback | an ordinary Newspeak method | The fallback was real code (operation list, section 3.1). |
| Callback arguments | extra JS arguments raise `ArgumentError` | dropped; missing ones are nil | Decided 2026-11-10. |
| Float printing | double-conversion's shortest form | Rust's shortest round-trip form (`1.0`, `0.30000000000000004`, `1e300`) | Same values; spelling of large and small exponents may differ. Check in the differential tests. |
| `asStringExponential:` | double-conversion (`1.23e+4`?) | Rust (`1.23e4`) | To verify and, if needed, align. |
| `String>>hash` | psoup's function | FNV-1a | Hash values are not portable between VMs; only equal strings hashing equally is guaranteed. |
| `identityHash` | address-derived | the allocation sequence number | Decided: deterministic. |
| Transcendental functions | the host's `libm` | bundled `libm` crate (musl port) | Decided: bit-identical on every host. Results may differ from psoup in the last bit. |
| SmallInteger range | word-sized (31 bits on wasm32, 63 natively) | 63 bits everywhere | Decided 2026-11-17. |
