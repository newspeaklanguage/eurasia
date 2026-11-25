# Libraries the Eurasia pages load

Third-party JavaScript the IDE, viewer and site pages load before the VM. This
directory has the same layout as a served site, so `scripts/site-runtime.sh`
and `scripts/ide.sh` copy it as it is, all but this README.

Only the files the pages load are here: the pages name them, and so does the
Newspeak code that loads CodeMirror (`codemirror.js`, `codemirror.css`,
`addon/display/autorefresh.js`). A page that needs another file needs it added
here first.

| Served path | Library | Version | Licence | Upstream |
|---|---|---|---|---|
| `CodeMirror/lib/codemirror.{js,css}`, `CodeMirror/addon/display/autorefresh.js` | CodeMirror | 5.45.0 | MIT (`CodeMirror/LICENSE`) | https://codemirror.net/5/ |
| `jszip.min.js` | JSZip | 3.10.1 | MIT or GPLv3, at the user's choice (in the file's header) | https://stuk.github.io/jszip/ |
| `vendor/buffer-6.0.3.bundle.min.js` | buffer (with base64-js and ieee754) | 6.0.3 | MIT (`vendor/buffer-LICENSE`, `vendor/base64-js-LICENSE`); BSD-3-Clause (`vendor/ieee754-LICENSE`) | https://github.com/feross/buffer |
| `vendor/isomorphic-git-1.38.4.umd.min.js`, `vendor/isomorphic-git-1.38.4-http-web.umd.js` | isomorphic-git | 1.38.4 | MIT | https://github.com/isomorphic-git/isomorphic-git |
| `vendor/lightning-fs-4.6.2.min.js` | lightning-fs | 4.6.2 | MIT | https://github.com/isomorphic-git/lightning-fs |
| `vendor/diff_match_patch-2019-07.js` | diff-match-patch | 2019-07 | Apache 2.0 (`vendor/diff_match_patch-LICENSE`) | https://github.com/google/diff-match-patch |

The copies are the ones psoup's IDE serves from `newspeak/out/`, byte for
byte; isomorphic-git and lightning-fs are also tracked in `newspeak/vendor/`.
The licence texts of isomorphic-git and lightning-fs are not here yet: neither
build carries one, and they are to be copied from the upstream repositories
at those versions.
