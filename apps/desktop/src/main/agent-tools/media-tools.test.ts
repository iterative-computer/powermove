import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';

import type { AgentToolContent } from '../../shared/ipc';
import type { AgentMediaSource } from '../../shared/media-tools';
import { TRANSCRIPTION_MODEL_MISSING, type Transcript } from '../../shared/transcription';
import { TranscriptionModelMissingError, type TranscriptionService } from '../transcription/service';
import { parseProbe } from './media-ffmpeg';
import { AgentMediaTools, type MediaToolContext } from './media-tools';

/*
 * A real smoke run: footage generated with ffmpeg lavfi (a flat blue shot,
 * a hard cut to SMPTE bars at 4 s, a sine tone muted 2–3.5 s and 5.5–6.2 s)
 * goes through every media tool with the bundled ffmpeg.
 */

const run = promisify(execFile);
const ffmpeg = path.resolve(__dirname, '../../../node_modules/ffmpeg-static/ffmpeg');
let directory = '';
let footage = '';

async function generate(): Promise<void> {
  directory = await mkdtemp(path.join(os.tmpdir(), 'powermove-media-tools-test-'));
  footage = path.join(directory, 'interview.mp4');
  await run(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'lavfi', '-i', 'color=c=0x2050c0:s=640x360:r=30:d=4',
    '-f', 'lavfi', '-i', 'smptebars=s=640x360:r=30:d=4',
    '-f', 'lavfi', '-i', 'sine=f=440:d=8:sample_rate=48000',
    '-filter_complex', "[0][1]concat=n=2:v=1:a=0[v];[2]volume=enable='between(t,2,3.5)+between(t,5.5,6.2)':volume=0,volume=4[a]",
    '-map', '[v]', '-map', '[a]', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-g', '30', '-c:a', 'aac', '-b:a', '128k', footage
  ]);
}

function source(layer?: AgentMediaSource['layer']): AgentMediaSource {
  return {
    path: footage, origin: 'source',
    asset: { id: 'a1', name: 'interview.mp4', kind: 'video', duration: 8, hasAudio: true, proxy: false },
    ...(layer ? { layer } : {})
  };
}

function context(target: AgentMediaSource, extra: Partial<MediaToolContext> = {}): MediaToolContext & { resolve: ReturnType<typeof vi.fn> } {
  return {
    resolve: vi.fn(async () => target),
    renderFrames: async () => { throw new Error('not used'); },
    composition: async () => { throw new Error('not used'); },
    ...extra
  } as MediaToolContext & { resolve: ReturnType<typeof vi.fn> };
}

const json = (content: AgentToolContent[]) => JSON.parse((content[0] as { text: string }).text);
const images = (content: AgentToolContent[]) => content.filter((item): item is Extract<AgentToolContent, { type: 'image' }> => item.type === 'image');

/** Mean RGB of an encoded image, decoded by ffmpeg itself. */
async function meanColor(data: Uint8Array): Promise<[number, number, number]> {
  const file = path.join(directory, `probe-${Math.random().toString(36).slice(2)}.img`);
  await writeFile(file, data);
  const { stdout } = await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-i', file, '-vf', 'scale=1:1:flags=area', '-f', 'rawvideo', '-pix_fmt', 'rgb24', 'pipe:1'], { encoding: 'buffer' });
  return [stdout[0]!, stdout[1]!, stdout[2]!];
}

async function imageSize(data: Uint8Array): Promise<{ width?: number; height?: number }> {
  const file = path.join(directory, `size-${Math.random().toString(36).slice(2)}.img`);
  await writeFile(file, data);
  const result = await run(ffmpeg, ['-hide_banner', '-i', file]).catch((error) => error as { stderr: string });
  const video = parseProbe(String((result as { stderr: string }).stderr)).video[0];
  return { width: video?.width, height: video?.height };
}

const tools = (options: Partial<ConstructorParameters<typeof AgentMediaTools>[0]> = {}) => new AgentMediaTools({ ffmpegPath: ffmpeg, ...options });

beforeAll(generate, 60_000);
afterAll(async () => { if (directory) await rm(directory, { recursive: true, force: true }); });

describe('media tools on generated footage', () => {
  it('probes container and streams', async () => {
    const result = json(await tools().call('probe_media', { assetId: 'a1' }, context(source())));
    expect(result).toMatchObject({ asset: { id: 'a1', name: 'interview.mp4', kind: 'video' }, format: 'mov/mp4' });
    expect(result.duration).toBeCloseTo(8, 1);
    expect(result.video[0]).toMatchObject({ codec: 'h264', width: 640, height: 360, fps: 30, pixelFormat: 'yuv420p' });
    expect(result.video[0].alpha).toBeUndefined();
    expect(result.audio[0]).toMatchObject({ codec: 'aac', sampleRate: 48000, channels: 1 });
  });

  it('samples source frames at explicit and negative times', async () => {
    const content = await tools().call('sample_media_frames', { assetId: 'a1', times: [1, -1] }, context(source()));
    const result = json(content);
    expect(result.frames).toEqual([{ index: 0, time: 1 }, { index: 1, time: 7 }]);
    expect(result).toMatchObject({ width: 512, height: 288, quality: 'small', timeBase: 'source' });
    const frames = images(content);
    expect(frames).toHaveLength(2);
    expect(frames.every((frame) => frame.mimeType === 'image/jpeg')).toBe(true);
    const [r, g, b] = await meanColor(frames[0]!.data);
    expect(b).toBeGreaterThan(150); expect(r).toBeLessThan(60); expect(g).toBeLessThan(110);
    const bars = await meanColor(frames[1]!.data);
    expect(bars[1]).toBeGreaterThan(80);
  });

  it('finds the hard cut with auto frames', async () => {
    const content = await tools().call('sample_media_frames', { assetId: 'a1', auto: true }, context(source()));
    const result = json(content);
    expect(result.auto).toMatchObject({ distinctStates: 2, scan: '2fps' });
    // A state is grabbed once it has settled; `since` is where the change began (the cut).
    expect(result.frames).toEqual([{ index: 0, time: 0 }, { index: 1, time: 4.5, since: 4 }]);
    expect(images(content)).toHaveLength(2);
  });

  it('caps frames per call and points at the contact sheet', async () => {
    await expect(tools().call('sample_media_frames', { assetId: 'a1', count: 9 }, context(source()))).rejects.toThrow(/media_contact_sheet/);
  });

  it('maps a clip window into source time for layer frames', async () => {
    const layer = { id: 'L1', name: 'Interview', type: 'video', from: 10, duration: 6, timing: { constant: true, samples: [[10, 1], [16, 7]] as Array<[number, number]> }, sourceStart: 1, sourceEnd: 7 };
    const result = json(await tools().call('sample_media_frames', { layerId: 'L1', times: [11, 15] }, context(source(layer))));
    expect(result.timeBase).toBe('composition');
    expect(result.frames).toEqual([{ index: 0, time: 11, sourceTime: 2 }, { index: 1, time: 15, sourceTime: 6 }]);
    expect(result.layer).toMatchObject({ id: 'L1', from: 10, sourceStart: 1, sourceEnd: 7 });
  });

  it('builds one labelled contact sheet', async () => {
    const content = await tools().call('media_contact_sheet', { assetId: 'a1', count: 6 }, context(source()));
    const result = json(content);
    expect(result.cells.map((cell: { time: number }) => cell.time)).toEqual([0, 1.593, 3.187, 4.78, 6.373, 7.967]);
    expect(result).toMatchObject({ columns: 3, rows: 2 });
    const sheet = images(content);
    expect(sheet).toHaveLength(1);
    expect(await imageSize(sheet[0]!.data)).toEqual({ width: 3 * result.cellWidth + 16, height: 2 * result.cellHeight + 12 });
  });

  it('tiles live composition frames for a composition sheet', async () => {
    const still = path.join(directory, 'comp.png');
    await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=red:s=320x180:d=1', '-frames:v', '1', still]);
    const png = new Uint8Array(await readFile(still));
    const renderFrames = vi.fn(async (times: number[]) => times.map(() => ({ data: png, mimeType: 'image/png' as const })));
    const ctx = context(source(), { renderFrames, composition: async () => ({ duration: 10, fps: 30, width: 1920, height: 1080, workArea: [0, 10] }) });
    const content = await tools().call('media_contact_sheet', { target: 'composition', count: 4 }, ctx);
    expect(ctx.resolve).not.toHaveBeenCalled();
    expect(renderFrames).toHaveBeenCalledWith([0, 3.322, 6.644, 9.967], expect.any(Number));
    expect(json(content)).toMatchObject({ target: 'composition', columns: 2, rows: 2 });
    const [r] = await meanColor(images(content)[0]!.data);
    expect(r).toBeGreaterThan(150);
  });

  it('measures silences and loudness, with a shaded waveform', async () => {
    const content = await tools().call('media_waveform', { assetId: 'a1' }, context(source()));
    const result = json(content);
    expect(result.silences).toHaveLength(2);
    const [[a0, a1], [b0, b1]] = result.silences;
    expect(a0).toBeCloseTo(2, 1); expect(a1).toBeCloseTo(3.5, 1);
    expect(b0).toBeCloseTo(5.5, 1); expect(b1).toBeCloseTo(6.2, 1);
    expect(result.silentSeconds).toBeCloseTo(2.2, 0);
    expect(result.loudness.integratedLufs).toBeLessThan(-5);
    expect(result.loudness.clipping).toBe(false);
    const wave = images(content);
    expect(wave).toHaveLength(1);
    expect(wave[0]!.mimeType).toBe('image/png');
    expect(await imageSize(wave[0]!.data)).toEqual({ width: 1200, height: 222 });
  });

  it('returns JSON only on request and maps silences into the composition for a clip', async () => {
    const layer = { id: 'L1', name: 'Interview', type: 'video', from: 10, duration: 6, timing: { constant: true, samples: [[10, 1], [16, 7]] as Array<[number, number]> }, sourceStart: 1, sourceEnd: 7 };
    const content = await tools().call('media_waveform', { layerId: 'L1', image: false }, context(source(layer)));
    expect(images(content)).toHaveLength(0);
    const result = json(content);
    expect(result.window).toEqual({ start: 1, end: 7 });
    const [[c0, c1], [d0, d1]] = result.compositionSilences;
    expect(c0).toBeCloseTo(11, 1); expect(c1).toBeCloseTo(12.5, 1);
    expect(d0).toBeCloseTo(14.5, 1); expect(d1).toBeCloseTo(15.2, 1);
  });

  it('flags audio driven over full scale', async () => {
    const loud = path.join(directory, 'loud.wav');
    await run(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'sine=f=440:d=2', '-af', 'volume=16', '-c:a', 'pcm_s16le', loud]);
    const target: AgentMediaSource = { path: loud, origin: 'source', asset: { id: 'a2', name: 'loud.wav', kind: 'audio', duration: 2, hasAudio: true, proxy: false } };
    const result = json(await tools().call('media_waveform', { assetId: 'a2', image: false }, context(target)));
    expect(result.loudness.clipping).toBe(true);
    await expect(tools().call('sample_media_frames', { assetId: 'a2' }, context(target))).rejects.toThrow(/no video stream/);
  });
});

describe('transcribe_media against the transcription seam', () => {
  const transcript: Transcript = {
    modelId: 'test-model', language: 'en', duration: 8,
    segments: [
      { text: 'Hello there.', start: 0.5, end: 1.5, words: [{ text: 'Hello', start: 0.5, end: 0.9 }, { text: 'there.', start: 1, end: 1.5 }] },
      { text: 'Second line.', start: 4, end: 5, words: [{ text: 'Second', start: 4, end: 4.4 }, { text: 'line.', start: 4.5, end: 5 }] }
    ]
  };
  const service = (transcribe: TranscriptionService['transcribe']): TranscriptionService & { requestModel: ReturnType<typeof vi.fn> } =>
    ({ status: async () => ({ models: [], activeModelId: null }), transcribe, requestModel: vi.fn() });

  it('asks the user for a model and tells the agent to wait when none is downloaded', async () => {
    const seam = service(async () => { throw new TranscriptionModelMissingError(); });
    const call = tools({ transcription: () => seam }).call('transcribe_media', { assetId: 'a1' }, context(source()));
    const error = await call.then(() => null, (reason: Error) => reason);
    expect(error).toBeInstanceOf(Error);
    const payload = JSON.parse(error!.message);
    expect(payload).toMatchObject({ status: 'model-required', code: TRANSCRIPTION_MODEL_MISSING });
    expect(payload.message).toMatch(/download/);
    expect(seam.requestModel).toHaveBeenCalledWith(expect.stringContaining('interview.mp4'));
  });

  it('returns timestamped text for an asset, from one cached transcription', async () => {
    const transcribe = vi.fn(async () => transcript);
    const media = tools({ transcription: () => service(transcribe) });
    const first = json(await media.call('transcribe_media', { assetId: 'a1' }, context(source())));
    expect(first).toMatchObject({ timeBase: 'source', modelId: 'test-model', language: 'en', segmentCount: 2 });
    expect(first.text).toBe('[0:00.50–0:01.50] Hello there.\n[0:04.00–0:05.00] Second line.');
    expect(transcribe).toHaveBeenCalledWith({ path: footage }, expect.any(Function), expect.any(AbortSignal));
    await media.call('transcribe_media', { assetId: 'a1' }, context(source()));
    expect(transcribe).toHaveBeenCalledTimes(1);
  });

  it('maps a retimed clip\'s words into composition time and drops what the clip does not play', async () => {
    const transcribe = vi.fn(async () => transcript);
    const layer = { id: 'L1', name: 'Interview', type: 'video', from: 10, duration: 1.5, timing: { constant: true, samples: [[10, 3], [11.5, 6]] as Array<[number, number]> }, sourceStart: 3, sourceEnd: 6 };
    const result = json(await tools({ transcription: () => service(transcribe) }).call('transcribe_media', { layerId: 'L1', format: 'words' }, context(source(layer))));
    expect(transcribe).toHaveBeenCalledWith({ path: footage, start: 3, end: 6 }, expect.any(Function), expect.any(AbortSignal));
    expect(result.timeBase).toBe('composition');
    expect(result.segments).toEqual([{ start: 10.5, end: 11, text: 'Second line.', words: [['Second', 10.5, 10.7], ['line.', 10.75, 11]] }]);
  });

  it('pages long transcripts with a cursor', async () => {
    const long: Transcript = { modelId: 'm', duration: 3000, segments: Array.from({ length: 400 }, (_, index) => ({ text: `Segment ${index} ${'word '.repeat(12)}`, start: index * 5, end: index * 5 + 4, words: [] })) };
    const media = tools({ transcription: () => service(async () => long) });
    const first = json(await media.call('transcribe_media', { assetId: 'a1' }, context(source())));
    expect(first.nextCursor).toBeGreaterThan(10);
    expect(first.text.length).toBeLessThanOrEqual(12_000);
    const second = json(await media.call('transcribe_media', { assetId: 'a1', cursor: first.nextCursor }, context(source())));
    expect(second.text.startsWith(`[${''}`)).toBe(true);
    expect(second.text).toContain(`Segment ${first.nextCursor} `);
    expect(second.cursor).toBe(first.nextCursor);
  });

  it('answers "still transcribing" for a long job and hands over the result on the next call', async () => {
    let finish: (value: Transcript) => void = () => {};
    const transcribe = vi.fn(() => new Promise<Transcript>((resolve) => { finish = resolve; }));
    const media = tools({ transcription: () => service(transcribe), transcribeWaitMs: 20 });
    const pending = json(await media.call('transcribe_media', { assetId: 'a1' }, context(source())));
    expect(pending).toMatchObject({ status: 'transcribing' });
    finish(transcript);
    await new Promise((resolve) => setTimeout(resolve, 10));
    const done = json(await media.call('transcribe_media', { assetId: 'a1' }, context(source())));
    expect(done.segmentCount).toBe(2);
    expect(transcribe).toHaveBeenCalledTimes(1);
  });
});
