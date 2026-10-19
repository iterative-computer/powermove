/*
 * The contained runtimes (contained.ts, sandbox.ts, sandbox-runtime.ts)
 * against the built bundle: generated scripts run with the frame's results
 * and error texts, and a contained process cannot read the user's files,
 * write, start processes, or open connections (unless the manifest grants
 * the network), whatever it imports.
 */
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { createServer, type Server } from 'node:net';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { install as installScripting } from '../../renderer/src/legacy/core/scripting';
import { containment, spawnContained } from './contained';
import { runtimeReads, scriptRuntime } from './sandbox';
import { buildEngineBundle } from './test-host';

let bundle: { script: string; dispose(): void } | null = null;
let runtime = '';
let server: Server;
let port = 0;
let scratch = '';
beforeAll(async () => {
  bundle = await buildEngineBundle();
  runtime = path.join(path.dirname(bundle.script), 'sandbox-runtime.mjs');
  server = createServer((socket) => socket.end('hello'));
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  port = (server.address() as { port: number }).port;
  scratch = realpathSync(mkdtempSync(path.join(tmpdir(), 'pm-contained-')));
}, 180_000);
afterAll(() => { bundle?.dispose(); server?.close(); if (scratch) rmSync(scratch, { recursive: true, force: true }); });

const PM: any = { uid: (prefix = 'x') => `${prefix}1` };
installScripting(PM);
const source = (): string => PM.Script.test.workerSource;

describe('generated scripts in a contained process', () => {
  it('returns what the script returns, as the frame does', async () => {
    const run = scriptRuntime(runtime, () => {});
    const result = await run({ source: source(), code: `return [PM.setLayer(PM.layers[0].id, { name: 'Renamed ' + PM.input.n })];`, project: { layers: [{ id: 'L1', name: 'One' }], composition: {} }, input: { n: 2 }, runId: 'r1', runtime: 900 });
    expect(result).toEqual({ ok: true, result: [{ type: 'set_layer', target: 'L1', patch: { name: 'Renamed 2' } }] });
  }, 30_000);

  it('stops a script past its time limit with the frame’s message', async () => {
    const run = scriptRuntime(runtime, () => {});
    const result = await run({ source: source(), code: 'for (;;) {}', project: {}, input: {}, runId: 'r2', runtime: 300 });
    expect(result).toEqual({ ok: false, error: 'Script exceeded its time limit' });
  }, 30_000);

  it('reports a thrown error, and native process members are gone', async () => {
    const run = scriptRuntime(runtime, () => {});
    const thrown = await run({ source: source(), code: `PM.assert(false, 'nope')`, project: {}, input: {}, runId: 'r3', runtime: 900 });
    expect(thrown).toEqual({ ok: false, error: 'nope' });
    const reach = await run({ source: source(), code: `return [process.getBuiltinModule, process.binding, process.dlopen, process.kill].map((m) => typeof m)`, project: {}, input: {}, runId: 'r4', runtime: 900 });
    expect(reach).toEqual({ ok: true, result: ['undefined', 'undefined', 'undefined', 'undefined'] });
  }, 30_000);
});

/** Runs a probe module in a contained process; it reports one JSON line of outcomes. */
async function probe(network: boolean): Promise<Record<string, string>> {
  const file = path.join(scratch, `probe-${network}.mjs`);
  writeFileSync(file, `
import fs from 'node:fs';
import net from 'node:net';
import cp from 'node:child_process';
const out = {};
const tryIt = (name, fn) => { try { fn(); out[name] = 'allowed'; } catch (e) { out[name] = e.code || e.message; } };
tryIt('readHome', () => fs.readdirSync(${JSON.stringify(homedir())}));
tryIt('readEtc', () => fs.readFileSync('/etc/hosts'));
tryIt('write', () => fs.writeFileSync(${JSON.stringify(path.join(scratch, 'written'))}, 'x'));
tryIt('exec', () => cp.execFileSync('/usr/bin/true'));
out.env = Object.keys(process.env).sort().join(',');
const socket = net.connect(${port}, '127.0.0.1');
socket.on('data', () => { out.connect = 'allowed'; finish(); });
socket.on('error', (e) => { out.connect = e.code || e.message; finish(); });
function finish() { process.send(out); }
`);
  const child = spawnContained({ script: file, args: [], read: [...runtimeReads(runtime)], network });
  return new Promise((resolve, reject) => {
    child.on('message', (message) => { resolve(message as Record<string, string>); child.kill(); });
    child.on('exit', (code) => reject(new Error(`probe exited ${code}`)));
  });
}

describe('a contained process', () => {
  it('reads nothing of the user’s, writes nothing, starts nothing, and gets a clean environment', async () => {
    const out = await probe(false);
    expect(out.readHome).toBe('ERR_ACCESS_DENIED');
    expect(out.readEtc).toBe('ERR_ACCESS_DENIED');
    expect(out.write).toBe('ERR_ACCESS_DENIED');
    expect(out.exec).toBe('ERR_ACCESS_DENIED');
    expect(out.env).toBe('LANG,PATH,TZ');
  }, 30_000);

  it.runIf(containment() === 'seatbelt+permission')('cannot connect without the network permission, and can with it (the OS sandbox)', async () => {
    expect((await probe(false)).connect).not.toBe('allowed');
    expect((await probe(true)).connect).toBe('allowed');
  }, 30_000);
});
