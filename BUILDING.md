# Building Eurasia and the Newspeak applications on it

What it takes to build the VM, the applications of the build list (the IDE,
the document viewer, the command-line tools) and the site runtime, on a
machine that has none of it yet. Nothing here depends on a particular
machine: every input is a file tracked in one of the three repositories below,
or a tool installed as described.

## Checkouts

Three git checkouts, side by side:

```
eurasia/          https://github.com/newspeaklanguage/eurasia
newspeak/         https://github.com/newspeaklanguage/newspeak
primordialsoup/   https://github.com/newspeaklanguage/primordialsoup (until stage 0; see below)
```

Elsewhere, set `NEWSPEAK` and `PRIMORDIALSOUP` to their directories; every
script reads them.

## Tools

- **Rust**, through [rustup](https://rustup.rs). The toolchain (version, the
  `wasm32-unknown-unknown` target, clippy, rustfmt) is pinned in
  `rust-toolchain.toml`; rustup installs it the first time `cargo` runs here.
- **Node 22** or later: the build runs on the wasm32 VM under Node
  (`web/run-node.mjs`).
- **python3**: the scripts that generate entries and natives tables.
- **git**: the build stamps each image with the checkouts' commits.

### psoup, until stage 0

The build's own tool image (EurasiaBuild, which compiles and packages the
applications on Eurasia) is still made on psoup, by `scripts/tool-image.py`.
That needs, in the primordialsoup checkout:

- `out/ReleaseX64/primordialsoup`, the native psoup VM, and
- `out/snapshots/WebCompiler.vfuel`,

both made by its `./build` (a C++ compiler and SCons; see
`primordialsoup/docs/building.md`). On Apple Silicon the x86-64 binary runs
under Rosetta. Emscripten is *not* needed: nothing here uses psoup's wasm
build.

Stage 0 (`newspeak/EURASIA_BUILD_AND_DEPLOY_DESIGN_2026-11-21.md`, section
3.3) checks in a build image so that psoup is no longer needed; this section
goes away then.

## Building

| Command | Result |
|---|---|
| `scripts/check.sh` | The VM's checks: formatting, clippy, the tests, the wasm32 build. |
| `scripts/build.sh [NAME ...]` | The applications of `newspeak/EurasiaBuildList.ns` (or the named ones), each as `target/build/NAME/base.eus`, with its entry, module list (`NAME.modules.txt`) and image resources (`NAME.images.txt`). |
| `scripts/ide.sh` | The IDE and the viewer, staged for a browser in `target/web` (and in `newspeak/out/`, unless `EURASIA_NO_OUT=1`). |
| `scripts/site-runtime.sh OUTDIR` | The site runtime: what a site of Newspeak documents serves besides its documents. |

The build is deterministic: the same checkouts give the same images, but for
the build stamp (BuildInfo). `eurasia --methods IMAGE` and
`eurasia --census IMAGE` compare two images while ignoring it
(`docs/COMMAND_LINE.md`).

## The site runtime

`scripts/site-runtime.sh OUTDIR` builds the document viewer and writes

| File | What |
|---|---|
| `eurasia.mjs`, `eurasia_web.wasm` | the loader and the VM |
| `images.mjs` | saved images: Save Image keeps the running image in the browser, offered at the next load |
| `viewer/base.eus` | the document viewer (AmpleforthViewer) |
| `eurasia-viewer.html` | its page: `eurasia-viewer.html?docName=<document>` shows `<document>.zip` from the same server |
| `deploy-boot.js` | the boot of every document page, and of `eurasia-viewer.html`, from newspeak |
| `custom-post.js` | page helpers (`safeDownloadBlob`), from primordialsoup |
| `CodeMirror/`, `jszip.min.js`, `vendor/` | the libraries, from `web/libs` (versions and licences in its README) |
| `*.png` | the images the viewer loads, from the newspeak checkout |

Each image the viewer loads (its `viewer.images.txt`) must be tracked in the
newspeak checkout as `<name>.png`; the script stops and names any that is not.
Nothing comes from `newspeak/out/`, which git ignores.

To try it, put a document's zip beside the runtime and serve the directory:

```sh
scripts/site-runtime.sh /tmp/site
cp Room101.zip /tmp/site/                  # any document's zip, e.g. from the blog's repository
python3 -m http.server -d /tmp/site 8000
# open http://localhost:8000/eurasia-viewer.html?docName=Room101
```
