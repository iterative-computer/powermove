/* Ported from js/core/model.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {
const uid = PM.uid;

/** A property channel. v = static value, kf = sorted keyframes, expr = optional expression. */
const P = (v: any, o: any = {}) => ({ v, kf: [], expr: null, ...o });
PM.P = P;

const KF = (t: any, v: any, ease = 'power') => {
  const { eo, ei } = PM.Ease.handles(ease);
  return { t, v, eo, ei, hold: false, i: uid('k') };
};
PM.KF = KF;

/* ── channel schema ────────────────────────────────────── */
const CH = {
  'anchor.x':   { label: 'Anchor X', group: 'Transform', unit: 'px', step: 1 },
  'anchor.y':   { label: 'Anchor Y', group: 'Transform', unit: 'px', step: 1 },
  'position.x': { label: 'Position X', group: 'Transform', unit: 'px', step: 1 },
  'position.y': { label: 'Position Y', group: 'Transform', unit: 'px', step: 1 },
  'scale.x':    { label: 'Scale X', group: 'Transform', unit: '%', step: .5 },
  'scale.y':    { label: 'Scale Y', group: 'Transform', unit: '%', step: .5 },
  'rotation':   { label: 'Rotation', group: 'Transform', unit: '°', step: 1 },
  'opacity':    { label: 'Opacity', group: 'Transform', unit: '%', step: 1, min: 0, max: 100 },
  'skew':       { label: 'Skew', group: 'Transform', unit: '°', step: .5, min: -80, max: 80 },
};
/* UI grouping: rows that pair two channels behind one label. */
const PAIRS = [
  ['Anchor Point', ['anchor.x', 'anchor.y'], 'anchor'],
  ['Position', ['position.x', 'position.y'], 'position'],
  ['Scale', ['scale.x', 'scale.y'], 'scale'],
  ['Rotation', ['rotation'], 'rotation'],
  ['Opacity', ['opacity'], 'opacity'],
  ['Skew', ['skew'], 'skew'],
];
PM.CH = CH; PM.PAIRS = PAIRS;

const BLENDS = ['normal', 'add', 'screen', 'multiply', 'overlay', 'softlight', 'difference', 'lighten', 'darken'];
PM.BLENDS = BLENDS;

const TYPE_META: any = {
  solid:  { icon: 'grid',   color: '#4C8DFF', label: 'Solid' },
  text:   { icon: 'code',   color: '#E8E2CF', label: 'Text' },
  shape:  { icon: 'grid',   color: '#6C7BE8', label: 'Shape' },
  image:  { icon: 'layers', color: '#A9A9AE', label: 'Image' },
  video:  { icon: 'cam',    color: '#3B62E8', label: 'Video' },
  audio:  { icon: 'clock',  color: '#4C8DFF', label: 'Audio', visual: false, transform: false, effects: false, masks: false, pickable: false },
  shader: { icon: 'wand',   color: '#FF6B1A', label: 'Shader' },
  null:   { icon: 'dot',    color: '#6a6a70', label: 'Null', visual: false, pickable: false },
  precomp:{ icon: 'layers', color: '#3FCF8E', label: 'Precomp' },
};
PM.TYPE_META = TYPE_META;

/* ── masks ─────────────────────────────────────────────── */
/* Parametric layer masks (rect / ellipse) living in layer space, fully animatable.
   Geometry follows the layer transform; feather is a signed-distance falloff. */
PM.MASK_SHAPES = ['rect', 'ellipse'];
PM.mkMask = (shape = 'rect', comp: any) => {
  comp = comp || PM.proj;
  const s = Math.round(Math.min(comp.w, comp.h) * .5);
  return {
    id: uid('K'), shape, mode: 'add', on: true,
    p: { x: P(0), y: P(0), w: P(s), h: P(s), rotation: P(0), feather: P(24) },
  };
};

/* ── layer factory ─────────────────────────────────────── */
function baseLayer(type: any, name: any, comp: any) {
  const w = comp ? comp.w : 1920, hgt = comp ? comp.h : 1080;
  const transformable = TYPE_META[type].transform !== false;
  const L: any = {
    id: uid('L'), type, name,
    from: 0, dur: comp ? comp.dur : 5,
    on: true, lock: false, solo: false, shy: false, collapsed: true,
    color: TYPE_META[type].color, blend: 'normal', mblur: false, parent: null,
    p: transformable ? {
      'anchor.x': P(0), 'anchor.y': P(0),
      'position.x': P(w / 2), 'position.y': P(hgt / 2),
      'scale.x': P(100), 'scale.y': P(100),
      'rotation': P(0), 'opacity': P(100), 'skew': P(0),
    } : {},
    fx: [],
    transitionIn: null,
    transitionOut: null,
    masks: [],
    d: {},
    locked_intent: {},   // hand edits the agent must preserve
  };
  return L;
}

const DEFAULTS: any = {
  solid:  (L: any, c: any) => { L.d = { color: '#1b1b1f', w: c.w, h: c.h, radius: 0 }; L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0; },
  shape:  (L: any, c: any) => { L.d = { shape: 'rect', color: '#E8E2CF', w: 480, h: 480, radius: 24, stroke: 0, strokeColor: '#ffffff', points: 5 }; },
  text:   (L: any, c: any) => { L.d = { text: 'Powermove', font: 'SF Pro Display', weight: 600, size: 128, tracking: -2, leading: 1.1, color: '#F2F2F2', align: 'center', italic: false }; },
  image:  (L: any) => { L.d = { asset: null, fit: 'cover', w: 1920, h: 1080 }; },
  video:  (L: any) => { L.d = { asset: null, fit: 'cover', trim: 0, speed: 1, w: 1920, h: 1080 }; },
  audio:  (L: any) => { L.d = { asset: null, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 }; L.color = '#4C8DFF'; },
  shader: (L: any, c: any) => {
    L.d = { code: PM.SHADER_TEMPLATE, w: c.w, h: c.h, uniforms: {} };
    L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0;
  },
  null:   (L: any) => { L.d = {}; },
  precomp:(L: any, c: any) => { L.d = { comp: null, w: c.w, h: c.h }; L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0; },
};

PM.mkLayer = (type: any, opts: any = {}, comp: any) => {
  comp = comp || PM.proj;
  const L = baseLayer(type, opts.name || TYPE_META[type].label, comp);
  DEFAULTS[type] && DEFAULTS[type](L, comp);
  if (opts.d) Object.assign(L.d, opts.d);
  if (opts.from != null) L.from = opts.from;
  if (opts.dur != null) L.dur = opts.dur;
  if (opts.color) L.color = opts.color;
  if (opts.p) for (const k in opts.p) if (L.p[k]) L.p[k].v = opts.p[k];
  if (['solid', 'shader', 'precomp'].includes(type)) { L.p['position.x'].v = 0; L.p['position.y'].v = 0; }
  return L;
};

/* ── project factory ───────────────────────────────────── */
PM.normalizeFill = (value: any, fallback = '#000000') => {
  const color = (v: any) => typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v) ? v.toUpperCase() : fallback;
  const raw = value && typeof value === 'object' ? value : {};
  const type = ['solid', 'linear', 'radial', 'none'].includes(raw.type) ? raw.type : 'solid';
  let stops = (Array.isArray(raw.stops) ? raw.stops : []).slice(0, 8).map((stop: any, index: any) => ({
    id: typeof stop?.id === 'string' && stop.id ? stop.id : `stop-${index + 1}`,
    color: color(stop?.color), position: PM.clamp(Number(stop?.position) || 0, 0, 100),
  })).sort((a: any, b: any) => a.position - b.position);
  if (!stops.length) stops = [{ id: 'stop-1', color: color(raw.color || fallback), position: 0 }];
  if (!['solid', 'none'].includes(type) && stops.length < 2) stops.push({ id: 'stop-2', color: stops[0].color, position: 100 });
  if (['solid', 'none'].includes(type)) stops = [{ ...stops[0], position: 0 }];
  return { type, angle: PM.clamp(Number(raw.angle) || 0, -180, 180), stops };
};

PM.mkProject = (o: any = {}) => ({
  id: uid('P'),
  name: o.name || 'Untitled',
  w: o.w || 1920, h: o.h || 1080, fps: o.fps || 30, dur: o.dur || 10,
  bg: o.bg || '#000000',
  backgroundFill: PM.normalizeFill(o.backgroundFill, o.bg || '#000000'),
  layers: [],
  comps: {},          // nested compositions, referenced by precomp layers (d.comp = comp id)
  assets: {},
  markers: [],
  work: [0, o.dur || 10],
  params: {},         // agent/workspace-exposed scene parameters
  revision: 0,
  edits: [],          // shared UI/agent transaction provenance
  created: Date.now(),
});

/* ── lookups & mutation helpers ────────────────────────── */
/* Render scope: while a nested composition renders, lookups resolve inside it.
   Empty stack = the main project (all UI/tool paths). */
PM.scope = [];
PM.curComp = () => PM.scope[PM.scope.length - 1] || PM.proj;
PM.L = (id: any) => {
  const c = PM.curComp();
  return c.layers.find((l: any) => l.id === id) || null;
};
PM.byName = (n: any) => {
  const q = String(n).toLowerCase().trim();
  return PM.proj.layers.find((l: any) => l.name.toLowerCase() === q)
      || PM.proj.layers.find((l: any) => l.name.toLowerCase().includes(q)) || null;
};
PM.sel = { layers: [], keys: [], chan: null };

PM.selectLayers = (ids: any, add = false) => {
  ids = ([] as any[]).concat(ids).filter(Boolean);
  PM.sel.layers = add ? [...new Set([...PM.sel.layers, ...ids])] : ids;
  if (!add) PM.sel.keys = [];
  PM.bus.emit('sel');
  PM.invalidate();
};
PM.selLayers = () => PM.sel.layers.map(PM.L).filter(Boolean);
PM.firstSel = () => PM.selLayers()[0] || null;

PM.addLayer = (L: any, at: any) => {
  PM.proj.layers.splice(at == null ? 0 : at, 0, L);
  PM.bus.emit('layers');
  return L;
};
PM.removeLayers = (ids: any) => {
  ids = ([] as any[]).concat(ids);
  PM.proj.layers = PM.proj.layers.filter((l: any) => !ids.includes(l.id));
  PM.proj.layers.forEach((l: any) => { if (ids.includes(l.parent)) l.parent = null; });
  /* garbage-collect compositions that are no longer referenced by any precomp layer */
  const referenced = new Set<any>();
  const scan = (layers: any) => layers.forEach((l: any) => {
    if (l.type === 'precomp' && l.d && l.d.comp) referenced.add(l.d.comp);
  });
  scan(PM.proj.layers);
  Object.values(PM.proj.comps || {}).forEach((c: any) => scan(c.layers));
  for (const cid of Object.keys(PM.proj.comps || {})) if (!referenced.has(cid)) delete PM.proj.comps[cid];
  PM.sel.layers = PM.sel.layers.filter((i: any) => !ids.includes(i));
  PM.bus.emit('layers');
};

/* ── precompose: collapse layers into a nested composition (AE-style) ──
   The nested comp shares the parent's dimensions, so layer coordinates stay
   valid and the visual result is identical to the un-nested stack. */
PM.precompose = (ids: any, name: any) => {
  ids = ([] as any[]).concat(ids);
  const sel = PM.proj.layers.filter((l: any) => ids.includes(l.id));
  if (!sel.length) return null;
  const compId = uid('C');
  const sub = PM.mkProject({
    name: name || 'Precomp', w: PM.proj.w, h: PM.proj.h,
    fps: PM.proj.fps, dur: PM.proj.dur, bg: '#000000',
  });
  sub.id = compId;
  sub.layers = sel;
  /* parenting across the comp boundary would be ambiguous — break it explicitly */
  sub.layers.forEach((l: any) => { if (l.parent && !ids.includes(l.parent)) l.parent = null; });
  const start = Math.min(...sel.map((l: any) => l.from));
  const end = Math.max(...sel.map((l: any) => l.from + l.dur));
  const span = Math.max(.04, end - start);
  /* Nested rendering receives layer-local time (T - precomp.from), so children
     must be rebased to the nested composition's zero. Keeping root-relative
     starts here made every non-zero precompose silently disappear. */
  sub.layers.forEach((l: any) => { l.from = Math.max(0, l.from - start); });
  sub.dur = span;
  sub.work = [0, span];
  const idx = Math.min(...sel.map((l: any) => PM.proj.layers.indexOf(l)));
  const L = PM.mkLayer('precomp', { name: name || ('Precomp ' + (Object.keys(PM.proj.comps).length + 1)), d: { comp: compId, w: PM.proj.w, h: PM.proj.h } }, PM.proj);
  L.from = Math.max(0, Math.min(start, end - .04));
  L.dur = span;
  PM.proj.comps[compId] = sub;
  PM.proj.layers = PM.proj.layers.filter((l: any) => !ids.includes(l.id));
  PM.proj.layers.forEach((l: any) => { if (l.parent && ids.includes(l.parent)) l.parent = null; });
  PM.sel.layers = [];
  PM.proj.layers.splice(Math.min(idx, PM.proj.layers.length), 0, L);
  PM.selectLayers(L.id);
  PM.bus.emit('layers');
  return L;
};

/** Resolve a precomp layer to its nested project (null when unresolved). */
PM.compOf = (L: any) => {
  if (!L || L.type !== 'precomp' || !L.d || !L.d.comp) return null;
  const c = PM.proj.comps && PM.proj.comps[L.d.comp];
  return c && Array.isArray(c.layers) ? c : null;
};

/* Deep clone that also refreshes ids. */
PM.cloneLayer = (L: any) => {
  const c = JSON.parse(JSON.stringify(L));
  c.id = uid('L');
  c.name = L.name.replace(/ (\d+)$/, '') + ' ' + (PM.proj.layers.filter((x: any) => x.name.startsWith(L.name.replace(/ \d+$/, ''))).length + 1);
  const renew = (prop: any) => (prop?.kf || []).forEach((kf: any) => { kf.i = uid('k'); });
  Object.values(c.p || {}).forEach(renew);
  (c.fx || []).forEach((fx: any) => Object.values(fx.p || {}).forEach(renew));
  (c.masks || []).forEach((mask: any) => Object.values(mask.p || {}).forEach(renew));
  Object.values(c.d?.uniforms || {}).forEach(renew);
  for (const field of ['transitionIn', 'transitionOut']) Object.values(c[field]?.p || {}).forEach(renew);
  return c;
};

/* ── serialization ─────────────────────────────────────── */
PM.serialize = () => JSON.stringify({ v: PM.version, proj: PM.proj, ws: PM.WS && PM.WS.current }, null, 1);
PM.deserialize = (json: any) => {
  const o = typeof json === 'string' ? JSON.parse(json) : json;
  return o.proj || o;
};
}
