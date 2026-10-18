/**
 * Canvas2D references for pm-vector's compat level 1 rasterizer
 * (docs/engine/vector.md).
 *
 * Rasterizes every case of cases.ts with today's raster code in the app's
 * Electron (hidden window; the same Chromium and GPU path the app uses) and
 * writes, outside the repository:
 *
 *   <out>/<case>.png      the canvas as the compositor's texture holds it:
 *                         premultiplied RGBA8, top-down (the PNG carries the
 *                         premultiplied bytes as they are)
 *   <out>/manifest.json   Chromium and GPU facts, every case's size, layer
 *                         geometry, and errors
 *
 * and tests/fixtures/canvas-cases.json (the case list the Rust tests read).
 *
 * Starts Electron: run under the machine-wide lock.
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-vector/tools/canvas-ref/run.ts [--out <dir>]
 */
import { execSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { cases, type Case } from './cases';

const here = path.dirname(new URL(import.meta.url).pathname);
const crate = path.join(here, '../..');
const repo = path.join(crate, '../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const commit = execSync('git rev-parse --short=8 HEAD', { cwd: repo }).toString().trim();
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const out = path.resolve(flag('--out') || path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus', `canvas-${stamp}-${commit}`));
if (out.startsWith(path.resolve(repo) + path.sep)) throw new Error('references never go inside the repository');

// The page's copy of rasterShape and rr must be today's code, character for character.
const between = (text: string, a: string, b: string) => {
  const i = text.indexOf(a), j = text.indexOf(b, i);
  if (i < 0 || j < 0) throw new Error(`markers ${a} … ${b} not found`);
  return text.slice(i, j);
};
const raster = readFileSync(path.join(repo, 'apps/desktop/src/renderer/src/legacy/gl/raster.ts'), 'utf8');
const page = readFileSync(path.join(here, 'page.ts'), 'utf8');
const legacyCopy = between(raster, 'function rr(', '/* ── captions');
const pageCopy = between(page, 'function rr(', '// END VERBATIM');
if (legacyCopy.trim() !== pageCopy.trim()) throw new Error('page.ts rasterShape differs from legacy/gl/raster.ts; update the verbatim copy');

// `--cases <file.json>` runs a custom case list ({ cases: [...] }) for
// experiments against Chromium; the fixture is only rewritten for the
// built-in list.
const custom = flag('--cases');
const list: Case[] = custom ? JSON.parse(readFileSync(custom, 'utf8')).cases : cases();
if (!custom) writeFileSync(path.join(crate, 'tests/fixtures/canvas-cases.json'), JSON.stringify({ cases: list }, null, 1) + '\n');

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

const build = await Bun.build({ entrypoints: [path.join(here, 'page.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
mkdirSync(out, { recursive: true });
const manifest: any = { kind: 'canvas2d-raster', premultiplied: true, rows: 'top-down', commit, cases: {} as Record<string, unknown> };
let finish!: (ok: boolean) => void;
const finished = new Promise<boolean>(r => { finish = r; });
const server = Bun.serve({
  port: 0,
  hostname: '127.0.0.1',
  async fetch(req) {
    const url = new URL(req.url);
    const q = url.searchParams;
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname === '/cases') return new Response(JSON.stringify(list), { headers: { 'content-type': 'application/json' } });
    const body = new Uint8Array(await req.arrayBuffer());
    if (url.pathname === '/gpu') { manifest.gpu = JSON.parse(new TextDecoder().decode(body)); return new Response('ok'); }
    if (url.pathname === '/case') {
      const id = q.get('id')!;
      if (!/^[a-z0-9.-]+$/i.test(id)) return new Response('bad id', { status: 400 });
      if (q.get('kind') === 'error') { manifest.cases[id] = JSON.parse(new TextDecoder().decode(body)); return new Response('ok'); }
      const w = Number(q.get('w')), h = Number(q.get('h'));
      // texImage2D from a canvas without UNPACK_FLIP_Y puts the canvas's top
      // row at texture row 0, which readPixels returns first: the bytes are
      // already top-down.
      writeFileSync(path.join(out, `${id}.png`), png(w, h, body));
      manifest.cases[id] = { width: w, height: h, ...JSON.parse(q.get('meta') || '{}'), sha256: createHash('sha256').update(body).digest('hex') };
      return new Response('ok');
    }
    if (url.pathname === '/done') { finish(true); return new Response('ok'); }
    if (url.pathname === '/failed') { console.error(new TextDecoder().decode(body)); finish(false); return new Response('ok'); }
    return new Response('not found', { status: 404 });
  },
});

// The electron package's index exports the binary's path.
const electron: string = (await import(path.join(repo, 'apps/desktop/node_modules/electron/index.js'))).default;
// A parent running inside Electron (or an agent host) may export
// ELECTRON_RUN_AS_NODE, which would start Electron as plain Node.
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(electron, [path.join(here, 'electron-main.cjs'), `--url=http://127.0.0.1:${server.port}/`], { stdio: ['ignore', 'inherit', 'inherit'], env });
const timeout = setTimeout(() => { console.error('capture timed out'); finish(false); }, 15 * 60 * 1000);
const ok = await finished;
clearTimeout(timeout);
// The host reports GPU facts shortly after the page loads.
for (let i = 0; i < 100 && !manifest.gpu; i++) await Bun.sleep(100);
child.kill();
server.stop(true);
manifest.chromium = manifest.gpu?.chrome;
writeFileSync(path.join(out, 'manifest.json'), JSON.stringify(manifest, null, 1));
console.log(`${Object.keys(manifest.cases).length} of ${list.length} cases in ${out}`);
process.exit(ok ? 0 : 1);
