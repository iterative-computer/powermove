// Writes v1.2.0 project files with embedded media using v1's own save path
// (main/project-files.ts beginIncremental -> ProjectSave, PMV4; and
// shared/project-container.ts encodeProjectContainer, PMV3), v1's
// projectFingerprint (savedHash) and stringifyAsync, and v1's media
// fingerprint rule. Usage: bun generate.ts "$PM_V1_REF/apps/desktop/src" <out-dir>
import { deflateSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { writeFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
const [src, out] = process.argv.slice(2);
const { ProjectFiles } = await import(path.join(src, 'main/project-files.ts'));
const { projectFingerprint } = await import(path.join(src, 'renderer/src/legacy/core/project-fingerprint.ts'));
const { stringifyAsync } = await import(path.join(src, 'renderer/src/legacy/core/serialize-async.ts'));
const { sha256HexOf } = await import(path.join(src, 'shared/sha256.ts'));
const { encodeProjectContainer } = await import(path.join(src, 'shared/project-container.ts'));

function png(w: number, h: number): Uint8Array {
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (b: Uint8Array) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type: string, data: Uint8Array) => { const b = Buffer.alloc(12 + data.length); b.writeUInt32BE(data.length, 0); b.write(type, 4, 'ascii'); Buffer.from(data).copy(b, 8); b.writeUInt32BE(crc(b.subarray(4, 8 + data.length)), 8 + data.length); return b; };
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const o = y * (w * 3 + 1) + 1 + x * 3; raw[o] = Math.round(255 * x / (w - 1)); raw[o + 1] = 90; raw[o + 2] = Math.round(255 * y / (h - 1)); }
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', new Uint8Array())]);
}
// v1 core/media.ts fingerprint(): 64 KiB start/middle/end samples behind a size header.
async function fingerprint(bytes: Uint8Array) {
  const size = bytes.length, sample = Math.min(64 * 1024, size);
  const offsets = [...new Set([0, Math.max(0, Math.floor((size - sample) / 2)), Math.max(0, size - sample)])];
  const header = new TextEncoder().encode(`powermove-media-v2\n${size}\n`);
  const parts = [header, ...offsets.map(o => bytes.subarray(o, o + sample))];
  const all = new Uint8Array(parts.reduce((s, p) => s + p.length, 0)); let at = 0; for (const p of parts) { all.set(p, at); at += p.length; }
  return `v2:${size}:${await sha256HexOf(all)}`;
}
mkdirSync(out, { recursive: true });
// Each project file with the library entry v1 kept for it (`<file>.json`):
// - Embedded.pmv: PMV4 (desktop Save), each item with its SHA-256.
// - Other.pmv: another project; its asset has the same id but other media.
// - Embedded-pmv3.pmv: PMV3 (browser download), items without a checksum.
// - Converted-pmv3.pmv: PMV3 whose asset was converted on import, so its
//   fingerprint names the chosen file, not the stored bytes.
async function save(file: string, w: number, h: number, format: 'pmv4' | 'pmv3', converted = false) {
  const image = png(w, h);
  const fp = await fingerprint(converted ? png(w + 1, h) : image);
  const asset = { id: 'gradient', name: 'Gradient.png', kind: 'image', type: 'image/png', w, h, fingerprint: fp, storageKey: `media:${fp}` };
  const proj = { id: 'v1-embedded', name: 'Embedded media', w: 640, h: 360, fps: 30, dur: 2, bg: '#202020',
    layers: [{ id: 'picture', type: 'image', name: 'Gradient', from: 0, dur: 2, d: { asset: 'gradient', fit: 'contain', w: 1, h: 1 } }],
    assets: { gradient: asset }, comps: {} };
  // app.ts saveProject(): the serialized envelope and savedHash = projectFingerprint(projectJSON).
  const projectJSON = await stringifyAsync(proj);
  const json = Buffer.from(`{"v":"1.2.0","proj":${projectJSON},"history":null,"ws":null}`);
  if (format === 'pmv3') {
    writeFileSync(path.join(out, file), encodeProjectContainer(JSON.parse(json.toString()), [{ id: 'gradient', type: 'image/png', data: image }]));
  } else {
    const dir = path.join(out, 'work');
    mkdirSync(dir, { recursive: true });
    const files = new ProjectFiles(path.join(dir, 'registry.json'), path.join(dir, 'backups'));
    const upload = await files.beginIncremental('v1-embedded', [{ id: 'gradient', revision: `v1-revision-${w}`, type: 'image/png', length: image.length }], json.length, path.join(out, file));
    await upload.write(null, json);
    for (const id of upload.required) await upload.write(id, image);
    await upload.finish();
  }
  const savedHash = await projectFingerprint(projectJSON);
  writeFileSync(path.join(out, `${file}.json`), JSON.stringify({ project: { v: '1.2.0', proj }, savedHash, baselineHash: savedHash }, null, 1) + '\n');
  console.log(JSON.stringify({ file, savedHash, fileSha256: createHash('sha256').update(await Bun.file(path.join(out, file)).bytes()).digest('hex') }));
}
await save('Embedded.pmv', 320, 180, 'pmv4');
await save('Other.pmv', 160, 90, 'pmv4');
await save('Embedded-pmv3.pmv', 320, 180, 'pmv3');
await save('Converted-pmv3.pmv', 320, 180, 'pmv3', true);
