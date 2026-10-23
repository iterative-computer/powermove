/** Depth-8 composition qualification. Run the built app under pm-electron-lock.
 * Synthetic references stay in the external output directory. No build occurs
 * under the lease; launchLegacyApp removes its isolated profile in finally. */
import path from 'node:path';
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { launchLegacyApp } from './app';
import { blendModes, transitions, repoRoot } from './catalog';
import { newProject, type Builder, type Doc } from './doc';
import { encodePng } from './png';

const out = path.resolve(process.argv[2] ?? '');
if (!process.argv[2] || out === repoRoot || out.startsWith(repoRoot + path.sep))
  throw new Error('Supply an external output directory');
mkdirSync(out, { recursive: true });
const gradient = { type: 'linear', angle: 27, stops: [
  { id: 'a', color: '#20283C', position: 0 }, { id: 'b', color: '#70627B', position: 100 },
] };
type Witness = { id: string; doc: Doc; time: number; mbSamples: number; transparent: boolean; covers: string[] };
const cases: Witness[] = [];
const make = (id: string) => newProject(`depth-expanded/${id}`, {
  name: `Depth eight ${id}`, w: 320, h: 180, dur: 2, backgroundFill: gradient, shutter: 1.25,
});
const record = (id: string, b: Builder, covers: string[], time = .5, mbSamples = 0, transparent = false) => {
  b.root.colorDepth = 8;
  cases.push({ id, doc: structuredClone(b.root), time, mbSamples, transparent, covers });
};
const art = (b: Builder) => {
  b.add(b.layer('solid', { d: { color: '#357C93B3', w: 220, h: 115 },
    p: { 'position.x': 38, 'position.y': 32, rotation: -9, opacity: 71 } }));
  return b.add(b.layer('solid', { d: { color: '#C777A3CC', w: 125, h: 79 },
    p: { 'position.x': 126, 'position.y': 69, rotation: 13, opacity: 67 } }));
};
if (blendModes.length !== 9) throw new Error('Blend mode catalog changed; qualify every mode');
for (const mode of blendModes) for (const transparent of [false, true]) {
  const id = `blend-${mode}-${transparent ? 'alpha' : 'opaque'}`, b = make(id);
  art(b);
  b.add(b.layer('solid', { blend: mode, d: { color: '#9BBD6DAA', w: 182, h: 98 },
    p: { 'position.x': 83, 'position.y': 47, rotation: -17, opacity: 63 } }));
  record(id, b, [`blend ${mode}`, 'translucent overlapping sources'], .5, 0, transparent);
}
for (const kind of ['layers', 'group', 'precomp']) {
  const b = make(`motion-${kind}`);
  const draw = (group?: string) => {
    b.add(b.layer('solid', { group, mblur: kind === 'layers', d: { color: '#C87D9AB3', w: 65, h: 43 },
      p: { 'position.x': b.ch(30, [[0, 30], [1, 240]]), 'position.y': 28, opacity: 73 } }));
    b.add(b.layer('solid', { group, mblur: kind === 'layers', d: { color: '#4B9CB9CC', w: 115, h: 15 },
      p: { 'position.x': 160, 'position.y': 116, 'anchor.x': 57, 'anchor.y': 7,
        rotation: b.ch(-75, [[0, -75], [1, 290]]), opacity: 61,
        'scale.x': b.ch(70, [[0, 70], [1, 130]]) } }));
  };
  if (kind === 'layers') draw();
  if (kind === 'group') {
    const group = b.layer('group', { mblur: true, p: { opacity: 71,
      'position.x': b.ch(0, [[0, 0], [1, 22]]) } });
    draw(group.id); b.add(group);
  }
  if (kind === 'precomp') {
    const sub = b.composition({ name: 'Moving nested source', w: 320, h: 180, dur: 2,
      backgroundFill: { type: 'none', stops: [{ id: 'none', color: '#000000', position: 0 }] } });
    b.within(sub, () => draw());
    b.add(b.layer('precomp', { mblur: true, d: { comp: sub.id, w: 320, h: 180 },
      p: { opacity: 69, 'position.x': b.ch(0, [[0, 0], [1, 20]]) } }));
  }
  for (const n of [3, 6, 16, 20]) record(`motion-${kind}-${n}`, b,
    ['motion blur', kind, 'translation', 'rotation', 'scale', 'partial alpha', `${n} samples`], .5, n);
}
for (const transition of transitions) {
  const b = make(`transition-${transition.id}`), layer = art(b);
  const params = Object.fromEntries(transition.params.map(p => [p.k,
    p.k === 'angle' ? 31 : p.k === 'softness' ? 19 : p.k === 'direction' ? 27 : p.def]));
  layer.transitionIn = b.transition(transition.id, .8, params);
  layer.transitionOut = b.transition(transition.id, .6, params);
  record(`transition-${transition.id}-in`, b, [`transition ${transition.id}`, 'in', 'partial opacity'], .4);
  record(`transition-${transition.id}-out`, b, [`transition ${transition.id}`, 'out', 'partial opacity'], 1.7);
}
{
  const b = make('adjustment'); art(b);
  const adjust = b.add(b.layer('adjustment', { blend: 'multiply',
    fx: [b.effect('color', { saturation: -41, contrast: 21 }), b.effect('blur', { amount: 7 })],
    p: { opacity: b.ch(74, [[0, 74], [2, 38]]) } }));
  adjust.masks.push(b.mask('ellipse', { x: 168, y: 82, w: 190, h: 122, feather: 17 }));
  b.add(b.layer('solid', { d: { color: '#C4B37299', w: 43, h: 36 }, p: { 'position.x': 228, 'position.y': 18 } }));
  record('adjustment-early', b, ['adjustment', 'effect chain', 'mask', 'multiply', 'animated opacity'], .4);
  record('adjustment-late', b, ['adjustment', 'effect chain', 'mask', 'multiply', 'animated opacity'], 1.4);
}
{
  const b = make('precomp');
  const inner = b.composition({ name: 'Inner composition', w: 210, h: 130, dur: 2, backgroundFill: gradient });
  b.within(inner, () => {
    b.add(b.layer('solid', { d: { color: '#B07897B3', w: 141, h: 75 }, p: { 'position.x': 19, 'position.y': 15, opacity: 72 } }));
    b.add(b.layer('solid', { blend: 'screen', d: { color: '#5B9EABB3', w: 110, h: 59 }, p: { 'position.x': 71, 'position.y': 52, rotation: -13 } }));
  });
  const outer = b.composition({ name: 'Outer composition', w: 240, h: 150, dur: 2, backgroundFill: gradient });
  b.within(outer, () => b.add(b.layer('precomp', { d: { comp: inner.id, w: 210, h: 130 },
    p: { 'position.x': 11, 'position.y': 8, opacity: 79, rotation: 3 } })));
  b.add(b.layer('precomp', { d: { comp: outer.id, w: 240, h: 150 },
    p: { 'position.x': 42, 'position.y': 17, opacity: 68, rotation: 7 } }));
  record('precomp-nested', b, ['two precomp levels', 'nested screen blend', 'transform', 'opacity']);
}
for (const feather of [0, 11]) {
  const b = make(`path-mask-${feather}`), layer = art(b);
  const blob = b.path('Bezier mask', [[-55, -20, 0, 0, 22, -35], [20, -43, -19, 0, 35, 13],
    [62, 14, 0, -25, -9, 36], [-12, 48, 30, 0, -40, 0]], { closed: true });
  layer.masks.push(b.mask('rect', { x: 64, y: 38, feather }, { path: blob }));
  layer.masks.push(b.mask('ellipse', { x: 60, y: 35, w: 19, h: 23, feather: 2 }, { mode: 'subtract' }));
  record(`path-mask-${feather}`, b, ['Bezier path mask', 'parametric subtract', `feather ${feather}`]);
}
for (const mode of ['alpha', 'alpha-inverted', 'luma', 'luma-inverted']) {
  const b = make(`matte-${mode}`), layer = art(b);
  const source = b.add(b.layer('solid', { on: false, d: { color: '#97979799', w: 110, h: 100 },
    p: { 'position.x': 138, 'position.y': 48, rotation: 11, opacity: 69 } }));
  source.masks.push(b.mask('ellipse', { x: 55, y: 50, w: 104, h: 92, feather: 9 }));
  layer.matteSource = source.id; layer.matteMode = b.ch(mode);
  record(`matte-${mode}`, b, [`track matte ${mode}`, 'masked translucent source']);
}
for (const [effect, params] of [
  ['blur', { amount: 9 }], ['glow', { radius: 21, threshold: 24, intensity: 118 }],
  ['shadow', { distance: 13, angle: 127, softness: 17, opacity: 63, color: '#352947' }],
  ['color', { contrast: 23, saturation: -37, exposure: .21 }],
  ['levels', { inBlack: 9, inWhite: 83, gamma: 1.31 }],
] as const) {
  const b = make(`effect-${effect}`), layer = art(b);
  layer.fx.push(b.effect(effect, params));
  record(`effect-${effect}`, b, [`effect ${effect}`, 'transformed translucent source']);
}
if (process.argv.includes('--prepare')) {
  writeFileSync(path.join(out, 'cases.json'), JSON.stringify(cases, null, 2));
  console.log(`Prepared ${cases.length} depth-8 witnesses`); process.exit(0);
}
const app = await launchLegacyApp();
const samples: any[] = [];
const motionResults = new Map<string, Set<string>>();
try {
  for (const witness of cases) {
    const captured = await app.evaluate(async (witness) => {
      const PM = (window as any).PM;
      PM.pause(); PM.replaceProject(witness.doc); PM.setTime(witness.time, { raw: true, force: true });
      await PM.projectAssetsReady; await PM.prepareFrame(witness.time);
      const gl = PM.GL.gl;
      for (let n = 0; n < 32 && gl.getError(); n++);
      const bytes = PM.GL.renderToPixels(witness.time, 320, 180, { exporting: true,
        transparent: witness.transparent, mblur: witness.mbSamples > 0,
        mbSamples: witness.mbSamples, shutter: witness.doc.shutter });
      const error = gl.getError();
      if (PM.GL.colorDepth !== 8 || error || PM.GL.errors.size)
        throw new Error(`Depth8 capture failed: actual=${PM.GL.colorDepth} error=${error} programs=${JSON.stringify(Array.from(PM.GL.errors))}`);
      const pixels = new Float32Array(bytes.length);
      for (let y = 0; y < 180; y++) for (let x = 0; x < 1280; x++)
        pixels[y * 1280 + x] = bytes[(179 - y) * 1280 + x] / 255;
      if (new Set(Array.from(bytes)).size < 16) throw new Error('Degenerate composition witness');
      return { doc: JSON.parse(JSON.stringify(PM.proj)), pixels: Array.from(pixels) };
    }, witness);
    const id = witness.id;
    if (witness.mbSamples > 0) {
      const family = id.replace(/-\d+$/, '');
      const hashes = motionResults.get(family) ?? new Set<string>();
      hashes.add(createHash('sha256').update(new Uint8Array(new Float32Array(captured.pixels).buffer)).digest('hex'));
      motionResults.set(family, hashes);
    }
    writeFileSync(path.join(out, `${id}.json`), JSON.stringify(captured.doc));
    writeFileSync(path.join(out, `${id}.rgba32f`), new Uint8Array(new Float32Array(captured.pixels).buffer));
    writeFileSync(path.join(out, `${id}.png`), encodePng({ width: 320, height: 180, channels: 4,
      data: Uint8Array.from(captured.pixels, n => Math.round(n * 255)) }));
    samples.push({ id, depth: 8, width: 320, height: 180, time: witness.time,
      mbSamples: witness.mbSamples, transparent: witness.transparent, covers: witness.covers,
      exportPng: false, warmFrames: 0 });
    console.log(`Captured ${id}`);
  }
  if (motionResults.size !== 3 || [...motionResults.values()].some(hashes => hashes.size !== 4))
    throw new Error('Each motion-blur family must produce four distinct sample-count results');
  writeFileSync(path.join(out, 'manifest.json'), JSON.stringify({ samples }, null, 2));
} finally { await app.close(); }
