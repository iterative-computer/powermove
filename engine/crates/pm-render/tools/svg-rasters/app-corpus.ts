/**
 * The corpus projects with level 1 SVG images, drawn by the engine in the
 * app with "Use New Engine" on (decision S1; docs/engine/vector.md, "Level 1
 * measured"), compared with the legacy capture.
 *
 * Each project is opened as the corpus capture opened it (engine/corpus:
 * `pm-open-project`, its media imported through `PM.assets.add`, the
 * references rewritten, reopened). Every captured sample is drawn by
 * `PM.EngineRender.snapshotSync` (the engine, with Chromium's rasters from
 * this window) with the capture's options (opaque, 16 motion blur samples)
 * and compared with the capture's frame. As `tests/svg_corpus.rs`, the
 * *SVG pixels* are those the frame changes when the SVG image layers are
 * hidden (opacity 0), less the text class's (text and captions layers
 * hidden, grown by 3): they must be exact. The whole frame and today's
 * compositor drawing the same frame live (`PM.GL.renderToPixels`) are
 * printed beside them.
 *
 *   bun run --cwd apps/desktop build && (cd apps/desktop && node scripts/build-engine-addon.mjs)
 *   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-render/tools/svg-rasters/app-corpus.ts [--corpus DIR] [--only PREFIX] [--diffs DIR]
 *
 * `--diffs` writes legacy, engine, and differing-pixel PNGs of failures
 * (outside the repository). Exit status 1 when any SVG pixel differs, a
 * project is routed, or a frame fails.
 */
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';

import { launchLegacyApp, type LegacyApp } from '../../../../corpus/src/app';
import { decodePng, encodePng } from '../../../../corpus/src/png';
import { installCorpusPage } from '../../../../corpus/src/page';

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.join(here, '../../../../..');
const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i < 0 ? undefined : args[i + 1]; };
const root = path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus');
const corpus = flag('--corpus') ?? path.join(root, readdirSync(root).filter((d) => d.startsWith('v1-')).sort().at(-1)!);
const only = flag('--only');
const diffs = flag('--diffs');
if (diffs && path.resolve(diffs).startsWith(path.resolve(repo) + path.sep)) throw new Error('evidence never goes inside the repository');

type Sample = { time: number; width: number; height: number; png: { file: string } };
type Record_ = { id: string; extensions?: string[]; media: Record<string, { source: { name: string; path: string } }>; samples: Sample[]; document: { file: string }; loaded: { file: string } };

/** Every project directory of the capture (`projects/<group>/<name>`). */
function projectRecords(): Record_[] {
  const out: Record_[] = [];
  const base = path.join(corpus, 'projects');
  for (const group of readdirSync(base)) for (const name of readdirSync(path.join(base, group))) {
    const file = path.join(base, group, name, 'project.json');
    if (existsSync(file)) out.push(JSON.parse(readFileSync(file, 'utf8')));
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

/** Whether the loaded project has an image layer over an SVG asset (any composition). */
function hasSvgImage(loaded: any): boolean {
  const svg = new Set(Object.values<any>(loaded.assets ?? {}).filter((a) => a?.format === 'svg' || /\.svg$/i.test(String(a?.name ?? ''))).map((a) => a.id));
  const comps = [loaded, ...Object.values<any>(loaded.comps ?? {})];
  return comps.some((c) => (c.layers ?? []).some((l: any) => l?.type === 'image' && svg.has(l?.d?.asset)));
}

const records = projectRecords().filter((r) => (!only || r.id.startsWith(only)) && r.loaded?.file && hasSvgImage(JSON.parse(readFileSync(path.join(corpus, r.loaded.file), 'utf8'))));
console.log(`${records.length} corpus projects with level 1 SVG images in ${path.basename(corpus)}`);

process.env.POWERMOVE_NEW_ENGINE = '1';

/** Page side: the engine's frames and masks for one sample, judged against the reference. */
function judge(args: { time: number; width: number; height: number; reference: string; keep: boolean }) {
  const PM = (window as any).PM;
  const decode = (b64: string) => { const s = atob(b64), out = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i); return out; };
  const encode = (u: Uint8Array) => { let s = ''; for (let i = 0; i < u.length; i += 0x8000) s += String.fromCharCode(...u.subarray(i, i + 0x8000)); return btoa(s); };
  const { time, width, height } = args;
  const reference = decode(args.reference);
  const comps = [PM.proj, ...Object.values<any>(PM.proj.comps ?? {})];
  const svg = new Set(Object.values<any>(PM.proj.assets ?? {}).filter((a: any) => a?.format === 'svg').map((a: any) => a.id));
  const layersOf = (pick: (l: any) => boolean) => comps.flatMap((c: any) => (c.layers ?? []).filter(pick));
  const svgLayers = layersOf((l) => l.type === 'image' && svg.has(l.d?.asset));
  const textLayers = layersOf((l) => l.type === 'text' || l.type === 'captions');
  const snap = () => PM.EngineRender.snapshotSync({ time, width, height, pixels: 'opaque', motionBlurSamples: 16, background: true }).data as Uint8Array;
  /** Draws with `layers` at opacity 0 (restored after). */
  const without = (layers: any[]) => {
    if (!layers.length) return null;
    const saved = layers.map((l) => JSON.stringify(l.p.opacity));
    for (const l of layers) l.p.opacity = { v: 0, kf: [], expr: null };
    PM.touch();
    try { return snap(); } finally {
      layers.forEach((l, i) => { l.p.opacity = JSON.parse(saved[i]!); });
      PM.touch();
    }
  };
  const ours = snap();
  const base = without(svgLayers);
  const plain = without(textLayers);
  const n = width * height;
  const differs = (a: Uint8Array, b: Uint8Array | null) => { const m = new Uint8Array(n); if (b) for (let i = 0; i < n; i++) { const k = i * 4; if (a[k] !== b[k] || a[k + 1] !== b[k + 1] || a[k + 2] !== b[k + 2] || a[k + 3] !== b[k + 3]) m[i] = 1; } return m; };
  let text = differs(ours, plain);
  for (let r = 0; r < 3; r++) {
    const grown = new Uint8Array(n);
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
      if (!text[y * width + x]) continue;
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
        const xx = x + dx, yy = y + dy;
        if (xx >= 0 && yy >= 0 && xx < width && yy < height) grown[yy * width + xx] = 1;
      }
    }
    text = grown;
  }
  const svgMask = differs(ours, base);
  for (let i = 0; i < n; i++) if (text[i]) svgMask[i] = 0;
  const stat = (a: Uint8Array, mask: (i: number) => boolean) => {
    let pixels = 0, exact = 0, max = 0;
    for (let i = 0; i < n; i++) {
      if (!mask(i)) continue;
      pixels++;
      let d = 0;
      for (let c = 0; c < 3; c++) d = Math.max(d, Math.abs(a[i * 4 + c]! - reference[i * 4 + c]!));
      if (!d) exact++;
      if (d > max) max = d;
    }
    return { pixels, exact, max };
  };
  // Today's compositor, live, for the same frame (the capture's options).
  let live: { pixels: number; exact: number; max: number } | null = null;
  try {
    const shutter = PM.proj.shutter || .5;
    const legacy = PM.GL.renderToPixels(time, width, height, { mblur: true, mbSamples: 16, shutter, opaque: true, topDownOpaque: true });
    if (legacy?.length === n * 4) live = stat(legacy, () => true);
  } catch { live = null; }
  return {
    svgLayers: svgLayers.length,
    svg: stat(ours, (i) => !!svgMask[i]),
    whole: stat(ours, () => true),
    live,
    route: PM.EngineConnection.status().route,
    reason: PM.EngineConnection.status().routeReason ?? null,
    engine: args.keep ? encode(ours) : null,
  };
}

let app: LegacyApp | null = null;
let appExtensions = '';
const failed: string[] = [];
const lines: string[] = [];
let frames = 0, svgPixels = 0, invisible = 0;
const started = performance.now();
try {
  for (const record of records) {
    const extensions = record.extensions ?? [];
    if (!app || appExtensions !== extensions.join(',')) {
      await app?.close();
      app = await launchLegacyApp({ extensions });
      appExtensions = extensions.join(',');
      await app.evaluate(installCorpusPage);
      await app.waitFor(() => (window as any).PM?.EngineConnection?.status?.().connected === true, undefined, 60_000);
    }
    const errorsBefore = app.pageErrors.length;
    try {
      const doc = JSON.parse(readFileSync(path.join(corpus, record.document.file), 'utf8'));
      await app.evaluate((d: any) => (window as any).__corpus.open(d), doc);
      const media = Object.values(record.media ?? {});
      if (media.length) {
        await app.evaluate(() => (window as any).__corpus.mediaInput());
        await app.setInputFiles('#corpus-media', media.map((m) => path.isAbsolute(m.source.path) ? m.source.path : path.join(repo, m.source.path)));
        const imported: Record<string, { id: string }> = await app.evaluate(() => (window as any).__corpus.importMedia());
        const map: Record<string, string> = {};
        for (const m of media) if (imported[m.source.name]?.id) map[m.source.name] = imported[m.source.name]!.id;
        const { project } = await app.evaluate((m: any) => (window as any).__corpus.rewriteMedia(m), map);
        await app.evaluate((d: any) => (window as any).__corpus.open(d), project);
        await app.evaluate(() => (window as any).__corpus.waitForMedia(30000));
      }
      // The engine has the document and its media; nothing routed.
      await app.waitFor(() => (window as any).PM.EngineConnection.status().route !== 'pending', undefined, 45_000).catch(() => {});
      for (const s of record.samples) {
        const ref = decodePng(new Uint8Array(readFileSync(path.join(corpus, s.png.file))));
        if (ref.channels !== 4 || ref.width !== s.width || ref.height !== s.height) throw new Error(`reference ${s.png.file} is not ${s.width}x${s.height} RGBA`);
        const r = await app.evaluate(judge, { time: s.time, width: s.width, height: s.height, reference: Buffer.from(ref.data).toString('base64'), keep: !!diffs });
        frames++;
        svgPixels += r.svg.pixels;
        if (!r.svg.pixels) invisible++;
        const line = `${record.id.padEnd(34)} t=${String(s.time).padEnd(5)} SVG px ${String(r.svg.pixels).padStart(7)} exact ${String(r.svg.exact).padStart(7)} max ${String(r.svg.max).padStart(3)} | frame exact ${(100 * r.whole.exact / r.whole.pixels).toFixed(3)}% max ${r.whole.max} | today live ${r.live ? `${(100 * r.live.exact / r.live.pixels).toFixed(3)}% max ${r.live.max}` : 'n/a'} | ${r.route}${r.reason ? ` (${r.reason})` : ''}`;
        lines.push(line);
        console.log(line);
        if (r.svg.max !== 0 || r.route !== 'engine') {
          failed.push(line);
          if (diffs && r.engine) {
            mkdirSync(diffs, { recursive: true });
            const stem = path.join(diffs, `${record.id.replace(/\//g, '_')}-${s.time}`);
            const ours = new Uint8Array(Buffer.from(r.engine, 'base64'));
            const side = new Uint8Array(s.width * 3 * s.height * 4);
            for (let y = 0; y < s.height; y++) for (let x = 0; x < s.width; x++) {
              const i = (y * s.width + x) * 4;
              const put = (col: number, px: ArrayLike<number>) => { const o = (y * s.width * 3 + col * s.width + x) * 4; side[o] = px[0]!; side[o + 1] = px[1]!; side[o + 2] = px[2]!; side[o + 3] = 255; };
              put(0, ref.data.subarray(i, i + 4));
              put(1, ours.subarray(i, i + 4));
              const d = Math.max(...[0, 1, 2].map((c) => Math.abs(ours[i + c]! - ref.data[i + c]!)));
              const g = (ref.data[i]! + ref.data[i + 1]! + ref.data[i + 2]!) / 6;
              put(2, d ? [255, 0, 0] : [g, g, g]);
            }
            writeFileSync(`${stem}-legacy-engine-diff.png`, encodePng({ width: s.width * 3, height: s.height, channels: 4, data: side }));
          }
        }
      }
    } catch (error) {
      const line = `${record.id}: ${(error as Error).message.split('\n')[0]}`;
      failed.push(line);
      console.log(`FAILED ${line}`);
    }
    const errors = app.pageErrors.slice(errorsBefore);
    if (errors.length) failed.push(`${record.id}: page errors: ${errors.join(' | ').slice(0, 400)}`);
  }
} finally {
  await app?.close();
}
console.log(`\n${records.length} projects with level 1 SVG images; ${frames} frames (${svgPixels} SVG pixels; ${invisible} frames show none) in ${((performance.now() - started) / 1000).toFixed(0)} s; ${failed.length} failed`);
for (const f of failed) console.log(`  FAILED ${f}`);
process.exit(failed.length ? 1 : 0);
