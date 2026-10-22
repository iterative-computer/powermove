/** Generate independent Blender map PNGs from original corpus media in the
 * app's hidden Electron Chromium. The parity test first writes <sha>.json
 * needs to an external directory; this writes <sha>.png beside each need.
 * Run under pm-electron-lock, with the app's dependencies installed:
 *   bun engine/crates/pm-render/tools/blender-png/run.ts --needs /tmp/...
 * Never copy the legacy capture's expected Blender PNG into this directory.
 */
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.join(here, '../../../../..');
const index = process.argv.indexOf('--needs');
if (index < 0 || !process.argv[index + 1]) throw new Error('usage: run.ts --needs EXTERNAL_DIRECTORY');
const dir = path.resolve(process.argv[index + 1]!);
if (dir.startsWith(path.resolve(repo) + path.sep)) throw new Error('converted corpus media must stay outside Git');
type Need = { content: string; source: string };
const needs: Need[] = readdirSync(dir).filter(n => /^[0-9a-f]{64}\.json$/.test(n))
  .map(n => JSON.parse(readFileSync(path.join(dir, n), 'utf8')) as Need)
  .filter(n => !existsSync(path.join(dir, `${n.content}.png`)));
for (const n of needs) {
  if (!/^[0-9a-f]{64}$/.test(n.content) ||
    createHash('sha256').update(readFileSync(n.source)).digest('hex') !== n.content) {
    throw new Error(`source changed for ${n.content}`);
  }
}
if (!needs.length) { console.log('No missing Chromium Blender PNGs.'); process.exit(0); }

const build = await Bun.build({ entrypoints: [path.join(here, 'page.ts')], target: 'browser' });
if (!build.success) throw new Error(String(build.logs));
const bundle = await build.outputs[0]!.text();
const byId = new Map(needs.map(n => [n.content, n]));
const done = new Set<string>();
const failures: string[] = [];
let finish!: (ok: boolean) => void;
const finished = new Promise<boolean>(resolve => { finish = resolve; });
const server = Bun.serve({ hostname: '127.0.0.1', port: 0, async fetch(request) {
  const url = new URL(request.url), id = url.searchParams.get('id') ?? '';
  if (url.pathname === '/') return new Response('<script src="/page.js"></script>', { headers: { 'content-type': 'text/html' } });
  if (url.pathname === '/page.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
  if (url.pathname === '/needs') return Response.json(needs.map(n => n.content));
  if (url.pathname === '/source') {
    const n = byId.get(id);
    if (!n) return new Response('unknown source', { status: 404 });
    const bytes = readFileSync(n.source);
    const mime = bytes.subarray(4, 8).toString() === 'ftyp' ? 'image/avif' : 'image/svg+xml';
    return new Response(bytes, { headers: { 'content-type': mime } });
  }
  if (url.pathname === '/png') {
    if (!byId.has(id)) return new Response('unknown result', { status: 404 });
    const bytes = new Uint8Array(await request.arrayBuffer());
    if (bytes.length < 33 || !Buffer.from(bytes.subarray(0, 8)).equals(Buffer.from([137,80,78,71,13,10,26,10]))) {
      failures.push(`${id}: invalid PNG`);
    } else {
      writeFileSync(path.join(dir, `${id}.png`), bytes);
      done.add(id);
    }
    return new Response('ok');
  }
  if (url.pathname === '/error') { failures.push(`${id}: ${await request.text()}`); return new Response('ok'); }
  if (url.pathname === '/done') { finish(true); return new Response('ok'); }
  return new Response('not found', { status: 404 });
}});
const electron: string = (await import(path.join(repo, 'apps/desktop/node_modules/electron/index.js'))).default;
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(electron, [path.join(repo, 'engine/crates/pm-vector/tools/canvas-ref/electron-main.cjs'), `--url=http://127.0.0.1:${server.port}/`],
  { stdio: ['ignore', 'inherit', 'inherit'], env });
const timeout = setTimeout(() => finish(false), 15 * 60 * 1000);
const ok = await finished;
clearTimeout(timeout);
child.kill();
server.stop(true);
for (const f of failures) console.error(f);
console.log(`${done.size}/${needs.length} Chromium Canvas2D PNGs produced in ${dir}`);
process.exit(ok && !failures.length && done.size === needs.length ? 0 : 1);
