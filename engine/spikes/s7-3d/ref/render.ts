// Reference renderer driver: serves the corpus and the bundled production runtime, renders
// each scene in headless Chromium on the real GPU (ANGLE Metal), and stores outputs in
// out/<tag>/<scene>/ as PNG plus the recorded WebGL2 trace.
//
//   bun ref/render.ts [scene ...] [--tag ref] [--no-trace]
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, dirname, extname } from 'node:path';
import { encodePng } from './png';

const root = join(dirname(new URL(import.meta.url).pathname), '..');
const repo = join(root, '../../..');
const { chromium } = await import(join(repo, 'apps/desktop/node_modules/playwright/index.mjs'));

const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); if (i < 0) return undefined; const v = args[i + 1]; args.splice(i, 2); return v; };
const tag = flag('--tag') || 'ref';
const noTrace = args.includes('--no-trace');
const ids: string[] = args.filter(a => !a.startsWith('--'));
const corpus = join(root, 'out/corpus');
const sceneIds: string[] = ids.length ? ids : JSON.parse(readFileSync(join(corpus, 'index.json'), 'utf8'));

const build = await Bun.build({ entrypoints: [join(root, 'ref/entry.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();

const types: Record<string, string> = { '.json': 'application/json', '.png': 'image/png', '.glb': 'model/gltf-binary', '.webp': 'image/webp', '.avif': 'image/avif' };
const server = Bun.serve({
  port: 0,
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname.startsWith('/corpus/')) {
      const file = join(corpus, decodeURIComponent(url.pathname.slice(8)));
      if (!file.startsWith(corpus) || !existsSync(file)) return new Response('missing', { status: 404 });
      return new Response(Bun.file(file), { headers: { 'content-type': types[extname(file)] || 'application/octet-stream' } });
    }
    if (url.pathname.startsWith('/out/') && req.method === 'POST') {
      const rel = decodeURIComponent(url.pathname.slice(5));
      const target = join(root, 'out', rel);
      if (!target.startsWith(join(root, 'out'))) return new Response('bad path', { status: 400 });
      mkdirSync(dirname(target), { recursive: true });
      const body = new Uint8Array(await req.arrayBuffer());
      if (target.endsWith('.rgba')) {
        // GL rows are bottom-up; PNGs are top-down. Values are the premultiplied canvas.
        const w = Number(url.searchParams.get('w')), h = Number(url.searchParams.get('h'));
        const flipped = new Uint8Array(body.length);
        for (let y = 0; y < h; y++) flipped.set(body.subarray((h - 1 - y) * w * 4, (h - y) * w * 4), y * w * 4);
        writeFileSync(target.replace(/\.rgba$/, '.png'), encodePng(w, h, flipped));
      } else writeFileSync(target, body);
      return new Response('ok');
    }
    return new Response('not found', { status: 404 });
  },
});

const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const page = await browser.newPage();
page.on('console', (m: any) => { if (m.type() === 'error' || m.type() === 'warning') console.log(`[page ${m.type()}] ${m.text()}`); });
page.on('pageerror', (e: any) => console.log(`[page error] ${e.message}`));
await page.goto(`http://localhost:${server.port}/`);
await page.waitForFunction(() => (globalThis as any).S7);
const version = browser.version();
const summary: any[] = [];
for (const id of sceneIds) {
  const t0 = performance.now();
  try {
    const info = await page.evaluate(([id, tag, trace]: [string, string, boolean]) => (globalThis as any).S7.renderScene(id, { tag, trace }), [id, tag, !noTrace]);
    console.log(`${id}: ${info.outputs.length} outputs, ${info.drawables.length} drawables, ${(performance.now() - t0).toFixed(0)} ms${info.warnings.length ? `, warnings: ${info.warnings.join('; ')}` : ''}`);
    summary.push({ id, renderer: info.renderer, samples: info.samples, three: info.three });
  } catch (error) {
    console.log(`${id}: FAILED ${(error as Error).message}`);
    summary.push({ id, error: String(error) });
  }
}
mkdirSync(join(root, 'out', tag), { recursive: true });
writeFileSync(join(root, 'out', tag, 'run.json'), JSON.stringify({ chromium: version, scenes: summary }, null, 1));
await browser.close();
server.stop(true);
