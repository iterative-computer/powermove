/** Public HDR/P3 controls. Capture with video-parity.mjs under pm-electron-lock. */
import { mkdir, readFile, writeFile, copyFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { deflateSync } from 'node:zlib';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const [mode, destination] = process.argv.slice(2);
if (!['generate', 'pack'].includes(mode) || !destination || !path.isAbsolute(destination)
    || path.resolve(destination).startsWith(root + path.sep)) {
  throw new Error('usage: video-hdr-parity.mjs generate|pack ABSOLUTE_DIRECTORY_OUTSIDE_REPO');
}
const fixture = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round4');
const prior = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round3');
const reused = ['av1-10bit-2020-pq-r3', 'av1-10bit-2020-hlg-r3', 'hevc10-2020-pq-r3',
  'hevc10-p3-pq-r3', 'vp9-10bit-2020-limited-r3'];
const controls = [['hevc10-p3-sdr-r4', 12, 'smpte432'], ['hevc10-2020-sdr-r4', 9, 'bt2020']];
const ids = [...reused, ...controls.map(([id]) => id)];
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const ffprobe = file => JSON.parse(execFileSync(process.env.PM_PARITY_FFPROBE || 'ffprobe',
  ['-v', 'error', '-show_streams', '-of', 'json', file], { encoding: 'utf8' })).streams[0];
await mkdir(destination, { recursive: true });

if (mode === 'generate') {
  const original = JSON.parse(await readFile(path.join(prior, 'references.json'), 'utf8'));
  const sources = [];
  for (const id of reused) {
    const source = original.sources.find(source => source.id === id);
    if (!source || hash(await readFile(path.join(prior, source.file))) !== source.sourceSha256) {
      throw new Error(`Missing or changed committed synthetic source: ${id}`);
    }
    await copyFile(path.join(prior, source.file), path.join(destination, source.file));
  }
  for (const [id, primaries, name] of controls) {
    execFileSync(process.env.PM_PARITY_FFMPEG || 'ffmpeg', ['-v', 'error', '-y', '-f', 'lavfi', '-i',
      'testsrc2=size=256x160:rate=12:duration=1.5', '-vf',
      "geq=lum='32+0.65*lum(X,Y)+20*X/W':cb='128+0.65*(cb(X,Y)-128)':cr='128+0.65*(cr(X,Y)-128)'",
      '-filter_threads', '1', '-threads', '1', '-c:v', 'libx265', '-pix_fmt', 'yuv420p10le',
      '-x265-params', 'pools=1:frame-threads=1:log-level=error', '-tag:v', 'hvc1',
      '-colorspace', 'bt709', '-color_primaries', name, '-color_trc', 'bt709', '-color_range', 'tv',
      '-bsf:v', `hevc_metadata=colour_primaries=${primaries}:transfer_characteristics=1:matrix_coefficients=1:video_full_range_flag=0`,
      '-an', path.join(destination, `${id}.mp4`)], { stdio: 'inherit' });
  }
  for (const id of ids) {
    const file = `${id}.${id.startsWith('vp9') ? 'webm' : 'mp4'}`;
    const bytes = await readFile(path.join(destination, file));
    sources.push({ id, file, times: [0.125, 0.625, 1.125], captureColorProbes: true,
      sourceSha256: hash(bytes), probe: ffprobe(path.join(destination, file)) });
  }
  await writeFile(path.join(destination, 'sources.json'), JSON.stringify(sources, null, 2));
} else {
  const sources = JSON.parse(await readFile(path.join(destination, 'sources.json'), 'utf8'));
  const capture = JSON.parse(await readFile(path.join(destination, 'chromium.json'), 'utf8'));
  if (sources.length !== ids.length || sources.some((source, i) => source.id !== ids[i])
      || capture.rows?.length !== ids.length) throw new Error('Incomplete HDR/P3 capture set');
  const copies = [], references = [], gaps = [];
  for (const source of sources) {
    const bytes = await readFile(path.join(destination, source.file));
    const row = capture.rows.find(row => row.id === source.id);
    if (!row || row.file !== source.file || hash(bytes) !== source.sourceSha256
        || row.sourceSha256 !== source.sourceSha256) throw new Error(`Changed source binding: ${source.id}`);
    copies.push([source.file, bytes]);
    if (row.error) {
      gaps.push({ source: source.id, error: row.error });
      continue;
    }
    if (row.frames?.length !== 3) throw new Error(`Incomplete source: ${source.id}`);
    for (const [sample, frame] of row.frames.entries()) {
      const expected = Math.floor(source.times[sample] * 12) / 12;
      if (frame.time !== source.times[sample] || Math.abs(frame.timestamp / 1e6 - expected) > 0.02
          || Math.abs(frame.presentedMediaTime - expected) > 0.02
          || frame.w !== source.probe.width || frame.h !== source.probe.height) {
        throw new Error(`Wrong presentation: ${source.id}-${sample}`);
      }
      const refs = {};
      const raster = async file => {
        const bytes = await readFile(path.join(destination, file));
        if (bytes.length !== frame.w * frame.h * 4) throw new Error(`Wrong raster size: ${file}`);
        const target = `${file}.rgba.zlib`;
        copies.push([target, deflateSync(bytes, { level: 9 })]);
        return { file: target, sha256: hash(bytes) };
      };
      for (const kind of ['rgba', 'live', 'canvas', 'premult']) refs[kind] = await raster(`${source.id}-${sample}.${kind}`);
      const probes = {};
      for (const name of ['srgbGpu', 'displayP3Cpu', 'displayP3Gpu']) {
        const probe = frame.colorProbes?.[name];
        if (!probe || probe.error) {
          gaps.push({ source: source.id, sample, probe: name, error: probe?.error || 'missing probe' });
          continue;
        }
        const requested = name === 'srgbGpu' ? 'srgb' : 'display-p3';
        if (probe.settings?.colorSpace !== requested || probe.imageDataColorSpace !== requested) {
          throw new Error(`Wrong Canvas color space: ${source.id}-${sample} ${name}`);
        }
        const images = {};
        for (const kind of ['canvas', 'premult']) {
          const image = probe[kind];
          const bytes = await readFile(path.join(destination, image.file));
          if (hash(bytes) !== image.sha256 || bytes.length !== image.size) throw new Error(`Changed probe: ${image.file}`);
          images[kind] = await raster(image.file);
        }
        probes[name] = { ...images, settings: probe.settings, imageDataColorSpace: probe.imageDataColorSpace };
      }
      references.push({ source: source.id, media: source.file, sample, time: frame.time, timestamp: frame.timestamp,
        width: frame.w, height: frame.h, colorSpace: frame.colorSpace, format: frame.format,
        codedWidth: frame.codedWidth, codedHeight: frame.codedHeight, visibleRect: frame.visibleRect,
        canvasSettings: frame.canvasSettings, webglColorSpace: frame.webglColorSpace, ...refs, probes });
    }
  }
  await mkdir(fixture, { recursive: true });
  for (const [file, bytes] of copies) await writeFile(path.join(fixture, file), bytes);
  await writeFile(path.join(fixture, 'references.json'), JSON.stringify({
    capturedWith: capture.versions, decision: 'Match captured legacy output; no implicit HDR appearance repair.',
    sources: sources.map(({ id, file, times, sourceSha256, probe }) => ({ id, file, times, sourceSha256,
      width: probe.width, height: probe.height, codec: probe.codec_name, pixelFormat: probe.pix_fmt,
      colorRange: probe.color_range, colorSpace: probe.color_space,
      colorPrimaries: probe.color_primaries, colorTransfer: probe.color_transfer })), references, gaps,
  }, null, 2));
  console.log(`Packed ${sources.length} sources, ${references.length} HDR/P3 control frames, ${gaps.length} gaps`);
  if (gaps.length) process.exitCode = 1;
}
