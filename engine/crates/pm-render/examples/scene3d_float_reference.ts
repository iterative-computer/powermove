/**
 * Capture an independent Three.js r180 RGBA32F, 8x linear scene reference.
 * Run under pm-electron-lock because this opens Chromium:
 *   bun engine/crates/pm-render/examples/scene3d_float_reference.ts \
 *     --bundle <prebuilt-browser-bundle> \
 *     --capture <existing-scene3d-capture> --out /tmp/ne-w5/l4/float-reference
 * Build the browser bundle outside the lock with `bun build` first.
 * The capture supplies authored fixture textures. Output stays outside Git.
 */
import path from 'node:path';
import { existsSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';

const crate = path.resolve(import.meta.dir, '..');
const repo = path.resolve(crate, '../../..');
const argv = process.argv.slice(2);
const flag = (key: string) => { const n = argv.indexOf(key); return n < 0 ? undefined : argv[n + 1]; };
const capture = flag('--capture'), output = flag('--out'), bundlePath = flag('--bundle');
if (!capture || !output || !bundlePath) throw new Error('--capture, --out and --bundle are required');
const out = path.resolve(output), source = path.resolve(capture);
if (out.startsWith(repo + path.sep)) throw new Error('reference evidence must stay outside Git');
mkdirSync(out, { recursive: true });
rmSync(path.join(out, 'manifest.json'), { force: true });
const fixtureName = argv.includes('--gltf') ? 'scene3d-gltf-cases.json' : 'scene3d-cases.json';
const fixture = JSON.parse(readFileSync(path.join(crate, 'tests/fixtures', fixtureName), 'utf8'));
const only = flag('--only');
const cases = fixture.cases.filter((c: any) => !only || c.id.startsWith(only));
const bundle = readFileSync(path.resolve(bundlePath), 'utf8');
const server = Bun.serve({ port: 0, async fetch(request) {
  const url = new URL(request.url);
  if (url.pathname === '/') return new Response('<!doctype html><script src="/bundle.js"></script>',
    { headers: { 'content-type': 'text/html' } });
  if (url.pathname === '/bundle.js') return new Response(bundle,
    { headers: { 'content-type': 'text/javascript' } });
  if (url.pathname.startsWith('/tex/')) {
    const name = path.basename(url.pathname);
    return new Response(Bun.file(path.join(source, 'tex', name)));
  }
  if (url.pathname.startsWith('/models/')) {
    const name = path.basename(decodeURIComponent(url.pathname.slice(8)));
    return new Response(Bun.file(path.join(source, 'models', name)));
  }
  if (url.pathname.startsWith('/out/') && request.method === 'POST') {
    const relative = decodeURIComponent(url.pathname.slice(5));
    const destination = path.resolve(out, relative);
    if (!destination.startsWith(out + path.sep)) return new Response('bad output', { status: 400 });
    mkdirSync(path.dirname(destination), { recursive: true });
    writeFileSync(destination, new Uint8Array(await request.arrayBuffer()));
    return new Response('ok');
  }
  return new Response('not found', { status: 404 });
} });
const { chromium } = await import(path.join(repo, 'apps/desktop/node_modules/playwright/index.mjs'));
let browser;
try {
  browser = await chromium.launch({ headless: true,
    args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
  const page = await browser.newPage();
  page.on('pageerror', error => console.error(error));
  await page.goto(`http://localhost:${server.port}/`);
  await page.waitForFunction(() => !!(globalThis as any).Scene3dFloatReference);
  const refused: string[] = [];
  for (const c of cases) {
    try {
      await page.evaluate(c => (globalThis as any).Scene3dFloatReference.renderCase(c), c);
      console.log(c.id);
    } catch (error) {
      if (fixtureName !== 'scene3d-gltf-cases.json' || !existsSync(path.join(source, `${c.id}.refused.txt`)))
        throw error;
      refused.push(c.id);
      console.log(`${c.id}: independently refused`);
    }
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({
    three: '0.180.0', chromium: browser.version(), scale: 8, format: 'rgba32float',
    space: 'premultiplied linear sRGB', fixture: fixtureName,
    sampling: '8x8 spatial; 8x8 subpixel camera jitter at native size for pixel-width lines and points; triangle barycentric one-pixel wireframe so r180 material derivatives are defined',
    look: 'authored editor material state applied explicitly',
    captureId: path.basename(source),
    cases: cases.map((c: any) => c.id), refused,
  }, null, 2));
} finally {
  await browser?.close();
  server.stop(true);
}
