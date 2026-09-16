import { copyFile, mkdtemp, rm, writeFile, readFile, readdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import { IPC } from '../shared/ipc';
import { MAX_PROXY_CHUNK_BYTES, MediaProxyService, registerMediaProxyIpc } from './media-proxy';

const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

async function fixture(): Promise<{ root: string; source: string }> {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-media-proxy-test-'));
  roots.push(root);
  const source = path.join(root, 'source.mov');
  await writeFile(source, Uint8Array.from({ length: 23 }, (_, index) => index + 1));
  return { root, source };
}

describe('media playback proxies', () => {
  it('reads a converted file in bounded chunks and removes it on release', async () => {
    const { root, source } = await fixture();
    const service = new MediaProxyService(root, (input, output) => copyFile(input, output));
    const proxy = await service.create(source);

    expect(proxy.size).toBe(23);
    expect([...await service.read(proxy.token, 0, 7)]).toEqual([1, 2, 3, 4, 5, 6, 7]);
    expect([...await service.read(proxy.token, 20, 7)]).toEqual([21, 22, 23]);
    await expect(service.read(proxy.token, 0, MAX_PROXY_CHUNK_BYTES + 1)).rejects.toThrow('Invalid playback proxy range');

    await service.release(proxy.token);
    await expect(service.read(proxy.token, 0, 1)).rejects.toThrow('expired');
  });

  it('rejects untrusted and malformed IPC requests before touching media', async () => {
    const { root } = await fixture();
    const service = new MediaProxyService(root, async () => undefined);
    const handlers = new Map<string, (...args: any[]) => any>();
    const ipcMain = {
      handle: (channel: string, handler: (...args: any[]) => any) => handlers.set(channel, handler)
    } as unknown as IpcMain;
    const event = {} as IpcMainInvokeEvent;

    registerMediaProxyIpc(ipcMain, service, { isTrustedSender: () => false });
    await expect(handlers.get(IPC.mediaProxyCreate)?.(event, { sourcePath: '/tmp/video.mov', name: 'video.mov' }))
      .rejects.toThrow('Unauthorized IPC sender');

    const trustedHandlers = new Map<string, (...args: any[]) => any>();
    registerMediaProxyIpc({
      handle: (channel: string, handler: (...args: any[]) => any) => trustedHandlers.set(channel, handler)
    } as unknown as IpcMain, service, { isTrustedSender: () => true });
    await expect(trustedHandlers.get(IPC.mediaProxyRead)?.(event, {
      token: '../not-a-token', offset: 0, length: 1
    })).rejects.toThrow('media-proxy:read');
  });
});


describe('native image sequence conversion', () => {
  it('packs sparse local frames consecutively and releases the generated clip', async () => {
    const { root } = await fixture();
    const first = path.join(root, "shot's 009.png"), second = path.join(root, "shot's 11.png");
    await writeFile(first, 'first'); await writeFile(second, 'second');
    const service = new MediaProxyService(root, async () => {}, async (pattern, fps, count, output) => {
      expect(fps).toBe(23.976); expect(count).toBe(2);
      expect(await readFile(pattern.replace('%08d', '00000000'), 'utf8')).toBe('first');
      expect(await readFile(pattern.replace('%08d', '00000001'), 'utf8')).toBe('second');
      await writeFile(output, 'sequence');
    });
    const result = await service.createSequence([second, first], 23.976);
    expect(Buffer.from(await service.read(result.token, 0, 8)).toString()).toBe('sequence');
    await service.release(result.token);
    expect((await readdir(root)).filter(name => name.startsWith('powermove-image-sequence-'))).toEqual([]);
  });
  it('rejects invalid inputs and cleans temporary files after failed conversion', async () => {
    const { root } = await fixture();
    const files = [path.join(root, 'f1.png'), path.join(root, 'f2.png')];
    for (const file of files) await writeFile(file, 'frame');
    const service = new MediaProxyService(root, async () => {}, async () => { throw new Error('decode failed'); });
    await expect(service.createSequence(files, 0)).rejects.toThrow('Frame rate');
    await expect(service.createSequence(['relative1.png', 'relative2.png'], 30)).rejects.toThrow('local');
    await expect(service.createSequence([files[0]!, path.join(root, 'f01.png')], 30)).rejects.toThrow('Duplicate frame');
    await expect(service.createSequence(files, 30)).rejects.toThrow('decode failed');
    expect((await readdir(root)).filter(name => name.startsWith('powermove-image-sequence-'))).toEqual([]);
    const handlers = new Map<string, (...args: any[]) => any>();
    registerMediaProxyIpc({ handle: (channel: string, handler: (...args: any[]) => any) => handlers.set(channel, handler) } as unknown as IpcMain,
      service, { isTrustedSender: () => false });
    await expect(handlers.get(IPC.mediaSequenceCreate)!({}, { sourcePaths: files, fps: 30 })).rejects.toThrow('Unauthorized');
  });
});

describe('editing preview uploads', () => {
  it('streams the original in ordered chunks and releases both source and preview', async () => {
    const { root } = await fixture();
    const before = await readdir(root);
    const convert = async (source: string, output: string) => {
      expect([...await readFile(source)]).toEqual([1, 2, 3, 4]);
      await writeFile(output, new Uint8Array([9, 8]));
    };
    const service = new MediaProxyService(root, convert, undefined, convert);
    const token = await service.beginPreview(4);
    await expect(service.writePreview(token, 1, new Uint8Array([1]))).rejects.toThrow('Invalid');
    await service.writePreview(token, 0, new Uint8Array([1, 2]));
    await expect(service.finishPreview(token)).rejects.toThrow('Incomplete');
    await service.writePreview(token, 2, new Uint8Array([3, 4]));
    expect(await service.finishPreview(token)).toEqual({ token, size: 2 });
    expect([...await service.read(token, 0, 2)]).toEqual([9, 8]);
    await service.release(token);
    expect(await readdir(root)).toEqual(before);
  });

  it('cleans up a cancelled upload and a failed conversion', async () => {
    const { root } = await fixture();
    const before = await readdir(root);
    const fail = async () => { throw Error('conversion failed'); };
    const service = new MediaProxyService(root, fail, undefined, fail);
    const cancelled = await service.beginPreview(4);
    await service.writePreview(cancelled, 0, new Uint8Array([1, 2]));
    await service.release(cancelled);
    const failed = await service.beginPreview(1);
    await service.writePreview(failed, 0, new Uint8Array([1]));
    await expect(service.finishPreview(failed)).rejects.toThrow('conversion failed');
    expect(await readdir(root)).toEqual(before);
  });
});


it('allows preview uploads above 1 GB while retaining chunk validation and cleanup', async () => {
  const { root } = await fixture();
  const before = await readdir(root);
  const service = new MediaProxyService(root, async () => {}, undefined, async () => {});
  try {
    const token = await service.beginPreview(1024 * 1024 * 1024 + 1);
    await service.writePreview(token, 0, new Uint8Array([1]));
    await expect(service.finishPreview(token)).rejects.toThrow('Incomplete');
    await expect(service.beginPreview(Infinity)).rejects.toThrow('Invalid');
  } finally { await service.dispose(); }
  expect(await readdir(root)).toEqual(before);
});
