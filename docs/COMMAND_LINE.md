# Command-line Eurasia

psoup's command-line applications (the parser, the pretty-printer, the
round-trip check, the test runner) run on Eurasia's wasm32 build under Node,
with the host's files. This is the command-line Eurasia for now: the native
build stays interpreted, and a native VM of its own comes later, designed
apart from the web one.

## Building everything

```bash
scripts/build.sh [NAME ...]
```

builds the applications of `newspeak/EurasiaBuildList.ns` (the IDE, the
parser, the pretty-printer, the round-trip check, the attachment diff, the
test runner, CounterApp, TodoMVC) into `target/build/<name>/base.eus`,
packaging them all with one packager on Eurasia (`newspeak/EurasiaBuild.ns`),
so shared modules are compiled once. Each application is its main module and a
platform definition (base; with mirrors; with Hopscotch; with both); its
modules are found from its manifest sends. Images of platforms without mirrors
keep no sources. Packaging all eight takes 13.5 s, genesis well under a
second. The natives catalog the build uses is
`newspeak/platform/Eurasia/EurasiaNatives.ns`, generated from
`kernel/natives.tsv` by `scripts/natives-module.py` (`--check` compares them;
`scripts/check.sh` runs it). See `newspeak/EURASIA_BUILD_AND_DEPLOY_DESIGN_2026-11-21.md`.

## Comparing images

`eurasia --methods IMAGE` lists an image's methods (where, selector, header,
literals, bytecode), sorted, independent of sequence numbers;
`eurasia --census IMAGE` counts its objects by class. Two images of the same
program give the same lists: this is how an image deployed from a running
program (Build IDE image) is checked against one built from source.

## Use, one application

```bash
scripts/tool-image.py TOOL.ns [MODULE.ns ...] OUTDIR
EURASIA_JIT=1 node web/run-node.mjs target/wasm32-unknown-unknown/release/eurasia_web.wasm OUTDIR/base.eus ARGS...
```

TOOL.ns is a psoup application (`packageUsing: manifest`, `main:args:`);
the MODULEs are what it takes from its manifest. For the typechecker tests:

```bash
scripts/tool-image.py ../newspeak/NewspeakTestRunner.ns ../primordialsoup/newspeak/Minitest.ns target/tools/testrunner
```

then run it with the arguments `tool/run-tests.sh` gives psoup's runner (the
configuration's name, then the files: NewspeakTypechecker.ns,
NewspeakTypecheckerTesting.ns, NewspeakTypecheckerTestingConfiguration.ns).
`EURASIA_JIT=1` compiles methods; without it the tool is interpreted.

## How

- `scripts/tool-image.py` generates an entry (`OUTDIR/EurasiaTool.ns`) with
  a slot for each platform module, the extra modules and the tool, a
  manifest answering each by name, and a platform with mirrors
  (RuntimeWithMirrors over Runtime), as `EurasiaDebuggerProbe` builds it. It
  packages that with EurasiaPackager and runs genesis.
- The tools read and write files with psoup's primitive pragmas 130 and
  128. EurasiaPackager maps them to Eurasia's natives VmReadFile (92) and
  VmWriteFile (91) (`psoupPrimitiveNatives`), recorded per module and
  selector. The web host serves them through the page's `files` option, and
  `run-node.mjs` gives Node's `readFileSync` and `writeFileSync`; a page
  without `files` refuses them.

## What it needed

- **Files** in the web host (above).
- **Namespace parity.** Tools find the platform's classes in the slots of
  RuntimeWithMirrors (the test runner's namespace; the typechecker resolves
  type paths such as `Collections List[String]` and the kernel's mixins
  there). Eurasia's now has psoup's InternalKernel, Kernel, Collections and
  Actors, recorded when the platform is made.
- **Identity hashes that spread** (kernel version 2). The hash was the
  allocation sequence number itself, which clustered objects allocated in a
  regular rhythm in tables that mask the low bits: WeakMap made the
  typechecker tests five times slower. It is now a fixed bijection of the
  sequence number (`natives::identity_hash`, splitmix64's finalizer modulo
  2^62). Images of kernel version 1 are refused.
- **`vm commit:` keeps identity hashes.** A commit forwards each old program
  object (mixin, class) to its new one. The new one had its own sequence
  number, so a table keyed by the old object (the typechecker's
  `Map[Mixin, ...]`) lost it after an install. Now the new object takes the
  old one's sequence number, as psoup's become copies the hash. This is also
  the IDE's case: every accept goes through the AtomicInstaller.

## Results (2026-11-21)

Typechecker tests, `NewspeakTypecheckerTestingConfiguration`:

| | passed | time |
|---|---|---|
| psoup | 337 / 340 | 2.0 s |
| Eurasia, compiled | 337 / 340 | 4.7 s |
| Eurasia, interpreted | 337 / 340 | 9.7 s |

The three failures are the same on both: they need HopscotchWebIDE, which
the runner does not load.

Packaging the IDE (EurasiaPackager over its 104 modules; genesis on native
Eurasia then takes 0.07 s). Every run wrote the same genesis image, byte for
byte, so Eurasia packages itself:

| packager on | time |
|---|---|
| psoup (x86-64, under Rosetta) | 9.5 s |
| Eurasia wasm, compiled | 10.7 s |
| Eurasia native, interpreted | 17.3 s |
| Eurasia wasm, interpreted | 43.2 s |

(`scripts/tool-image.py ../newspeak/EurasiaPackager.ns target/tools/packager`,
then the arguments `scripts/ide.sh` gives psoup's packager.)

The parser tool gives psoup's output on files without psoup's primitive
pragmas; a file with them is refused by Eurasia's V5 compilation, as
expected. It is about twice as slow as psoup on a large file (Browsing.ns),
and starts in 0.14 s.
