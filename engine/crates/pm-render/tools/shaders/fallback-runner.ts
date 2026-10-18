/*
 * Answers the shader gate's fallback requests with the app's own fallback
 * runner (`apps/desktop/src/renderer/src/engine-connection/shader-fallback.ts`)
 * in headless Chromium with ANGLE on Metal, as the renderer runs it:
 * `<corpus>/fallback/requests/<key>.json` → `<corpus>/fallback/pixels/<key>.f16`.
 * `tests/shader_gate.rs` then draws those pixels through the engine and
 * compares the frame with the compositor's WebGL output.
 *
 * Usage (under the machine-wide Electron lock):
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock \
 *     bun engine/crates/pm-render/tools/shaders/fallback-runner.ts [--corpus <dir>]
 * The browser is headless; nothing is shown on screen.
 */
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readdir, readFile, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { REPO } from '../effects/preambles';

const { chromium } = await import(join(REPO, 'apps', 'desktop', 'node_modules', 'playwright', 'index.mjs'));
const args = process.argv.slice(2);
const ROOT = join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');
const corpus = args.includes('--corpus')
  ? resolve(args[args.indexOf('--corpus') + 1]!)
  : join(ROOT, (await readdir(ROOT)).filter((d) => d.startsWith('shaders-')).sort().at(-1) ?? 'missing');
const requests = join(corpus, 'fallback', 'requests');
const pixels = join(corpus, 'fallback', 'pixels');
await mkdir(pixels, { recursive: true });
if (!existsSync(requests)) throw new Error(`no fallback requests in ${requests}: run tests/shader_gate.rs first`);

const entry = join(await mkdtemp(join(tmpdir(), 'pm-fallback-runner-')), 'entry.ts');
await writeFile(entry, `import * as runner from ${JSON.stringify(join(REPO, 'apps/desktop/src/renderer/src/engine-connection/shader-fallback.ts'))};\n(globalThis as any).PMShaderFallback = runner;\n`);
const built = await Bun.build({ entrypoints: [entry], format: 'iife', target: 'browser' });
if (!built.success) throw new Error(built.logs.join('\n'));
const bundle = await built.outputs[0]!.text();

const browser = await chromium.launch({ headless: true, args: ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'] });
const page = await browser.newPage();
await page.goto('about:blank');
await page.addScriptTag({ content: bundle });
await page.evaluate(() => { (window as any).runner = new (window as any).PMShaderFallback.ShaderFallbackRunner(); });

let answered = 0, failed = 0;
for (const file of (await readdir(requests)).filter((f) => f.endsWith('.json')).sort()) {
  const request = JSON.parse(await readFile(join(requests, file), 'utf8'));
  const result = await page.evaluate(async (r: any) => {
    try {
      const out = await (window as any).runner.render(r);
      const bytes = new Uint8Array(out.rgba16f.buffer);
      let binary = '';
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return { ok: true, data: btoa(binary) };
    } catch (error: any) {
      return { ok: false, error: `${error?.reason ?? 'error'}: ${error?.message ?? error}` };
    }
  }, request);
  if (result.ok) {
    await writeFile(join(pixels, `${request.key}.f16`), Buffer.from(result.data, 'base64'));
    answered++;
  } else {
    console.log(`${request.layer} ${request.key.slice(0, 12)}: ${result.error}`);
    failed++;
  }
}
await browser.close();
console.log(`${answered} answered, ${failed} failed → ${pixels} (${browser.version()})`);
