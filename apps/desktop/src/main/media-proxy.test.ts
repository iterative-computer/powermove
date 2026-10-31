import { chmod, copyFile, mkdir, mkdtemp, rm, writeFile, readFile, readdir } from 'node:fs/promises';
import { EventEmitter } from 'node:events';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';

import { IPC } from '../shared/ipc';
import { MAX_SEQUENCE_FRAMES } from '../shared/animated-image';
import { execFileSync } from 'node:child_process';
import {
  MAX_PROXY_CHUNK_BYTES, MediaProxyService, hdrProxyHelper, imageSequenceConverter, playbackConverter, playbackProxyArgs, previewArgs,
  probeVideoStream, registerMediaProxyIpc, sequenceProgressReader
} from './media-proxy';

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

  it('shares one in-flight conversion per unchanged source and gives each caller its own file', async () => {
    const { root, source } = await fixture();
    let conversions = 0, finish!: () => void;
    const gate = new Promise<void>(resolve => { finish = resolve; });
    const service = new MediaProxyService(root, async (input, output) => { conversions++; await gate; await copyFile(input, output); });
    const first = service.create(source), second = service.create(source);
    await new Promise(resolve => setTimeout(resolve, 20));
    finish();
    const [a, b] = await Promise.all([first, second]);
    expect(conversions).toBe(1);
    expect(a.token).not.toBe(b.token);
    await service.release(a.token);
    expect([...await service.read(b.token, 0, 3)]).toEqual([1, 2, 3]);
    await service.release(b.token);
    // Finished work is not cached: a later open converts the current file again.
    await writeFile(source, Uint8Array.from([9, 8, 7]));
    const later = await service.create(source);
    expect(conversions).toBe(2);
    expect([...await service.read(later.token, 0, 3)]).toEqual([9, 8, 7]);
    await service.release(later.token);
    expect((await readdir(root)).filter(name => name.startsWith('powermove-media-proxy-'))).toEqual([]);
  });

  it('cancels a conversion once every caller has gone, and on dispose', async () => {
    const { root, source } = await fixture();
    const signals: AbortSignal[] = [];
    const convert = (_input: string, _output: string, signal?: AbortSignal) => new Promise<void>((_, reject) => {
      signals.push(signal!);
      signal!.addEventListener('abort', () => reject(new Error('killed')), { once: true });
    });
    const service = new MediaProxyService(root, convert);
    const one = new AbortController(), two = new AbortController();
    const first = service.create(source, one.signal), second = service.create(source, two.signal);
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(signals).toHaveLength(1);
    one.abort(new Error('window closed'));
    await expect(first).rejects.toThrow('window closed');
    expect(signals[0]!.aborted).toBe(false);
    two.abort(new Error('window closed'));
    await expect(second).rejects.toThrow('window closed');
    expect(signals[0]!.aborted).toBe(true);

    const pending = service.create(source);
    await new Promise(resolve => setTimeout(resolve, 20));
    await service.dispose();
    await expect(pending).rejects.toThrow('killed');
    expect(signals[1]!.aborted).toBe(true);
    expect((await readdir(root)).filter(name => name.startsWith('powermove-media-proxy-'))).toEqual([]);
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
    const event = { sender: Object.assign(new EventEmitter(), { isDestroyed: () => false, send: () => undefined }) } as unknown as IpcMainInvokeEvent;

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

describe('graded footage proxies', () => {
  const ffmpeg = path.resolve('node_modules/ffmpeg-static/ffmpeg');
  const opaque = { index: 0, codec: 'hevc', alpha: false };
  const option = (args: string[], name: string) => args[args.indexOf(name) + 1];

  it('drops the alpha plane for opaque footage and keeps it for cutouts or unknown sources', () => {
    expect(option(playbackProxyArgs('in.mov', 'out.webm', opaque), '-pix_fmt')).toBe('yuv420p');
    expect(option(playbackProxyArgs('in.mov', 'out.webm', { ...opaque, codec: 'prores', alpha: true }), '-pix_fmt')).toBe('yuva420p');
    expect(option(playbackProxyArgs('in.mov', 'out.webm', undefined), '-pix_fmt')).toBe('yuva420p');
    expect(option(previewArgs('in.mov', 'out.webm', opaque), '-vf')).toMatch(/,format=yuv420p$/);
    expect(previewArgs('in.webm', 'out.webm', { ...opaque, codec: 'vp9', alpha: true }).slice(4, 6)).toEqual(['-c:v', 'libvpx-vp9']);
  });

  it('tone maps HDR, preserves log code values and leaves Rec.709 untouched', () => {
    const hdr = playbackProxyArgs('in.mov', 'out.webm', { ...opaque, color: { matrix: 'bt2020nc', primaries: 'bt2020', transfer: 'arib-std-b67' } });
    expect(option(hdr, '-vf')).toMatch(/^zscale=t=linear:npl=203:p=bt709,.*tonemap=tonemap=mobius.*,format=yuv420p$/);
    expect(option(hdr, '-color_trc')).toBe('bt709');
    const cutout = playbackProxyArgs('in.mov', 'out.webm', { ...opaque, alpha: true, color: { transfer: 'smpte2084' } });
    expect(option(cutout, '-vf')).toMatch(/format=gbrapf32le.*,format=yuva420p$/);
    const log = playbackProxyArgs('in.mov', 'out.webm', { ...opaque, color: { matrix: 'bt2020nc', primaries: 'bt2020', transfer: 'bt709' } });
    expect(option(log, '-vf')).toBe('zscale=m=bt709:r=tv,format=yuv420p');
    expect(option(log, '-color_primaries')).toBe('bt709');
    const sdr = playbackProxyArgs('in.mov', 'out.webm', { ...opaque, color: { matrix: 'bt709', primaries: 'bt709', transfer: 'bt709' } });
    expect(sdr).not.toContain('-vf');
    expect(sdr).not.toContain('-color_trc');
  });

  const HLG = ['-vf', 'format=gbrp,zscale=rangein=full:tin=iec61966-2-1:t=arib-std-b67:pin=bt709:p=bt2020:min=gbr:m=bt2020nc:r=tv:npl=203,format=yuv420p10le',
    '-c:v', 'libx265', '-tag:v', 'hvc1', '-x265-params', 'log-level=error', '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67', '-colorspace', 'bt2020nc'];
  const TONEMAP = 'zscale=t=linear:npl=203:p=bt709,format=gbrpf32le,tonemap=tonemap=mobius:param=0.9:peak=4.926:desat=0,zscale=t=bt709:m=bt709:r=tv';
  /** Planar Y'CbCr of one frame, with chroma upsampled to full size. */
  const ycbcr = (file: string, filter = 'null') => execFileSync(ffmpeg, ['-v', 'error', '-i', file, '-frames:v', '1',
    '-vf', `${filter},format=yuv444p`, '-f', 'rawvideo', '-']);

  it('converts real HLG footage into tagged Rec.709 at SDR white', async () => {
    const { root } = await fixture();
    const source = path.join(root, 'hlg.mov'), output = path.join(root, 'out.webm');
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=white:s=64x36:r=30:d=0.2', ...HLG, source]);
    expect((await probeVideoStream(ffmpeg, source))?.color).toMatchObject({ transfer: 'arib-std-b67', primaries: 'bt2020' });
    await playbackConverter(ffmpeg)(source, output);
    const converted = await probeVideoStream(ffmpeg, output);
    expect(converted).toMatchObject({ codec: 'vp9', alpha: false, pixelFormat: 'yuv420p', color: { matrix: 'bt709', primaries: 'bt709', transfer: 'bt709' } });
    const white = execFileSync(ffmpeg, ['-v', 'error', '-i', output, '-frames:v', '1', '-vf', 'scale=out_range=full,format=gray', '-f', 'rawvideo', '-']);
    // Diffuse white lands just under SDR white instead of Chromium's dim grey.
    expect(white[white.length >> 1]!).toBeGreaterThan(235);
  }, 30_000);

  it('hands opaque HDR footage to the GPU helper and falls back to FFmpeg when it fails', async () => {
    const { root } = await fixture();
    const hdr = path.join(root, 'hlg.mov'), sdr = path.join(root, 'sdr.mp4'), calls = path.join(root, 'calls');
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=white:s=64x36:r=30:d=0.2', ...HLG, hdr]);
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'lavfi', '-i', 'color=c=white:s=64x36:r=30:d=0.2', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', sdr]);
    const helper = async (name: string, body: string) => {
      const file = path.join(root, name);
      await writeFile(file, `#!/bin/sh\necho "$1" >> "${calls}"\n${body}\n`); await chmod(file, 0o755);
      return file;
    };
    const working = await helper('working', 'printf mp4 > "$2"'), broken = await helper('broken', 'printf partial > "$2"; exit 3');
    const gpu = path.join(root, 'gpu'), fallback = path.join(root, 'fallback'), plain = path.join(root, 'plain');
    await Promise.all([gpu, fallback, plain].map(dir => mkdir(dir)));
    await expect(playbackConverter(ffmpeg, working)(hdr, path.join(gpu, 'playback.webm')))
      .resolves.toEqual({ file: path.join(gpu, 'playback.mp4'), type: 'video/mp4' });
    expect(await readdir(gpu)).toEqual(['playback.mp4']);

    await expect(playbackConverter(ffmpeg, broken)(hdr, path.join(fallback, 'playback.webm'))).resolves.toBeUndefined();
    expect(await readdir(fallback)).toEqual(['playback.webm']);
    expect(await probeVideoStream(ffmpeg, path.join(fallback, 'playback.webm'))).toMatchObject({ codec: 'vp9', color: { transfer: 'bt709' } });

    await playbackConverter(ffmpeg, working)(sdr, path.join(plain, 'playback.webm'));
    expect(await readdir(plain)).toEqual(['playback.webm']);
    expect((await readFile(calls, 'utf8')).trim().split('\n')).toEqual([hdr, hdr]);
  }, 30_000);

  it('serves an MP4 proxy written by the converter with its own type', async () => {
    const { root, source } = await fixture();
    const service = new MediaProxyService(root, async (_input, output) => {
      const file = path.join(path.dirname(output), 'playback.mp4');
      await writeFile(file, 'mp4');
      return { file, type: 'video/mp4' as const };
    });
    const proxy = await service.create(source);
    expect(proxy).toMatchObject({ size: 3, type: 'video/mp4' });
    expect(Buffer.from(await service.read(proxy.token, 0, 3)).toString()).toBe('mp4');
    await service.dispose();
  });

  const helper = hdrProxyHelper(path.resolve('.'));
  it.skipIf(!helper)('tone maps on the GPU to the same pixels as FFmpeg, keeping sound', async () => {
    const { root } = await fixture();
    const source = path.join(root, 'hlg.mov'), output = path.join(root, 'playback.mp4');
    // Saturated and neutral bars, so gamut mapping, the shoulder and chroma all count.
    execFileSync(ffmpeg, ['-v', 'error',
      '-f', 'lavfi', '-i', 'color=c=red:s=64x180:r=30:d=0.5[a];color=c=0xBFBFBF:s=64x180:r=30:d=0.5[b];color=c=0x20C040:s=64x180:r=30:d=0.5[c];'
        + 'color=c=white:s=64x180:r=30:d=0.5[d];color=c=0x2050FF:s=64x180:r=30:d=0.5[e];[a][b][c][d][e]hstack=5',
      '-f', 'lavfi', '-i', 'sine=f=440:d=0.5', ...HLG, '-c:a', 'aac', '-shortest', source]);
    execFileSync(helper!, [source, output]);
    expect(await probeVideoStream(ffmpeg, output)).toMatchObject({
      codec: 'hevc', alpha: false, color: { range: 'tv', matrix: 'bt709', primaries: 'bt709', transfer: 'bt709' },
    });
    // Mapping a missing stream makes FFmpeg fail.
    execFileSync(ffmpeg, ['-v', 'error', '-i', output, '-map', '0:a:0', '-f', 'null', '-']);
    // Within the HEVC encoder's own rounding of the exact conversion.
    const gpu = ycbcr(output), exact = ycbcr(source, TONEMAP), plane = 320 * 180;
    for (const x of [32, 96, 160, 224, 288]) {
      for (let channel = 0; channel < 3; channel++) {
        const at = channel * plane + 90 * 320 + x;
        expect(Math.abs(gpu[at]! - exact[at]!), `bar at ${x}`).toBeLessThanOrEqual(2);
      }
    }
  }, 30_000);
});
