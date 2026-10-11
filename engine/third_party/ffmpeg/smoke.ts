/*
 * Functional smoke test for Powermove's FFmpeg build.
 *
 *   bun engine/third_party/ffmpeg/smoke.ts [out-dir]
 *
 * Runs today's real FFmpeg call sites against the built binary: the argument
 * builders are imported from the desktop app (media-ffmpeg.ts, media-proxy.ts,
 * transcription/audio.ts, render-encoder.ts), not copied, and their outputs are
 * decoded and checked. Call sites that depend on excluded components
 * (libx264, prores_ks) must fail with "Unknown encoder"; the VideoToolbox
 * replacements Phase 2 can move them to are exercised next to them.
 *
 * Nothing is wired into the app. Fixtures are generated into a temporary
 * directory that is removed afterwards. Exits non-zero when any check fails.
 */

import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.resolve(here, '../../..');
const out = path.resolve(process.argv[2] ?? path.join(here, 'out'));
const ffmpeg = path.join(out, 'bin', 'ffmpeg');
const ffprobe = path.join(out, 'bin', 'ffprobe');
const desktop = path.join(repo, 'apps/desktop/src');

// render-encoder.ts imports `app`, `BrowserWindow`, and `dialog` from electron at
// module scope; encoderArgs itself is pure. A stub module lets it load in bun.
Bun.plugin({
  name: 'electron-stub',
  setup(build) {
    build.module('electron', () => ({ exports: { app: {}, BrowserWindow: {}, dialog: {} }, loader: 'object' }));
  }
});

const media = await import(path.join(desktop, 'main/agent-tools/media-ffmpeg.ts'));
const transcription = await import(path.join(desktop, 'main/transcription/audio.ts'));
const proxy = await import(path.join(desktop, 'main/media-proxy.ts'));
const encoder = await import(path.join(desktop, 'main/render-encoder.ts'));

/* ── harness ─────────────────────────────────────────────── */

interface Result { code: number | null; stdout: Buffer; stderr: string }

function run(binary: string, args: string[], input?: Buffer): Promise<Result> {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { stdio: ['pipe', 'pipe', 'pipe'] });
    const chunks: Buffer[] = [];
    let stderr = '';
    child.stdout.on('data', (chunk: Buffer) => chunks.push(chunk));
    child.stderr.on('data', (chunk: Buffer) => { stderr = (stderr + chunk.toString()).slice(-64_000); });
    child.once('error', reject);
    child.once('close', (code) => resolve({ code, stdout: Buffer.concat(chunks), stderr }));
    child.stdin.on('error', () => undefined);
    if (input) child.stdin.end(input); else child.stdin.end();
  });
}

async function ok(args: string[], input?: Buffer): Promise<Result> {
  const result = await run(ffmpeg, args, input);
  if (result.code !== 0) throw new Error(`ffmpeg ${args.join(' ')}\n${result.stderr.trim().split('\n').slice(-6).join('\n')}`);
  return result;
}

const results: Array<{ name: string; pass: boolean; detail: string }> = [];
async function check(name: string, body: () => Promise<string>): Promise<void> {
  try {
    const detail = await body();
    results.push({ name, pass: true, detail });
    console.log(`PASS  ${name}${detail ? ` (${detail})` : ''}`);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    results.push({ name, pass: false, detail });
    console.log(`FAIL  ${name}\n      ${detail.replace(/\n/g, '\n      ')}`);
  }
}

function expect(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

interface StreamInfo { codec_name?: string; codec_tag_string?: string; codec_type?: string; width?: number; height?: number; pix_fmt?: string; color_transfer?: string; color_primaries?: string; color_space?: string; color_range?: string; nb_read_frames?: string; sample_rate?: string; tags?: Record<string, string> }
async function probe(file: string, countFrames = false): Promise<StreamInfo[]> {
  const result = await run(ffprobe, ['-v', 'error', ...(countFrames ? ['-count_frames'] : []), '-show_streams', '-of', 'json', file]);
  if (result.code !== 0) throw new Error(`ffprobe ${file}: ${result.stderr}`);
  return JSON.parse(result.stdout.toString()).streams as StreamInfo[];
}

/** RGBA of every pixel of frame 0 (decoded by `decoder` when given). */
async function rgba(file: string, decoder?: string, extra: string[] = []): Promise<Buffer> {
  const { stdout } = await ok(['-v', 'error', ...(decoder ? ['-c:v', decoder] : []), ...extra, '-i', file, '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', 'pipe:1']);
  return stdout;
}

/** Every channel of `actual` within `tolerance` code values of `want`. */
function near(actual: number[], want: number[], tolerance: number): boolean {
  return want.every((value, index) => Math.abs(actual[index]! - value) <= tolerance);
}

function pixel(buffer: Buffer, width: number, x: number, y: number): number[] {
  const offset = (y * width + x) * 4;
  return [buffer[offset]!, buffer[offset + 1]!, buffer[offset + 2]!, buffer[offset + 3]!];
}

/* ── fixtures ────────────────────────────────────────────── */

const W = 640, H = 360;
const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-ffmpeg-smoke-'));
const font = ['/System/Library/Fonts/Menlo.ttc', '/System/Library/Fonts/SFNSMono.ttf'].find((file) => existsSync(file)) ?? null;
const interview = path.join(dir, 'interview.mp4');
const footage = path.join(dir, 'footage-4444.mov');

console.log(`ffmpeg: ${ffmpeg}\nscratch: ${dir}\n`);

try {
  // The media-tools.test.ts fixture (a flat shot cut to bars at 4 s, a tone with
  // silences at 2–3.5 s and 5.5–6.2 s), encoded with VideoToolbox H.264 because
  // the original uses libx264.
  await check('fixture: H.264 + AAC interview (h264_videotoolbox, lavfi color/smptebars/sine, concat, volume)', async () => {
    await ok(['-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `color=c=0x2050c0:s=${W}x${H}:r=30:d=4`,
      '-f', 'lavfi', '-i', `smptebars=s=${W}x${H}:r=30:d=4`,
      '-f', 'lavfi', '-i', 'sine=f=440:d=8:sample_rate=48000',
      '-filter_complex', "[0][1]concat=n=2:v=1:a=0[v];[2]volume=enable='between(t,2,3.5)+between(t,5.5,6.2)':volume=0,volume=4[a]",
      '-map', '[v]', '-map', '[a]', '-c:v', 'h264_videotoolbox', '-b:v', '4M', '-pix_fmt', 'yuv420p', '-g', '30', '-c:a', 'aac', '-b:a', '128k', interview]);
    const streams = await probe(interview);
    expect(streams.some((s) => s.codec_name === 'h264' && s.width === W), 'no H.264 stream');
    expect(streams.some((s) => s.codec_name === 'aac'), 'no AAC stream');
    return `${(await stat(interview)).size} bytes`;
  });

  // ProRes 4444 with alpha (left half opaque, right half transparent), the shape
  // import-prores-alpha.spec.ts builds with prores_ks, here through VideoToolbox.
  await check('fixture: ProRes 4444 with alpha (prores_videotoolbox, lavfi testsrc2/geq/alphamerge)', async () => {
    await ok(['-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `testsrc2=s=${W}x${H}:r=30:d=2,format=rgb24[base];color=c=white:s=${W}x${H}:r=30:d=2,format=gray,geq=lum='if(lt(X,W/2),255,0)'[alpha];[base][alpha]alphamerge,format=bgra`,
      '-f', 'lavfi', '-i', 'sine=f=220:d=2:sample_rate=48000',
      '-map', '0:v', '-map', '1:a', '-c:v', 'prores_videotoolbox', '-profile:v', '4444', '-c:a', 'pcm_s16le', footage]);
    const [video] = (await probe(footage)).filter((s) => s.codec_type === 'video');
    expect(video?.codec_name === 'prores', `codec ${video?.codec_name}`);
    expect(/^yuva444p/.test(video?.pix_fmt ?? ''), `pixel format ${video?.pix_fmt} has no alpha`);
    return `${video!.pix_fmt}, ${(await stat(footage)).size} bytes`;
  });

  /* ── agent media tools (media-ffmpeg.ts) ───────────────── */

  await check('media-ffmpeg parseProbe on `ffmpeg -i` (H.264/AAC and ProRes 4444 alpha)', async () => {
    const a = media.parseProbe((await run(ffmpeg, ['-hide_banner', '-nostdin', '-i', interview])).stderr);
    expect(a.format === 'mov/mp4' && a.video[0]?.codec === 'h264' && a.video[0]?.width === W && a.audio[0]?.codec === 'aac', JSON.stringify(a));
    expect(Math.abs((a.duration ?? 0) - 8) < 0.1, `duration ${a.duration}`);
    const b = media.parseProbe((await run(ffmpeg, ['-hide_banner', '-nostdin', '-i', footage])).stderr);
    expect(b.video[0]?.codec === 'prores' && b.video[0]?.alpha === true, JSON.stringify(b.video));
    return `h264 ${a.video[0].width}x${a.video[0].height} ${a.duration}s; prores alpha=${b.video[0].alpha} ${b.video[0].pixelFormat}`;
  });

  await check('media-ffmpeg frameArgs: JPEG frame with drawtext label (libfreetype + libharfbuzz, Menlo)', async () => {
    expect(font, 'no system font found for the label');
    const output = path.join(dir, 'frame.jpg');
    await ok(media.frameArgs({ input: interview, time: 1, width: 384, height: 216, label: '00:01.00', fontFile: font, format: 'jpeg', output }));
    const [image] = await probe(output);
    expect(image?.codec_name === 'mjpeg' && image.width === 384 && image.height === 216, JSON.stringify(image));
    // The label box (black@0.6) darkens the bottom-left corner of the blue frame.
    const pixels = await rgba(output);
    const corner = pixel(pixels, 384, 14, 205), centre = pixel(pixels, 384, 300, 60);
    expect(centre[2]! > 150 && corner[2]! < centre[2]! - 40, `label not drawn: corner ${corner} centre ${centre}`);
    return `centre ${centre.slice(0, 3)}, label corner ${corner.slice(0, 3)}`;
  });

  await check('media-ffmpeg frameArgs: PNG frame from ProRes 4444 (software prores decoder)', async () => {
    const output = path.join(dir, 'frame.png');
    await ok(media.frameArgs({ input: footage, time: 0.5, width: 320, height: 180, format: 'png', output }));
    const [image] = await probe(output);
    expect(image?.codec_name === 'png' && image.width === 320, JSON.stringify(image));
    return `${image!.width}x${image!.height} ${image!.pix_fmt}`;
  });

  await check('media-ffmpeg triageScanArgs + pickInformativeTimes (fps, scale flags=area, format=gray, rawvideo)', async () => {
    const { stdout } = await ok(media.triageScanArgs(interview, 0, 8, 2));
    const grid = media.TRIAGE_GRID * media.TRIAGE_GRID;
    expect(stdout.length > 0 && stdout.length % grid === 0, `scan returned ${stdout.length} bytes`);
    const { picks } = media.pickInformativeTimes(new Uint8Array(stdout), 0, 2, 4);
    expect(picks.length >= 2, `expected the blue shot and the bars, got ${JSON.stringify(picks)}`);
    const keyframes = await ok(media.triageScanArgs(interview, 0, 8, 2, true));
    expect(keyframes.stdout.length % grid === 0, 'keyframe-only scan misaligned');
    return `${stdout.length / grid} frames scanned, picks at ${picks.map((p: { time: number }) => p.time).join(', ')} s`;
  });

  await check('media-ffmpeg contact sheet: padded labelled cells (frameArgs) tiled with tileArgs', async () => {
    const plan = media.planSheet(4, W / H);
    for (let index = 0; index < 4; index++) {
      await ok(media.frameArgs({ input: interview, time: index * 2, width: plan.cellWidth, height: plan.cellHeight, pad: true, label: `00:0${index * 2}.00`, fontFile: font, format: 'jpeg', output: path.join(dir, `cell-${String(index).padStart(3, '0')}.jpg`) }));
    }
    const sheet = path.join(dir, 'sheet.jpg');
    await ok(media.tileArgs(path.join(dir, 'cell-%03d.jpg'), plan, sheet));
    const [image] = await probe(sheet);
    const width = plan.columns * plan.cellWidth + 4 * (plan.columns + 1);
    expect(image?.width === width, `sheet ${image?.width}x${image?.height}, expected width ${width}`);
    return `${plan.columns}x${plan.rows} grid, ${image!.width}x${image!.height}`;
  });

  await check('media-ffmpeg analysisArgs: silencedetect + ebur128 + astats parsed by parseSilences/parseLoudness', async () => {
    const { stderr } = await ok(media.analysisArgs(interview, 0, 8, { thresholdDb: -40, minDuration: 0.4 }));
    const silences = media.parseSilences(stderr, 0, 8);
    expect(silences.length === 2, `silences ${JSON.stringify(silences)}`);
    expect(Math.abs(silences[0][0] - 2) < 0.1 && Math.abs(silences[0][1] - 3.5) < 0.1, `first silence ${silences[0]}`);
    expect(Math.abs(silences[1][0] - 5.5) < 0.1 && Math.abs(silences[1][1] - 6.2) < 0.1, `second silence ${silences[1]}`);
    const loudness = media.parseLoudness(stderr);
    expect(Number.isFinite(loudness.integratedLufs) && Number.isFinite(loudness.truePeakDbfs) && Number.isFinite(loudness.rmsDbfs), JSON.stringify(loudness));
    return `silences ${JSON.stringify(silences)}, ${loudness.integratedLufs} LUFS, true peak ${loudness.truePeakDbfs} dBFS`;
  });

  await check('media-ffmpeg waveformArgs: showwavespic + overlay + drawbox + pad + drawtext ruler', async () => {
    const output = path.join(dir, 'waveform.png');
    await ok(media.waveformArgs({ input: interview, start: 0, duration: 8, width: 800, height: 200, silences: [[2, 3.5], [5.5, 6.2]], fontFile: font, output }));
    const [image] = await probe(output);
    expect(image?.width === 800 && image.height === 200 + media.WAVEFORM_RULER, JSON.stringify(image));
    return `${image!.width}x${image!.height}`;
  });

  /* ── transcription (transcription/audio.ts) ────────────── */

  await check('transcription decodeArgs: 16 kHz mono f32le on stdout, parseDuration on the banner', async () => {
    const whole = await ok(transcription.decodeArgs(interview));
    expect(Math.abs(whole.stdout.length / 4 - 8 * 16000) <= 1600, `${whole.stdout.length / 4} samples for 8 s`);
    expect(Math.abs((transcription.parseDuration(whole.stderr) ?? 0) - 8) < 0.1, 'banner duration');
    const span = await ok(transcription.decodeArgs(interview, { start: 1, end: 3 }));
    expect(Math.abs(span.stdout.length / 4 - 2 * 16000) <= 320, `${span.stdout.length / 4} samples for a 2 s span`);
    const prores = await ok(transcription.decodeArgs(footage));
    expect(prores.stdout.length > 0, 'no audio decoded from the ProRes MOV');
    return `${whole.stdout.length / 4} samples (8 s), span ${span.stdout.length / 4}`;
  });

  /* ── import proxies (media-proxy.ts) ───────────────────── */

  const playback = path.join(dir, 'proxy.webm');
  await check('media-proxy playbackConverter: ProRes 4444 alpha → VP9 alpha + Opus WebM (libvpx-vp9, libopus)', async () => {
    await proxy.playbackConverter(ffmpeg)(footage, playback);
    const streams = await probe(playback);
    const video = streams.find((s) => s.codec_type === 'video');
    expect(video?.codec_name === 'vp9' && video.tags?.alpha_mode === '1', `video ${JSON.stringify(video)}`);
    expect(streams.some((s) => s.codec_name === 'opus'), 'no Opus audio');
    const pixels = await rgba(playback, 'libvpx-vp9');
    const left = pixel(pixels, W, 40, H / 2), right = pixel(pixels, W, W - 40, H / 2);
    expect(left[3]! > 250 && right[3]! < 5, `alpha lost: left ${left} right ${right}`);
    return `vp9 alpha_mode=1 + opus; alpha left ${left[3]} right ${right[3]}`;
  });

  await check('media-proxy previewConverter: libvpx-vp9 decode keeps WebM alpha into the all-intra preview', async () => {
    const preview = path.join(dir, 'preview.webm');
    await proxy.previewConverter(ffmpeg)(playback, preview);
    const pixels = await rgba(preview, 'libvpx-vp9');
    const left = pixel(pixels, W, 40, H / 2), right = pixel(pixels, W, W - 40, H / 2);
    expect(left[3]! > 250 && right[3]! < 5, `alpha lost: left ${left} right ${right}`);
    return `alpha left ${left[3]} right ${right[3]}`;
  });

  await check('media-proxy imageSequenceConverter: numbered PNGs → lossless VP9 WebM with -progress', async () => {
    const frames: string[] = [];
    for (let index = 0; index < 3; index++) {
      const file = path.join(dir, `seq-${index}.png`);
      await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', `color=c=0x${['ff0000', '00ff00', '0000ff'][index]}:s=64x64:d=1,format=rgba`, '-frames:v', '1', file]);
      frames.push(file);
    }
    const output = path.join(dir, 'sequence.webm');
    const progress: number[] = [];
    await proxy.imageSequenceConverter(ffmpeg)(path.join(dir, 'seq-%d.png'), 24, 3, output, (done: number) => progress.push(done));
    const [video] = await probe(output, true);
    expect(video?.codec_name === 'vp9' && video.nb_read_frames === '3', JSON.stringify(video));
    const first = pixel(await rgba(output, 'libvpx-vp9'), 64, 32, 32);
    expect(first[0]! > 250 && first[1]! < 5 && first[2]! < 5, `first frame ${first}`);
    return `3 frames, first pixel ${first.slice(0, 3)}, progress ${progress.join(',') || 'none reported'}`;
  });

  await check('media-proxy stillImageConverter: TIFF → PNG', async () => {
    const tiff = path.join(dir, 'still.tiff');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=0xff0000:s=160x90:d=1', '-frames:v', '1', '-update', '1', '-c:v', 'tiff', tiff]);
    const png = path.join(dir, 'still.png');
    await proxy.stillImageConverter(ffmpeg)(tiff, 'tiff', png);
    const colour = pixel(await rgba(png), 160, 80, 45);
    expect(colour[0]! > 250 && colour[1]! < 5 && colour[2]! < 5, `colour ${colour}`);
    return `pixel ${colour.slice(0, 3)}`;
  });

  /* ── export (render-encoder.ts) ────────────────────────── */

  const frameRgba = (width: number, height: number, count: number) => {
    const bytes = Buffer.alloc(width * height * 4 * count);
    for (let index = 0; index < width * height * count; index++) {
      const x = index % width;
      bytes[index * 4] = 255; bytes[index * 4 + 1] = 0; bytes[index * 4 + 2] = 0;
      bytes[index * 4 + 3] = x < width / 2 ? 255 : 0;
    }
    return bytes;
  };

  for (const format of ['mp4', 'prores'] as const) {
    const missing = format === 'mp4' ? 'libx264' : 'prores_ks';
    await check(`render-encoder encoderArgs (${format}) fails as expected: ${missing} is excluded`, async () => {
      const output = path.join(dir, `export-today.${format === 'mp4' ? 'mp4' : 'mov'}`);
      const args = encoder.encoderArgs({ width: 64, height: 64, fps: 30, format, alpha: format === 'prores', name: 'Smoke' }, output);
      expect(args.includes(missing), `encoderArgs no longer uses ${missing}`);
      const result = await run(ffmpeg, args, frameRgba(64, 64, 3));
      // The ProRes arguments fail at option parsing: -alpha_bits is a prores_ks private option.
      const reason = format === 'mp4' ? /Unknown encoder 'libx264'/ : /Unknown encoder 'prores_ks'|Unrecognized option 'alpha_bits'/;
      expect(result.code !== 0 && reason.test(result.stderr), `expected a ${missing} failure, got code ${result.code}: ${result.stderr}`);
      return result.stderr.trim().split('\n').find((line) => reason.test(line))!.replace(/^\[[^\]]*\] /, '');
    });
  }

  const swap = (args: string[], from: string[], to: string[]) => {
    const at = args.indexOf(from[0]!, args.indexOf('-c:v'));
    expect(at > 0, `cannot find ${from[0]} in ${args.join(' ')}`);
    return [...args.slice(0, at - 1), ...to, ...args.slice(at - 1 + from.length + 1)];
  };

  await check('Phase 2 candidate: encoderArgs mp4 with h264_videotoolbox in place of libx264 (tags, range, faststart)', async () => {
    const output = path.join(dir, 'export-vt.mp4');
    const today = encoder.encoderArgs({ width: 64, height: 64, fps: 30000 / 1001, format: 'mp4', alpha: false, name: 'Smoke', bitrateMbps: 16 }, output);
    const from = today.slice(today.indexOf('-c:v') + 1, today.indexOf('-b:v'));
    const args = swap(today, from, ['-c:v', 'h264_videotoolbox', '-pix_fmt', 'yuv420p']);
    // VideoToolbox takes primaries and transfer from the frames' attachments: the
    // output-level -color_primaries/-color_trc alone leave both unspecified in the
    // H.264 VUI, so the frames are tagged with setparams.
    const vf = args.indexOf('-vf') + 1;
    args[vf] = `${args[vf]},setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv`;
    await ok(args, frameRgba(64, 64, 30));
    const [video] = await probe(output, true);
    expect(video?.codec_name === 'h264' && video.nb_read_frames === '30', JSON.stringify(video));
    expect(video.color_primaries === 'bt709' && video.color_space === 'bt709' && video.color_range === 'tv' && video.color_transfer === 'iec61966-2-1', `tags ${video.color_primaries}/${video.color_transfer}/${video.color_space}/${video.color_range}`);
    // e2e/export-mp4.spec.ts asserts the file carries the avc1 sample entry.
    expect((await readFile(output)).toString('latin1').includes('avc1'), 'no avc1 sample entry in the MP4');
    const red = pixel(await rgba(output), 64, 8, 32);
    expect(near(red, [255, 0, 0], 2), `colour ${red}, want 255,0,0 within 2 CV`);
    return `30 frames, avc1, ${video.color_primaries}/${video.color_transfer}/${video.color_space}/${video.color_range}, pixel ${red.slice(0, 3)}`;
  });

  await check('Phase 2 candidate: encoderArgs prores with prores_videotoolbox 4444 in place of prores_ks (alpha kept)', async () => {
    const output = path.join(dir, 'export-vt.mov');
    const today = encoder.encoderArgs({ width: 64, height: 64, fps: 24000 / 1001, format: 'prores', alpha: true, name: 'Smoke' }, output);
    const start = today.indexOf('-c:v'), end = today.indexOf('-alpha_bits') + 2;
    // Today's scale filter (Rec.709 matrix, limited range) stays; its output is handed to
    // VideoToolbox as 16-bit 4:4:4:4 Y'CbCr. Handing VideoToolbox BGRA instead makes it
    // convert with the Rec.709 matrix while the stream is tagged smpte170m, ignoring
    // -colorspace: decoders then show 255,0,0 as 234,0,3.
    // Primaries and transfer come from frame attachments, as for H.264 above.
    const args = [...today.slice(0, start), '-c:v', 'prores_videotoolbox', '-profile:v', '4444', '-pix_fmt', 'ayuv64le', ...today.slice(end)];
    const vf = args.indexOf('-vf') + 1;
    args[vf] = `${args[vf]},setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv`;
    await ok(args, frameRgba(64, 64, 3));
    const [video] = await probe(output, true);
    expect(video?.codec_name === 'prores' && /^yuva444p/.test(video.pix_fmt ?? '') && video.nb_read_frames === '3', JSON.stringify(video));
    expect(video.color_primaries === 'bt709' && video.color_space === 'bt709' && video.color_range === 'tv' && video.color_transfer === 'iec61966-2-1', `tags ${video.color_primaries}/${video.color_transfer}/${video.color_space}/${video.color_range}`);
    // e2e/ae-audit-implementation.spec.ts:98 asserts the ProRes 4444 sample entry, ap4h.
    expect(video.codec_tag_string === 'ap4h' && (await readFile(output)).toString('latin1').includes('ap4h'), `codec tag ${video.codec_tag_string}, want ap4h`);
    const pixels = await rgba(output);
    const left = pixel(pixels, 64, 8, 32), right = pixel(pixels, 64, 56, 32);
    expect(near(left, [255, 0, 0, 255], 2) && right[3] === 0, `left ${left} (want 255,0,0,255 within 2 CV), right alpha ${right[3]} (want 0)`);
    return `${video.pix_fmt} ap4h, ${video.color_primaries}/${video.color_transfer}/${video.color_space}/${video.color_range}, left ${left}, right alpha ${right[3]}`;
  });

  await check('render-encoder audio mux step: PCM 24-bit into MOV and AAC into MP4 (-c:v copy)', async () => {
    const wav = path.join(dir, 'audio.wav');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=f=440:d=1:sample_rate=48000', '-c:a', 'pcm_f32le', wav]);
    for (const [video, codec, final] of [[path.join(dir, 'export-vt.mov'), 'pcm_s24le', 'final.mov'], [path.join(dir, 'export-vt.mp4'), 'aac', 'final.mp4']] as const) {
      const output = path.join(dir, final);
      await ok(['-hide_banner', '-loglevel', 'error', '-i', video, '-i', wav, '-map', '0:v', '-map', '1:a', '-c:v', 'copy', '-c:a', codec, '-shortest', '-y', output]);
      const streams = await probe(output);
      expect(streams.some((s) => s.codec_name === codec), `${final} has no ${codec}`);
    }
    return 'pcm_s24le in MOV, aac in MP4';
  });

  /* ── test-mcp.mjs and e2e verification shapes ─────────── */

  await check('apps/desktop/scripts/test-mcp.mjs shapes: -f null inspect, s16le audio, crop=2:2 rgb24 pixel', async () => {
    const exported = path.join(dir, 'final.mp4');
    await ok(['-hide_banner', '-i', exported, '-f', 'null', '-']);
    const audio = await ok(['-v', 'error', '-i', exported, '-map', '0:a:0', '-f', 's16le', '-']);
    expect(audio.stdout.length > 0, 'no s16le audio');
    const crop = await ok(['-v', 'error', '-i', exported, '-frames:v', '1', '-vf', 'crop=2:2:0:0', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-']);
    expect(crop.stdout.length === 12, `crop returned ${crop.stdout.length} bytes`);
    return `${audio.stdout.length} audio bytes, pixel ${[...crop.stdout.subarray(0, 3)]}`;
  });

  await check('e2e shapes: framemd5 (project-export-fixture), scale=1:1 rgb24 (export-video-frames), lossless VP9 from rawvideo', async () => {
    const exported = path.join(dir, 'export-vt.mp4');
    const md5 = await ok(['-v', 'error', '-i', exported, '-an', '-f', 'framemd5', 'pipe:1']);
    const hashes = md5.stdout.toString().split('\n').filter((line) => line && !line.startsWith('#'));
    expect(hashes.length === 30, `${hashes.length} framemd5 lines for 30 frames`);
    const one = await ok(['-v', 'error', '-i', exported, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
    expect(one.stdout.length === 90, `${one.stdout.length} bytes for 30 1x1 frames`);
    const raw = Buffer.alloc(64 * 64 * 3 * 2, 0x80);
    const lossless = path.join(dir, 'lossless.webm');
    await ok(['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', '10', '-i', 'pipe:0', '-c:v', 'libvpx-vp9', '-lossless', '1', '-g', '1', lossless], raw);
    const back = await rgba(lossless);
    expect(back[0] === 0x80 && back[1] === 0x80 && back[2] === 0x80, `lossless pixel ${[...back.subarray(0, 4)]}`);
    return `${hashes.length} framemd5 frames, lossless VP9 pixel ${[...back.subarray(0, 3)]}`;
  });

  await check('e2e media-intelligence graph: aiff in, aresample/aformat/anullsrc/atrim/concat, wav out', async () => {
    const aiff = path.join(dir, 'a.aiff');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=f=300:d=1:sample_rate=22050', '-c:a', 'pcm_s16be', aiff]);
    const wav = path.join(dir, 'speech.wav');
    await ok(['-hide_banner', '-loglevel', 'error', '-y', '-i', aiff, '-i', aiff, '-filter_complex',
      '[0:a]aresample=48000,aformat=channel_layouts=mono[a];[1:a]aresample=48000,aformat=channel_layouts=mono[b];anullsrc=r=48000:cl=mono,atrim=duration=0.5[s];[a][s][b]concat=n=3:v=0:a=1[out]',
      '-map', '[out]', wav]);
    const mp4 = path.join(dir, 'speech.mp4');
    await ok(['-hide_banner', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=30', '-i', wav,
      '-shortest', '-c:v', 'h264_videotoolbox', '-b:v', '2M', '-pix_fmt', 'yuv420p', '-g', '30', '-c:a', 'aac', '-b:a', '128k', mp4]);
    const duration = transcription.parseDuration((await run(ffmpeg, ['-hide_banner', '-i', mp4])).stderr);
    expect(duration !== null && Math.abs(duration - 2.5) < 0.15, `duration ${duration}`);
    return `speech.mp4 ${duration}s`;
  });

  /* ── engine-facing codecs and filters ──────────────────── */

  await check('VideoToolbox hwaccel decode of ProRes 4444 (strict: hw frames, hwdownload)', async () => {
    const hw = await ok(['-v', 'error', '-hwaccel', 'videotoolbox', '-hwaccel_output_format', 'videotoolbox_vld', '-i', footage,
      '-frames:v', '1', '-vf', 'hwdownload,format=ayuv64le,format=rgba', '-f', 'rawvideo', 'pipe:1']).catch(async (first) => {
      // Some VideoToolbox ProRes outputs download as other 4:4:4 alpha layouts; let ffmpeg negotiate.
      return ok(['-v', 'error', '-hwaccel', 'videotoolbox', '-hwaccel_output_format', 'videotoolbox_vld', '-i', footage,
        '-frames:v', '1', '-vf', 'hwdownload,format=nv12|p010le|ayuv64le|bgra|nv24|p416le,format=rgba', '-f', 'rawvideo', 'pipe:1']).catch(() => { throw first; });
    });
    const sw = await rgba(footage);
    expect(hw.stdout.length === sw.length, `hw ${hw.stdout.length} bytes vs sw ${sw.length}`);
    let maximum = 0;
    for (let index = 0; index < sw.length; index++) if (index % 4 !== 3) maximum = Math.max(maximum, Math.abs(hw.stdout[index]! - sw[index]!));
    expect(maximum <= 3, `VideoToolbox and software decodes differ by ${maximum} CV`);
    return `hw frame matches software decode within ${maximum} CV (8-bit RGB)`;
  });

  await check('HEVC: hevc_videotoolbox encode, VideoToolbox hwaccel decode', async () => {
    const hevc = path.join(dir, 'hevc.mp4');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=1', '-c:v', 'hevc_videotoolbox', '-b:v', '2M', '-tag:v', 'hvc1', hevc]);
    const decoded = await ok(['-v', 'error', '-hwaccel', 'videotoolbox', '-hwaccel_output_format', 'videotoolbox_vld', '-i', hevc, '-vf', 'hwdownload,format=nv12', '-f', 'null', '-']);
    expect(decoded.code === 0, 'decode failed');
    const [video] = await probe(hevc, true);
    expect(video?.codec_name === 'hevc' && video.nb_read_frames === '30', JSON.stringify(video));
    return '30 frames';
  });

  await check('AV1: libsvtav1 encode, libdav1d decode', async () => {
    const av1 = path.join(dir, 'av1.mp4');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=0xff0000:s=160x90:r=30:d=1', '-c:v', 'libsvtav1', '-crf', '45', '-preset', '12', '-pix_fmt', 'yuv420p', av1]);
    const red = pixel(await rgba(av1, 'libdav1d'), 160, 80, 45);
    expect(red[0]! > 230 && red[1]! < 30, `colour ${red}`);
    return `pixel ${red.slice(0, 3)}`;
  });

  await check('VP8 + Opus WebM (libvpx, libopus), decoded by native vp8 and libvpx', async () => {
    const vp8 = path.join(dir, 'vp8.webm');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=0x00ff00:s=160x90:r=30:d=1', '-f', 'lavfi', '-i', 'sine=d=1', '-c:v', 'libvpx', '-b:v', '500k', '-c:a', 'libopus', vp8]);
    const native = pixel(await rgba(vp8, 'vp8'), 160, 80, 45), lib = pixel(await rgba(vp8, 'libvpx'), 160, 80, 45);
    expect(native[1]! > 230 && lib[1]! > 230, `native ${native} libvpx ${lib}`);
    return `native ${native.slice(0, 3)}, libvpx ${lib.slice(0, 3)}`;
  });

  await check('zscale (libzimg) Rec.709 limited → RGB matches swscale within 2 CV', async () => {
    const z = await ok(['-v', 'error', '-i', interview, '-frames:v', '1', '-vf', 'zscale=matrixin=709:rangein=limited:transferin=709:primariesin=709:transfer=709:primaries=709:range=full,format=gbrp,format=rgba', '-f', 'rawvideo', 'pipe:1']);
    // The fixture carries no matrix tag, so swscale is told Rec.709 limited explicitly too.
    const { stdout: s } = await ok(['-v', 'error', '-i', interview, '-frames:v', '1', '-vf', 'scale=in_color_matrix=bt709:in_range=limited:flags=accurate_rnd+full_chroma_int,format=rgba', '-f', 'rawvideo', 'pipe:1']);
    let maximum = 0;
    for (let index = 0; index < s.length; index++) if (index % 4 !== 3) maximum = Math.max(maximum, Math.abs(z.stdout[index]! - s[index]!));
    expect(maximum <= 2, `zscale and swscale differ by ${maximum}`);
    return `max difference ${maximum} CV`;
  });

  await check('scene detection: select=gt(scene,0.3) and scdet find the cut at 4 s', async () => {
    const select = await ok(['-hide_banner', '-i', interview, '-an', '-vf', "select='gt(scene,0.3)',showinfo", '-f', 'null', '-']);
    const times = [...select.stderr.matchAll(/pts_time:([\d.]+)/g)].map((match) => Number(match[1]));
    expect(times.some((time) => Math.abs(time - 4) < 0.1), `select times ${times}`);
    const scdet = await ok(['-hide_banner', '-i', interview, '-an', '-vf', 'scdet=threshold=10', '-f', 'null', '-']);
    expect(/lavfi\.scd\.time: ?4(\.0+)?\b/.test(scdet.stderr) || /scd\.time.*4\./.test(scdet.stderr), 'scdet did not report 4 s');
    return `select at ${times.join(', ')} s`;
  });

  await check('bwdif deinterlace and AudioToolbox AAC (aac_at) run', async () => {
    await ok(['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=320x180:r=30:d=0.5', '-vf', 'bwdif', '-f', 'null', '-']);
    const m4a = path.join(dir, 'at.m4a');
    await ok(['-v', 'error', '-y', '-f', 'lavfi', '-i', 'sine=d=1:sample_rate=48000', '-c:a', 'aac_at', m4a]);
    const [audio] = await probe(m4a);
    expect(audio?.codec_name === 'aac', JSON.stringify(audio));
    return 'ok';
  });
} finally {
  await rm(dir, { recursive: true, force: true });
}

const failed = results.filter((result) => !result.pass);
console.log(`\nsmoke: ${results.length - failed.length}/${results.length} passed`);
if (process.env.PM_FFMPEG_SMOKE_REPORT) await writeFile(process.env.PM_FFMPEG_SMOKE_REPORT, JSON.stringify(results, null, 2));
process.exit(failed.length ? 1 : 0);
