import { copyFile, mkdtemp, rm, writeFile, readFile, readdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import { IPC } from '../shared/ipc';
import { MAX_SEQUENCE_FRAMES } from '../shared/animated-image';
import { MAX_PROXY_CHUNK_BYTES, MediaProxyService, imageSequenceConverter, registerMediaProxyIpc, sequenceProgressReader } from './media-proxy';

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
  it('parses split FFmpeg frame updates without regressing or exceeding the frame count', () => {
    const updates: number[] = [];
    const read = sequenceProgressReader(3, completed => updates.push(completed));
    read('frame=1\nfps=0\nfra');
    read(Buffer.from('me=2\nframe=1\nframe=2\nframe=9\nprogress=end\n'));
    expect(updates).toEqual([1, 2, 3]);
  });
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
  it('converts real numbered PNGs with the bundled encoder', async () => {
    const { root } = await fixture();
    const source = path.resolve('e2e/fixtures/still-red.png');
    const files = [path.join(root, 'frame-001.png'), path.join(root, 'frame-002.png')];
    await Promise.all(files.map(file => copyFile(source, file)));
    const service = new MediaProxyService(root, async () => {}, imageSequenceConverter(path.resolve('node_modules/ffmpeg-static/ffmpeg')));
    const result = await service.createSequence(files, 24);
    expect(result.size).toBeGreaterThan(100);
    await service.release(result.token);
  }, 30_000);
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

describe('animated image conversion', () => {
  const frames = async (pattern: string, count: number) =>
    Promise.all(Array.from({ length: count }, (_, index) =>
      readFile(pattern.replace('%08d', String(index).padStart(8, '0')), 'utf8')));

  it('holds each decoded frame for its own delay and releases the clip', async () => {
    const { root } = await fixture();
    const service = new MediaProxyService(root, async () => {}, async (pattern, fps, count, output, onProgress) => {
      expect(fps).toBe(100);
      expect(count).toBe(5);
      // A frame that lasts three slots is simply written three times.
      expect(await frames(pattern, count)).toEqual(['one', 'one', 'two', 'two', 'two']);
      onProgress?.(5);
      await writeFile(output, 'animation');
    });

    const progress: number[] = [];
    const token = await service.beginAnimation(100, [2, 3]);
    await service.writeAnimationFrame(token, 0, 0, Buffer.from('one'));
    // Frames larger than one IPC chunk arrive in order and append.
    await service.writeAnimationFrame(token, 1, 0, Buffer.from('tw'));
    await service.writeAnimationFrame(token, 1, 2, Buffer.from('o'));
    expect(await service.finishAnimation(token, completed => progress.push(completed))).toEqual({ token, size: 9 });
    expect(progress).toEqual([5]);
    expect(Buffer.from(await service.read(token, 0, 9)).toString()).toBe('animation');

    await service.release(token);
    expect((await readdir(root)).filter(name => name.startsWith('powermove-animation-'))).toEqual([]);
  });

  it('rejects timing, chunks and frame counts it cannot encode, and cleans up after a failure', async () => {
    const { root } = await fixture();
    const service = new MediaProxyService(root, async () => {}, async () => { throw new Error('encode failed'); });

    await expect(service.beginAnimation(0, [1])).rejects.toThrow('Frame rate');
    await expect(service.beginAnimation(30, [])).rejects.toThrow('Invalid animation frame timing');
    await expect(service.beginAnimation(30, [0])).rejects.toThrow('Invalid animation frame timing');
    await expect(service.beginAnimation(30, [MAX_SEQUENCE_FRAMES, MAX_SEQUENCE_FRAMES]))
      .rejects.toThrow('too many frames');

    const token = await service.beginAnimation(30, [1, 1]);
    await expect(service.writeAnimationFrame(token, 2, 0, Buffer.from('x'))).rejects.toThrow('Invalid animation frame chunk');
    await expect(service.writeAnimationFrame(token, 0, 4, Buffer.from('x'))).rejects.toThrow('Invalid animation frame chunk');
    await service.writeAnimationFrame(token, 0, 0, Buffer.from('x'));
    await expect(service.finishAnimation(token)).rejects.toThrow('missing frames');
    // An unfinished upload keeps its frames so the sender can still complete it.
    await service.writeAnimationFrame(token, 1, 0, Buffer.from('x'));
    await expect(service.finishAnimation(token)).rejects.toThrow('encode failed');
    expect((await readdir(root)).filter(name => name.startsWith('powermove-animation-'))).toEqual([]);
  });

  it('turns a conversion failure into a message instead of an unhandled IPC rejection', async () => {
    const { root } = await fixture();
    const service = new MediaProxyService(root, async () => {}, async () => { throw new Error('encode failed'); });
    const handlers = new Map<string, (...args: any[]) => any>();
    registerMediaProxyIpc({ handle: (channel: string, handler: (...args: any[]) => any) => handlers.set(channel, handler) } as unknown as IpcMain,
      service, { isTrustedSender: () => true });
    const event = { sender: { isDestroyed: () => true, send: () => undefined } } as unknown as IpcMainInvokeEvent;

    const token = await handlers.get(IPC.mediaAnimationBegin)!(event, { fps: 30, repeats: [1] });
    await handlers.get(IPC.mediaAnimationFrame)!(event, { token, index: 0, offset: 0, data: Buffer.from('x') });
    expect(await handlers.get(IPC.mediaAnimationFinish)!(event, { token }))
      .toEqual({ ok: false, error: 'Could not import this animation · encode failed' });
    await expect(handlers.get(IPC.mediaAnimationFrame)!(event, { token: '../escape', index: 0, offset: 0, data: Buffer.from('x') }))
      .rejects.toThrow(IPC.mediaAnimationFrame);
  });
});

describe('still image conversion', () => {
  it('converts a format Chromium cannot decode and releases the result', async () => {
    const { root } = await fixture();
    const source = path.join(root, 'photo.heic');
    await writeFile(source, 'heif');
    const service = new MediaProxyService(root, async () => {}, undefined, undefined, async (input, extension, output) => {
      expect(await readFile(input, 'utf8')).toBe('heif');
      expect(extension).toBe('heic');
      await writeFile(output, 'png!');
    });

    const converted = await service.createStillImage(source);
    expect(Buffer.from(await service.read(converted.token, 0, 4)).toString()).toBe('png!');
    await service.release(converted.token);
    expect((await readdir(root)).filter(name => name.startsWith('powermove-image-'))).toEqual([]);
  });

  it('refuses formats that need no conversion and cleans up after a failed one', async () => {
    const { root } = await fixture();
    const png = path.join(root, 'already.png'), tiff = path.join(root, 'scan.tiff');
    await writeFile(png, 'png'); await writeFile(tiff, 'tiff');
    const service = new MediaProxyService(root, async () => {}, undefined, undefined,
      async () => { throw new Error('convert failed'); });

    await expect(service.createStillImage(png)).rejects.toThrow('does not need converting');
    await expect(service.createStillImage('scan.tiff')).rejects.toThrow('does not need converting');
    await expect(service.createStillImage(path.join(root, 'missing.tiff'))).rejects.toThrow();
    await expect(service.createStillImage(tiff)).rejects.toThrow('convert failed');
    expect((await readdir(root)).filter(name => name.startsWith('powermove-image-'))).toEqual([]);

    const unavailable = new MediaProxyService(root, async () => {});
    await expect(unavailable.createStillImage(tiff)).rejects.toThrow('unavailable');
  });
});
