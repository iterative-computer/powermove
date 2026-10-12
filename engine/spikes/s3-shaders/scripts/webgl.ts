/*
 * WebGL2 reference renderer: replays every corpus case in headless Chromium
 * (Playwright) exactly as the legacy compositor does (PM.VERT, RGBA16F FBOs
 * with LINEAR + CLAMP_TO_EDGE, clear to transparent, blending off, a
 * TRIANGLE_STRIP quad, the recorded uniform calls), then reads back RGBA16F.
 *
 * Usage: bun scripts/webgl.ts [--angle metal|swiftshader] [--filter <s>] [--perturb]
 * Output: work/webgl/<angle>/<case>/<variant>.f16 and work/webgl/<angle>/status.json
 *
 * --perturb renders every case again with each float uniform except the quad
 * geometry (u_m, u_uv) scaled by 1 + 2^-20 (about 8 float ulps), into
 * work/webgl/<angle>-perturbed. Comparing that with the unperturbed run shows
 * which shaders are so ill-conditioned that rounding-level differences
 * (expression order, fused multiply-add) exceed the parity tolerance by
 * themselves, independent of any translator.
 *
 * The browser is headless; nothing is shown on screen.
 */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const SPIKE = join(import.meta.dir, '..');
const WORK = join(SPIKE, 'work');
const { chromium } = await import(join(SPIKE, '..', '..', '..', 'apps', 'desktop', 'node_modules', 'playwright', 'index.mjs'));

const args = process.argv.slice(2);
const angle = args.includes('--angle') ? args[args.indexOf('--angle') + 1]! : 'metal';
const filter = args.includes('--filter') ? args[args.indexOf('--filter') + 1]! : null;
const perturb = args.includes('--perturb');
const corpus = JSON.parse(await readFile(join(WORK, 'corpus.json'), 'utf8'));
const out = join(WORK, 'webgl', perturb ? `${angle}-perturbed` : angle);
await mkdir(out, { recursive: true });

const launchArgs = angle === 'swiftshader'
  ? ['--use-angle=swiftshader', '--enable-unsafe-swiftshader']
  : ['--use-angle=metal', '--enable-gpu', '--ignore-gpu-blocklist'];
const browser = await chromium.launch({ headless: true, args: launchArgs });
const page = await browser.newPage();
await page.goto('about:blank');

const images: Record<string, { width: number; height: number; data: string }> = {};
for (const image of corpus.images) {
  images[image.name] = { width: image.width, height: image.height, data: (await readFile(join(WORK, image.file))).toString('base64') };
}

const renderer = await page.evaluate(({ vertex, images }) => {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 4;
  const gl = canvas.getContext('webgl2', { antialias: false, premultipliedAlpha: true, preserveDrawingBuffer: false })!;
  if (!gl.getExtension('EXT_color_buffer_float')) throw new Error('EXT_color_buffer_float unavailable');
  const w = window as any;
  w.gl = gl;
  w.vertex = vertex;
  w.quad = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, w.quad);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
  gl.disable(gl.DEPTH_TEST);
  const texture = (width: number, height: number, data: Uint16Array | null) => {
    const tex = gl.createTexture()!;
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, width, height, 0, gl.RGBA, gl.HALF_FLOAT, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    return tex;
  };
  w.makeTexture = texture;
  w.images = {};
  for (const [name, image] of Object.entries(images)) {
    const bytes = Uint8Array.from(atob(image.data), (c) => c.charCodeAt(0));
    w.images[name] = texture(image.width, image.height, new Uint16Array(bytes.buffer));
  }
  const info = gl.getExtension('WEBGL_debug_renderer_info');
  return info ? String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));
}, { vertex: corpus.vertex, images });
console.log(`WebGL renderer: ${renderer}`);

// A filtered run updates its cases and keeps the rest of an earlier full run.
const previous = filter ? await readFile(join(out, 'status.json'), 'utf8').then((t) => JSON.parse(t).cases, () => ({})) : {};
const status: Record<string, unknown> = { ...previous };
let compiled = 0, failed = 0;
for (const c of corpus.cases) {
  if (filter && !c.id.includes(filter)) continue;
  const result = await page.evaluate(({ c, perturb }: { c: any; perturb: boolean }) => {
    const w = window as any;
    const gl: WebGL2RenderingContext = w.gl;
    const shader = (type: number, source: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, source);
      gl.compileShader(s);
      return s;
    };
    const v = shader(gl.VERTEX_SHADER, w.vertex);
    const f = shader(gl.FRAGMENT_SHADER, c.frag);
    const program = gl.createProgram()!;
    gl.attachShader(program, v);
    gl.attachShader(program, f);
    gl.linkProgram(program);
    const fragLog = gl.getShaderInfoLog(f) || '';
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      const log = (gl.getShaderParameter(f, gl.COMPILE_STATUS) ? '' : fragLog) + (gl.getProgramInfoLog(program) || '');
      gl.deleteProgram(program); gl.deleteShader(v); gl.deleteShader(f);
      return { ok: false, log };
    }
    gl.deleteShader(v); gl.deleteShader(f);
    const a = gl.getAttribLocation(program, 'a_pos');
    const outputs: Record<string, string> = {};
    const started = performance.now();
    for (const variant of c.variants) {
      const targets: { tex: WebGLTexture; fb: WebGLFramebuffer }[] = [];
      for (const pass of variant.passes) {
        const tex = w.makeTexture(variant.width, variant.height, null);
        const fb = gl.createFramebuffer()!;
        gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
        gl.viewport(0, 0, variant.width, variant.height);
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
        gl.useProgram(program);
        gl.bindBuffer(gl.ARRAY_BUFFER, w.quad);
        if (a >= 0) { gl.enableVertexAttribArray(a); gl.vertexAttribPointer(a, 2, gl.FLOAT, false, 0, 0); }
        for (let unit = 0; unit < 8; unit++) { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, null); }
        for (const unit of pass.units) {
          gl.activeTexture(gl.TEXTURE0 + unit.unit);
          const src: string = unit.src;
          gl.bindTexture(gl.TEXTURE_2D, src.startsWith('pass:') ? targets[Number(src.slice(5))]!.tex : w.images[src]);
        }
        gl.activeTexture(gl.TEXTURE0);
        for (const call of pass.calls) {
          let location = gl.getUniformLocation(program, call.name);
          if (location === null && call.alt) location = gl.getUniformLocation(program, call.alt);
          if (location === null) continue;
          const geometry = call.name === 'u_m' || call.name === 'u_uv';
          const x = perturb && call.fn !== '1i' && !geometry ? call.v.map((v: number) => Math.fround(v * (1 + 2 ** -20))) : call.v;
          if (call.fn === '1f') gl.uniform1f(location, x[0]);
          else if (call.fn === '2f') gl.uniform2f(location, x[0], x[1]);
          else if (call.fn === '3f') gl.uniform3f(location, x[0], x[1], x[2]);
          else if (call.fn === '4f') gl.uniform4f(location, x[0], x[1], x[2], x[3]);
          else if (call.fn === '1i') gl.uniform1i(location, x[0]);
          else if (call.fn === 'm3') gl.uniformMatrix3fv(location, false, new Float32Array(x));
        }
        while (gl.getError() !== gl.NO_ERROR) { /* mismatched glUniform calls, as in the compositor */ }
        gl.disable(gl.BLEND);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        targets.push({ tex, fb });
      }
      const last = targets[targets.length - 1]!;
      gl.bindFramebuffer(gl.FRAMEBUFFER, last.fb);
      const pixels = new Float32Array(variant.width * variant.height * 4);
      gl.readPixels(0, 0, variant.width, variant.height, gl.RGBA, gl.FLOAT, pixels);
      // f32 to f16 bits (values come from RGBA16F storage, so this is exact).
      const halfs = new Uint16Array(pixels.length);
      const f = new Float32Array(1), u = new Uint32Array(f.buffer);
      for (let i = 0; i < pixels.length; i++) {
        f[0] = pixels[i]!;
        const bits = u[0]!, sign = (bits >>> 16) & 0x8000, exp = (bits >>> 23) & 0xff, mant = bits & 0x7fffff;
        if (exp === 0xff) halfs[i] = sign | 0x7c00 | (mant ? 0x200 : 0);
        else if (exp - 112 >= 0x1f) halfs[i] = sign | 0x7c00;
        else if (exp - 112 <= 0) halfs[i] = exp - 112 < -10 ? sign : sign | ((mant | 0x800000) >>> (126 - exp));
        else halfs[i] = sign | ((exp - 112) << 10) | (mant >>> 13);
      }
      const bytes = new Uint8Array(halfs.buffer);
      let binary = '';
      for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      outputs[variant.name] = btoa(binary);
      for (const t of targets) { gl.deleteFramebuffer(t.fb); gl.deleteTexture(t.tex); }
    }
    gl.deleteProgram(program);
    const error = gl.getError();
    return { ok: true, outputs, ms: performance.now() - started, glError: error };
  }, { c, perturb });
  if (!result.ok) {
    failed++;
    status[c.id] = { compiled: false, log: result.log };
    continue;
  }
  compiled++;
  status[c.id] = { compiled: true, ms: result.ms };
  const dir = join(out, c.id.replaceAll('/', '__'));
  await mkdir(dir, { recursive: true });
  for (const [name, data] of Object.entries(result.outputs as Record<string, string>)) await writeFile(join(dir, `${name}.f16`), Buffer.from(data, 'base64'));
}
await writeFile(join(out, 'status.json'), JSON.stringify({ renderer, browser: browser.version(), angle, perturb, cases: status }, null, 1));
await browser.close();
console.log(`${compiled} compiled, ${failed} failed (${angle})`);
