/*
 * The browser's reconnecting link against a real host: drop the socket, and
 * the next call goes out on a fresh one without the caller noticing.
 */
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { WebSocket } from 'ws';

// Main's modules import electron; the serve bundle aliases it to the stub, and so does this test.
vi.mock('electron', () => import('./electron-stub'));

import { Connection, type LinkSocket } from '../shared/link';
import { IPC } from '../shared/ipc';
import { WEB, type WebHello } from '../shared/wire';
import { ReconnectingLink } from '../shared/reconnect';
import { serve, type RunningServer } from './index';

let dir: string;
let server: RunningServer;
const sockets: WebSocket[] = [];

function open(): Promise<LinkSocket> {
  return new Promise((resolve, reject) => {
    const url = new URL(server.url);
    const socket = new WebSocket(`ws://${url.host}/__powermove/ws`, { headers: { cookie: `pm_session=${server.token}` } });
    sockets.push(socket);
    socket.once('open', () => resolve(socket as unknown as LinkSocket));
    socket.once('error', reject);
  });
}

beforeAll(async () => {
  process.env['POWERMOVE_FONTCONFIG'] = '0';
  dir = await mkdtemp(path.join(tmpdir(), 'pm-reconnect-'));
  server = await serve({
    host: '127.0.0.1', port: 0, userData: path.join(dir, 'profile'), exportsDir: path.join(dir, 'exports'),
    rendererDir: path.join(dir, 'renderer'), resourcesDir: path.join(dir, 'resources'), appPath: dir,
    version: '0.0.0-test', codexBinary: null, claudeBinary: null, engineScript: null, installKind: 'source', insecure: true,
    log: () => undefined
  });
}, 30_000);

afterAll(async () => {
  for (const socket of sockets) socket.terminate();
  await server.close();
  await rm(dir, { recursive: true, force: true });
});

describe('reconnecting link against the host', () => {
  it('carries a call across a dropped socket and lands queued store writes', async () => {
    const link = new ReconnectingLink({ open, minDelayMs: 10, maxDelayMs: 50 });
    link.adopt(new Connection(await open()));
    const hello = await link.invoke<WebHello>(WEB.hello);
    expect(hello.version).toBe('0.0.0-test');

    const reconnected = new Promise<void>((resolve) => link.on('__reconnected', () => resolve()));
    const dropped = new Promise<void>((resolve) => link.on('__closed', () => resolve()));
    sockets.at(-1)!.terminate();
    await dropped;
    expect(link.isConnected).toBe(false);
    // Written while the socket is down: must arrive on the next one, before the read.
    link.send(IPC.storeSet, { key: 'projectState.reconnect-test', value: { ok: true } });
    const value = await link.sync<string | null>(IPC.storeGetSync, { key: 'projectState.reconnect-test' });
    await reconnected;
    expect(link.isConnected).toBe(true);
    expect(JSON.parse(value ?? 'null')).toEqual({ ok: true });
    expect(sockets).toHaveLength(2);
    link.close();
  });

  it('accepts the resume form of run attachment', async () => {
    const link = new ReconnectingLink({ open });
    link.adopt(new Connection(await open()));
    await expect(link.invoke(WEB.runsAttach, { runId: 'nope', since: 3 })).rejects.toThrow(/not on this host/);
    await expect(link.invoke(WEB.runsAttach, 'nope')).rejects.toThrow(/not on this host/);
    link.close();
  });
});
