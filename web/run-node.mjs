// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

// Runs a snapshot on Eurasia's wasm32 build under Node, through the same
// loader the browser uses (eurasia.mjs). Lines the program prints go to
// stdout; the exit status is the program's (0 when the entry returns, 70 when
// the VM fails). With EURASIA_WEB_STATS set, it reports on stderr how many
// times the run waited for a timer. Usage: node run-node.mjs
// <eurasia_web.wasm> <snapshot>.

import { readFileSync } from 'node:fs';
import { run } from './eurasia.mjs';

const [wasmPath, snapshotPath] = process.argv.slice(2);
if (!wasmPath || !snapshotPath) {
  console.error('usage: node run-node.mjs <eurasia_web.wasm> <snapshot>');
  process.exit(2);
}
try {
  const result = await run(readFileSync(wasmPath), readFileSync(snapshotPath));
  if (process.env.EURASIA_WEB_STATS) console.error(`eurasia: ${result.resumes} timer waits`);
  process.exitCode = result.status === 'exited' ? result.code : 0;
} catch (e) {
  console.error(`eurasia: ${e.message}`);
  process.exitCode = 70;
}
