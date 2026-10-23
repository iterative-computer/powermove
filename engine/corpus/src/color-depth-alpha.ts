/** Capture today's actual transparent PNG export. Run the built app under
 * pm-electron-lock. Artifacts stay in the supplied external directory and
 * launchLegacyApp removes its isolated profile in finally. */
import path from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { repoRoot } from './catalog';
import { newProject } from './doc';

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
  const reference = path.join(out, 'transparent-8.legacy-export.png');
  await app.evaluateMain(({ dialog }, filePath: string) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, reference);
  const sample = await app.evaluate(async (doc) => {
    const PM = (window as any).PM;
    PM.replaceProject({ ...doc, colorDepth: 8 }); PM.pause(); PM.setTime(.5, { raw: true, force: true });
    await PM.projectAssetsReady; await PM.prepareFrame(.5);
    const pixels = PM.GL.renderToPixels(.5, 320, 180, { transparent: true, mblur: false, exporting: true });
    if (PM.GL.colorDepth !== 8) throw new Error('Legacy did not render at depth 8');
    const alphaCodes = new Set<number>();
    for (let i = 3; i < pixels.length; i += 4) alphaCodes.add(pixels[i]);
    if (!alphaCodes.has(0) || alphaCodes.size < 32) throw new Error('Transparent export witness lacks varied blur alpha');
    const topDown = new Float32Array(pixels.length);
    for (let y = 0; y < 180; y++) for (let x = 0; x < 1280; x++) topDown[y * 1280 + x] = pixels[(179 - y) * 1280 + x] / 255;
    const result = await PM.Export.run({ format: 'still', alpha: true, scale: 1, mblur: false, audio: false, reportPath: true });
    if (result?.error || result?.cancelled) throw new Error(JSON.stringify(result));
    return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(topDown), alphaCodes: alphaCodes.size };
  }, b.root);
  writeFileSync(path.join(out, 'transparent-8.json'), JSON.stringify(sample.doc));
  writeFileSync(path.join(out, 'transparent-8.rgba32f'), new Uint8Array(new Float32Array(sample.pixels).buffer));
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples: [{ id: 'transparent-8', depth: 8, width: 320, height: 180, time: .5,
    exportReference: path.basename(reference), alphaCodes: sample.alphaCodes }] }, null, 2));
  console.log('Captured actual transparent PNG export at depth 8');
} finally { await app.close(); }
