/**
 * three.js r180 references for pm-render's 3D pass (docs/engine/pm-render.md).
 *
 * Renders every case of tests/fixtures/three-scenes.json with the production
 * SceneRuntime in headless Chromium (ANGLE on Metal, the GPU path the app uses)
 * and writes, outside the repository:
 *
 *   <out>/<case>/msaa.png    the layer's premultiplied canvas, top-down (what the
 *                            compositor uploads)
 *   <out>/<case>/noaa.png    the same passes without MSAA (edge classification)
 *   <out>/<case>/record.json program sources and every draw's uniform values
 *
 * and tests/fixtures/three-programs.json (SHA-256 of each case's program sources,
 * for the byte-for-byte program generation test).
 *
 * Starts Chromium: run under the machine-wide lock.
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/three-ref/run.ts [--out <dir>]
 */
import { createHash } from 'node:crypto';
import { execSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';
import { homedir } from 'node:os';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const crate = path.join(here, '../..');
const repo = path.join(crate, '../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const commit = execSync('git rev-parse --short=8 HEAD', { cwd: repo }).toString().trim();
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const out = path.resolve(flag('--out') || path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus', `three-${stamp}-${commit}`));
if (out.startsWith(path.resolve(repo) + path.sep)) throw new Error('references never go inside the repository');
const cases = JSON.parse(readFileSync(path.join(crate, 'tests/fixtures/three-scenes.json'), 'utf8')).cases;

const crcTable = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = (b: Uint8Array) => { let c = 0xffffffff; for (const x of b) c = crcTable[(c ^ x) & 255]! ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
function chunk(type: string, data: Uint8Array) {
  const t = new TextEncoder().encode(type), out = new Uint8Array(12 + data.length), v = new DataView(out.buffer);
  v.setUint32(0, data.length); out.set(t, 4); out.set(data, 8);
  v.setUint32(8 + data.length, crc(out.subarray(4, 8 + data.length)));
  return out;
}
function png(w: number, h: number, rgba: Uint8Array) {
  const raw = new Uint8Array((w * 4 + 1) * h);
  for (let y = 0; y < h; y++) raw.set(rgba.subarray(y * w * 4, (y + 1) * w * 4), y * (w * 4 + 1) + 1);
  const ihdr = new Uint8Array(13), v = new DataView(ihdr.buffer);
  v.setUint32(0, w); v.setUint32(4, h); ihdr[8] = 8; ihdr[9] = 6;
  const parts = [new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw)), chunk('IEND', new Uint8Array())];
  return Buffer.concat(parts);
}

const build = await Bun.build({ entrypoints: [path.join(here, 'page.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const server = Bun.serve({
  port: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
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
mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const fixtures: any[] = [];
try {
  const page = await browser.newPage();
  page.on('pageerror', (e: any) => console.log(`[page error] ${e.message}`));
  page.on('console', (m: any) => { if (m.type() === 'error') console.log(`[page] ${m.text()}`); });
  await page.goto(`http://localhost:${server.port}/`);
  await page.waitForFunction(() => (globalThis as any).ThreeRef);
  for (const c of cases) {
    const info = await page.evaluate(([c]: [any]) => (globalThis as any).ThreeRef.renderCase(c, 'x'), [c]);
    const record = JSON.parse(readFileSync(path.join(out, c.id, 'record.json'), 'utf8'));
    const sha = (s: string) => createHash('sha256').update(s).digest('hex');
    if (record.programs.length !== 1) throw new Error(`${c.id}: expected one program, got ${record.programs.length}`);
    const p = record.programs[0];
    const lights = c.lights.filter((l: any) => l.visible !== false).length;
    fixtures.push({ case: c.id, doubleSided: c.object.material.doubleSided, directionalLights: lights, hemisphereLights: 1,
      vertex: { sha256: sha(p.vertex), bytes: Buffer.byteLength(p.vertex) }, fragment: { sha256: sha(p.fragment), bytes: Buffer.byteLength(p.fragment) } });
    console.log(`${c.id}: ${info.draws} draws, MSAA samples ${record.samples}`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ three: '0.180.0', chromium: browser.version(), commit, cases: cases.map((c: any) => c.id) }, null, 1));
} finally {
  await browser.close();
  server.stop(true);
}
writeFileSync(path.join(crate, 'tests/fixtures/three-programs.json'), JSON.stringify({ three: '0.180.0', programs: fixtures }, null, 1) + '\n');
console.log(`references in ${out}`);
