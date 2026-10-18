/*
 * The serve supervisor's host services (engine/main.ts linkServices) against
 * a real serve host: the pool's workers read and write the profile store and
 * the project session over the engine's link, a tab on the same project
 * sees a worker's edits as session patches, and the worker sees the tab's.
 */
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { WebSocket } from 'ws';

vi.mock('electron', () => import('../../server/electron-stub'));

import { Connection, type LinkSocket } from '../../shared/link';
import { IPC } from '../../shared/ipc';
import { WEB } from '../../shared/wire';
import { serve, type RunningServer } from '../../server/index';
import { linkServices } from '../main';
import { DocumentHostPool } from './pool';
import { INSTANCE_STATE_KEY } from './protocol';
import { BRIDGE, FIXTURE, WASM, buildEngineBundle } from './test-host';

let dir: string;
let server: RunningServer;
let bundle: { script: string; dispose(): void } | null = null;
const sockets: WebSocket[] = [];

function connect(query = ''): Promise<Connection> {
  return new Promise((resolve, reject) => {
    const url = new URL(server.url);
    const socket = new WebSocket(`ws://${url.host}/__powermove/ws${query}`, { headers: { cookie: `pm_session=${server.token}` } });
    sockets.push(socket);
    socket.once('open', () => resolve(new Connection(socket as unknown as LinkSocket)));
    socket.once('error', reject);
  });
}

beforeAll(async () => {
  process.env['POWERMOVE_FONTCONFIG'] = '0';
  bundle = await buildEngineBundle();
  dir = await mkdtemp(path.join(tmpdir(), 'pm-dochost-serve-'));
  server = await serve({
    host: '127.0.0.1', port: 0, userData: path.join(dir, 'profile'), exportsDir: path.join(dir, 'exports'),
    rendererDir: path.join(dir, 'renderer'), resourcesDir: path.join(dir, 'resources'), appPath: dir, ffmpeg: 'ffmpeg',
    version: '0.0.0-test', codexBinary: null, claudeBinary: null, engineScript: null, installKind: 'source', insecure: true,
    log: () => undefined,
  });
}, 180_000);

afterAll(async () => {
  for (const socket of sockets) socket.terminate();
  await server?.close();
  if (dir) await rm(dir, { recursive: true, force: true });
  bundle?.dispose();
});

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('the document host behind powermove serve', () => {
  it('shares the session with a tab and persists to the host profile store', async () => {
    const tab = await connect();
    await tab.invoke(WEB.hello);
    const project = { ...structuredClone(FIXTURE), id: 'Pserve', name: 'Served' };
    await tab.invoke(WEB.syncJoin, { projectId: 'Pserve', doc: project });

    const engine = await connect('?engine=1');
    await engine.invoke(WEB.hello);
    const pool = new DocumentHostPool({ services: linkServices(engine, () => {}), newEngine: true, script: bundle!.script, env: { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE } });
    try {
      const tabPatches: any[] = [];
      tab.on(WEB.syncPatch, (patch) => tabPatches.push(patch));
      const added = await pool.call('Pserve', { runId: 'run-s', callId: 's1', tool: 'apply_commands', arguments: { commands: [{ type: 'add_layer', layerType: 'shape', name: 'From the worker' }], label: 'serve' }, baseRevision: 0 });
      expect(added.error ?? null).toBeNull();
      expect(tabPatches.length).toBeGreaterThan(0);
      expect(tabPatches.every((p) => p.projectId === 'Pserve')).toBe(true);
      await pool.call('Pserve', { runId: 'run-s', callId: 's2', tool: '__finish_run', arguments: { commit: true }, baseRevision: added.revision ?? 0 });

      // The tab renames the project; the worker sees it on its next read.
      await tab.invoke(WEB.syncPatch, { projectId: 'Pserve', patches: [{ path: ['name'], exists: true, value: 'Renamed in the tab' }] });
      await new Promise((resolve) => setTimeout(resolve, 300));
      const state = await pool.call('Pserve', { runId: 'run-t', callId: 's3', tool: 'get_project_state', arguments: {}, baseRevision: 0 });
      expect(JSON.stringify(state.content)).toContain('Renamed in the tab');
      expect(JSON.stringify(state.content)).toContain('From the worker');

      const saved = JSON.parse((await engine.sync<string | null>(IPC.storeGetSync, { key: INSTANCE_STATE_KEY('Pserve') })) ?? 'null');
      expect(saved?.history?.entries?.map((e: any) => e.label)).toEqual(['Agent · serve']);
    } finally {
      await pool.shutdown();
    }
  }, 120_000);
});
