/** Capture the shipped compositor at every project depth.
 * Build the desktop first, then run under pm-electron-lock:
 *   bun engine/corpus/src/color-depth.ts <output-directory-outside-repo>
 * All images/documents stay outside Git; the isolated app profile is removed
 * by launchLegacyApp.close() on success and failure.
 */
import path from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';
import { newProject } from './doc';
import { encodePng } from './png';

const out = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || out === repoRoot || out.startsWith(repoRoot + path.sep)) throw new Error('Supply an output directory outside the repository');
mkdirSync(out, { recursive: true });
const gradient = { type: 'linear', angle: 27, stops: [
  { id: 'a', color: '#101420', position: 0 }, { id: 'b', color: '#484262', position: 100 },
] };
const projects = ['gradient', 'blend-stack', 'masked-matte', 'blur', 'glow', 'vector', 'text'].map(kind => {
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
  for (const { kind, doc } of projects) for (const depth of [8, 16, 32]) {
    const sample = await app.evaluate(async ({ doc, depth }) => {
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
      return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels), dither: gl.isEnabled(gl.DITHER), supported: PM.GL.supportedColorDepths };
    }, { doc, depth });
    const id = `${kind}-${depth}`;
    const pixels = new Float32Array(sample.pixels);
    writeFileSync(path.join(out, `${id}.json`), JSON.stringify(sample.doc));
    writeFileSync(path.join(out, `${id}.rgba32f`), new Uint8Array(pixels.buffer));
    writeFileSync(path.join(out, `${id}.png`), encodePng({ width: 320, height: 180, channels: 4, data: new Uint8Array(sample.pixels.map((v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255))) }));
    samples.push({ id, depth, width: 320, height: 180, time: .5, dither: sample.dither, supported: sample.supported });
    console.log(`Captured ${id}`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
} finally { await app.close(); }
