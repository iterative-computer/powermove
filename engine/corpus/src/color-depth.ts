/** Capture the shipped compositor at every project depth.
 * Build the desktop and prepare the runner outside the Electron lock:
 *   bun engine/corpus/src/color-depth.ts <external-directory> --prepare
 * Then capture under pm-electron-lock:
 *   bun engine/corpus/src/color-depth.ts <external-directory>
 * All images/documents stay outside Git; the isolated app profile is removed
 * by launchLegacyApp.close() on success and failure.
 */
import path from 'node:path';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';
import { newProject } from './doc';
import { encodePng } from './png';


const out = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || out === repoRoot || out.startsWith(repoRoot + path.sep)) throw new Error('Supply an output directory outside the repository');
mkdirSync(out, { recursive: true });
const bundlePath = path.join(out, 'fallback.bundle.js');
if (process.argv.includes('--prepare')) {
  const { build } = await import(path.join(repoRoot, 'apps/desktop/node_modules/esbuild/lib/main.js'));
  const bundle = await build({ stdin: { contents: `export { ShaderFallbackRunner } from './src/renderer/src/engine-connection/shader-fallback';`, resolveDir: path.join(repoRoot, 'apps/desktop') }, bundle: true, write: false, format: 'iife', globalName: 'DepthFallback', target: 'es2022' });
  writeFileSync(bundlePath, bundle.outputFiles[0]!.text);
  process.exit(0);
}
const fallbackBundle = readFileSync(bundlePath, 'utf8');
const gradient = { type: 'linear', angle: 27, stops: [
  { id: 'a', color: '#101420', position: 0 }, { id: 'b', color: '#484262', position: 100 },
] };
const projects = (process.argv.includes('--fallback-only') ? [] : process.argv.includes('--text-only') ? ['text'] : ['gradient', 'blend-stack', 'masked-matte', 'blur', 'glow', 'vector', 'text']).map(kind => {
  const b = newProject(`depth/${kind}`, { name: `Color depth ${kind}`, w: 320, h: 180, dur: 2, backgroundFill: gradient });
  if (kind !== 'gradient') {
    const l = b.add(b.layer(kind === 'vector' ? 'shape' : kind === 'text' ? 'text' : 'solid', {
      d: kind === 'text' ? { text: 'Color depth', font: 'Arial', size: 34, weight: 400, color: '#C7B5ED', align: 'left' }
        : { color: '#B47AEEB3', shape: 'ellipse', w: 170, h: 110, radius: 18 },
      p: { 'position.x': kind === 'solid' ? 37 : 120, 'position.y': 95, rotation: 11, opacity: 73 },
    }));
    if (kind === 'blur' || kind === 'glow') l.fx.push(b.effect(kind, kind === 'blur' ? { amount: 11 } : { radius: 25, threshold: 20, intensity: 120 }));
    if (kind === 'masked-matte') {
      l.masks.push(b.mask('ellipse', { x: 70, y: 40, w: 120, h: 70, feather: 25 }));
      const matte = b.add(b.layer('solid', { on: false, d: { color: '#B0B0B099', w: 300, h: 160 } }));
      l.matteSource = matte.id; l.matteMode = b.ch('luma');
    }
    if (kind === 'blend-stack') for (const [i, mode] of ['normal', 'screen', 'multiply', 'overlay', 'softlight', 'add', 'difference'].entries()) {
      b.add(b.layer('solid', { blend: mode, d: { color: i % 2 ? '#32719E99' : '#AC539DB3', w: 165, h: 80 }, p: { 'position.x': i * 22, 'position.y': i * 12, opacity: 67 } }));
    }
  }
  return { kind, doc: b.root };
});
const app = await launchLegacyApp();
const samples: any[] = [];
try {
  await app.evaluateScript(`(()=>{${fallbackBundle}; globalThis.DepthFallback = DepthFallback; return true;})()`);
  const fallback = await app.evaluate(async () => {
    const runner = new (window as any).DepthFallback.ShaderFallbackRunner();
    try {
      const request = { key: 'depth-eight', layer: 'test', width: 256, height: 4, colorDepth: 8,
        vertex: '#version 300 es\nin vec2 a_pos; void main(){gl_Position=vec4(a_pos*2.-1.,0,1);}',
        fragment: '#version 300 es\nprecision highp float; uniform highp sampler2D image; out vec4 o; void main(){o=texelFetch(image,ivec2(gl_FragCoord.xy),0);}',
        calls: [{ fn: '1i', name: 'image', v: [0] }], textures: 1,
        units: [{ unit: 0, texture: 0, chain: true }], morePasses: [[], []], stage: 'test', message: '' };
      const input = Uint16Array.from({ length: 256 * 4 * 4 }, (_, i) => i % 256);
      let differing = 0, compared = 0;
      for (let i = 0; i < 3; i++) {
        const sync = runner.renderSync(request, [input]);
        const asyncResult = await runner.render(request, [input]);
        for (const pixels of [sync.rgba16f, asyncResult.rgba16f]) for (let j = 0; j < input.length; j++) {
          compared++; if (pixels[j] !== input[j]) differing++;
        }
        runner.renderSync({ ...request, key: 'depth-sixteen', colorDepth: 16 }, [new Uint16Array(input.length).fill(0x3800)]);
      }
      if (differing) throw new Error(`RGBA8 fallback changed ${differing}/${compared} codes`);
      const floats = Float32Array.from({ length: input.length }, (_, i) => [0.5001234, 0.00012345, 1.0001234, 1][i % 4]!);
      const bits = new Uint32Array(floats.buffer);
      let floatCompared = 0, floatDiffering = 0;
      for (let i = 0; i < 3; i++) {
        const r = { ...request, key: 'depth-thirty-two', colorDepth: 32 };
        const sync = runner.renderSync(r, [floats]);
        const asyncResult = await runner.render(r, [floats]);
        for (const pixels of [sync.rgba32f, asyncResult.rgba32f]) {
          if (!(pixels instanceof Float32Array)) throw new Error('Float32 runner result missing');
          const actual = new Uint32Array(pixels.buffer, pixels.byteOffset, pixels.length);
          for (let j = 0; j < bits.length; j++) { floatCompared++; if (actual[j] !== bits[j]) floatDiffering++; }
        }
        runner.renderSync({ ...request, key: 'depth-sixteen', colorDepth: 16 }, [new Uint16Array(input.length).fill(0x3800)]);
      }
      if (floatDiffering) throw new Error(`RGBA32F fallback changed ${floatDiffering}/${floatCompared} float bit patterns`);
      return { compared, differing, floatCompared, floatDiffering, passes: 3, depthSwitches: 6 };
    } finally { runner.dispose(); }
  });
  writeFileSync(path.join(out, 'fallback.json'), JSON.stringify(fallback, null, 2));
  console.log(`Fallback exact codes: ${fallback.compared}, differences: ${fallback.differing}`);
  for (const { kind, doc } of projects) for (const depth of [8, 16, 32]) {
    const sample = await app.evaluate(async ({ doc, depth, kind }) => {
      const PM = (window as any).PM;
      PM.replaceProject({ ...doc, colorDepth: depth }); PM.pause();
      await PM.projectAssetsReady; await PM.prepareFrame(.5);
      const gl = PM.GL.gl;
      for (let n = 0; n < 32 && gl.getError(); n++);
      let raw: Float32Array | undefined;
      const read = gl.readPixels.bind(gl);
      gl.readPixels = (...args: any[]) => { const result = read(...args); if (args[5] === gl.FLOAT) raw = new Float32Array(args[6]); return result; };
      let bytes: Uint8Array;
      try { bytes = PM.GL.renderToPixels(.5, 320, 180, { mblur: false, exporting: true }); }
      finally { gl.readPixels = read; }
      const error = gl.getError();
      if (error || PM.GL.colorDepth !== depth) throw new Error(`Depth ${depth}, actual ${PM.GL.colorDepth}, GL error ${error}`);
      const pixels = new Float32Array(320 * 180 * 4);
      for (let y = 0; y < 180; y++) for (let x = 0; x < 320 * 4; x++) {
        const from = (179 - y) * 320 * 4 + x;
        pixels[y * 320 * 4 + x] = raw ? raw[from]! : bytes![from]! / 255;
      }
      let source: { width: number; height: number; pixels: number[]; layer: string } | undefined;
      if (kind === 'text') {
        const layer = PM.proj.layers.find((l: any) => l.type === 'text');
        const raster = PM.raster(layer, 1, .5);
        const uploaded = PM.GL.texes.get('r:' + raster.key);
        if (!uploaded || !raster.cv) throw new Error('Missing uploaded text source');
        const fb = gl.createFramebuffer(), previous = gl.getParameter(gl.FRAMEBUFFER_BINDING);
        try {
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, uploaded.tex, 0);
          if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) throw new Error('Incomplete text source framebuffer');
          const data = new Uint8Array(raster.cv.width * raster.cv.height * 4);
          gl.readPixels(0, 0, raster.cv.width, raster.cv.height, gl.RGBA, gl.UNSIGNED_BYTE, data);
          // Canvas uploads are top-down; attachment reads preserve that row order.
          source = { width: raster.cv.width, height: raster.cv.height, pixels: Array.from(data), layer: layer.id };
        } finally { gl.bindFramebuffer(gl.FRAMEBUFFER, previous); gl.deleteFramebuffer(fb); }
      }
      return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels), source, dither: gl.isEnabled(gl.DITHER), supported: PM.GL.supportedColorDepths };
    }, { doc, depth, kind });
    const id = `${kind}-${depth}`;
    const pixels = new Float32Array(sample.pixels);
    writeFileSync(path.join(out, `${id}.json`), JSON.stringify(sample.doc));
    writeFileSync(path.join(out, `${id}.rgba32f`), new Uint8Array(pixels.buffer));
    writeFileSync(path.join(out, `${id}.png`), encodePng({ width: 320, height: 180, channels: 4, data: new Uint8Array(sample.pixels.map((v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255))) }));
    if (sample.source) writeFileSync(path.join(out, `${id}.source.rgba`), new Uint8Array(sample.source.pixels));
    const source = sample.source ? { width: sample.source.width, height: sample.source.height, layer: sample.source.layer, scale: 1 } : undefined;
    samples.push({ id, depth, width: 320, height: 180, time: .5, source, dither: sample.dither, supported: sample.supported });
    console.log(`Captured ${id}`);
  }
  if (samples.length) writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
} finally { await app.close(); }
