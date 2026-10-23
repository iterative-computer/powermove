/** Capture the shipped Chromium decoder and texImage2D paths in a hidden app.
 * Build the app first. Generate outside the Electron lease, then capture under
 * pm-electron-lock. Operational evidence lives outside the repository; pack
 * copies only public synthetic media and pixel references into the fixture.
 *   node scripts/video-parity.mjs generate /absolute/evidence [--round2]
 *   pm-electron-lock node scripts/video-parity.mjs capture /absolute/evidence
 */
import { mkdir, readFile, writeFile, mkdtemp, rm, access, copyFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const [mode, destination, selection] = process.argv.slice(2);
if (!['generate', 'capture', 'png', 'pack'].includes(mode) || !destination || !path.isAbsolute(destination)
    || path.resolve(destination).startsWith(root + path.sep) || (selection && !['--round2', '--round3'].includes(selection))) {
  throw new Error('usage: video-parity.mjs generate|capture|png|pack ABSOLUTE_DIRECTORY_OUTSIDE_REPO [--round2|--round3]');
}
await mkdir(destination, { recursive: true });
const recipes = [
  ['h264-601-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-hd-709-matrix-only', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt709', '-color_range', 'tv']],
  ['h264-709-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['h264-2020-limited', 'mp4', ['libx264', '-pix_fmt', 'yuv420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['h264-601-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['h264-2020-full', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-601-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-709-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-2020-limited', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['hevc10-601-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-709-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['hevc10-2020-full', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error:range=full', '-tag:v', 'hvc1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'pc']],
  ['vp9-709-limited', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['vp9-untagged', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'realtime', '-cpu-used', '8'], []],
  ['av1-709-limited', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores422-709', 'mov', ['prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
  ['prores4444-709', 'mov', ['prores_ks', '-profile:v', '4', '-pix_fmt', 'yuva444p10le'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv']],
];
// New names keep earlier synthetic sources and their committed references fixed.
// Options describe source geometry, alpha and intentional transfer/primaries.
const round3Recipes = [
  ['vp9-601-limited-r3', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'good', '-cpu-used', '5'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'tv'], { size: '256x144' }],
  ['vp9-709-full-r3', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p', '-deadline', 'good', '-cpu-used', '5'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '272x160' }],
  ['vp9-10bit-709-limited-r3', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p10le', '-deadline', 'good', '-cpu-used', '5'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv'], { size: '256x160' }],
  ['vp9-10bit-2020-limited-r3', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuv420p10le', '-deadline', 'good', '-cpu-used', '5'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'tv'], { size: '272x144' }],
  ['vp9-alpha-709-r3', 'webm', ['libvpx-vp9', '-pix_fmt', 'yuva420p', '-deadline', 'good', '-cpu-used', '5'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv'], { size: '256x144', alpha: true }],
  ['av1-10bit-709-limited-r3', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p10le', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'tv'], { size: '256x144' }],
  ['av1-10bit-2020-pq-r3', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p10le', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'smpte2084', '-color_range', 'tv'], { size: '256x160', hdr: 'PQ' }],
  ['av1-10bit-2020-hlg-r3', 'mp4', ['libsvtav1', '-pix_fmt', 'yuv420p10le', '-preset', '12', '-svtav1-params', 'lp=1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'arib-std-b67', '-color_range', 'tv'], { size: '272x144', hdr: 'HLG' }],
  ['h264-601-full-r3', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'smpte170m', '-color_primaries', 'smpte170m', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '256x160' }],
  ['h264-709-full-r3', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '272x160' }],
  ['h264-709-full-256x150-r3', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '256x150' }],
  ['h264-709-full-258x146-r3', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '258x146' }],
  ['h264-2020-full-r3', 'mp4', ['libx264', '-pix_fmt', 'yuvj420p'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'bt709', '-color_range', 'pc'], { size: '272x144' }],
  ['hevc-alpha-untagged-r3', 'mov', ['hevc_videotoolbox', '-pix_fmt', 'bgra', '-alpha_quality', '0.8', '-tag:v', 'hvc1'], [], { size: '256x144', alpha: true }],
  ['hevc10-2020-pq-r3', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt2020nc', '-color_primaries', 'bt2020', '-color_trc', 'smpte2084', '-color_range', 'tv'], { size: '256x144', hdr: 'PQ' }],
  ['hevc10-p3-pq-r3', 'mp4', ['libx265', '-pix_fmt', 'yuv420p10le', '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1'], ['-colorspace', 'bt709', '-color_primaries', 'smpte432', '-color_trc', 'smpte2084', '-color_range', 'tv'], { size: '256x160', hdr: 'PQ', gamut: 'Display P3' }],
];
async function packRound3() {
  const fixture = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round3');
  const sources = JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
  const capture = JSON.parse(await readFile(path.join(destination, 'chromium.json'), 'utf8'));
  if (sources.length !== round3Recipes.length || sources.some((s, i) => s.round !== 3 || s.id !== round3Recipes[i][0])
      || capture.rows?.length !== sources.length) throw new Error('Round 3 source/capture manifest is incomplete');
  const hash = bytes => createHash('sha256').update(bytes).digest('hex');
  const references = [], gaps = [], copies = [];
  // Validate all external evidence before putting any synthetic file in Git.
  for (const source of sources) {
    const row = capture.rows.find(candidate => candidate.id === source.id);
    if (!row) throw new Error(`Missing capture: ${source.id}`);
    if (row.file !== source.file || row.sourceSha256 !== source.sourceSha256) {
      throw new Error(`Capture belongs to a different generated source: ${source.id}`);
    }
    const original = await readFile(path.join(destination, source.file));
    if (hash(original) !== source.sourceSha256) throw new Error(`Changed source: ${source.id}`);
    copies.push([source.file, original]);
    const mediaFile = row.playback?.file || source.file;
    if (row.playback) {
      const playback = await readFile(path.join(destination, mediaFile));
      if (!row.playback.liveElementMatched || hash(playback) !== row.playback.sha256) {
        throw new Error(`Changed or unverified playback rendition: ${source.id}`);
      }
      copies.push([mediaFile, playback]);
    }
    if (row.error) {
      if (row.frames?.length) throw new Error(`Partial failed capture: ${source.id}`);
      gaps.push({ source: source.id, status: 'appDecodeError', error: row.error });
      continue;
    }
    if (row.frames?.length !== source.times.length) throw new Error(`Incomplete capture: ${source.id}`);
    for (const [sample, frame] of row.frames.entries()) {
      const expected = Math.floor(source.times[sample] * 12) / 12;
      if (frame.time !== source.times[sample] || frame.w !== source.probe.width || frame.h !== source.probe.height
          || Math.abs(frame.presentedMediaTime - expected) > 0.02
          || Math.abs(frame.timestamp / 1_000_000 - expected) > 0.02
          || frame.nativeReference !== `${source.id}-${sample}.premult`
          || frame.nativeLiveReference !== `${source.id}-${sample}.live`) {
        throw new Error(`Wrong source frame or presentation: ${source.id}-${sample}`);
      }
      if (frame.codedRawSha256) {
        const coded = await readFile(path.join(destination, `${source.id}-${sample}.coded`));
        if (coded.length !== frame.codedRawSize || hash(coded) !== frame.codedRawSha256) {
          throw new Error(`Changed coded VideoFrame copy: ${source.id}-${sample}`);
        }
      }
      const refs = {}, pixels = {};
      for (const kind of ['rgba', 'canvas', 'premult', 'live']) {
        const bytes = await readFile(path.join(destination, `${source.id}-${sample}.${kind}`));
        if (bytes.length !== frame.w * frame.h * 4) throw new Error(`Bad raster size: ${source.id}-${sample}.${kind}`);
        pixels[kind] = bytes;
        const file = `${source.id}-${sample}.${kind}.rgba.zlib`;
        refs[kind] = { file, sha256: hash(bytes) };
        copies.push([file, deflateSync(bytes, { level: 9 })]);
      }
      let alphaMin = 255, alphaMax = 0;
      for (let p = 3; p < pixels.canvas.length; p += 4) {
        const alpha = pixels.canvas[p];
        alphaMin = Math.min(alphaMin, alpha); alphaMax = Math.max(alphaMax, alpha);
        if (alpha !== pixels.premult[p]) throw new Error(`Premult upload changed alpha: ${source.id}-${sample}`);
        if (pixels.rgba[p] !== pixels.live[p]) throw new Error(`Live video upload changed alpha: ${source.id}-${sample}`);
      }
      if (source.alpha) {
        if (alphaMin >= alphaMax) {
          throw new Error(`Alpha source lacks nonuniform transparency: ${source.id}-${sample} [${alphaMin}, ${alphaMax}]`);
        }
        if (pixels.canvas.equals(pixels.premult)) {
          throw new Error(`Alpha Canvas upload was not premultiplied: ${source.id}-${sample}`);
        }
        if (pixels.rgba.equals(pixels.live)) {
          throw new Error(`Alpha live upload was not premultiplied: ${source.id}-${sample}`);
        }
      }
      if (!source.alpha && !pixels.canvas.equals(pixels.premult)) {
        throw new Error(`Opaque upload changed Canvas values: ${source.id}-${sample}`);
      }
      if (!source.alpha && !pixels.rgba.equals(pixels.live)) {
        throw new Error(`Opaque live upload changed direct texture: ${source.id}-${sample}`);
      }
      references.push({ source: source.id, sample, time: frame.time, width: frame.w, height: frame.h,
        original: source.file, media: mediaFile, importPath: row.playback ? 'PM.pickFiles playback rendition' : 'native Chromium video',
        format: frame.format, timestamp: frame.timestamp, colorSpace: frame.colorSpace,
        videoFrame: { codedWidth: frame.codedWidth, codedHeight: frame.codedHeight,
          visibleRect: frame.visibleRect, displayWidth: frame.displayWidth, displayHeight: frame.displayHeight,
          planes: frame.planes, rawError: frame.rawError, codedPlanes: frame.codedPlanes,
          codedRawError: frame.codedRawError, codedRawSize: frame.codedRawSize,
          codedRawSha256: frame.codedRawSha256 },
        alphaRange: [alphaMin, alphaMax], nativeReference: frame.nativeReference,
        nativeLiveReference: frame.nativeLiveReference,
        actualCanvas: refs.canvas, premultipliedCanvas: refs.premult, texture: refs.rgba,
        premultipliedTexture: refs.live,
        rowOrder: 'sourceTopDown' });
    }
  }
  await mkdir(fixture, { recursive: true });
  for (const [file, bytes] of copies) await writeFile(path.join(fixture, file), bytes);
  await writeFile(path.join(fixture, 'references.json'), JSON.stringify({
    capturedWith: capture.versions, sources: sources.map(source => {
      const row = capture.rows.find(candidate => candidate.id === source.id);
      return { id: source.id, file: source.file, times: source.times, sourceSha256: source.sourceSha256,
        width: source.probe.width, height: source.probe.height, codec: source.probe.codec_name,
        pixelFormat: source.probe.pix_fmt, colorRange: source.probe.color_range,
        colorSpace: source.probe.color_space, colorPrimaries: source.probe.color_primaries,
        colorTransfer: source.probe.color_transfer, expectedAlpha: Boolean(source.alpha),
        hdr: source.hdr || null, gamut: source.gamut || null, playback: row.playback || null,
        status: row.error ? 'appDecodeError' : 'captured', error: row.error || null };
    }), references, gaps,
  }, null, 2));
  console.log(`Round 3 packed ${sources.length} sources, ${references.length} frames, ${gaps.length} app gaps`);
  if (gaps.length) process.exitCode = 1;
}
if (mode === 'pack') {
  if (selection === '--round3') await packRound3();
  else {
  const fixture = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round2');
  const sources = JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
  const capture = JSON.parse(await readFile(path.join(destination, 'chromium.json'), 'utf8'));
  const references = [];
  // Validate the complete set before writing anything into the public repo.
  for (const source of sources) {
    const row = capture.rows.find(candidate => candidate.id === source.id);
    if (!row || row.error || row.frames?.length !== source.times.length) throw new Error(`Incomplete capture: ${source.id}: ${row?.error || 'missing frames'}`);
    const original = await readFile(path.join(destination, source.file));
    if (createHash('sha256').update(original).digest('hex') !== source.sourceSha256) throw new Error(`Source changed after generation: ${source.id}`);
    const mediaFile = row.playback?.file || source.file;
    if (source.id.startsWith('prores') && (!row.playback || !row.playback.playbackProxy || !row.playback.liveElementMatched)) throw new Error(`ProRes import bypassed playback proxy: ${source.id}`);
    if (row.playback) {
      const media = await readFile(path.join(destination, mediaFile));
      if (createHash('sha256').update(media).digest('hex') !== row.playback.sha256) throw new Error(`Playback rendition changed: ${source.id}`);
    }
    for (const [sample, frame] of row.frames.entries()) {
      if (frame.time !== source.times[sample] || frame.w !== source.probe.width || frame.h !== source.probe.height) throw new Error(`Bad frame metadata: ${source.id}-${sample}`);
      const expectedTime = Math.floor(frame.time * 12) / 12;
      if (Math.abs(frame.presentedMediaTime - expectedTime) > 0.02 || Math.abs(frame.timestamp / 1_000_000 - expectedTime) > 0.02) {
        throw new Error(`Stale or wrong presented frame: ${source.id}-${sample}`);
      }
      const refs = {};
      const pixels = {};
      if (frame.nativeReference !== `${source.id}-${sample}.premult`) throw new Error(`Missing native upload reference: ${source.id}-${sample}`);
      for (const kind of ['rgba', 'canvas', 'premult']) {
        const rgba = await readFile(path.join(destination, `${source.id}-${sample}.${kind}`));
        if (rgba.length !== frame.w * frame.h * 4) throw new Error(`Bad RGBA size: ${source.id}-${sample}.${kind}`);
        pixels[kind] = rgba;
        refs[kind] = { file: `${source.id}-${sample}.${kind}.rgba.zlib`, sha256: createHash('sha256').update(rgba).digest('hex') };
      }
      if (source.id === 'prores4444-709') {
        if (pixels.canvas.equals(pixels.premult)) throw new Error(`Alpha upload was not premultiplied: ${source.id}-${sample}`);
        for (let alpha = 3; alpha < pixels.canvas.length; alpha += 4) {
          if (pixels.canvas[alpha] !== pixels.premult[alpha]) throw new Error(`Alpha changed during upload: ${source.id}-${sample}`);
        }
      } else if (!pixels.canvas.equals(pixels.premult)) {
        throw new Error(`Opaque canvas changed during upload: ${source.id}-${sample}`);
      }
      references.push({ source: source.id, sample, time: frame.time, width: frame.w, height: frame.h,
        original: source.file, media: mediaFile, importPath: row.playback ? 'PM.pickFiles → VP9 playback proxy' : 'native Chromium video',
        format: frame.format, timestamp: frame.timestamp, colorSpace: frame.colorSpace,
        nativeReference: frame.nativeReference, actualCanvas: refs.canvas,
        premultipliedCanvas: refs.premult, texture: refs.rgba, rowOrder: 'sourceTopDown' });
    }
  }
  await mkdir(fixture, { recursive: true });
  for (const source of sources) {
    const row = capture.rows.find(candidate => candidate.id === source.id);
    await copyFile(path.join(destination, source.file), path.join(fixture, source.file));
    if (row.playback) await copyFile(path.join(destination, row.playback.file), path.join(fixture, row.playback.file));
    for (let sample = 0; sample < source.times.length; sample++) for (const kind of ['rgba', 'canvas', 'premult']) {
      const rgba = await readFile(path.join(destination, `${source.id}-${sample}.${kind}`));
      await writeFile(path.join(fixture, `${source.id}-${sample}.${kind}.rgba.zlib`), deflateSync(rgba, { level: 9 }));
    }
  }
  await writeFile(path.join(fixture, 'references.json'), JSON.stringify({
    capturedWith: capture.versions, sources: sources.map(({ id, file, times, probe, sourceSha256 }) => ({
      id, file, times, sourceSha256, codec: probe.codec_name, pixelFormat: probe.pix_fmt,
      colorRange: probe.color_range, colorSpace: probe.color_space, colorPrimaries: probe.color_primaries,
      colorTransfer: probe.color_transfer,
      playback: capture.rows.find(candidate => candidate.id === id).playback || null,
    })), references,
  }, null, 2));
  }
} else if (mode === 'generate') {
  const rows = [];
  const selected = selection === '--round3' ? round3Recipes : recipes.filter(([id]) => !selection
    || id.startsWith('hevc10-') || id.startsWith('prores'));
  for (const [id, ext, codec, tags, options = {}] of selected) {
    const file = `${id}.${ext}`;
    const alpha = options.alpha || (selection === '--round2' && id === 'prores4444-709');
    const colour = id.includes('601') ? 6 : id.includes('2020') ? 9 : id.includes('p3') ? 12 : 1;
    const matrix = id.includes('p3') ? 1 : colour;
    const transfer = options.hdr === 'PQ' ? 16 : options.hdr === 'HLG' ? 18 : 1;
    const full = id.endsWith('full') || id.includes('-full-');
    const bitstream = selection === '--round3' && id.startsWith('vp9-')
      ? ['-bsf:v', `vp9_metadata=color_space=${id.includes('601') ? 3 : id.includes('2020') ? 5 : 2}:color_range=${full ? 1 : 0}`]
      : selection === '--round3' && id.startsWith('av1-')
        ? ['-bsf:v', `av1_metadata=color_primaries=${colour}:transfer_characteristics=${transfer}:matrix_coefficients=${matrix}:color_range=${full ? 1 : 0}`]
        : id.startsWith('h264-') && !id.endsWith('matrix-only')
      ? ['-bsf:v', `h264_metadata=colour_primaries=${colour}:transfer_characteristics=${transfer}:matrix_coefficients=${matrix}:video_full_range_flag=${full ? 1 : 0}`]
      : id.startsWith('hevc10-')
        ? ['-bsf:v', `hevc_metadata=colour_primaries=${colour}:transfer_characteristics=${transfer}:matrix_coefficients=${matrix}:video_full_range_flag=${full ? 1 : 0}`]
        : [];
    // Detail, gradients, motion and midtones in every codec; deliberately not
    // only saturated patches, which hide reconstruction and rounding errors.
    execFileSync(process.env.PM_PARITY_FFMPEG || 'ffmpeg', ['-v', 'error', '-y', '-f', 'lavfi', '-i',
      `testsrc2=size=${options.size || (id.includes('-hd-') ? '1280x720' : '256x144')}:rate=12:duration=1.5`, '-vf',
      `${alpha ? `format=${id === 'prores4444-709' ? 'yuva444p10le' : 'yuva444p'},` : ''}geq=lum='32+0.65*lum(X,Y)+20*X/W':cb='128+0.65*(cb(X,Y)-128)':cr='128+0.65*(cr(X,Y)-128)'${alpha ? (id === 'prores4444-709' ? ":a='256+767*X/W'" : ":a='64+191*X/W'") : ''}${id.startsWith('hevc-alpha') ? ',format=bgra' : ''}`,
      '-filter_threads', '1', '-threads', '1', '-c:v', ...codec, ...tags, ...bitstream, '-an', path.join(destination, file)],
    { stdio: 'inherit' });
    const probe = JSON.parse(execFileSync(process.env.PM_PARITY_FFPROBE || 'ffprobe', ['-v', 'error', '-show_streams', '-of', 'json', path.join(destination, file)], { encoding: 'utf8' }));
    rows.push({ id, file, times: [0.125, 0.625, 1.125], probe: probe.streams[0],
      sourceSha256: createHash('sha256').update(await readFile(path.join(destination, file))).digest('hex'),
      ...(selection === '--round3' ? { round: 3, ...options } : {}) });
  }
  await writeFile(path.join(destination, 'sources.json'), JSON.stringify(rows, null, 2));
} else {
  await access(path.join(desktop, 'out/main/index.js'));
  const userData = await mkdtemp(path.join(os.tmpdir(), 'pm-video-parity-'));
  let app;
  try {
    const env = { ...process.env, POWERMOVE_USER_DATA: userData, POWERMOVE_BACKGROUND_TEST: '1',
      POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '0', POWERMOVE_DISABLE_RUNTIME_UPDATES: '1',
      POWERMOVE_FAKE_CHATGPT_STATUS: 'connected' };
    delete env.ELECTRON_RUN_AS_NODE;
    app = await _electron.launch({ args: [path.join(desktop, 'out/main/index.js')], cwd: desktop, env, timeout: 120_000 });
    const page = await app.firstWindow();
    await page.waitForFunction(() => Boolean(window.PM?.mkProject && window.PM?.GL), undefined, { timeout: 60_000 });
    await page.evaluate(() => {
      const PM = window.PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', {
        detail: PM.mkProject({ w: 256, h: 144, fps: 12, dur: 3, name: 'Synthetic video parity' }),
      }));
      PM.ProjectsScreen.hide();
      PM.setTime(0, { raw: true, force: true });
    });
    const rows = mode === 'png' ? [] : JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
    const report = { versions: await app.evaluate(() => process.versions), rows: [] };
    if (process.env.PM_PARITY_DIM_PROBE) {
      const probePath = path.resolve(process.env.PM_PARITY_DIM_PROBE);
      if (probePath.startsWith(root + path.sep)) throw new Error('Dimension probe must be external synthetic media');
      const probe = await page.evaluate(async bytes => {
        const video = document.createElement('video');
        video.muted = true;
        const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)]));
        let frame;
        try {
          await new Promise((resolve, reject) => {
            video.onloadeddata = resolve;
            video.onerror = () => reject(new Error(`dimension probe video error ${video.error?.code}: ${video.error?.message}`));
            video.src = url;
          });
          await new Promise((resolve, reject) => { video.onseeked = resolve; video.onerror = reject; video.currentTime = 0.125; });
          frame = new VideoFrame(video);
          const visible = frame.visibleRect;
          let planes = null, rawError = null, codedPlanes = null, codedRaw = null, codedRawError = null;
          try { planes = await frame.copyTo(new Uint8Array(frame.allocationSize())); }
          catch (e) { rawError = String(e); }
          try {
            const rect = { x: 0, y: 0, width: frame.codedWidth, height: frame.codedHeight };
            const full = new Uint8Array(frame.allocationSize({ rect }));
            codedPlanes = await frame.copyTo(full, { rect });
            let binary = '';
            for (let i = 0; i < full.length; i += 8192) binary += String.fromCharCode(...full.subarray(i, i + 8192));
            codedRaw = btoa(binary);
          } catch (e) { codedRawError = String(e); }
          return { videoWidth: video.videoWidth, videoHeight: video.videoHeight, format: frame.format,
            codedWidth: frame.codedWidth, codedHeight: frame.codedHeight, displayWidth: frame.displayWidth,
            displayHeight: frame.displayHeight, visibleRect: { x: visible.x, y: visible.y,
              width: visible.width, height: visible.height }, planes, rawError,
            codedPlanes, codedRaw, codedRawError,
            timestamp: frame.timestamp, colorSpace: frame.colorSpace.toJSON() };
        } finally { frame?.close(); video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); }
      }, [...await readFile(probePath)]);
      if (probe.codedRaw) {
        const bytes = Buffer.from(probe.codedRaw, 'base64');
        await writeFile(path.join(destination, 'dimension-probe.coded'), bytes);
        probe.codedRawSize = bytes.length;
        probe.codedRawSha256 = createHash('sha256').update(bytes).digest('hex');
        delete probe.codedRaw;
      }
      await writeFile(path.join(destination, 'dimension-probe.json'), JSON.stringify({ source: path.basename(probePath), ...probe }, null, 2));
    }
    for (const row of rows) {
      const imported = row.id.startsWith('prores');
      if (imported) {
        // The shipped app imports unsupported ProRes through its real picker
        // and converts it to the VP9 playback rendition used by the timeline.
        const chooser = page.waitForEvent('filechooser');
        await page.evaluate(() => window.PM.pickFiles());
        await (await chooser).setFiles(path.join(destination, row.file));
        await page.waitForFunction(name => [...window.PM.assets.map.values()].some(asset => asset.name === name),
          row.file, { timeout: 120_000 });
      }
      const bytes = imported ? null : [...await readFile(path.join(destination, row.file))];
      const captured = await page.evaluate(async ({ bytes, row, imported }) => {
        const encode = bytes => {
          let binary = '';
          for (let offset = 0; offset < bytes.length; offset += 8192) binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
          return btoa(binary);
        };
        const asset = imported ? [...window.PM.assets.map.values()].find(asset => asset.name === row.file) : null;
        if (imported && (!asset?.el?.src || !asset.playbackProxy)) throw new Error(`Imported asset ${row.file} has no playback video element`);
        const video = document.createElement('video');
        video.muted = true; video.preload = 'auto';
        let playback = null;
        let playbackBlob = null;
        if (imported) {
          playbackBlob = await window.PM.MediaStore.get(asset.storageKey);
          if (!playbackBlob || playbackBlob.type !== 'video/webm' || !asset.playbackProxy) {
            throw new Error(`Imported ${row.file} has no durable WebM playback proxy`);
          }
          const durable = new Uint8Array(await playbackBlob.arrayBuffer());
          const liveResponse = await fetch(asset.el.src);
          if (!liveResponse.ok) throw new Error(`Could not read live ${row.file} playback blob`);
          const live = new Uint8Array(await liveResponse.arrayBuffer());
          if (durable.length !== live.length || durable.some((byte, index) => byte !== live[index])) {
            throw new Error(`Imported ${row.file} persisted a different proxy than the live playback element`);
          }
          playback = { file: `${row.id}.playback.webm`, type: playbackBlob.type, size: playbackBlob.size,
            bytes: encode(durable), liveElementMatched: true,
            playbackProxy: asset.playbackProxy, playbackProxyVersion: asset.playbackProxyVersion };
        }
        const url = URL.createObjectURL(playbackBlob || new Blob([new Uint8Array(bytes)]));
        const timeout = (promise) => {
          let timer;
          return Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('video timeout')), 15_000); })])
            .finally(() => clearTimeout(timer));
        };
        try {
          await timeout(new Promise((resolve, reject) => {
            if (video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) return resolve();
            video.addEventListener('loadeddata', resolve, { once: true });
            video.addEventListener('error', () => reject(new Error(`HTMLVideoElement error ${video.error?.code}: ${video.error?.message}`)), { once: true });
            video.src = url;
          }));
          const frames = [];
          for (const time of row.times) {
            const seekStart = performance.now();
            const expectedFrameTime = Math.floor(time * 12) / 12;
            const presentation = new Promise((resolve, reject) => {
              const presented = (_now, metadata) => {
                if (metadata.mediaTime > expectedFrameTime + 0.02) return reject(new Error(`Presentation skipped requested frame: ${metadata.mediaTime} > ${expectedFrameTime}`));
                if (metadata.mediaTime >= expectedFrameTime - 0.02) resolve(metadata.mediaTime);
                else video.requestVideoFrameCallback(presented);
              };
              video.requestVideoFrameCallback(presented);
            });
            await timeout(new Promise((resolve, reject) => {
              video.onseeked = resolve; video.onerror = reject; video.currentTime = time;
            }));
            let presentedMediaTime;
            try {
              presentedMediaTime = await timeout(presentation);
            } finally { video.pause(); }
            const seekDecodeMs = performance.now() - seekStart;
            const w = video.videoWidth, h = video.videoHeight;
            const canvas = new OffscreenCanvas(w, h);
            const gl = canvas.getContext('webgl2');
            if (!gl) throw new Error('WebGL2 unavailable');
            const texture = gl.createTexture(), fbo = gl.createFramebuffer();
            let premultTexture = null, liveTexture = null;
            let vf;
            try {
              gl.bindTexture(gl.TEXTURE_2D, texture);
              gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video);
              gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
              gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
              if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete framebuffer');
              const rgba = new Uint8Array(w * h * 4);
              gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
              if (gl.getError() !== gl.NO_ERROR) throw new Error('video texture readback failed');
              liveTexture = gl.createTexture();
              gl.bindTexture(gl.TEXTURE_2D, liveTexture);
              gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
              gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
              try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video); }
              finally { gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); }
              gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, liveTexture, 0);
              if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete live video framebuffer');
              const live = new Uint8Array(w * h * 4);
              gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, live);
              if (gl.getError() !== gl.NO_ERROR) throw new Error('premultiplied live video texture readback failed');
              const two = new OffscreenCanvas(w, h).getContext('2d', { willReadFrequently: true });
              if (!two) throw new Error('Canvas2D unavailable');
              two.drawImage(video, 0, 0);
              premultTexture = gl.createTexture();
              gl.bindTexture(gl.TEXTURE_2D, premultTexture);
              gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
              gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
              try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, two.canvas); }
              finally { gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); }
              gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, premultTexture, 0);
              if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete premultiplied canvas framebuffer');
              const premult = new Uint8Array(w * h * 4);
              gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, premult);
              if (gl.getError() !== gl.NO_ERROR) throw new Error('premultiplied canvas texture readback failed');
              const colorProbes = {};
              if (row.captureColorProbes) {
                for (const [name, colorSpace, willReadFrequently] of [
                  ['srgbGpu', 'srgb', false],
                  ['displayP3Cpu', 'display-p3', true],
                  ['displayP3Gpu', 'display-p3', false],
                ]) {
                  let probeTexture;
                  try {
                    const context = new OffscreenCanvas(w, h).getContext('2d', { colorSpace, willReadFrequently });
                    if (!context) throw new Error(`Canvas2D ${colorSpace} unavailable`);
                    context.drawImage(video, 0, 0);
                    probeTexture = gl.createTexture();
                    gl.bindTexture(gl.TEXTURE_2D, probeTexture);
                    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
                    try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, context.canvas); }
                    finally { gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); }
                    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, probeTexture, 0);
                    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('incomplete color-probe framebuffer');
                    const uploaded = new Uint8Array(w * h * 4);
                    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, uploaded);
                    if (gl.getError() !== gl.NO_ERROR) throw new Error('color-probe texture readback failed');
                    const pixels = context.getImageData(0, 0, w, h, { colorSpace, pixelFormat: 'rgba-unorm8' });
                    colorProbes[name] = { settings: context.getContextAttributes(), imageDataColorSpace: pixels.colorSpace,
                      canvas: encode(pixels.data), premult: encode(uploaded) };
                  } catch (error) { colorProbes[name] = { error: String(error) }; }
                  finally { gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); if (probeTexture) gl.deleteTexture(probeTexture); }
                }
              }
              vf = new VideoFrame(video);
              let raw, planes, rawError, coded, codedPlanes, codedRawError;
              try { raw = new Uint8Array(vf.allocationSize()); planes = await vf.copyTo(raw); } catch (e) { rawError = String(e); }
              try {
                const rect = { x: 0, y: 0, width: vf.codedWidth, height: vf.codedHeight };
                coded = new Uint8Array(vf.allocationSize({ rect }));
                codedPlanes = await vf.copyTo(coded, { rect });
              } catch (e) { codedRawError = String(e); }
              const visible = vf.visibleRect;
              frames.push({ time, presentedMediaTime, seekDecodeMs, w, h,
                nativeReference: `${row.id}-${frames.length}.premult`, nativeLiveReference: `${row.id}-${frames.length}.live`,
                rgba: encode(rgba), live: encode(live), canvas: encode(two.getImageData(0, 0, w, h).data), premult: encode(premult),
                raw: planes ? encode(raw) : null, planes, rawError,
                coded: codedPlanes ? encode(coded) : null, codedPlanes, codedRawError,
                format: vf.format, timestamp: vf.timestamp,
                codedWidth: vf.codedWidth, codedHeight: vf.codedHeight,
                visibleRect: visible && { x: visible.x, y: visible.y, width: visible.width, height: visible.height },
                displayWidth: vf.displayWidth, displayHeight: vf.displayHeight,
                ...(row.captureColorProbes ? { colorProbes, canvasSettings: two.getContextAttributes(),
                  webglColorSpace: { drawingBuffer: gl.drawingBufferColorSpace, unpack: gl.unpackColorSpace,
                    conversion: gl.getParameter(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL) } } : {}),
                colorSpace: vf.colorSpace.toJSON() });
            } finally { vf?.close(); gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false); gl.deleteTexture(texture); if (liveTexture) gl.deleteTexture(liveTexture); if (premultTexture) gl.deleteTexture(premultTexture); gl.deleteFramebuffer(fbo); gl.getExtension('WEBGL_lose_context')?.loseContext(); }
          }
          return { frames, playback };
        } catch (e) { return { error: String(e) }; }
        finally { video.removeAttribute('src'); video.load(); URL.revokeObjectURL(url); }
      }, { bytes, row, imported });
      if (captured.playback) {
        await writeFile(path.join(destination, captured.playback.file), Buffer.from(captured.playback.bytes, 'base64'));
        captured.playback.sha256 = createHash('sha256').update(Buffer.from(captured.playback.bytes, 'base64')).digest('hex');
        delete captured.playback.bytes;
      }
      for (const [i, frame] of (captured.frames || []).entries()) {
        for (const [name, probe] of Object.entries(frame.colorProbes || {})) {
          for (const kind of ['canvas', 'premult']) {
            if (!probe[kind]) continue;
            const bytes = Buffer.from(probe[kind], 'base64');
            const file = `${row.id}-${i}.${name}.${kind}`;
            await writeFile(path.join(destination, file), bytes);
            probe[kind] = { file, sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length };
          }
        }
        for (const field of ['rgba', 'live', 'canvas', 'premult', 'raw', 'coded']) {
          if (frame[field]) {
            const bytes = Buffer.from(frame[field], 'base64');
            await writeFile(path.join(destination, `${row.id}-${i}.${field}`), bytes);
            if (field === 'coded') {
              frame.codedRawSize = bytes.length;
              frame.codedRawSha256 = createHash('sha256').update(bytes).digest('hex');
            }
          }
          delete frame[field];
        }
      }
      report.rows.push({ ...row, ...captured });
      console.log(row.id, captured.error || captured.frames.map(f => `${f.format}@${f.timestamp}`).join(' '));
      await writeFile(path.join(destination, 'chromium.json'), JSON.stringify(report, null, 2));
    }
    const png = await page.evaluate(async () => {
      const canvas = document.createElement('canvas'); canvas.width = 256; canvas.height = 256;
      const context = canvas.getContext('2d'); const data = context.createImageData(256, 256);
      // Every possible premultiplied channel/alpha pair through alphaFrame's
      // truncation and Canvas2D's real putImageData -> PNG encoding path.
      for (let a = 0; a < 256; a++) for (let c = 0; c < 256; c++) {
        const offset = (a * 256 + c) * 4, straight = a ? Math.min(255, Math.floor(c * 255 / a)) : 0;
        data.data.set([straight, straight, straight, a], offset);
      }
      context.putImageData(data, 0, 0);
      const blob = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
      if (!blob) throw new Error('Canvas PNG encode failed');
      return [...new Uint8Array(await blob.arrayBuffer())];
    });
    await writeFile(path.join(destination, 'canvas-alpha-table.png'), new Uint8Array(png));
    await writeFile(path.join(destination, 'versions.json'), JSON.stringify(report.versions, null, 2));
    if (report.rows.some(row => row.error || row.frames?.length !== row.times.length)) process.exitCode = 1;
  } finally {
    try {
      if (app) {
        // An imported project can show an unsaved-changes quit prompt. Do not
        // leave Electron (or the leased lock) waiting behind that dialog.
        const child = app.process();
        const closing = app.close().catch(() => undefined);
        let timer;
        try { await Promise.race([closing, new Promise(resolve => { timer = setTimeout(resolve, 5_000); })]); }
        finally { clearTimeout(timer); }
        if (child.exitCode === null) child.kill('SIGKILL');
        await closing;
      }
    } finally { await rm(userData, { recursive: true, force: true }); }
  }
}
