import { EventEmitter } from 'node:events';
import { mkdtemp, readdir, rm, type FileHandle } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import type { IpcMainInvokeEvent } from 'electron';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { IPC } from '../shared/ipc';
import { registerRemoteMediaIpc, RemoteMediaService, type Transport } from './remote-media';

const PNG = Uint8Array.from([137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 1, 2, 3, 4]);
const scratch: string[] = [];
const services: RemoteMediaService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.releaseOwner(7)));
  await Promise.all(scratch.splice(0).map(dir => rm(dir, { recursive: true, force: true })));
});
const deferred = () => {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
};

async function setup(transport?: Transport) {
  const directory = await mkdtemp(path.join(tmpdir(), 'powermove-remote-lifetime-'));
  scratch.push(directory);
  const service = new RemoteMediaService({ directory, resolve: async () => [{ address: '93.184.215.14', family: 4 }],
    transport: transport ?? (async () => ({ status: 200, headers: {}, cancel() {}, body: (async function* () { yield PNG; })() })) });
  services.push(service);
  const sender = Object.assign(new EventEmitter(), { id: 7, isDestroyed: () => false, isCrashed: () => false });
  const event = { sender } as unknown as IpcMainInvokeEvent;
  const handlers = new Map<string, (event: IpcMainInvokeEvent, ...args: any[]) => Promise<any>>();
  registerRemoteMediaIpc({ handle: (channel, handler) => { handlers.set(channel, handler as never); } }, { isTrustedSender: () => true }, service);
  const fetch = () => handlers.get(IPC.remoteMediaFetch)!(event, 'https://images.example.com/image.png');
  const read = (token: string) => handlers.get(IPC.remoteMediaRead)!(event, { token, offset: 0, length: 8 });
  const files = async () => {
    const folders = await readdir(directory);
    return (await Promise.all(folders.map(folder => readdir(path.join(directory, folder))))).flat();
  };
  const held = (token: string) => (service as unknown as { held: Map<string, { path: string; handle: FileHandle | null }> }).held.get(token)!;
  return { service, sender, event, fetch, read, files, held };
}

function lose(sender: EventEmitter, reason: 'navigation'|'crash'|'destroy') {
  if (reason === 'navigation') sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: false });
  else sender.emit(reason === 'crash' ? 'render-process-gone' : 'destroyed');
}

describe('remote download renderer document lifetime', () => {
  it.each(['navigation', 'crash', 'destroy'] as const)('closes held native files and invalidates tokens on %s', async reason => {
    const { sender, fetch, read, files, held } = await setup();
    for (let epoch = 0; epoch < (reason === 'destroy' ? 1 : 2); epoch++) {
      const info = await fetch();
      expect(await read(info.token)).toEqual(PNG.subarray(0, 8));
      const handle = held(info.token).handle!;
      expect(handle.fd).toBeGreaterThanOrEqual(0);
      lose(sender, reason);
      await vi.waitFor(async () => { expect(handle.fd).toBe(-1); expect(await files()).toEqual([]); });
      await expect(read(info.token)).rejects.toThrow('Unknown download');
    }
  });

  it('preserves a readable file through same-document and subframe navigation', async () => {
    const { sender, fetch, read, files } = await setup();
    const info = await fetch();
    sender.emit('did-start-navigation', { isMainFrame: false, isSameDocument: false });
    sender.emit('did-start-navigation', { isMainFrame: true, isSameDocument: true });
    expect(await read(info.token)).toEqual(PNG.subarray(0, 8));
    expect(await files()).toEqual([info.token]);
  });

  it.each(['navigation', 'crash', 'destroy'] as const)('discards a fetch completing after %s and keeps the next document usable', async reason => {
    const entered = deferred(), gate = deferred();
    let calls = 0;
    const { sender, fetch, read, files } = await setup(async () => {
      if (++calls === 1) { entered.resolve(); await gate.promise; }
      return { status: 200, headers: {}, cancel() {}, body: (async function* () { yield PNG; })() };
    });
    const pending = fetch().then(value => ({ value }), error => ({ error }));
    await entered.promise;
    lose(sender, reason);
    gate.resolve();
    const result = await pending;
    expect(result).toMatchObject({ error: expect.any(Error) });
    expect(await files()).toEqual([]);
    if (reason !== 'destroy') {
      const next = await fetch();
      expect(await read(next.token)).toEqual(PNG.subarray(0, 8));
      expect(await files()).toEqual([next.token]);
      lose(sender, reason);
      await vi.waitFor(async () => expect(await files()).toEqual([]));
    }
  });


});
