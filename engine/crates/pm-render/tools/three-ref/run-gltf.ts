/**
 * three.js r180 references for glTF in the native 3D pass (docs/engine/gltf.md).
 *
 * Builds the synthetic models (tools/three-ref/gltf/models.ts; WebP through
 * `cwebp`, AVIF through FFmpeg's libaom: set FFMPEG to an ffmpeg with
 * libaom-av1, e.g. ffmpeg-static), copies the Khronos sample models from
 * the corpus directory (`khronos-gltf/`, downloaded there, never into the
 * repository), writes tests/fixtures/scene3d-gltf-cases.json, and renders
 * every case with the production SceneRuntime and importer in headless
 * Chromium (ANGLE on Metal), outside the repository:
 *
 *   <out>/models/<file>             the models every case loads
 *   <out>/<case>/<k>.msaa.png       output k, the premultiplied canvas, top-down
 *   <out>/<case>/<k>.noaa.png       the same without MSAA (edge classification)
 *   <out>/<case>/<k>.json           program sources and every draw's uniforms
 *   <out>/<case>/pose.json          posed drawables and lights of the model layers
 *   <out>/<case>/picks.json         three.js raycasts on a ray grid
 *
 * Starts Chromium: run under the machine-wide lock.
 *   FFMPEG=<ffmpeg> ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/three-ref/run-gltf.ts [--out <dir>] [--only <prefix>]
 */
import { execFileSync, execSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { cases } from './gltf-cases';
import * as models from './gltf/models';
import { encodePng } from './gltf/png';
import * as tex from './gltf/textures';

const here = path.dirname(new URL(import.meta.url).pathname);
const crate = path.join(here, '../..');
const repo = path.join(crate, '../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const commit = execSync('git rev-parse --short=8 HEAD', { cwd: repo }).toString().trim();
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const corpus = path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus');
const out = path.resolve(flag('--out') || path.join(corpus, `gltf-scene3d-${stamp}-${commit}`));
if (out.startsWith(path.resolve(repo) + path.sep)) throw new Error('references never go inside the repository');
const dir = path.join(out, 'models');
mkdirSync(dir, { recursive: true });

// Synthetic models.
const work = path.join(tmpdir(), `pm-gltf-${process.pid}`);
mkdirSync(work, { recursive: true });
writeFileSync(path.join(work, 'orientation.png'), encodePng(256, 256, tex.orientation().pixels));
writeFileSync(path.join(work, 'checker.png'), encodePng(256, 256, tex.checker().pixels));
execFileSync('cwebp', ['-quiet', '-lossless', '-exact', path.join(work, 'orientation.png'), '-o', path.join(work, 'orientation.webp')]);
const ffmpeg = process.env.FFMPEG;
if (!ffmpeg) throw new Error('set FFMPEG to an ffmpeg with libaom-av1 (ffmpeg-static)');
execFileSync(ffmpeg, ['-loglevel', 'error', '-y', '-i', path.join(work, 'checker.png'), '-c:v', 'libaom-av1', '-still-picture', '1',
  '-crf', '18', '-cpu-used', '4', '-pix_fmt', 'yuv444p', '-color_primaries', 'bt709', '-color_trc', 'iec61966-2-1', '-colorspace', 'bt709',
  '-color_range', 'pc', path.join(work, 'checker.avif')]);
writeFileSync(path.join(dir, 'skinned.glb'), models.skinned());
writeFileSync(path.join(dir, 'pbr-ext.glb'), models.pbrExtensions());
writeFileSync(path.join(dir, 'transmission.glb'), models.transmission());
writeFileSync(path.join(dir, 'webp-avif.glb'), models.webpAvif(readFileSync(path.join(work, 'orientation.webp')), readFileSync(path.join(work, 'checker.avif'))));

// Khronos sample models.
const khronosDir = path.join(corpus, 'khronos-gltf');
if (existsSync(khronosDir)) for (const f of readdirSync(khronosDir)) if (/\.(glb|gltf)$/.test(f)) copyFileSync(path.join(khronosDir, f), path.join(dir, f));
const available = new Set(readdirSync(dir));
const all = cases(available);
writeFileSync(path.join(crate, 'tests/fixtures/scene3d-gltf-cases.json'), JSON.stringify({ three: '0.180.0', cases: all }, null, 1) + '\n');
const only = flag('--only');
const selected = only ? all.filter(c => c.id.startsWith(only)) : all;

const crcTable = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = (b: Uint8Array) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255]! ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
function chunk(type: string, data: Uint8Array) {
  const t = new TextEncoder().encode(type), o = new Uint8Array(12 + data.length), v = new DataView(o.buffer);
  v.setUint32(0, data.length); o.set(t, 4); o.set(data, 8);
  v.setUint32(8 + data.length, crc(o.subarray(4, 8 + data.length)));
  return o;
}
function png(w: number, h: number, rgba: Uint8Array) {
  const raw = new Uint8Array((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) raw.set(rgba.subarray(y * w * 4, (y + 1) * w * 4), y * (w * 4 + 1) + 1);
  const ihdr = new Uint8Array(13), v = new DataView(ihdr.buffer);
  v.setUint32(0, w); v.setUint32(4, h); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', new Uint8Array())]);
}

const build = await Bun.build({ entrypoints: [path.join(here, 'scene-page.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const server = Bun.serve({
  port: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname.startsWith('/models/')) {
      const f = path.join(dir, path.basename(decodeURIComponent(url.pathname.slice(8))));
      return existsSync(f) ? new Response(Bun.file(f)) : new Response('not found', { status: 404 });
    }
    if (url.pathname.startsWith('/out/') && req.method === 'POST') {
      const target = path.join(out, decodeURIComponent(url.pathname.slice(5)).replace(/^[^/]*\//, ''));
      if (!target.startsWith(out + path.sep)) return new Response('bad path', { status: 400 });
      mkdirSync(path.dirname(target), { recursive: true });
      const body = new Uint8Array(await req.arrayBuffer());
      if (target.endsWith('.rgba')) {
        const w = Number(url.searchParams.get('w')), h = Number(url.searchParams.get('h'));
        const flipped = new Uint8Array(body.length);
        for (let y = 0; y < h; y++) flipped.set(body.subarray((h - 1 - y) * w * 4, (h - y) * w * 4), y * w * 4);
        writeFileSync(target.replace(/\.rgba$/, '.png'), png(w, h, flipped));
      } else writeFileSync(target, body);
      return new Response('ok');
    }
    return new Response('not found', { status: 404 });
  },
});

const { chromium } = await import(path.join(repo, 'apps/desktop/node_modules/playwright/index.mjs'));
const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const failures: string[] = [];
try {
  const page = await browser.newPage();
  page.on('pageerror', (e: any) => console.log(`[page error] ${e.message}`));
  page.on('console', (m: any) => { if (m.type() === 'error') console.log(`[page] ${m.text()}`); });
  await page.goto(`http://localhost:${server.port}/`);
  await page.waitForFunction(() => (globalThis as any).ThreeRef);
  const extensions = await page.evaluate(() => document.createElement('canvas').getContext('webgl2')!.getSupportedExtensions());
  for (const c of selected) {
    try {
      const info = await page.evaluate(([c]: [any]) => (globalThis as any).ThreeRef.renderCase(c), [c]);
      console.log(`${c.id}: ${info.outputs.map((o: any) => o.draws).join(', ')} draws`);
    } catch (e: any) {
      // The importer refused the model: the case records the refusal.
      failures.push(c.id);
      writeFileSync(path.join(out, `${c.id}.refused.txt`), String(e?.message || e));
      console.log(`${c.id}: refused (${String(e?.message || e).split('\n')[0]})`);
    }
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ three: '0.180.0', chromium: browser.version(), commit, extensions,
    cases: selected.map(c => c.id), refused: failures }, null, 1));
} finally {
  await browser.close();
  server.stop(true);
}
console.log(`references in ${out}`);
