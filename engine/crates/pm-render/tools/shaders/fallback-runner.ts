/*
 * Answers the shader gate's fallback requests with the app's own fallback
 * runner (`apps/desktop/src/renderer/src/engine-connection/shader-fallback.ts`)
 * in headless Chromium with ANGLE on Metal, as the renderer runs it:
 * `<corpus>/fallback/requests/<key>.json` → `<corpus>/fallback/pixels/<key>.f16` (or `.f32` at depth 32).
 * `tests/shader_gate.rs` then draws those pixels through the engine and
 * compares the frame with the compositor's WebGL output.
 *
 * `--fallback <dir>` answers `<dir>/requests` into `<dir>/pixels` instead,
 * with each request's textures read from `<dir>/requests/<key>-<i>.f16` (or `.f32` at depth 32)
 * (the effects corpus's `fallback-effects`, written by
 * `tests/effects_webgl.rs`'s `fallback_effects_match_webgl`).
 *
 * Usage (under the machine-wide Electron lock):
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
 *     bun engine/crates/pm-render/tools/shaders/fallback-runner.ts [--corpus <dir> | --fallback <dir>]
 * The browser is headless; nothing is shown on screen.
 */
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { REPO } from '../effects/preambles';

const { chromium } = await import(join(REPO, 'apps', 'desktop', 'node_modules', 'playwright', 'index.mjs'));
const args = process.argv.slice(2);
const ROOT = join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');
const flag = (name: string) => (args.includes(name) ? resolve(args[args.indexOf(name) + 1]!) : null);
const base = flag('--fallback') ?? join(
  flag('--corpus') ?? join(ROOT, (await readdir(ROOT)).filter((d) => d.startsWith('shaders-')).sort().at(-1) ?? 'missing'),
  'fallback',
);
const requests = join(base, 'requests');
const pixels = join(base, 'pixels');
await mkdir(pixels, { recursive: true });
if (!existsSync(requests)) throw new Error(`no fallback requests in ${requests}: run tests/shader_gate.rs first`);

const temp = await mkdtemp(join(tmpdir(), 'pm-fallback-runner-'));
let browser: Awaited<ReturnType<typeof chromium.launch>> | null = null;
try {
  const entry = join(temp, 'entry.ts');
  await writeFile(entry, `import * as runner from ${JSON.stringify(join(REPO, 'apps/desktop/src/renderer/src/engine-connection/shader-fallback.ts'))};\n(globalThis as any).PMShaderFallback = runner;\n`);
  const built = await Bun.build({ entrypoints: [entry], format: 'iife', target: 'browser' });
  if (!built.success) throw new Error(built.logs.join('\n'));
  const bundle = await built.outputs[0]!.text();

  browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
  const page = await browser.newPage();
  await page.goto('about:blank');
  await page.addScriptTag({ content: bundle });
  await page.evaluate(() => { (window as any).runner = new (window as any).PMShaderFallback.ShaderFallbackRunner(); });

  let answered = 0, failed = 0;
  for (const file of (await readdir(requests)).filter((f) => f.endsWith('.json')).sort()) {
  const request = JSON.parse(await readFile(join(requests, file), 'utf8'));
  const suffix = request.colorDepth === 32 ? 'f32' : 'f16';
  const textures: string[] = [];
  for (let i = 0; i < (request.textures ?? 0); i++) {
    textures.push((await readFile(join(requests, `${request.key}-${i}.${suffix}`))).toString('base64'));
  }
  const result = await page.evaluate(async ([r, encoded]: [any, string[]]) => {
    try {
      const inputs = encoded.map((b64) => {
        const binary = atob(b64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
        return r.colorDepth === 32 ? new Float32Array(bytes.buffer) : new Uint16Array(bytes.buffer);
      });
      const out = await (window as any).runner.render(r, inputs);
      const values = r.colorDepth === 32 ? out.rgba32f : out.rgba16f;
      const bytes = new Uint8Array(values.buffer, values.byteOffset, values.byteLength);
      let binary = '';
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return { ok: true, data: btoa(binary) };
    } catch (error: any) {
      return { ok: false, error: `${error?.reason ?? 'error'}: ${error?.message ?? error}` };
    }
  }, [request, textures] as [any, string[]]);
  if (result.ok) {
    await writeFile(join(pixels, `${request.key}.${suffix}`), Buffer.from(result.data, 'base64'));
    answered++;
  } else {
    console.log(`${request.layer} ${request.key.slice(0, 12)}: ${result.error}`);
    failed++;
  }
  }
  console.log(`${answered} answered, ${failed} failed → ${pixels} (${browser.version()})`);
} finally {
  await browser?.close();
  await rm(temp, { recursive: true, force: true });
}
