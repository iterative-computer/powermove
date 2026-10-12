// Drives the shipped Blender worker (apps/desktop/src/main/blender-worker.ts) the way
// BlenderWorker does (same flags, stdin envelopes, request files), with snapshots built
// by the reference page's port of renderSnapshot. Two worker variants:
//   today:  the worker unchanged (8-bit PNG, default dither, AgX)
//   pinned: plan section 17's settings (16-bit RGBA PNG, AgX, look None, dither 0, sRGB
//           display, gamma 1, image color management following the scene, fast compression)
// Each job renders every layer in one invocation ("batched") and, for batching equality,
// each layer in its own fresh Blender process ("single").
//
//   bun blender/run.ts [--blender /Applications/Blender.app/Contents/MacOS/Blender]
//                      [--variant pinned|today] [--engine eevee|cycles] [--samples N]
//                      [--device cpu|auto|gpu] [--no-single] scene ...
import { mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { join, dirname, extname } from 'node:path';
import { spawn } from 'node:child_process';

const root = join(dirname(new URL(import.meta.url).pathname), '..');
const repo = join(root, '../../..');
const args = process.argv.slice(2);
const flag = (name: string, fallback: string) => { const i = args.indexOf(name); if (i < 0) return fallback; const v = args[i + 1]!; args.splice(i, 2); return v; };
const blender = flag('--blender', '/Applications/Blender.app/Contents/MacOS/Blender');
const variant = flag('--variant', 'pinned');
const engine = flag('--engine', 'eevee');
const samples = Number(flag('--samples', '16'));
const device = flag('--device', 'cpu');
const single = !args.includes('--no-single');
const scenes = args.filter(a => !a.startsWith('--'));

const { BLENDER_WORKER } = await import(join(repo, 'apps/desktop/src/main/blender-worker.ts'));
const PINNED = [
  "scene.render.image_settings.file_format='PNG';scene.render.image_settings.color_mode='RGBA';scene.render.image_settings.color_depth='16';scene.render.image_settings.compression=15",
  "scene.render.dither_intensity=0",
  "scene.display_settings.display_device='sRGB';scene.view_settings.view_transform='AgX';scene.view_settings.look='None';scene.view_settings.gamma=1",
  "scene.render.image_settings.color_management='FOLLOW_SCENE'",
].map(l => '    ' + l).join('\n');
function workerSource(): string {
  if (variant === 'today') return BLENDER_WORKER;
  const anchor = '    scene.render.use_file_extension=True\n';
  if (!BLENDER_WORKER.includes(anchor)) throw new Error('worker anchor moved');
  return BLENDER_WORKER.replace(anchor, anchor + PINNED + '\n');
}

// Snapshots from the reference page (production SceneRuntime in headless Chromium).
const build = await Bun.build({ entrypoints: [join(root, 'ref/entry.ts')], target: 'browser', minify: false });
if (!build.success) { console.error(build.logs); process.exit(1); }
const bundle = await build.outputs[0]!.text();
const corpus = join(root, 'out/corpus');
const types: Record<string, string> = { '.json': 'application/json', '.png': 'image/png', '.glb': 'model/gltf-binary' };
const server = Bun.serve({ port: 0, fetch(req) {
  const url = new URL(req.url);
  if (url.pathname === '/') return new Response('<!doctype html><meta charset=utf-8><body><script src="/bundle.js"></script>', { headers: { 'content-type': 'text/html' } });
  if (url.pathname === '/bundle.js') return new Response(bundle, { headers: { 'content-type': 'text/javascript' } });
  const file = join(corpus, decodeURIComponent(url.pathname.slice(8)));
  if (url.pathname.startsWith('/corpus/') && file.startsWith(corpus) && existsSync(file)) return new Response(Bun.file(file), { headers: { 'content-type': types[extname(file)] || 'application/octet-stream' } });
  return new Response('not found', { status: 404 });
} });
const { chromium } = await import(join(repo, 'apps/desktop/node_modules/playwright/index.mjs'));
const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const page = await browser.newPage();
await page.goto(`http://localhost:${server.port}/`);
await page.waitForFunction(() => (globalThis as any).S7);
const snapshots: Record<string, any> = {};
for (const id of scenes) snapshots[id] = await page.evaluate(([id, s]: [string, any]) => (globalThis as any).S7.blenderSnapshot(id, s), [id, { engine, samples, denoise: false, device }]);
await browser.close();
server.stop(true);

class Worker {
  proc; buffer = ''; waiting = new Map<string, (m: any) => void>(); stderr = '';
  constructor(script: string) {
    this.proc = spawn(blender, ['--background', '--factory-startup', '--disable-autoexec', '--python-exit-code', '1', '--python', script], { stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, PYTHONUNBUFFERED: '1' } });
    this.proc.stderr.on('data', (b: Buffer) => { this.stderr = (this.stderr + b).slice(-8000); });
    this.proc.stdout.on('data', (b: Buffer) => {
      this.buffer += b;
      let i;
      while ((i = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, i); this.buffer = this.buffer.slice(i + 1);
        if (!line.startsWith('PM_BLENDER:')) continue;
        const msg = JSON.parse(line.slice(11)); this.waiting.get(msg.id)?.(msg); this.waiting.delete(msg.id);
      }
    });
  }
  run(id: string, requestPath: string): Promise<any> {
    return new Promise((resolve, reject) => {
      this.waiting.set(id, m => (m.ok ? resolve(m) : reject(new Error(m.error + '\n' + this.stderr))));
      this.proc.once('close', code => reject(new Error(`Blender exited ${code}\n${this.stderr}`)));
      this.proc.stdin.write(JSON.stringify({ id, path: requestPath }) + '\n');
    });
  }
  close() { this.proc.stdin.end(); }
}

const tag = `${variant}-${engine}${engine === 'cycles' ? samples + (device === 'cpu' ? '' : '-' + device) : ''}`;
const results: any[] = [];
for (const id of scenes) {
  const { snapshot, assets } = snapshots[id];
  const base = join(root, 'out/blender', id, tag);
  rmSync(base, { recursive: true, force: true });
  mkdirSync(base, { recursive: true });
  writeFileSync(join(base, 'snapshot.json'), JSON.stringify(snapshot));
  const script = join(base, 'worker.py');
  writeFileSync(script, workerSource());
  const job = async (name: string, outputs: string[] | null, worker: Worker) => {
    const dir = join(base, name);
    mkdirSync(dir, { recursive: true });
    const files: Record<string, string> = {};
    for (const a of assets) { const f = join(dir, a.id + extname(a.name)); writeFileSync(f, Buffer.from(a.data, 'base64')); files[a.id] = f; }
    const request = { id: `render_${name.replace(/[^a-zA-Z0-9]/g, '')}`, operation: 'render', snapshot: outputs ? { ...snapshot, outputs } : snapshot, assets: files, dir };
    const requestPath = join(dir, 'request.json');
    writeFileSync(requestPath, JSON.stringify(request));
    const t0 = performance.now();
    await worker.run(request.id, requestPath);
    return { ms: performance.now() - t0, manifest: JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8')) };
  };
  const batchedWorker = new Worker(script);
  const batched = await job('batched', null, batchedWorker);
  batchedWorker.close();
  const singles: any[] = [];
  if (single) {
    for (const layer of snapshot.objects.map((o: any) => o.id)) {
      const w = new Worker(script);
      singles.push({ layer, ...(await job(`single-${layer}`, [layer], w)) });
      w.close();
    }
  }
  results.push({ scene: id, tag, batched_ms: batched.ms, manifest: batched.manifest, singles: singles.map(s => ({ layer: s.layer, ms: s.ms })) });
  console.log(`${id} [${tag}]: batched ${batched.ms.toFixed(0)} ms (${batched.manifest.files.length} layers, ${batched.manifest.renderEngine})${single ? `, singles ${singles.map(s => s.ms.toFixed(0)).join('/')} ms` : ''}`);
}
writeFileSync(join(root, 'out/blender', `run-${tag}.json`), JSON.stringify(results, null, 1));
