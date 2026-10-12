// Pick reference driver (S7 deliverable 3): runs `pickScene` (ref/page.ts) in headless
// Chromium for each pick scene and writes out/picks/<scene>.json.
//
//   bun ref/picks.ts [scene ...]
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, dirname, extname } from 'node:path';

const root = join(dirname(new URL(import.meta.url).pathname), '..');
const repo = join(root, '../../..');
const { chromium } = await import(join(repo, 'apps/desktop/node_modules/playwright/index.mjs'));
const corpus = join(root, 'out/corpus');
const plan: Record<string, { times: number[]; cols: number; rows: number }> = JSON.parse(readFileSync(join(corpus, 'picks.json'), 'utf8'));
const ids = process.argv.slice(2).length ? process.argv.slice(2) : Object.keys(plan);

const build = await Bun.build({ entrypoints: [join(root, 'ref/entry.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const types: Record<string, string> = { '.json': 'application/json', '.png': 'image/png', '.glb': 'model/gltf-binary', '.webp': 'image/webp', '.avif': 'image/avif' };
const server = Bun.serve({
  port: 0,
  fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
    if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
    if (url.pathname.startsWith('/corpus/')) {
      const file = join(corpus, decodeURIComponent(url.pathname.slice(8)));
      if (!file.startsWith(corpus) || !existsSync(file)) return new Response('missing', { status: 404 });
      return new Response(Bun.file(file), { headers: { 'content-type': types[extname(file)] || 'application/octet-stream' } });
    }
    return new Response('not found', { status: 404 });
  },
});
const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const page = await browser.newPage();
page.on('pageerror', (e: any) => console.log(`[page error] ${e.message}`));
await page.goto(`http://localhost:${server.port}/`);
await page.waitForFunction(() => (globalThis as any).S7);
mkdirSync(join(root, 'out/picks'), { recursive: true });
for (const id of ids) {
  const t0 = performance.now();
  const result = await page.evaluate(([id, options]: [string, any]) => (globalThis as any).S7.pickScene(id, options), [id, plan[id]]);
  writeFileSync(join(root, 'out/picks', `${id}.json`), JSON.stringify(result));
  const frames = result.frames.map((f: any) => `t=${f.time}: ${f.hits.filter((h: any) => h).length}/${f.rays.length} hits, prod first ${f.prodFirstMs.toFixed(1)} ms, median ${f.prodMedianMs.toFixed(2)} ms, median on hits ${f.prodHitMedianMs.toFixed(2)} ms`);
  console.log(`${id} (${(performance.now() - t0).toFixed(0)} ms)\n  ${frames.join('\n  ')}`);
}
await browser.close();
server.stop(true);
