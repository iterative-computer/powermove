/*
 * Compile-and-link oracle: asks WebGL2 in headless Chromium whether each
 * `<index>.frag` in a directory links with the compositor's vertex shader
 * (PM.VERT from work/corpus.json). Used on fuzz mutants and resource bombs
 * written by `s3-shaders fuzz-dump`, to compare the engine's accept/reject
 * decision with WebGL's.
 *
 * Each program gets a deadline; a hang closes the browser and the run
 * continues in a fresh one. Nothing is drawn.
 *
 * Usage: bun scripts/webgl-compile.ts --dir work/fuzz/mutants-seed1 [--angle metal|swiftshader] [--deadline 20] [--draw]
 *
 * --draw also draws one 1x1 point after linking, which makes ANGLE build the
 * Metal pipeline (link alone may not), with a 1x1 RGBA16F texture on unit 0
 * as the compositor binds u_tex, so `drawError` is what the compositor's draw
 * would raise. (Before 2026-10-12 the render target itself sat on unit 0, so
 * any program sampling unit 0 failed the draw as a feedback loop.) Programs whose name starts with
 * `loop-` are never drawn: they are meant to run for seconds on the GPU.
 * Output: <dir>/webgl-<angle>.json
 */
import { existsSync, readFileSync } from 'node:fs';
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const SPIKE = join(import.meta.dir, '..');
const { chromium } = await import(join(SPIKE, '..', '..', '..', 'apps', 'desktop', 'node_modules', 'playwright', 'index.mjs'));

const args = process.argv.slice(2);
const opt = (name: string, fallback: string) => (args.includes(name) ? args[args.indexOf(name) + 1]! : fallback);
const dir = join(SPIKE, opt('--dir', 'work/fuzz/mutants-seed1'));
const angle = opt('--angle', 'metal');
const deadline = Number(opt('--deadline', '20')) * 1000;
const draw = args.includes('--draw');
const vertex: string = JSON.parse(await readFile(join(SPIKE, 'work', 'corpus.json'), 'utf8')).vertex;
const files = (await readdir(dir)).filter((f) => f.endsWith('.frag')).sort((a, b) => parseInt(a) - parseInt(b));
const launchArgs = angle === 'swiftshader' ? ['--use-angle=swiftshader', '--enable-unsafe-swiftshader'] : ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'];

// browser.close() can wait forever on a hung GPU process, so after a grace
// period every descendant process of this script (only this run's browser)
// is killed.
let browser: any = null, page: any = null, renderer = '';
function descendants(pid: number): number[] {
  const out = Bun.spawnSync(['pgrep', '-P', String(pid)]).stdout.toString().split('\n').filter(Boolean).map(Number);
  return out.flatMap((child) => [child, ...descendants(child)]);
}
async function close() {
  const closing = browser ? browser.close().catch(() => {}) : Promise.resolve();
  await Promise.race([closing, new Promise((resolve) => setTimeout(resolve, 5000))]);
  for (const pid of descendants(process.pid)) { try { process.kill(pid, 'SIGKILL'); } catch {} }
  browser = page = null;
}
async function open(tries = 3): Promise<void> {
  try {
    await openOnce();
  } catch (error) {
    await close();
    if (tries <= 1) throw error;
    await new Promise((resolve) => setTimeout(resolve, 2000));
    return open(tries - 1);
  }
}
async function openOnce() {
  browser = await chromium.launch({ headless: true, args: launchArgs });
  page = await browser.newPage();
  await page.goto('about:blank');
  renderer = await page.evaluate(() => {
    const gl = document.createElement('canvas').getContext('webgl2')!;
    (window as any).gl = gl;
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    return info ? String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));
  });
}

const results: Record<string, { linked: boolean | null; log?: string; lost?: boolean; ms: number; outcome?: string }> = {};
await open();
async function attempt(source: string, drawIt: boolean) {
  const started = performance.now();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<'timeout'>((resolve) => { timer = setTimeout(() => resolve('timeout'), deadline); });
  const work = page.evaluate(({ vertex, source, drawIt }: { vertex: string; source: string; drawIt: boolean }) => {
    const gl: WebGL2RenderingContext = (window as any).gl;
    if (gl.isContextLost()) return { linked: null, log: 'context lost before compiling' };
    const make = (type: number, text: string) => { const s = gl.createShader(type)!; gl.shaderSource(s, text); gl.compileShader(s); return s; };
    const v = make(gl.VERTEX_SHADER, vertex), f = make(gl.FRAGMENT_SHADER, source);
    const p = gl.createProgram()!;
    gl.attachShader(p, v); gl.attachShader(p, f); gl.linkProgram(p);
    const linked = !!gl.getProgramParameter(p, gl.LINK_STATUS);
    const log = linked ? '' : ((gl.getShaderInfoLog(f) || '') + (gl.getProgramInfoLog(p) || '')).slice(0, 400);
    let drawError = 0;
    if (linked && drawIt) {
      // One pixel through a framebuffer: forces pipeline creation. The target
      // is not bound to any texture unit (sampling it would be a feedback
      // loop, which WebGL rejects at draw); unit 0 holds a 1x1 RGBA16F input,
      // as the compositor's u_tex does, so sampler2D uniforms (all on unit 0
      // by default) sample a valid texture.
      gl.getExtension('EXT_color_buffer_float');
      const target = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, target);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      const input = gl.createTexture(); gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, input);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, 1, 1, 0, gl.RGBA, gl.HALF_FLOAT, new Uint16Array([0x3c00, 0x3800, 0, 0x3c00]));
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      const fb = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
      gl.viewport(0, 0, 1, 1); gl.useProgram(p);
      gl.drawArrays(gl.POINTS, 0, 1);
      const px = new Uint8Array(4); gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      drawError = gl.getError();
      gl.bindFramebuffer(gl.FRAMEBUFFER, null); gl.deleteFramebuffer(fb);
      gl.bindTexture(gl.TEXTURE_2D, null); gl.deleteTexture(target); gl.deleteTexture(input);
    }
    gl.deleteProgram(p); gl.deleteShader(v); gl.deleteShader(f);
    return { linked, log, lost: gl.isContextLost(), ...(drawIt ? { drawError } : {}) };
  }, { vertex, source, drawIt }).catch((error: unknown) => ({ linked: null, log: `page error: ${String(error).slice(0, 200)}` }));
  const result = await Promise.race([work, timeout]);
  clearTimeout(timer);
  return { result, ms: performance.now() - started };
}

for (const file of files) {
  const index = parseInt(file);
  const source = await readFile(join(dir, file), 'utf8');
  const drawIt = draw && !(existsSync(join(dir, `${index}.name`)) && readFileSync(join(dir, `${index}.name`), 'utf8').startsWith('loop-'));
  process.stderr.write(`${index} `);
  let { result, ms } = await attempt(source, drawIt);
  // A previous program can take the GPU process down; the context comes back
  // lost. Retry once in a fresh browser so the answer belongs to this program.
  if (result !== 'timeout' && result.linked === null) {
    await close();
    await open();
    ({ result, ms } = await attempt(source, drawIt));
  }
  if (result === 'timeout') {
    results[index] = { linked: null, ms, outcome: 'timeout' };
  } else {
    results[index] = { ...result, ms, ...(result.lost ? { outcome: 'context lost after link' } : {}) };
  }
  if (result === 'timeout' || result.linked === null || result.lost) { await close(); await open(); }
}
await close();
await writeFile(join(dir, `webgl-${angle}${draw ? '-draw' : ''}.json`), JSON.stringify({ renderer, angle, results }, null, 1));
const linked = Object.values(results).filter((r) => r.linked === true).length;
console.log(`${files.length} programs: ${linked} linked, ${Object.values(results).filter((r) => r.linked === false).length} rejected, ${Object.values(results).filter((r) => r.linked === null).length} no answer (${renderer})`);
