// Copyright (c) 2026-2027, Gilad Bracha. All rights reserved. Use of this source
// code is governed by a BSD-style license that can be found in the LICENSE file.

// The JavaScript host of Eurasia's wasm32 build (crates/eurasia-web): it
// instantiates the VM, gives it the host imports, loads a snapshot and runs
// it. The VM never blocks: when nothing can run before the next timer it
// returns, and this loader resumes it from a JavaScript timer, so the
// browser's event loop keeps running. Works in browsers and in Node.

const RETURNED = 0;
const EXITED = 1;
const WAITING = 2;
const FAILED = -1;

// Runs `snapshot` (bytes) on the VM in `wasm` (bytes or a compiled
// WebAssembly.Module). `print` receives each line the program prints.
// Answers a promise of {status: 'returned', resumes} or {status: 'exited',
// code, resumes}, where resumes counts the waits for a timer;
// a VM failure rejects it with an Error.
export async function run(wasm, snapshot, { print = console.log } = {}) {
  let memory = null;
  const decoder = new TextDecoder();
  const bytesAt = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);
  const imports = {
    eurasia: {
      print: (ptr, len) => print(decoder.decode(bytesAt(ptr, len))),
      monotonic_nanos: () => Math.floor(performance.now() * 1e6),
      utc_nanos: () => Date.now() * 1e6,
      random_bytes: (ptr, len) => crypto.getRandomValues(bytesAt(ptr, len)),
    },
  };
  const module = wasm instanceof WebAssembly.Module ? wasm : await WebAssembly.compile(wasm);
  const instance = await WebAssembly.instantiate(module, imports);
  const vm = instance.exports;
  memory = vm.memory;

  const buffer = vm.eurasia_buffer(snapshot.length);
  bytesAt(buffer, snapshot.length).set(snapshot);

  let resumes = 0;
  return new Promise((resolve, reject) => {
    const settle = (status) => {
      switch (status) {
        case RETURNED:
          resolve({ status: 'returned', resumes });
          return;
        case EXITED:
          resolve({ status: 'exited', code: vm.eurasia_exit_code(), resumes });
          return;
        case WAITING: {
          const delay = (vm.eurasia_wakeup_nanos() - performance.now() * 1e6) / 1e6;
          setTimeout(() => {
            resumes += 1;
            settle(vm.eurasia_resume());
          }, Math.max(0, delay));
          return;
        }
        case FAILED:
          reject(new Error(decoder.decode(bytesAt(vm.eurasia_error(), vm.eurasia_buffer_length()))));
          return;
        default:
          reject(new Error(`eurasia: unknown status ${status}`));
      }
    };
    settle(vm.eurasia_start());
  });
}
