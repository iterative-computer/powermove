import { EventEmitter } from 'node:events';
import { copyFile, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { afterEach, expect, it, vi } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

const io = vi.hoisted(() => ({ open: vi.fn(), writeFile: vi.fn(), handles: [] as Array<{ file: string; flags: string; handle: FileHandle }> }));
vi.mock('node:fs/promises', async importOriginal => {
  const fs = await importOriginal<typeof import('node:fs/promises')>();
  io.writeFile.mockImplementation(fs.writeFile);
  io.open.mockImplementation(async (file, flags, mode) => {
    const handle = await fs.open(file, flags, mode);
    vi.spyOn(handle, 'close');
    const read = handle.read.bind(handle), write = handle.writeFile.bind(handle);
    vi.spyOn(handle, 'read').mockImplementation(read);
    vi.spyOn(handle, 'writeFile').mockImplementation(write);
    io.handles.push({ file: String(file), flags, handle });
    return handle;
  });
  return { ...fs, open: io.open, writeFile: io.writeFile };
});

import { MediaProxyService, registerMediaProxyIpc } from './media-proxy';
import { IPC } from '../shared/ipc';

const roots: string[] = [], services: MediaProxyService[] = [], unblock: Array<() => void> = [];
afterEach(async () => {
  unblock.splice(0).forEach(resolve => resolve());
  await Promise.all(services.splice(0).map(service => service.dispose()));
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  io.handles.length = 0;
  vi.clearAllMocks();
});

async function fixture(preview?: (source: string, output: string) => Promise<void>) {
  const root = await mkdtemp(path.join(tmpdir(), 'powermove-proxy-io-test-'));
  roots.push(root);
  const source = path.join(root, 'clip.mov');
  await writeFile(source, Uint8Array.from({ length: 32 }, (_, index) => index + 1));
  const service = new MediaProxyService(root, copyFile, undefined, preview);
  services.push(service);
  return { root, source, service };
}

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  unblock.push(resolve);
  return { promise, resolve };
}

function ipc(service: MediaProxyService) {
  const handlers = new Map<string, (...args: any[]) => any>();
  registerMediaProxyIpc({ handle: (channel: string, fn: (...args: any[]) => any) => handlers.set(channel, fn) } as unknown as IpcMain,
    service, { isTrustedSender: () => true });
  let destroyed = false;
  const sender = Object.assign(new EventEmitter(), { isDestroyed: () => destroyed, send: vi.fn() });
  return {
    sender,
    call: (channel: string, value: unknown, from = sender) => handlers.get(channel)!({ sender: from } as unknown as IpcMainInvokeEvent, value),
    destroy: () => { destroyed = true; sender.emit('destroyed'); },
  };
}

it('shares one descriptor across overlapping positioned reads and closes it on release', async () => {
  const { source, service } = await fixture();
  const proxy = await service.create(source);
  const chunks = await Promise.all(Array.from({ length: 16 }, (_, index) => service.read(proxy.token, index * 2, 2)));
  expect(chunks.flatMap(chunk => [...chunk])).toEqual(Array.from({ length: 32 }, (_, index) => index + 1));
  expect(io.handles.filter(item => item.file.endsWith('playback.webm'))).toHaveLength(1);
  const handle = io.handles[0]!.handle;
  expect(handle.close).not.toHaveBeenCalled();
  for (const chunk of chunks) {
    expect(chunk.byteOffset).toBe(0);
    expect(chunk.buffer.byteLength).toBe(2);
  }
  chunks[0]![0] = 0;
  expect([...await service.read(proxy.token, 0, 2)]).toEqual([1, 2]);
  await service.release(proxy.token);
  expect(handle.close).toHaveBeenCalledTimes(1);
  expect(handle.fd).toBe(-1);
  await expect(service.read(proxy.token, 0, 1)).rejects.toThrow('expired');
});

it('appends queued preview chunks through one descriptor and closes it before conversion', async () => {
  const { service } = await fixture(async (source, output) => {
    const writer = io.handles.find(item => item.file.endsWith('/source'))!.handle;
    expect(writer.fd).toBe(-1);
    expect([...await readFile(source)]).toEqual(Array.from({ length: 32 }, (_, index) => index + 1));
    await writeFile(output, 'preview');
  });
  const token = await service.beginPreview(32);
  await Promise.all(Array.from({ length: 16 }, (_, index) => service.writePreview(token, index * 2, new Uint8Array([index * 2 + 1, index * 2 + 2]))));
  expect(io.handles.filter(item => item.file.endsWith('/source') && item.flags === 'a')).toHaveLength(1);
  expect(io.writeFile.mock.calls.filter(([file]) => String(file).endsWith('/source'))).toHaveLength(0);
  await service.finishPreview(token);
  expect(Buffer.from(await service.read(token, 0, 7)).toString()).toBe('preview');
  await service.release(token);
  expect(io.handles.every(item => item.handle.fd === -1)).toBe(true);
});

it('waits for an accepted read during release, duplicate release and process disposal', async () => {
  const { root, source, service } = await fixture();
  const proxy = await service.create(source);
  await service.read(proxy.token, 0, 1);
  const handle = io.handles[0]!.handle;
  const gate = deferred(), started = deferred();
  const read = vi.mocked(handle.read), original = read.getMockImplementation()!;
  read.mockImplementationOnce(async (...args: any[]) => {
    started.resolve(); await gate.promise;
    return original.apply(handle, args as any);
  });
  const reading = service.read(proxy.token, 3, 3);
  await started.promise;
  const releasing = service.release(proxy.token), again = service.release(proxy.token), disposing = service.dispose();
  expect(handle.close).not.toHaveBeenCalled();
  await expect(service.read(proxy.token, 0, 1)).rejects.toThrow('expired');
  gate.resolve();
  expect([...await reading]).toEqual([4, 5, 6]);
  await Promise.all([releasing, again, disposing]);
  expect(handle.close).toHaveBeenCalledTimes(1);
  expect(handle.fd).toBe(-1);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('closes a descriptor whose open was still pending when its token was released', async () => {
  const { source, service } = await fixture();
  const proxy = await service.create(source);
  const gate = deferred(), started = deferred(), original = io.open.getMockImplementation()!;
  io.open.mockImplementationOnce(async (...args) => { started.resolve(); await gate.promise; return original(...args); });
  const reading = service.read(proxy.token, 0, 3);
  await started.promise;
  const releasing = service.release(proxy.token);
  gate.resolve();
  expect([...await reading]).toEqual([1, 2, 3]);
  await releasing;
  expect(io.handles).toHaveLength(1);
  expect(io.handles[0]!.handle.fd).toBe(-1);
});

it('retries failed opens and reads without returning shared, padded or pooled bytes', async () => {
  const { source, service } = await fixture();
  const proxy = await service.create(source);
  io.open.mockRejectedValueOnce(new Error('temporary open failure'));
  await expect(service.read(proxy.token, 0, 2)).rejects.toThrow('temporary open failure');
  expect([...await service.read(proxy.token, 0, 2)]).toEqual([1, 2]);
  const { handle, file } = io.handles[0]!;
  vi.mocked(handle.read).mockRejectedValueOnce(new Error('temporary read failure'));
  await expect(service.read(proxy.token, 0, 2)).rejects.toThrow('temporary read failure');
  expect([...await service.read(proxy.token, 2, 2)]).toEqual([3, 4]);
  await writeFile(file, new Uint8Array([8, 9, 10]));
  const short = await service.read(proxy.token, 1, 10);
  expect([...short]).toEqual([9, 10]);
  expect(short.buffer.byteLength).toBe(2);
  expect((await service.read(proxy.token, 10, 1)).buffer.byteLength).toBe(0);
  await service.release(proxy.token);
  expect(handle.fd).toBe(-1);
});

it('finishes already queued appends before cancelling an upload', async () => {
  const { root, service } = await fixture(async () => {});
  const token = await service.beginPreview(4);
  const gate = deferred(), started = deferred(), original = io.open.getMockImplementation()!;
  io.open.mockImplementationOnce(async (...args) => {
    const handle = await original(...args);
    const write = vi.mocked(handle.writeFile), nativeWrite = write.getMockImplementation()!;
    write.mockImplementationOnce(async (...data: any[]) => {
      started.resolve(); await gate.promise; return nativeWrite.apply(handle, data as any);
    });
    return handle;
  });
  const first = service.writePreview(token, 0, new Uint8Array([1, 2]));
  const second = service.writePreview(token, 2, new Uint8Array([3, 4]));
  await started.promise;
  const releasing = service.release(token);
  expect(io.handles[0]!.handle.close).not.toHaveBeenCalled();
  await expect(service.writePreview(token, 4, new Uint8Array([5]))).rejects.toThrow('Invalid');
  gate.resolve();
  await Promise.all([first, second, releasing]);
  expect(io.handles[0]!.handle.writeFile).toHaveBeenCalledTimes(2);
  expect(io.handles[0]!.handle.close).toHaveBeenCalledTimes(1);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('cleans a preview writer immediately when an append fails', async () => {
  const { root, service } = await fixture(async () => {});
  const token = await service.beginPreview(2);
  await service.writePreview(token, 0, new Uint8Array([1]));
  const handle = io.handles[0]!.handle;
  vi.mocked(handle.writeFile).mockRejectedValueOnce(new Error('disk full'));
  await expect(service.writePreview(token, 1, new Uint8Array([2]))).rejects.toThrow('disk full');
  expect(handle.close).toHaveBeenCalledTimes(1);
  expect(handle.fd).toBe(-1);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('settles a failed append, waiting finish, release and disposal without a cleanup deadlock', async () => {
  const { root, service } = await fixture(async () => {});
  const token = await service.beginPreview(1);
  const gate = deferred(), started = deferred(), original = io.open.getMockImplementation()!;
  io.open.mockImplementationOnce(async (...args) => {
    const handle = await original(...args);
    vi.mocked(handle.writeFile).mockImplementationOnce(async () => {
      started.resolve(); await gate.promise; throw new Error('disk full');
    });
    return handle;
  });
  const rejectedWrite = expect(service.writePreview(token, 0, new Uint8Array([1]))).rejects.toThrow('disk full');
  const rejectedFinish = expect(service.finishPreview(token)).rejects.toThrow('disk full');
  await started.promise;
  const releasing = service.release(token), disposing = service.dispose();
  gate.resolve();
  await Promise.all([rejectedWrite, rejectedFinish, releasing, disposing]);
  expect(io.handles[0]!.handle.close).toHaveBeenCalledTimes(1);
  expect(io.handles[0]!.handle.fd).toBe(-1);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('retains the WebM signature and source suffix with a reused preview writer', async () => {
  const prefix = new Uint8Array([0x1a, 0x45, 0xdf, 0xa3]);
  const { service } = await fixture(async (source, output) => {
    expect(source.endsWith('/source.webm')).toBe(true);
    expect([...await readFile(source)]).toEqual([...prefix]);
    await writeFile(output, 'preview');
  });
  const token = await service.beginPreview(4);
  await service.writePreview(token, 0, prefix.subarray(0, 2));
  await service.writePreview(token, 2, prefix.subarray(2));
  await service.finishPreview(token);
  await service.release(token);
  expect(io.handles.every(item => item.handle.fd === -1)).toBe(true);
});

it('waits for a cancelled conversion without resurrecting its expired token', async () => {
  const gate = deferred(), started = deferred();
  const { root, service } = await fixture(async (source, output) => {
    started.resolve(); await gate.promise;
    expect([...await readFile(source)]).toEqual([1]);
    await writeFile(output, 'preview');
  });
  const token = await service.beginPreview(1);
  await service.writePreview(token, 0, new Uint8Array([1]));
  const finished = service.finishPreview(token);
  const rejected = expect(finished).rejects.toThrow('expired');
  await started.promise;
  const releasing = service.release(token);
  expect((await readdir(root)).some(file => file.startsWith('powermove-preview-'))).toBe(true);
  gate.resolve();
  await Promise.all([releasing, rejected]);
  expect(await readdir(root)).toEqual(['clip.mov']);
  expect(io.handles.every(item => item.handle.fd === -1)).toBe(true);
});

it('releases cached readers and partial writers when their window is destroyed', async () => {
  const { root, source, service } = await fixture(async () => {});
  const bridge = ipc(service);
  const proxy = await bridge.call(IPC.mediaProxyCreate, { sourcePath: source, name: 'clip.mov' });
  await bridge.call(IPC.mediaProxyRead, { token: proxy.token, offset: 0, length: 3 });
  const upload = await bridge.call(IPC.mediaPreviewBegin, 4);
  await bridge.call(IPC.mediaPreviewChunk, { token: upload, offset: 0, data: new Uint8Array([1, 2]) });
  expect(bridge.sender.listenerCount('destroyed')).toBe(1);
  bridge.destroy();
  await service.dispose();
  expect(io.handles.every(item => item.handle.fd === -1)).toBe(true);
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('detaches the window listener on explicit release and failed conversion', async () => {
  const { source, service } = await fixture(async () => { throw new Error('decode failed'); });
  const bridge = ipc(service);
  const proxy = await bridge.call(IPC.mediaProxyCreate, { sourcePath: source, name: 'clip.mov' });
  await bridge.call(IPC.mediaProxyRelease, proxy.token);
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  const upload = await bridge.call(IPC.mediaPreviewBegin, 1);
  await bridge.call(IPC.mediaPreviewChunk, { token: upload, offset: 0, data: new Uint8Array([1]) });
  await expect(bridge.call(IPC.mediaPreviewFinish, upload)).rejects.toThrow('decode failed');
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  expect(io.handles.every(item => item.handle.fd === -1)).toBe(true);
});

it('preserves capability reads and release from another trusted window', async () => {
  const { source, service } = await fixture();
  const bridge = ipc(service);
  const another = Object.assign(new EventEmitter(), { isDestroyed: () => false, send: vi.fn() });
  const proxy = await bridge.call(IPC.mediaProxyCreate, { sourcePath: source, name: 'clip.mov' });
  expect([...await bridge.call(IPC.mediaProxyRead, { token: proxy.token, offset: 5, length: 2 }, another)]).toEqual([6, 7]);
  await bridge.call(IPC.mediaProxyRelease, proxy.token, another);
  expect(io.handles[0]!.handle.fd).toBe(-1);
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
});

it('cleans descriptors, files and window listeners even when an earlier release observer throws', async () => {
  const { root, source, service } = await fixture();
  service.onRelease(() => { throw new Error('observer failed'); });
  const bridge = ipc(service);
  const proxy = await bridge.call(IPC.mediaProxyCreate, { sourcePath: source, name: 'clip.mov' });
  await bridge.call(IPC.mediaProxyRead, { token: proxy.token, offset: 0, length: 1 });
  await expect(bridge.call(IPC.mediaProxyRelease, proxy.token)).rejects.toThrow('observer failed');
  expect(io.handles[0]!.handle.fd).toBe(-1);
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('keeps cleanup visible to disposal reentered from a release observer', async () => {
  const { root, source, service } = await fixture();
  const proxy = await service.create(source);
  await service.read(proxy.token, 0, 1);
  const handle = io.handles[0]!.handle, gate = deferred(), started = deferred();
  const read = vi.mocked(handle.read), nativeRead = read.getMockImplementation()!;
  read.mockImplementationOnce(async (...args: any[]) => {
    started.resolve(); await gate.promise; return nativeRead.apply(handle, args as any);
  });
  const reading = service.read(proxy.token, 1, 1);
  await started.promise;
  let disposed = false, disposing: Promise<void> | undefined;
  service.onRelease(() => { disposing = service.dispose().then(() => { disposed = true; }); });
  const releasing = service.release(proxy.token);
  await Promise.resolve(); await Promise.resolve();
  expect(disposed).toBe(false);
  expect(handle.close).not.toHaveBeenCalled();
  gate.resolve();
  await Promise.all([reading, releasing, disposing]);
  expect(disposed).toBe(true);
  expect(handle.fd).toBe(-1);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('keeps animation conversion files alive during cancellation and then removes them', async () => {
  const { root } = await fixture();
  const gate = deferred(), started = deferred();
  const service = new MediaProxyService(root, copyFile, async (pattern, _fps, _count, output) => {
    started.resolve(); await gate.promise;
    expect(await readFile(pattern.replace('%08d', '00000000'), 'utf8')).toBe('frame');
    await writeFile(output, 'animation');
  });
  services.push(service);
  const token = await service.beginAnimation(30, [1]);
  await service.writeAnimationFrame(token, 0, 0, Buffer.from('frame'));
  const finishing = service.finishAnimation(token);
  const rejected = expect(finishing).rejects.toThrow('expired');
  await started.promise;
  const releasing = service.release(token);
  gate.resolve();
  await Promise.all([releasing, rejected]);
  expect(await readdir(root)).toEqual(['clip.mov']);
});

it('cleans imports that complete after their window closes or the service is disposed', async () => {
  const { root, source } = await fixture();
  for (const closed of ['window', 'service']) {
    const gate = deferred(), started = deferred();
    const service = new MediaProxyService(root, async (input, output) => {
      started.resolve(); await gate.promise; await copyFile(input, output);
    });
    services.push(service);
    const bridge = ipc(service);
    const creating = bridge.call(IPC.mediaProxyCreate, { sourcePath: source, name: 'clip.mov' });
    await started.promise;
    if (closed === 'window') bridge.destroy(); else await service.dispose();
    gate.resolve();
    expect(await creating).toMatchObject({ ok: false });
    expect(await readdir(root)).toEqual(['clip.mov']);
    expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  }
});

it('does not adopt a token released between service creation and its IPC reply', async () => {
  const { root, service } = await fixture(async () => {});
  const gate = deferred(), created = deferred(), begin = service.beginPreview.bind(service);
  vi.spyOn(service, 'beginPreview').mockImplementationOnce(async total => {
    const token = await begin(total);
    created.resolve(); await gate.promise; return token;
  });
  const bridge = ipc(service);
  const rejected = expect(bridge.call(IPC.mediaPreviewBegin, 1)).rejects.toThrow('expired');
  await created.promise;
  await service.dispose();
  gate.resolve();
  await rejected;
  expect(bridge.sender.listenerCount('destroyed')).toBe(0);
  expect(await readdir(root)).toEqual(['clip.mov']);
});
