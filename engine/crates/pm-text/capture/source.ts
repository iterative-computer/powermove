/* Captures the source-texture set of the text raster gate (`pm-render`
   tests/text_raster.rs, `text-source-*`): the text layers of a real project
   as today's app rasterizes them at the density their world transform asks
   for (the layer's source texture), read back premultiplied the way the
   compositor uploads them. These are the large, scaled rasters the
   synthetic sets do not reach (3.5x and 7x of a variable project font).

   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/source.ts \
     [--project <loaded.json>] [--times 16,18,24] [--out <dir>]
     -> <dir>/manifest.json, <dir>/<case>.rgba, <dir>/cases.json,
        <dir>/pipeline.json (world matrix, density and geometry per case),
        <dir>/source.json (where the project came from)

   The project defaults to `real/powermove-ad` of the newest `text-iso-*`
   capture (`engine/corpus/src/capture.ts --text-only`); the times default
   to the frames whose text the FONTS-R ratchet lists
   (`pm-render/tests/fixtures/text-residuals.json`). Cases are named
   `real-<time>-<layer id>`. The legacy app must be built
   (`bun run --cwd apps/desktop build`); it runs hidden with the native
   engine off. The pixels are renders of a private project and its fonts:
   they go to the private corpus (`~/Library/Application Support/Powermove
   Engine Corpus`, or `$PM_ENGINE_CORPUS_ROOT`) and never into the
   repository. */
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { launchLegacyApp } from '../../../corpus/src/app';

const repo = resolve(import.meta.dir, '..', '..', '..', '..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
const corpusRoot = process.env.PM_ENGINE_CORPUS_ROOT ?? join(homedir(), 'Library', 'Application Support', 'Powermove Engine Corpus');

function defaultProject(): string {
  const iso = readdirSync(corpusRoot).filter(n => n.startsWith('text-iso-')).sort()
    .map(n => join(corpusRoot, n, 'real', 'powermove-ad', 'loaded.json')).filter(existsSync);
  const newest = iso.pop();
  if (!newest) throw new Error(`no text-iso-* capture with real/powermove-ad in ${corpusRoot}; pass --project`);
  return newest;
}

const projectPath = resolve(flag('--project') ?? defaultProject());
const times = (flag('--times') ?? '16,18,24').split(',').map(Number);
if (times.some(t => !Number.isFinite(t) || t < 0)) throw new Error('--times takes non-negative seconds, comma-separated');
const stamp = new Date().toISOString().replace(/[-:]/g, '').replace(/\..*/, '').replace('T', '-');
const out = resolve(flag('--out') ?? join(corpusRoot, `text-source-${stamp}`));
if (out.startsWith(resolve(repo) + '/')) throw new Error('refusing to write captured pixels inside the repository');
if (existsSync(join(out, 'manifest.json'))) throw new Error(`${out} already holds a capture`);
mkdirSync(out, { recursive: true });

interface Raster {
  id: string; d: Record<string, unknown>; world: number[]; ss: number;
  geometry: Record<string, unknown>; width: number; height: number; rgba: string;
}

const doc = JSON.parse(readFileSync(projectPath, 'utf8'));
const app = await launchLegacyApp();
try {
  await app.evaluate((doc: any) => { const PM = (window as any).PM; PM.replaceProject(doc); PM.pause(); }, doc);
  const cases: unknown[] = [], shots: unknown[] = [], rows: unknown[] = [];
  for (const time of times) {
    const rasters = await app.evaluate(({ time }: { time: number }) => {
      const PM = (window as any).PM;
      PM.beginEval(time);
      PM.time = time;
      const found: any[] = [];
      for (const L of PM.proj.layers) {
        if (L.type !== 'text' || time < L.from || time >= L.from + L.dur) continue;
        const world = PM.worldMatrix(L, time), m = world.elements ?? world;
        const v = Array.from(m) as number[];
        const mat = v.length === 16 ? [v[0], v[1], v[4], v[5], v[12], v[13]] : v;
        // The compositor's continuous raster scale: the world transform's
        // largest singular value, clamped to [0.25, 32], in quarter steps up.
        const [a, b, c, d] = mat;
        const aa = a * a + b * b, bb = a * c + b * d, cc = c * c + d * d;
        const largest = Math.sqrt(Math.max(0, (aa + cc + Math.sqrt(Math.max(0, (aa - cc) ** 2 + 4 * bb * bb))) / 2));
        const ss = Math.ceil(Math.max(0.25, Math.min(32, largest)) * 4 - 1e-9) / 4;
        const geometry = PM.textRasterGeometry(L, ss, time), r = PM.raster(L, ss, time);
        if (!r.cv) continue;
        const bytes = r.cv.getContext('2d').getImageData(0, 0, r.cv.width, r.cv.height).data;
        // Premultiplied, as the compositor uploads the canvas.
        for (let i = 0; i < bytes.length; i += 4) for (let k = 0; k < 3; k++) bytes[i + k] = Math.round(bytes[i + k] * bytes[i + 3] / 255);
        let s = '';
        for (let i = 0; i < bytes.length; i += 32768) s += String.fromCharCode(...bytes.subarray(i, i + 32768));
        found.push({ id: L.id, d: PM.resolveContent(L, time), world: mat, ss, geometry, width: r.cv.width, height: r.cv.height, rgba: btoa(s) });
      }
      return found;
    }, { time }) as Raster[];
    for (const r of rasters) {
      const id = `real-${time}-${r.id}`;
      writeFileSync(join(out, `${id}.rgba`), Buffer.from(r.rgba, 'base64'));
      cases.push({ id, group: 'real-scaled', kind: 'text', d: r.d, scale: r.ss });
      shots.push({ id, width: r.width, height: r.height });
      const { rgba: _, ...row } = r;
      rows.push({ time, ...row });
    }
  }
  if (!cases.length) throw new Error(`no text layer rasterized at ${times.join(', ')} s`);
  writeFileSync(join(out, 'cases.json'), JSON.stringify({ cases, projectFonts: [], variableSources: {} }));
  writeFileSync(join(out, 'manifest.json'), JSON.stringify({ cases: shots }));
  writeFileSync(join(out, 'pipeline.json'), JSON.stringify(rows, null, 2));
  writeFileSync(join(out, 'source.json'), JSON.stringify({ project: projectPath, capture: basename(resolve(projectPath, '..', '..', '..')), times }, null, 2));
  console.log(`captured ${cases.length} source textures into ${out}`);
} finally {
  await app.close();
}
process.exit(0);
