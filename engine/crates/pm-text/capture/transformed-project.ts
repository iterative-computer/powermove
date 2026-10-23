/* A synthetic project for the transformed-text gates: text under a skew or
   rotation animator, at 2x (layer scale 200%), on both sides of Skia's
   `ApproximateTransformedTextSize` path threshold (256 device px) and of
   `fitsInAtlasDirect` (256 px glyph images); see
   docs/engine/text-render.md, "Textexact2 round 2".

   bun engine/crates/pm-text/capture/transformed-project.ts <out.json>

   Then capture it with today's app (source.ts) into its own set, never
   `text-source-*` (that would replace the real project's capture in the
   gate). Hide the layers so the preview draws no text first: the gate
   replays the glyph atlas from empty, in capture order.

   ~/Library/Caches/powermove-engine/bin/pm-electron-lock bun engine/crates/pm-text/capture/source.ts \
     --project <out.json> --times 0 --hide-layers --out "<corpus>/text-transformed-<UTC stamp>"

   Only fonts that ship with macOS are named; the project is safe to keep
   in the repository. The captured pixels are not and stay in the corpus. */

const ch = (v: unknown) => ({ v, kf: [], expr: null });

const transform = (x: number, y: number, scale: number) => ({
  perspective: ch(50), 'position.z': ch(0), 'anchor.z': ch(0), 'scale.z': ch(100),
  'rotation.x': ch(0), 'rotation.y': ch(0), 'orientation.x': ch(0), 'orientation.y': ch(0),
  'orientation.z': ch(0), 'anchor.x': ch(0), 'anchor.y': ch(0), 'position.x': ch(x),
  'position.y': ch(y), 'scale.x': ch(scale), 'scale.y': ch(scale), rotation: ch(0),
  opacity: ch(100), skew: ch(0),
});

// At time 0 the "in" animator holds its full skew or rotation.
//
// The size gate (`ApproximateTransformedTextSize`): size times the largest
// scale of 2 x [1, -tan(skew); 0, 1]. 262.7 and 265.9 draw as paths; 233.1
// and 239.7 stay in the atlas (distance fields). 75 px under 80 degrees is
// far above it (text_gpu's skew80 render).
//
// The atlas gate (`SkGlyphDigest::fitsInAtlasDirect`): Zapfino at 50 px
// rotated 45 degrees or skewed 30 degrees stays below the distance-field
// range (100 and 133 px), so its glyphs are direct masks, except those
// whose transformed image exceeds 256 px: they draw as paths. One degree
// keeps every mask.
type Case = [id: string, font: string, text: string, size: number, animator: 'skew' | 'rotation', degrees: number];
const CASES: Case[] = [
  ['Lskew60s60', 'Arial', 'Hill.', 60, 'skew', 60],
  ['Lskew55s60', 'Arial', 'Hill.', 60, 'skew', 55],
  ['Lskew30s100', 'Arial', 'Hill.', 100, 'skew', 30],
  ['Lskew20s100', 'Arial', 'Hill.', 100, 'skew', 20],
  ['Lskew80s75', 'Arial', 'Hill.', 75, 'skew', 80],
  ['Zrot45s50', 'Zapfino', 'gag', 50, 'rotation', 45],
  ['Zskew30s50', 'Zapfino', 'gag', 50, 'skew', 30],
  ['Zrot1s50', 'Zapfino', 'gag', 50, 'rotation', 1],
];

const layers = CASES.map(([id, font, text, size, animator, degrees], i) => ({
  id, type: 'text', name: id, from: 0, dur: 10, on: true, lock: false, shy: false,
  collapsed: true, threeD: false, solo: false, color: '#4C8DFF', blend: 'normal', mblur: false,
  parent: null, group: null, matteSource: null, p: transform(300 + (i % 4) * 400, 300 + Math.floor(i / 4) * 400, 200), fx: [],
  transitionIn: null, transitionOut: null, masks: [], locked_intent: {},
  d: {
    text, boxWidth: ch(0), boxHeight: ch(0), font, weight: 400, size,
    tracking: 0, leading: 1.15, color: '#101010', align: 'left', italic: false,
    animators: [{
      id: 'a1', name: 'transform', enabled: true, mode: 'stagger', unit: 'characters',
      order: 'forward', direction: 'in', easing: 'linear',
      p: { amount: ch(100), delay: ch(0), duration: ch(1), stagger: ch(0), [animator]: ch(degrees) },
    }],
  },
}));

const project = {
  id: 'Ptransformed', name: 'transformed', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000',
  backgroundFill: { type: 'none' }, compId: 'Ptransformed-root', compName: 'transformed',
  markers: [], work: [0, 10], params: {}, revision: 0, edits: [], created: 0, shutter: 0.5,
  layers, assets: {}, comps: {},
};

const out = process.argv[2];
if (!out) throw new Error('usage: bun transformed-project.ts <out.json>');
await Bun.write(out, JSON.stringify(project, null, 1));
console.log(`${layers.length} layers into ${out}`);
