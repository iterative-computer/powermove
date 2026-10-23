/** Isolate Chromium's HDR RGB-to-Canvas transform from codec/YUV conversion.
 * Run under pm-electron-lock with an absolute evidence directory outside Git.
 * Replays the committed public RGB palette, without codec or YUV conversion.
 */
import { readFile, writeFile, mkdtemp, rm, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { inflateSync } from 'node:zlib';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron } from 'playwright';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const [directory] = process.argv.slice(2);
if (!directory || !path.isAbsolute(directory) || path.resolve(directory).startsWith(root + path.sep)) {
  throw new Error('usage: video-hdr-transform-probe.mjs ABSOLUTE_EVIDENCE_DIRECTORY_OUTSIDE_REPO');
}
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const fixture = path.join(root, 'engine/crates/pm-render/tests/fixtures/video-parity-round4/hdr-transform-oracle');
const manifest = JSON.parse(await readFile(path.join(fixture, 'manifest.json'), 'utf8'));
const source = inflateSync(await readFile(path.join(fixture, manifest.input.file)));
const { width, height, uniqueColors } = manifest;
if (sha256(source) !== manifest.input.sha256 || source.length !== width * height * 4) throw new Error('Changed RGB oracle input');
await mkdir(directory, { recursive: true });
await writeFile(path.join(directory, 'hdr-transform-input.rgba'), source);
await rm(path.join(directory, 'hdr-transform.json'), { force: true });
const report = { width, height, uniqueColors, sourceSha256: sha256(source), rows: [] };
const profile = await mkdtemp(path.join(os.tmpdir(), 'pm-hdr-transform-'));
let app;
try {
  const env = { ...process.env, POWERMOVE_USER_DATA: profile, POWERMOVE_BACKGROUND_TEST: '1',
    POWERMOVE_DEVTOOLS: '0', POWERMOVE_NEW_ENGINE: '0', POWERMOVE_FAKE_CHATGPT_STATUS: 'connected',
    POWERMOVE_DISABLE_RUNTIME_UPDATES: '1' };
  delete env.ELECTRON_RUN_AS_NODE;
  app = await _electron.launch({ args: [path.join(desktop, 'out/main/index.js')], cwd: desktop, env, timeout: 120_000 });
  const page = await app.firstWindow();
  await page.waitForFunction(() => Boolean(window.PM?.GL), undefined, { timeout: 60_000 });
  report.versions = await app.evaluate(() => process.versions);
  for (const [signal, primaries] of [['hlg', 'bt2020'], ['pq', 'bt2020'], ['pq', 'smpte432']]) {
    const result = await page.evaluate(({ bytes, width, height, signal, primaries }) => {
      const requested = { primaries, transfer: signal, matrix: 'rgb', fullRange: true };
      const frame = new VideoFrame(new Uint8Array(bytes), { format: 'RGBA', codedWidth: width,
        codedHeight: height, timestamp: 0, colorSpace: requested });
      try {
        const actual = frame.colorSpace.toJSON();
        const probes = [];
        for (const willReadFrequently of [true, false]) {
          const context = new OffscreenCanvas(width, height).getContext('2d', { colorSpace: 'srgb', willReadFrequently });
          if (!context) throw new Error('sRGB Canvas unavailable');
          context.drawImage(frame, 0, 0);
          const image = context.getImageData(0, 0, width, height, { colorSpace: 'srgb' });
          let binary = '';
          for (let i = 0; i < image.data.length; i += 8192) binary += String.fromCharCode(...image.data.subarray(i, i + 8192));
          probes.push({ settings: context.getContextAttributes(), colorSpace: image.colorSpace, rgba: btoa(binary) });
        }
        return { requested, actual, probes };
      } finally { frame.close(); }
    }, { bytes: [...source], width, height, signal, primaries });
    for (const [index, probe] of result.probes.entries()) {
      const bytes = Buffer.from(probe.rgba, 'base64');
      if (bytes.length !== source.length) throw new Error('Incomplete HDR transform readback');
      const file = `hdr-transform-${signal}-${primaries}-${index === 0 ? 'cpu' : 'gpu'}.rgba`;
      await writeFile(path.join(directory, file), bytes);
      delete probe.rgba;
      Object.assign(probe, { file, sha256: sha256(bytes) });
    }
    report.rows.push({ signal, primaries, ...result });
  }
  report.complete = true;
  await writeFile(path.join(directory, 'hdr-transform.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ complete: true, colors: uniqueColors, rows: report.rows.map(({ actual }) => actual) }));
} finally {
  try { await app?.close(); } finally { await rm(profile, { recursive: true, force: true }); }
}
