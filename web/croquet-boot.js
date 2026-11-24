// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.
//
// Starts Eurasia on croquet.html, once the Croquet session exists: the support
// script (croquet-glue.js) loads this file from Session.join's continuation,
// as a JS deploy's glue loads its program. The image is the app parameter's
// (viewer, the default, or ide): <app>/croquet.eus, staged by scripts/ide.sh.
(async () => {
  const params = new URLSearchParams(location.search);
  const app = params.get('app') || 'viewer';
  const fetchBytes = async (url) => {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`cannot fetch ${url}: ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
  };
  try {
    const { run } = await import('./eurasia.mjs');
    const [wasm, snapshot] = await Promise.all([fetchBytes('eurasia_web.wasm'), fetchBytes(`${app}/croquet.eus`)]);
    const jit = params.has('interpret') ? 0 : 1;
    const result = await run(wasm, snapshot, { print: (line) => console.log(line), jit });
    console.log('eurasia:', result.status === 'exited' ? `exited ${result.code}` : 'idle');
  } catch (e) {
    console.error('eurasia:', e.message);
    document.body.append(`Eurasia failed: ${e.message}`);
  }
})();
