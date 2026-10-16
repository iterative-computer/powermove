#!/usr/bin/env node
/* The Node (V8) side of `pm-cli parity eval --wasm`: answers pm-wasm queries
   with the built WebAssembly module, the way the renderer and the headless
   document host load it.

     node engine/crates/pm-cli/scripts/parity-wasm.mjs <pm_wasm.wasm>

   Prints "ready", then reads one JSON command per line on stdin:

     { "doc": <canonical project JSON file>, "answers": <registry answers file>,
       "faces": [<face file>, ...], "outlines": <bool>,
       "requests": <one JSON request per line>, "out": <replies file> }

   For each command it instantiates a fresh module instance (as a new
   renderer would), adds the registry answers and faces (`pm_fonts_answers`,
   `pm_fonts_add`; a face file is already in `pm_fonts_add`'s layout), loads
   the document (`pm_resync`), answers every request (`pm_query`), writes
   one reply per line, and prints "done <count>". Errors print "error <message>". */
import { readFileSync, writeFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

const [modulePath] = process.argv.slice(2);
if (!modulePath) {
  console.error('usage: parity-wasm.mjs <pm_wasm.wasm>');
  process.exit(2);
}
const module = await WebAssembly.compile(readFileSync(modulePath));
const decoder = new TextDecoder();
const encoder = new TextEncoder();

async function run(command) {
  // In the renderer `Math.pow` and `**` come from the page's V8 (pm-expr's js-pow).
  const { exports: x } = await WebAssembly.instantiate(module, { pm_expr: { pow: Math.pow } });
  const putBytes = (bytes) => {
    const ptr = x.pm_in(bytes.length || 1);
    new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
    return bytes.length;
  };
  const put = (text) => putBytes(encoder.encode(text));
  const out = (len) => decoder.decode(new Uint8Array(x.memory.buffer, x.pm_out_ptr(), len));
  if (x.pm_fonts_answers(put(readFileSync(command.answers, 'utf8'))) !== 0) {
    throw new Error('pm_fonts_answers: ' + out(x.pm_out_len()));
  }
  for (const face of command.faces) {
    if (x.pm_fonts_add(putBytes(readFileSync(face))) !== 0) throw new Error('pm_fonts_add: ' + face);
  }
  x.pm_fonts_outlines(command.outlines ? 1 : 0);
  if (x.pm_resync(put(readFileSync(command.doc, 'utf8')), 1) !== 0) {
    throw new Error('pm_resync: ' + out(x.pm_out_len()));
  }
  x.pm_in_release();
  const requests = readFileSync(command.requests, 'utf8').split('\n');
  const replies = [];
  for (const request of requests) {
    if (!request) continue;
    replies.push(out(x.pm_query(put(request))));
  }
  writeFileSync(command.out, replies.length ? replies.join('\n') + '\n' : '');
  return replies.length;
}

console.log('ready');
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  if (!line.trim()) continue;
  try {
    const count = await run(JSON.parse(line));
    console.log(`done ${count}`);
  } catch (error) {
    console.log(`error ${String(error?.stack || error).replace(/\n/g, ' | ')}`);
  }
}
