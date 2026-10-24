import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { build } from 'esbuild';
import { afterAll, beforeAll, expect, it } from 'vitest';

let directory: string, module: string;
beforeAll(async () => {
  directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-cloud-queue-retention-'));
  module = path.join(directory, 'cloud.cjs');
  // Bundle the production factories and their real scanners/filesystem helpers.
  // ApiError shares the same bundled identity as publisher's guarded catch.
  await build({ stdin: { contents: `
    export { createPublisher } from './publish';
    export { createProvenanceStore, PROVENANCE_FILE } from './provenance';
    export { ApiError } from '@powermove/registry/wire';
  `, resolveDir: import.meta.dirname, loader: 'ts' }, outfile: module,
  bundle: true, platform: 'node', format: 'cjs' });
});
afterAll(async () => { if (directory) await rm(directory, { recursive: true, force: true }); });

function inspect(kind: 'provenance' | 'publisher') {
  return JSON.parse(execFileSync(process.execPath, ['--expose-gc', '-e', String.raw`
    const fs = require('node:fs/promises'), path = require('node:path');
    const { createPublisher, createProvenanceStore, PROVENANCE_FILE, ApiError } = require(${JSON.stringify(module)});
    const directory = ${JSON.stringify(directory)}, kind = ${JSON.stringify(kind)}, count = 2000;
    const turn = () => new Promise(resolve => setImmediate(resolve));
    async function collect() { for (let pass = 0; pass < 5; pass++) { await turn(); global.gc(); } }
    async function inspectCaller(service) {
      let result = kind === 'provenance' ? await service.read() : await service.prepare('queue-memory');
      const graph = kind === 'provenance' ? result['queue-memory'].future.entries : result.permissionFindings;
      const weak = new WeakRef(graph);
      await collect();
      // Closing a sheet or dropping a read result may free it; an active caller
      // still owns every returned item, including forward-compatible metadata.
      if (!weak.deref() || graph.length !== count) throw new Error('Caller lost its result');
      if (kind === 'provenance') {
        if (result['queue-memory'].future.entries[count - 1].label !== 'Synthetic future entry 1999') throw new Error('Metadata changed');
      } else {
        if (result.permissionFindings[0].line !== 1 || result.permissionFindings[count - 1].line !== count
          || result.permissionFindings.some(finding => finding.needs !== 'full-access' || finding.path !== 'index.ts')) throw new Error('Findings changed');
      }
      result = null;
      return weak;
    }
    (async () => {
      let service, detailCalls = 0;
      if (kind === 'provenance') {
        await fs.writeFile(path.join(directory, PROVENANCE_FILE), JSON.stringify({ 'queue-memory': {
          localId: 'queue-memory', envKey: 'local:synthetic', future: { entries: Array.from({ length: count },
            (_, index) => ({ index, label: 'Synthetic future entry ' + index, values: [1, 2, 3] })) }
        } }));
        service = createProvenanceStore(directory);
      } else {
        const userDir = path.join(directory, 'extensions'), folder = path.join(userDir, 'queue-memory');
        await fs.mkdir(folder, { recursive: true });
        const manifest = { id: 'queue-memory', name: 'Queue memory', version: '1.0.0', apiVersion: 3 };
        await fs.writeFile(path.join(folder, 'manifest.json'), JSON.stringify(manifest));
        await fs.writeFile(path.join(folder, 'index.ts'), 'api.render.draw();\n'.repeat(count));
        const unavailable = async () => { throw new Error('Unexpected publishing/network call'); };
        service = createPublisher({
          registry: { userDir, list: () => [{ id: manifest.id, scope: 'user', dir: folder, manifest,
            enabled: true, bundleUrl: null, bundleHash: null, health: { state: 'ok' }, updatedAt: 1 }],
            refresh: unavailable, emitChanged: () => {} },
          provenance: createProvenanceStore(path.join(directory, 'publisher-provenance')),
          api: { detail: async () => { detailCalls++; throw new ApiError({ error: 'not_found' }); },
            missing: unavailable, upload: unavailable, putRelease: unavailable, yank: unavailable, myRepos: unavailable },
          me: () => ({ publisher: { id: 'synthetic-publisher', handle: 'synthetic', tombstoned: false } }),
          signedIn: () => true, confirm: unavailable, log: () => {}
        });
      }
      // Keep the actual service and its queues reachable throughout collection,
      // independently of V8's handling of local variable liveness.
      globalThis.liveService = service;
      const weak = await inspectCaller(service);
      await collect();
      const retained = !!weak.deref();
      if (globalThis.liveService !== service || typeof service[kind === 'provenance' ? 'read' : 'prepare'] !== 'function') throw new Error('Service was lost');
      // No second queued operation may incidentally discard the old result.
      if (kind === 'publisher' && detailCalls !== 1) throw new Error('Unexpected prepare replay');
      console.log(JSON.stringify({ retained }));
    })().catch(error => { console.error(error); process.exit(1); });
  `], { cwd: directory, encoding: 'utf8', timeout: 10_000 }));
}

it('releases a provenance read graph after its caller drops it while preserving the live service', () => {
  expect(inspect('provenance')).toEqual({ retained: false });
}, 15_000);

it('releases real prepare findings after their caller drops them while preserving the live publisher', () => {
  expect(inspect('publisher')).toEqual({ retained: false });
}, 15_000);
