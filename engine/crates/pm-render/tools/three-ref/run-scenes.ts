/**
 * three.js r180 references for the native 3D scene pass (docs/engine/scene3d.md).
 *
 * Writes tests/fixtures/scene3d-cases.json (the sweep of tools/three-ref/cases.ts),
 * generates the texture images the cases use, and renders every case with the
 * production SceneRuntime in headless Chromium (ANGLE on Metal, the GPU path the
 * app uses), writing outside the repository:
 *
 *   <out>/tex/<name>.png             the texture images (8-bit RGBA, opaque)
 *   <out>/<case>/<k>.msaa.png       output k, the premultiplied canvas, top-down
 *   <out>/<case>/<k>.noaa.png       the same without MSAA (edge classification)
 *   <out>/<case>/<k>.json           program sources and every draw's uniforms
 *
 * Starts Chromium: run under the machine-wide lock.
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/three-ref/run-scenes.ts [--out <dir>] [--only <prefix>]
 */
import { execSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import { homedir } from 'node:os';
import path from 'node:path';
import { cases } from './cases';

const here = path.dirname(new URL(import.meta.url).pathname);
const crate = path.join(here, '../..');
const repo = path.join(crate, '../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const commit = execSync('git rev-parse --short=8 HEAD', { cwd: repo }).toString().trim();
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const out = path.resolve(flag('--out') || path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus', `scene3d-${stamp}-${commit}`));
if (out.startsWith(path.resolve(repo) + path.sep)) throw new Error('references never go inside the repository');
const all = cases();
writeFileSync(path.join(crate, 'tests/fixtures/scene3d-cases.json'), JSON.stringify({ three: '0.180.0', cases: all }, null, 1) + '\n');
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

/** Deterministic texture images (opaque, top row first). */
function texture(name: string): [number, number, Uint8Array] {
  const [w, h] = name === 'checker-128x96' ? [128, 96] : name === 'dots-96x64' ? [96, 64] : [64, 64];
  const px = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const i = (y * w + x) * 4;
    let r = 0, g = 0, b = 0;
    if (name === 'checker-128x96') {
      const c = ((x >> 4) + (y >> 4)) & 1;
      r = c ? 230 : 40 + (x >> 1); g = c ? 200 - (y >> 1) : 60; b = c ? 60 : 180 + (y >> 2);
      if (y < 8) { r = 255; g = 0; b = 0; } // a red band marks the top rows
    } else if (name === 'bumps-64') {
      const nx = 0.6 * Math.sin(x / 64 * Math.PI * 4), ny = 0.6 * Math.cos(y / 64 * Math.PI * 4);
      const nz = Math.sqrt(Math.max(0, 1 - nx * nx - ny * ny));
      r = Math.round((nx * 0.5 + 0.5) * 255); g = Math.round((ny * 0.5 + 0.5) * 255); b = Math.round((nz * 0.5 + 0.5) * 255);
    } else if (name === 'ramp-64') {
      r = Math.round(x / 63 * 255); g = Math.round(y / 63 * 255); b = 255 - Math.round((x + y) / 126 * 255);
    } else if (name === 'dots-96x64') {
      const dx = (x % 16) - 7.5, dy = (y % 16) - 7.5, on = dx * dx + dy * dy < 25;
      r = on ? 255 : 10; g = on ? 180 : 10; b = on ? 40 : 20;
    }
    px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = 255;
  }
  return [w, h, px];
}
mkdirSync(path.join(out, 'tex'), { recursive: true });
const names = new Set(all.flatMap(c => Object.values(c.textures || {})));
for (const name of names) { const [w, h, px] = texture(name); writeFileSync(path.join(out, 'tex', `${name}.png`), png(w, h, px)); }

const build = await Bun.build({ entrypoints: [path.join(here, 'scene-page.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const server = Bun.serve({
  port: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname.startsWith('/tex/')) return new Response(Bun.file(path.join(out, 'tex', path.basename(url.pathname))));
    if (url.pathname.startsWith('/out/') && req.method === 'POST') {
      const target = path.join(out, decodeURIComponent(url.pathname.slice(5)).replace(/^[^/]*\//, ''));
      if (!target.startsWith(out + path.sep)) return new Response('bad path', { status: 400 });
      mkdirSync(path.dirname(target), { recursive: true });
      const body = new Uint8Array(await req.arrayBuffer());
      if (target.endsWith('.rgba')) {
        // GL rows are bottom-up; PNGs are top-down.
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
try {
  const page = await browser.newPage();
  page.on('pageerror', (e: any) => console.log(`[page error] ${e.message}`));
  page.on('console', (m: any) => { if (m.type() === 'error' || m.type() === 'warning') console.log(`[page] ${m.text()}`); });
  await page.goto(`http://localhost:${server.port}/`);
  await page.waitForFunction(() => (globalThis as any).ThreeRef);
  for (const c of selected) {
    const info = await page.evaluate(([c]: [any]) => (globalThis as any).ThreeRef.renderCase(c), [c]);
    console.log(`${c.id}: ${info.outputs.map((o: any) => o.draws).join(', ')} draws`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ three: '0.180.0', chromium: browser.version(), commit, cases: selected.map(c => c.id) }, null, 1));
} finally {
  await browser.close();
  server.stop(true);
}
console.log(`references in ${out}`);
