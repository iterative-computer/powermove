/** Capture today's actual transparent PNG export. Run the built app under
 * pm-electron-lock. Artifacts stay in the supplied external directory and
 * launchLegacyApp removes its isolated profile in finally. */
import path from 'node:path';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';
import { newProject } from './doc';
import { decodePng } from './png';

const out = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || out === repoRoot || out.startsWith(repoRoot + path.sep)) throw new Error('Supply an output directory outside the repository');
mkdirSync(out, { recursive: true });
const b = newProject('depth/transparent-export', { name: 'Transparent depth gate', w: 320, h: 180, dur: 2,
  backgroundFill: { type: 'none', stops: [{ id: 'a', color: '#000000', position: 0 }] } });
b.add(b.layer('solid', { d: { color: '#DF6030', w: 150, h: 110 }, p: { 'position.x': 100, 'position.y': 80, opacity: 50, rotation: 11 } }));
b.add(b.layer('solid', { d: { color: '#32719E99', w: 100, h: 80 }, p: { 'position.x': 180, 'position.y': 110, opacity: 67 } }));
const blurred = b.add(b.layer('solid', { d: { color: '#A67BC8', w: 90, h: 55 }, p: { 'position.x': 235, 'position.y': 45, opacity: 43, rotation: -17 } }));
blurred.fx.push(b.effect('blur', { amount: 9 }));
const app = await launchLegacyApp();
try {
  const samples = [];
  for (const depth of [8, 16, 32]) {
    const id = `transparent-${depth}`;
    const reference = path.join(out, `${id}.legacy-export.png`);
    await app.evaluateMain(({ dialog }, filePath: string) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, reference);
    const sample = await app.evaluate(async ({ doc, depth }) => {
      const PM = (window as any).PM;
      PM.replaceProject({ ...doc, colorDepth: depth }); PM.pause(); PM.setTime(.5, { raw: true, force: true });
      await PM.projectAssetsReady; await PM.prepareFrame(.5);
      const gl = PM.GL.gl;
      let raw: Float32Array | undefined;
      const read = gl.readPixels.bind(gl);
      gl.readPixels = (...args: any[]) => {
        const result = read(...args);
        if (args[5] === gl.FLOAT) raw = new Float32Array(args[6]);
        return result;
      };
      let pixels: Uint8Array;
      try { pixels = PM.GL.renderToPixels(.5, 320, 180, { transparent: true, mblur: false, exporting: true }); }
      finally { gl.readPixels = read; }
      if (pixels.length !== 320 * 180 * 4) throw new Error(`Depth ${depth} returned ${pixels.length} RGBA bytes`);
      if (PM.proj.colorDepth !== depth || PM.GL.colorDepth !== depth) {
        throw new Error(`Legacy requested depth ${depth}, project ${PM.proj.colorDepth}, render ${PM.GL.colorDepth}`);
      }
      if (depth !== 8 && !raw) throw new Error(`Depth ${depth} did not expose its float readback`);
      const alphaCodes = new Set<number>();
      for (let i = 3; i < pixels.length; i += 4) alphaCodes.add(pixels[i]!);
      if (!alphaCodes.has(0) || alphaCodes.size < 32 || ![...alphaCodes].some(code => code > 0 && code < 255)) {
        throw new Error(`Depth ${depth} transparent export witness lacks varied blur alpha`);
      }
      const topDown = new Float32Array(pixels.length);
      for (let y = 0; y < 180; y++) for (let x = 0; x < 1280; x++) {
        const from = (179 - y) * 1280 + x;
        topDown[y * 1280 + x] = raw ? raw[from]! : pixels[from]! / 255;
      }
      const result = await PM.Export.run({ format: 'still', alpha: true, scale: 1, mblur: false, audio: false, reportPath: true });
      if (result?.error || result?.cancelled) throw new Error(JSON.stringify(result));
      return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(topDown), alphaCodes: alphaCodes.size };
    }, { doc: b.root, depth });
    const png = decodePng(readFileSync(reference));
    if (png.width !== 320 || png.height !== 180 || png.channels !== 4) throw new Error(`Depth ${depth} did not export a 320×180 RGBA PNG`);
    const exportAlpha = new Set<number>();
    for (let i = 3; i < png.data.length; i += 4) exportAlpha.add(png.data[i]!);
    if (!exportAlpha.has(0) || exportAlpha.size < 32 || ![...exportAlpha].some(code => code > 0 && code < 255)) {
      throw new Error(`Depth ${depth} PNG lacks varied blur alpha`);
    }
    writeFileSync(path.join(out, `${id}.json`), JSON.stringify(sample.doc));
    writeFileSync(path.join(out, `${id}.rgba32f`), new Uint8Array(new Float32Array(sample.pixels).buffer));
    samples.push({ id, depth, width: 320, height: 180, time: .5,
      exportReference: path.basename(reference), alphaCodes: sample.alphaCodes, exportAlphaCodes: exportAlpha.size });
    console.log(`Captured actual transparent PNG export at depth ${depth}: ${sample.alphaCodes} readback, ${exportAlpha.size} PNG alpha codes`);
  }
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
} finally { await app.close(); }
