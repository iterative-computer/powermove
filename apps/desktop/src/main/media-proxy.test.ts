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
