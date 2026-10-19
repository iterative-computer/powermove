/**
 * Chromium's level 1 SVG rasters (decision S1; docs/engine/vector.md,
 * "Level 1 SVG images"), drawn by the app's own module in the app's
 * Electron (hidden window; the Chromium and GPU path the app uses).
 *
 *   --cases [--reference <canvas capture>]
 *       The 27 SVG cases of pm-vector's canvas-ref sweep, drawn by the
 *       module and compared byte for byte with the legacy capture (today's
 *       rasterSvgAsset and texFor upload, read back): the newest complete
 *       `canvas-*` capture unless named.
 *   --needs <dir>
 *       Every raster `<dir>/needs.json` lists (written by `pm-cli parity
 *       render --svg-rasters <dir>` and the corpus test's PM_SVG_RASTERS),
 *       written as `<dir>/<content>-<w>x<h>.rgba` (premultiplied RGBA8,
 *       top-down) after checking each SVG file still hashes to its content.
 *
 * Starts Electron: run under the machine-wide lock, from the worktree root.
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/svg-rasters/run.ts --cases
 */
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.join(here, '../../../../..');
const corpusRoot = path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const sha = (b: Uint8Array) => createHash('sha256').update(b).digest('hex');

type Job = { id: string; width: number; height: number; svg: Uint8Array; expect?: string; out?: string };
const jobs: Job[] = [];
let mode: 'cases' | 'needs';
let reference = '';
if (args.includes('--cases')) {
  mode = 'cases';
  reference = flag('--reference') ?? readdirSync(corpusRoot).filter((d) => d.startsWith('canvas-') && existsSync(path.join(corpusRoot, d, 'manifest.json'))).sort().at(-1)!.replace(/^/, corpusRoot + '/');
  const manifest = JSON.parse(readFileSync(path.join(reference, 'manifest.json'), 'utf8'));
  const cases = JSON.parse(readFileSync(path.join(repo, 'engine/crates/pm-vector/tests/fixtures/canvas-cases.json'), 'utf8')).cases;
  for (const c of cases) {
    if (c.kind !== 'svg') continue;
    jobs.push({ id: c.id, width: c.width, height: c.height, svg: new TextEncoder().encode(c.svg), expect: manifest.cases[c.id]?.sha256 });
  }
} else if (flag('--needs')) {
  mode = 'needs';
  const dir = path.resolve(flag('--needs')!);
  if (dir.startsWith(path.resolve(repo) + path.sep)) throw new Error('rasters never go inside the repository');
  const needs: { content: string; width: number; height: number; svg: string }[] = JSON.parse(readFileSync(path.join(dir, 'needs.json'), 'utf8'));
  for (const n of needs) {
    const svg = new Uint8Array(readFileSync(n.svg));
    if (sha(svg) !== n.content) throw new Error(`${n.svg} no longer hashes to ${n.content}`);
    jobs.push({ id: `${n.content}-${n.width}x${n.height}`, width: n.width, height: n.height, svg, out: path.join(dir, `${n.content}-${n.width}x${n.height}.rgba`) });
  }
} else {
  console.error('usage: run.ts --cases [--reference DIR] | --needs DIR');
  process.exit(64);
}

const build = await Bun.build({ entrypoints: [path.join(here, 'page.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const results = new Map<string, { sha?: string; error?: string; bytes?: number }>();
let finish!: (ok: boolean) => void;
const finished = new Promise<boolean>((r) => { finish = r; });
const byId = new Map(jobs.map((j) => [j.id, j]));
let gpu: unknown = null;
const server = Bun.serve({
  port: 0,
  hostname: '127.0.0.1',
  async fetch(req) {
    const url = new URL(req.url);
    const q = url.searchParams;
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname === '/jobs') return Response.json(jobs.map(({ id, width, height }) => ({ id, width, height })));
    if (url.pathname === '/svg') return new Response(byId.get(q.get('id')!)?.svg ?? null, { status: byId.has(q.get('id')!) ? 200 : 404 });
    const body = new Uint8Array(await req.arrayBuffer());
    if (url.pathname === '/gpu') { gpu = JSON.parse(new TextDecoder().decode(body)); return new Response('ok'); }
    if (url.pathname === '/raster') {
      const job = byId.get(q.get('id')!);
      if (!job) return new Response('bad id', { status: 400 });
      if (q.get('error')) { results.set(job.id, { error: new TextDecoder().decode(body) }); return new Response('ok'); }
      if (body.byteLength !== job.width * job.height * 4) { results.set(job.id, { error: `${body.byteLength} bytes` }); return new Response('ok'); }
      results.set(job.id, { sha: sha(body), bytes: body.byteLength });
      if (job.out) writeFileSync(job.out, body);
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
// The canvas reference tool's hidden host: never shown, never focused.
const child = spawn(electron, [path.join(repo, 'engine/crates/pm-vector/tools/canvas-ref/electron-main.cjs'), `--url=http://127.0.0.1:${server.port}/`], { stdio: ['ignore', 'inherit', 'inherit'], env });
const timeout = setTimeout(() => { console.error('capture timed out'); finish(false); }, 15 * 60 * 1000);
const ok = await finished;
clearTimeout(timeout);
for (let i = 0; i < 50 && !gpu; i++) await Bun.sleep(100);
child.kill();
server.stop(true);

const chrome = (gpu as { chrome?: string } | null)?.chrome ?? 'unknown';
if (mode === 'cases') {
  let exact = 0;
  const lines: string[] = [];
  for (const job of jobs) {
    const r = results.get(job.id);
    const same = !!r?.sha && r.sha === job.expect;
    if (same) exact++;
    lines.push(`${same ? 'exact' : 'DIFFERS'}  ${job.id}${r?.error ? `  error: ${r.error}` : ''}`);
  }
  console.log(lines.join('\n'));
  console.log(`\n${exact} of ${jobs.length} SVG cases byte-identical to ${path.basename(reference)} (Chromium ${chrome})`);
  process.exit(ok && exact === jobs.length ? 0 : 1);
} else {
  const failed = jobs.filter((j) => !results.get(j.id)?.sha);
  for (const j of failed) console.error(`not drawn: ${j.id}: ${results.get(j.id)?.error ?? 'no answer'}`);
  console.log(`${jobs.length - failed.length} of ${jobs.length} rasters written to ${path.resolve(flag('--needs')!)} (Chromium ${chrome})`);
  process.exit(ok && !failed.length ? 0 : 1);
}
