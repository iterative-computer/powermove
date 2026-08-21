/* Powermove — project model. Time is SECONDS. Channels are flat, dot-keyed, animatable. */
(() => {
const PM = window.PM, uid = PM.uid;

/** A property channel. v = static value, kf = sorted keyframes, expr = optional expression. */
const P = (v, o = {}) => ({ v, kf: [], expr: null, ...o });
PM.P = P;

const KF = (t, v, ease = 'power') => {
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

const TYPE_META = {
  solid:  { icon: 'grid',   color: '#4C8DFF', label: 'Solid' },
  text:   { icon: 'code',   color: '#E8E2CF', label: 'Text' },
  shape:  { icon: 'grid',   color: '#6C7BE8', label: 'Shape' },
  image:  { icon: 'layers', color: '#A9A9AE', label: 'Image' },
  video:  { icon: 'cam',    color: '#3B62E8', label: 'Video' },
  audio:  { icon: 'clock',  color: '#4C8DFF', label: 'Audio' },
  shader: { icon: 'wand',   color: '#FF6B1A', label: 'Shader' },
  null:   { icon: 'dot',    color: '#6a6a70', label: 'Null' },
  precomp:{ icon: 'layers', color: '#3FCF8E', label: 'Precomp' },
};
PM.TYPE_META = TYPE_META;

/* ── masks ─────────────────────────────────────────────── */
/* Parametric layer masks (rect / ellipse) living in layer space, fully animatable.
   Geometry follows the layer transform; feather is a signed-distance falloff. */
PM.MASK_SHAPES = ['rect', 'ellipse'];
PM.mkMask = (shape = 'rect', comp) => {
  comp = comp || PM.proj;
  const s = Math.round(Math.min(comp.w, comp.h) * .5);
  return {
    id: uid('K'), shape, mode: 'add', on: true,
    p: { x: P(0), y: P(0), w: P(s), h: P(s), rotation: P(0), feather: P(24) },
  };
};

/* ── layer factory ─────────────────────────────────────── */
function baseLayer(type, name, comp) {
  const w = comp ? comp.w : 1920, hgt = comp ? comp.h : 1080;
  const L = {
    id: uid('L'), type, name,
    from: 0, dur: comp ? comp.dur : 5,
    on: true, lock: false, solo: false, shy: false, collapsed: true,
    color: TYPE_META[type].color, blend: 'normal', mblur: false, parent: null,
    p: {
      'anchor.x': P(0), 'anchor.y': P(0),
      'position.x': P(w / 2), 'position.y': P(hgt / 2),
      'scale.x': P(100), 'scale.y': P(100),
      'rotation': P(0), 'opacity': P(100), 'skew': P(0),
    },
    fx: [],
    masks: [],
    d: {},
    locked_intent: {},   // hand edits the agent must preserve
  };
  return L;
}

const DEFAULTS = {
  solid:  (L, c) => { L.d = { color: '#1b1b1f', w: c.w, h: c.h, radius: 0 }; L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0; },
  shape:  (L, c) => { L.d = { shape: 'rect', color: '#E8E2CF', w: 480, h: 480, radius: 24, stroke: 0, strokeColor: '#ffffff', points: 5 }; },
  text:   (L, c) => { L.d = { text: 'Powermove', font: 'Geist', weight: 600, size: 128, tracking: -2, leading: 1.1, color: '#F2F2F2', align: 'center', italic: false }; },
  image:  (L) => { L.d = { asset: null, fit: 'cover', w: 1920, h: 1080 }; },
  video:  (L) => { L.d = { asset: null, fit: 'cover', trim: 0, speed: 1, w: 1920, h: 1080 }; },
  audio:  (L) => { L.d = { asset: null, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 }; L.color = '#4C8DFF'; },
  shader: (L, c) => {
    L.d = { code: PM.SHADER_TEMPLATE, w: c.w, h: c.h, uniforms: {} };
    L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0;
  },
  null:   (L) => { L.d = {}; },
  precomp:(L, c) => { L.d = { comp: null, w: c.w, h: c.h }; L.p['anchor.x'].v = 0; L.p['anchor.y'].v = 0; },
};

PM.mkLayer = (type, opts = {}, comp) => {
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
PM.mkProject = (o = {}) => ({
  id: uid('P'),
  name: o.name || 'Untitled',
  w: o.w || 1920, h: o.h || 1080, fps: o.fps || 30, dur: o.dur || 10,
  bg: o.bg || '#000000',
  layers: [],
  comps: {},          // nested compositions, referenced by precomp layers (d.comp = comp id)
  assets: {},
  markers: [],
  work: [0, o.dur || 10],
  params: {},         // agent/workspace-exposed scene parameters
  created: Date.now(),
});

/* ── lookups & mutation helpers ────────────────────────── */
/* Render scope: while a nested composition renders, lookups resolve inside it.
   Empty stack = the main project (all UI/tool paths). */
PM.scope = [];
PM.curComp = () => PM.scope[PM.scope.length - 1] || PM.proj;
PM.L = (id) => {
  const c = PM.curComp();
  return c.layers.find(l => l.id === id) || null;
};
PM.byName = (n) => {
  const q = String(n).toLowerCase().trim();
  return PM.proj.layers.find(l => l.name.toLowerCase() === q)
      || PM.proj.layers.find(l => l.name.toLowerCase().includes(q)) || null;
};
PM.sel = { layers: [], keys: [], chan: null };

PM.selectLayers = (ids, add = false) => {
  ids = [].concat(ids).filter(Boolean);
  PM.sel.layers = add ? [...new Set([...PM.sel.layers, ...ids])] : ids;
  if (!add) PM.sel.keys = [];
  PM.bus.emit('sel');
  PM.invalidate();
};
PM.selLayers = () => PM.sel.layers.map(PM.L).filter(Boolean);
PM.firstSel = () => PM.selLayers()[0] || null;

PM.addLayer = (L, at) => {
  PM.proj.layers.splice(at == null ? 0 : at, 0, L);
  PM.bus.emit('layers');
  return L;
};
PM.removeLayers = (ids) => {
  ids = [].concat(ids);
  PM.proj.layers = PM.proj.layers.filter(l => !ids.includes(l.id));
  PM.proj.layers.forEach(l => { if (ids.includes(l.parent)) l.parent = null; });
  /* garbage-collect compositions that are no longer referenced by any precomp layer */
  const referenced = new Set();
  const scan = (layers) => layers.forEach(l => {
    if (l.type === 'precomp' && l.d && l.d.comp) referenced.add(l.d.comp);
  });
  scan(PM.proj.layers);
  Object.values(PM.proj.comps || {}).forEach(c => scan(c.layers));
  for (const cid of Object.keys(PM.proj.comps || {})) if (!referenced.has(cid)) delete PM.proj.comps[cid];
  PM.sel.layers = PM.sel.layers.filter(i => !ids.includes(i));
  PM.bus.emit('layers');
};

/* ── precompose: collapse layers into a nested composition (AE-style) ──
   The nested comp shares the parent's dimensions, so layer coordinates stay
   valid and the visual result is identical to the un-nested stack. */
PM.precompose = (ids, name) => {
  ids = [].concat(ids);
  const sel = PM.proj.layers.filter(l => ids.includes(l.id));
  if (!sel.length) return null;
  const compId = uid('C');
  const sub = PM.mkProject({
    name: name || 'Precomp', w: PM.proj.w, h: PM.proj.h,
    fps: PM.proj.fps, dur: PM.proj.dur, bg: '#000000',
  });
  sub.id = compId;
  sub.layers = sel;
  /* parenting across the comp boundary would be ambiguous — break it explicitly */
  sub.layers.forEach(l => { if (l.parent && !ids.includes(l.parent)) l.parent = null; });
  const start = Math.min(...sel.map(l => l.from));
  const end = Math.max(...sel.map(l => l.from + l.dur));
  const idx = Math.min(...sel.map(l => PM.proj.layers.indexOf(l)));
  const L = PM.mkLayer('precomp', { name: name || ('Precomp ' + (Object.keys(PM.proj.comps).length + 1)), d: { comp: compId, w: PM.proj.w, h: PM.proj.h } }, PM.proj);
  L.from = Math.max(0, Math.min(start, end - .04));
  L.dur = Math.max(.04, end - L.from);
  PM.proj.comps[compId] = sub;
  PM.proj.layers = PM.proj.layers.filter(l => !ids.includes(l.id));
  PM.proj.layers.forEach(l => { if (l.parent && ids.includes(l.parent)) l.parent = null; });
  PM.sel.layers = [];
  PM.proj.layers.splice(Math.min(idx, PM.proj.layers.length), 0, L);
  PM.selectLayers(L.id);
  PM.bus.emit('layers');
  return L;
};

/** Resolve a precomp layer to its nested project (null when unresolved). */
PM.compOf = (L) => {
  if (!L || L.type !== 'precomp' || !L.d || !L.d.comp) return null;
  const c = PM.proj.comps && PM.proj.comps[L.d.comp];
  return c && Array.isArray(c.layers) ? c : null;
};

/* Deep clone that also refreshes ids. */
PM.cloneLayer = (L) => {
  const c = JSON.parse(JSON.stringify(L));
  c.id = uid('L');
  c.name = L.name.replace(/ (\d+)$/, '') + ' ' + (PM.proj.layers.filter(x => x.name.startsWith(L.name.replace(/ \d+$/, ''))).length + 1);
  for (const k in c.p) c.p[k].kf.forEach(kf => kf.i = uid('k'));
  return c;
};

/* ── serialization ─────────────────────────────────────── */
PM.serialize = () => JSON.stringify({ v: PM.version, proj: PM.proj, ws: PM.WS && PM.WS.current }, null, 1);
PM.deserialize = (json) => {
  const o = typeof json === 'string' ? JSON.parse(json) : json;
  return o.proj || o;
};
})();
