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

// The host-object imports of the call-based FFI (operation list, section 10;
// crates/eurasia-web). The VM names host objects by references, indices into
// `refs`: 0 is undefined and 1 the global object. An operation's arguments
// are staged one by one, then ffi_perform does it and answers the result's
// tag, whose payload ffi_result_number and ffi_result_string_copy read:
// 0 undefined, 1 null, 2 Boolean, 3 number, 4 string, 5 reference, 6 the
// host raised (the payload describes the exception), 7 a dead reference.
// The table only grows, as psoup's does; reclaiming it is open.
function hostObjects(bytesAt, decoder, encoder) {
  const refs = [undefined, globalThis];
  let staged = [];
  let number = 0;
  let string = new Uint8Array(0);
  const answer = (v) => {
    if (v === undefined) return 0;
    if (v === null) return 1;
    switch (typeof v) {
      case 'boolean': number = v ? 1 : 0; return 2;
      case 'number': number = v; return 3;
      case 'string': string = encoder.encode(v); return 4;
      default: refs.push(v); number = refs.length - 1; return 5;
    }
  };
  const operations = [
    (o, [k]) => o[k],
    (o, [k, v]) => { o[k] = v; },
    (o, [k]) => delete o[k],
    (o, [k]) => k in o,
    (o, [k, ...a]) => o[k](...a),
    (f, a) => f(...a),
    (c, a) => new c(...a),
    (o, [c]) => o instanceof c,
  ];
  return {
    ffi_arg_undefined: () => { staged.push(undefined); },
    ffi_arg_null: () => { staged.push(null); },
    ffi_arg_bool: (b) => { staged.push(b !== 0); },
    ffi_arg_number: (x) => { staged.push(x); },
    ffi_arg_string: (ptr, len) => { staged.push(decoder.decode(bytesAt(ptr, len))); },
    ffi_arg_ref: (r) => { staged.push(refs[r]); },
    ffi_perform: (op, receiver) => {
      const args = staged;
      staged = [];
      if (receiver >= refs.length) return 7;
      try {
        return answer(operations[op](refs[receiver], args));
      } catch (e) {
        string = encoder.encode(String(e));
        return 6;
      }
    },
    ffi_result_number: () => number,
    ffi_result_string_length: () => string.length,
    ffi_result_string_copy: (ptr) => { bytesAt(ptr, string.length).set(string); },
  };
}

// Runs `snapshot` (bytes) on the VM in `wasm` (bytes or a compiled
// WebAssembly.Module). `print` receives each line the program prints.
// Answers a promise of {status: 'returned', resumes} or {status: 'exited',
// code, resumes}, where resumes counts the waits for a timer;
// a VM failure rejects it with an Error.
export async function run(wasm, snapshot, { print = console.log } = {}) {
  let memory = null;
  const decoder = new TextDecoder();
  const encoder = new TextEncoder();
  const bytesAt = (ptr, len) => new Uint8Array(memory.buffer, ptr, len);
  const imports = {
    eurasia: {
      print: (ptr, len) => print(decoder.decode(bytesAt(ptr, len))),
      monotonic_nanos: () => Math.floor(performance.now() * 1e6),
      utc_nanos: () => Date.now() * 1e6,
      random_bytes: (ptr, len) => crypto.getRandomValues(bytesAt(ptr, len)),
      ...hostObjects(bytesAt, decoder, encoder),
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
