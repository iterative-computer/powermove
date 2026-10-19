/**
 * Path mask coverage references for pm-render's port of today's bezier path
 * mask route (docs/engine/composite.md "Path masks").
 *
 * Runs the coverage half of `applyPathMasks` (legacy/gl/compositor.ts, from
 * the function's first statement up to the coverage apply pass, sliced
 * verbatim) for every case of cases.ts in the app's Electron (hidden window;
 * the same Chromium, Canvas2D, and GPU path the app uses) and writes,
 * outside the repository:
 *
 *   <out>/<case>.png      the coverage canvas as the apply pass samples it
 *                         (premultiplied white: every channel is coverage)
 *   <out>/manifest.json   Chromium and GPU facts, the source slice's hash,
 *                         every case's size, and errors
 *
 * and tests/fixtures/mask-cases.json (the case list the Rust tests read).
 *
 * Starts Electron: run under the machine-wide lock.
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-vector/tools/mask-ref/run.ts [--out <dir>]
 */
import { execSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { deflateSync } from 'node:zlib';
import { cases } from './cases';

const here = path.dirname(new URL(import.meta.url).pathname);
const crate = path.join(here, '../..');
const repo = path.join(crate, '../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const commit = execSync('git rev-parse --short=8 HEAD', { cwd: repo }).toString().trim();
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const out = path.resolve(flag('--out') || path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus', `masks-${stamp}-${commit}`));
if (out.startsWith(path.resolve(repo) + path.sep)) throw new Error('references never go inside the repository');

// The coverage half of today's applyPathMasks, verbatim.
const compositor = readFileSync(path.join(repo, 'apps/desktop/src/renderer/src/legacy/gl/compositor.ts'), 'utf8');
const head = 'function applyPathMasks(L:any,T:number,srcF:any,W:number,H:number,masks:any[]) {';
const start = compositor.indexOf(head), end = compositor.indexOf('  const out=grab(W,H);', start);
if (start < 0 || end < 0) throw new Error('applyPathMasks changed; update the slice markers');
const body = compositor.slice(start + head.length, end);
const transpiled = new Bun.Transpiler({ loader: 'ts' }).transformSync(`export function coverage(L:any,T:number,W:number,H:number,masks:any[]) {${body}  return cv;\n}`);
const source = transpiled.replace(/^export\s+/m, '') + '\nreturn coverage;';
const sliceHash = createHash('sha256').update(body).digest('hex');

const list = cases();
writeFileSync(path.join(crate, 'tests/fixtures/mask-cases.json'), JSON.stringify({ cases: list }, null, 1) + '\n');

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

const build = await Bun.build({ entrypoints: [path.join(here, 'page.ts')], target: 'browser', minify: false, define: { COVERAGE_SOURCE: JSON.stringify(source) } });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
mkdirSync(out, { recursive: true });
const manifest: any = { kind: 'path-mask-coverage', premultiplied: true, rows: 'top-down', commit, sliceSha256: sliceHash, cases: {} as Record<string, unknown> };
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
    const body = new Uint8Array(await req.arrayBuffer());
    if (url.pathname === '/gpu') { manifest.gpu = JSON.parse(new TextDecoder().decode(body)); return new Response('ok'); }
    if (url.pathname === '/case') {
      const id = q.get('id')!;
      if (!/^[a-z0-9.-]+$/i.test(id)) return new Response('bad id', { status: 400 });
      if (q.get('kind') === 'error') { manifest.cases[id] = JSON.parse(new TextDecoder().decode(body)); return new Response('ok'); }
      const w = Number(q.get('w')), h = Number(q.get('h'));
      writeFileSync(path.join(out, `${id}.png`), png(w, h, body));
      manifest.cases[id] = { width: w, height: h, sha256: createHash('sha256').update(body).digest('hex') };
      return new Response('ok');
    }
    if (url.pathname === '/done') { finish(true); return new Response('ok'); }
    if (url.pathname === '/failed') { console.error(new TextDecoder().decode(body)); finish(false); return new Response('ok'); }
    return new Response('not found', { status: 404 });
  },
});

const electron: string = (await import(path.join(repo, 'apps/desktop/node_modules/electron/index.js'))).default;
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
// The canvas-ref host: hidden, unfocusable, never in the Dock.
const child = spawn(electron, [path.join(here, '../canvas-ref/electron-main.cjs'), `--url=http://127.0.0.1:${server.port}/`], { stdio: ['ignore', 'inherit', 'inherit'], env });
const timeout = setTimeout(() => { console.error('capture timed out'); finish(false); }, 15 * 60 * 1000);
const ok = await finished;
clearTimeout(timeout);
for (let i = 0; i < 100 && !manifest.gpu; i++) await Bun.sleep(100);
child.kill();
server.stop(true);
manifest.chromium = manifest.gpu?.chrome;
writeFileSync(path.join(out, 'manifest.json'), JSON.stringify(manifest, null, 1));
console.log(`${Object.keys(manifest.cases).length} of ${list.length} cases in ${out}`);
process.exit(ok ? 0 : 1);
