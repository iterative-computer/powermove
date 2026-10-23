// Deterministic synthetic projects for spike S5.
//
// `project884()` is shaped like the 884-layer project the plan names
// (docs/rust-engine.md, section 25): it reproduces that project's aggregate
// structure only (layer type counts, group nesting, parenting, transform
// channel set, animated channel counts, key and interpolation mix, effect
// parameter count, JSON size per layer, edit-log size). No content from the
// private project is used; every name, value, and id here is generated.
//
// `coverageProject()` is a smaller stress document for parity and bit-identity
// checks: dense keys, holds, auto and continuous bezier, extreme influences
// and speeds, large rotations, skew near 90 degrees, deep parenting.

export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const CHANNELS = [
  'position.x', 'position.y', 'position.z', 'anchor.x', 'anchor.y', 'anchor.z',
  'scale.x', 'scale.y', 'scale.z', 'rotation', 'rotation.x', 'rotation.y',
  'orientation.x', 'orientation.y', 'orientation.z', 'skew', 'opacity', 'perspective',
];

const WORDS = 'alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu'.split(' ');

function makeIds(rand) {
  const used = new Set();
  return (prefix) => {
    for (;;) {
      let s = prefix;
      for (let i = 0; i < 8; i++) s += '0123456789abcdefghijklmnopqrstuvwxyz'[Math.floor(rand() * 36)];
      if (!used.has(s)) { used.add(s); return s; }
    }
  };
}

const hex = (rand) => '#' + Math.floor(rand() * 0xffffff).toString(16).padStart(6, '0');
const pick = (rand, list) => list[Math.floor(rand() * list.length)];
const round = (v, digits = 3) => Number(v.toFixed(digits));
const prop = (v) => ({ v, kf: [], expr: null });

function transform(rand, w, h) {
  const values = {
    'position.x': round(rand() * w, 6), 'position.y': round(rand() * h, 6), 'position.z': 0,
    'anchor.x': round(rand() * 200 - 100, 6), 'anchor.y': round(rand() * 200 - 100, 6), 'anchor.z': 0,
    'scale.x': 100, 'scale.y': 100, 'scale.z': 100,
    rotation: rand() < 0.3 ? round(rand() * 360 - 180) : 0, 'rotation.x': 0, 'rotation.y': 0,
    'orientation.x': 0, 'orientation.y': 0, 'orientation.z': 0,
    skew: rand() < 0.05 ? round(rand() * 20 - 10) : 0, opacity: 100, perspective: 0,
  };
  const p = {};
  for (const key of CHANNELS) p[key] = prop(values[key]);
  return p;
}

function ease(rand, kind) {
  if (kind === 'easy') return { speed: 0, influence: 33.333333333333336 };
  return { speed: round(rand() * 400 - 200, 4), influence: round(10 + rand() * 80, 4) };
}

function key(rand, uid, t, v, inInterp, outInterp) {
  return {
    t, v, i: uid('k'),
    outInterp, outEase: ease(rand, rand() < 0.6 ? 'easy' : 'custom'),
    inInterp, inEase: ease(rand, rand() < 0.6 ? 'easy' : 'custom'),
    autoBezier: false, continuous: false,
  };
}

function baseLayer(rand, uid, type, w, h) {
  const from = rand() < 0.1 ? 0 : round(rand() * 50, 4);
  return {
    id: uid('l'), type, name: `${pick(rand, WORDS)} ${pick(rand, WORDS)} ${pick(rand, WORDS)} ${Math.floor(rand() * 100)}`,
    from, dur: round(1 + rand() * 20, 4), on: true, lock: false, shy: false, collapsed: false,
    color: pick(rand, ['red', 'yellow', 'aqua', 'pink', 'lavender', 'peach', 'seafoam', 'blue']),
    blend: 'normal', mblur: false, parent: null,
    p: type === 'audio' ? {} : transform(rand, w, h),
    fx: [], transitionIn: null, transitionOut: null, masks: [], d: {},
    locked_intent: false, solo: false, group: null,
  };
}

function content(rand, layer, uid) {
  switch (layer.type) {
    case 'shape':
      layer.d = {
        color: hex(rand), shape: pick(rand, ['rect', 'ellipse', 'polygon', 'star']),
        w: round(10 + rand() * 600, 5), h: round(10 + rand() * 400, 5), radius: round(rand() * 24),
        stroke: round(rand() * 4), strokeColor: hex(rand), points: 3 + Math.floor(rand() * 6),
      };
      break;
    case 'text':
      layer.d = {
        text: Array.from({ length: 2 + Math.floor(rand() * 5) }, () => pick(rand, WORDS)).join(' '),
        boxWidth: prop(round(100 + rand() * 900)), boxHeight: prop(round(40 + rand() * 300)),
        font: pick(rand, ['Inter', 'Geist', 'IBM Plex Sans', 'Source Serif 4']),
        weight: pick(rand, [400, 500, 600, 700]), size: round(12 + rand() * 120), tracking: 0,
        leading: round(1 + rand(), 2), color: hex(rand), align: pick(rand, ['left', 'center', 'right']),
        italic: false,
      };
      break;
    case 'solid':
      layer.d = { color: hex(rand), w: 1920, h: 1080, radius: 0 };
      break;
    case 'audio':
      layer.d = { asset: uid('a'), trim: 0, gain: 1, fadeIn: 0, fadeOut: 0 };
      break;
    default:
      layer.d = {};
  }
}

/** The 884-layer shape. */
export function project884(seed = 884) {
  const rand = rng(seed);
  const uid = makeIds(rand);
  const W = 1920, H = 1080;
  const layers = [];
  const add = (type, group) => {
    const layer = baseLayer(rand, uid, type, W, H);
    layer.group = group ?? null;
    content(rand, layer, uid);
    layers.push(layer);
    return layer;
  };
  // Group tree: 4 roots, 15 groups inside roots, 20 inside those (39 groups).
  const roots = Array.from({ length: 4 }, () => add('group', null));
  const mid = Array.from({ length: 15 }, (_, i) => add('group', roots[i % 4].id));
  const deep = Array.from({ length: 20 }, (_, i) => add('group', mid[i % 15].id));
  // 845 non-group layers: 21 ungrouped, 87 in roots, 690 in mid, 47 in deep.
  const types = [
    ...Array(706).fill('shape'), ...Array(131).fill('text'), ...Array(2).fill('solid'),
    ...Array(3).fill('null'), ...Array(3).fill('audio'),
  ];
  for (let i = types.length - 1; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [types[i], types[j]] = [types[j], types[i]];
  }
  const placement = [
    ...Array(21).fill(null), ...Array.from({ length: 87 }, (_, i) => roots[i % 4].id),
    ...Array.from({ length: 690 }, (_, i) => mid[i % 15].id), ...Array.from({ length: 47 }, (_, i) => deep[i % 20].id),
  ];
  const members = types.map((type, i) => add(type, placement[i]));
  // 8 parented layers, parented to null layers or earlier shapes.
  // A few heavier layers, as in the real project (SVG paths, anchor bounds).
  members.filter((l) => l.type === 'text').slice(0, 12).forEach((l) => {
    l.d.fontAnchorBounds = { x: round(rand() * 10, 6), y: round(-rand() * 40, 6), w: round(rand() * 400, 6), h: round(rand() * 60, 6) };
  });
  const svg = members.find((l) => l.type === 'shape');
  svg.d.paths = Array.from({ length: 24 }, () => ({ d: Array.from({ length: 12 }, () => `L${round(rand() * 500, 2)} ${round(rand() * 500, 2)}`).join(' '), fill: hex(rand) }));
  members.filter((l) => l.type === 'shape').slice(1, 56).forEach((l) => { l.scaleLinked = true; });
  const nulls = members.filter((l) => l.type === 'null');
  const shapes = members.filter((l) => l.type === 'shape');
  for (let i = 0; i < 8; i++) {
    const child = shapes[100 + i * 37];
    child.parent = (i < 3 ? nulls[i] : shapes[10 + i]).id;
  }
  // 66 animated channels with 143 keys: 1 x 4 keys, 9 x 3, 56 x 2.
  const animatedChannels = [
    ...Array(26).fill('position.y'), ...Array(16).fill('position.x'),
    ...Array(12).fill('scale.x'), ...Array(12).fill('scale.y'),
  ];
  const keyCounts = [4, ...Array(9).fill(3), ...Array(56).fill(2)];
  const middle = [
    ...Array(5).fill(['bezier', 'bezier']), ...Array(4).fill(['linear', 'linear']),
    ['linear', 'bezier'], ['bezier', 'linear'],
  ];
  let middleIndex = 0;
  const animatable = members.filter((l) => l.type === 'shape' || l.type === 'text');
  const targets = new Set();
  animatedChannels.forEach((channel, i) => {
    let layer;
    do layer = animatable[Math.floor(rand() * animatable.length)];
    while (targets.has(layer.id + channel));
    targets.add(layer.id + channel);
    const n = keyCounts[i];
    const base = layer.p[channel].v;
    const keys = [];
    let t = Math.round(rand() * 30) / 30;
    for (let k = 0; k < n; k++) {
      const v = round(base + (rand() * 2 - 1) * (channel.startsWith('scale') ? 60 : 400));
      let interp;
      if (k === 0) interp = ['linear', 'bezier'];
      else if (k === n - 1) interp = ['bezier', 'linear'];
      else interp = middle[middleIndex++ % middle.length];
      keys.push(key(rand, uid, round(t, 6), v, interp[0], interp[1]));
      t += Math.round(6 + rand() * 54) / 30;
    }
    layer.p[channel].kf = keys;
  });
  // 20 effects with 213 parameters on 10 layers.
  const paramCounts = [13, 12, 12, 11, 11, 11, 11, 11, 11, 11, 10, 10, 10, 10, 10, 10, 10, 10, 10, 9];
  paramCounts.forEach((count, i) => {
    const layer = shapes[200 + Math.floor(i / 2) * 19];
    const p = {};
    for (let k = 0; k < count; k++) p[`param${k}`] = prop(round(rand() * 100));
    layer.fx.push({ id: uid('f'), type: pick(rand, ['blur', 'glow', 'shadow', 'tint', 'noise']), on: true, p, ...(i ? { missing: false } : {}) });
  });
  const edits = Array.from({ length: 509 }, (_, i) => ({
    id: uid('e'), at: 1700000000000 + i * 1000, source: i % 5 ? 'user' : 'agent',
    label: `${pick(rand, ['Move', 'Scale', 'Rotate', 'Retime', 'Recolor'])} ${pick(rand, WORDS)}`,
    commands: Array.from({ length: 3 }, () => ({
      op: pick(rand, ['set', 'key', 'move']), layer: pick(rand, layers).id,
      path: ['p', pick(rand, CHANNELS), 'v'], value: round(rand() * 1000), prev: round(rand() * 1000),
      note: Array.from({ length: 6 }, () => pick(rand, WORDS)).join(' '),
    })),
  }));
  return {
    id: uid('proj'), name: 'S5 synthetic', w: W, h: H, fps: 30, dur: 60, bg: '#000000',
    backgroundFill: { type: 'solid', color: '#000000', opacity: 1, gradient: null, angle: 0 },
    compId: uid('comp'), compName: 'Main', layers, comps: {},
    assets: Object.fromEntries(Array.from({ length: 3 }, () => [uid('a'), { kind: 'audio', name: `${pick(rand, WORDS)}.wav`, dur: round(rand() * 60), sampleRate: 48000, channels: 2, bytes: Math.floor(rand() * 1e7), hash: uid('h') + uid('h') + uid('h') + uid('h') }])),
    markers: [], work: { start: 0, end: 60 },
    exportDefaults: { format: 'mp4', codec: 'h264', fps: 30, scale: 1, quality: 'high', audio: true, range: 'work' },
    params: {}, revision: 1, edits, created: 1700000000000, shutter: 180,
  };
}

/** Stress document for parity and bit-identity. */
export function coverageProject(seed = 55) {
  const rand = rng(seed);
  const uid = makeIds(rand);
  const layers = [];
  const group = baseLayer(rand, uid, 'group', 1920, 1080);
  group.p.rotation.v = 1234.5;
  layers.push(group);
  let previous = null;
  for (let i = 0; i < 60; i++) {
    const layer = baseLayer(rand, uid, 'shape', 1920, 1080);
    content(rand, layer, uid);
    if (i % 3 === 0) layer.group = group.id;
    if (previous && i % 4 === 1) layer.parent = previous.id;
    if (i % 10 === 0) layer.p.skew.v = 89.9;
    if (i % 7 === 0) layer.p.rotation.v = 1e5 + i;
    for (const channel of ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'skew', 'anchor.x', 'opacity']) {
      if (rand() < 0.5) continue;
      const n = 2 + Math.floor(rand() * 8);
      const keys = [];
      let t = -1 + rand() * 2;
      for (let k = 0; k < n; k++) {
        const kinds = ['linear', 'bezier', 'hold'];
        const k1 = key(rand, uid, round(t, 6), round((rand() * 2 - 1) * 1000, 4), pick(rand, kinds), pick(rand, kinds));
        const r = rand();
        if (r < 0.15) k1.autoBezier = true;
        else if (r < 0.3) k1.continuous = true;
        if (rand() < 0.1) k1.outEase = { speed: round(rand() * 1e5 - 5e4, 2), influence: rand() < 0.5 ? 0.1 : 100 };
        if (rand() < 0.1) k1.inEase = { speed: round(rand() * 1e5 - 5e4, 2), influence: rand() < 0.5 ? 0.05 : 250 };
        keys.push(k1);
        t += 0.01 + rand() * 3;
      }
      layer.p[channel].kf = keys;
    }
    layers.push(layer);
    previous = layer;
  }
  return { id: uid('proj'), name: 'S5 coverage', w: 1920, h: 1080, fps: 30, dur: 30, layers, comps: {}, revision: 1, edits: [] };
}

/** Sample times for vectors: frame-aligned, off-frame, negative, past the end. */
export function sampleTimes(count = 121, span = 60) {
  const times = [];
  for (let k = 0; k < count; k++) times.push(-1 + (k * (span + 2)) / (count - 1) + (k % 7) * 0.000731);
  return times;
}

/** Shape statistics, for checking the generator against its targets. */
export function shapeStats(project) {
  const L = project.layers;
  const byId = new Map(L.map((l) => [l.id, l]));
  const count = (xs) => xs.reduce((m, k) => ((m[k] = (m[k] || 0) + 1), m), {});
  const depth = (l) => { let d = 0, c = l; const seen = new Set(); while (c?.group && !seen.has(c)) { seen.add(c); c = byId.get(c.group); d++; } return d; };
  const props = L.flatMap((l) => Object.values(l.p || {}));
  const keys = props.flatMap((p) => p.kf || []);
  const sizes = L.map((l) => JSON.stringify(l).length).sort((a, b) => a - b);
  return {
    layers: L.length, types: count(L.map((l) => l.type)), depths: count(L.map(depth)),
    parented: L.filter((l) => l.parent).length, transformProps: props.length,
    animatedProps: props.filter((p) => p.kf?.length).length, keys: keys.length,
    interp: count(keys.map((k) => `${k.inInterp}/${k.outInterp}`)),
    fxParams: L.reduce((n, l) => n + l.fx.reduce((m, f) => m + Object.keys(f.p).length, 0), 0),
    layerBytes: { p50: sizes[sizes.length >> 1], p90: sizes[Math.floor(sizes.length * 0.9)], total: sizes.reduce((a, b) => a + b, 0) },
    editsBytes: JSON.stringify(project.edits).length, fileBytes: JSON.stringify(project).length,
  };
}
