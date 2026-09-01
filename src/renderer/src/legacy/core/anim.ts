/* Ported from js/core/anim.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { compileExpression } from './expression';

export function install(PM: PMRegistry): void {
const Ease = PM.Ease, clamp = PM.clamp;
let defaultHandles: any = { eo: [.33, 0], ei: [.67, 1] };
try { defaultHandles = Ease.handles('power') || defaultHandles; } catch (e) { }

let version = 0;
let parentIndexes = new WeakMap<object, any>();
PM.touch = () => {
  version++;
  /* hierarchy memos must never outlive an edit */
  woMemo.clear(); wmMemo.clear(); memoT = null;
  parentIndexes = new WeakMap();
};
PM.animVersion = () => version;
let memoT: any = null;
const woMemo = new Map(), wmMemo = new Map();

/* ── keyframe evaluation ───────────────────────────────── */
function evalKfs(kf: any, t: any) {
  const n = Array.isArray(kf) ? kf.length : 0;
  if (n === 0) return null;
  if (typeof t !== 'number' || !Number.isFinite(t)) return kf[0]?.v ?? null;
  if (t <= kf[0].t) return kf[0].v;
  if (t >= kf[n - 1].t) return kf[n - 1].v;
  let lo = 0, hi = n - 1;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; kf[m].t <= t ? (lo = m) : (hi = m); }
  const a = kf[lo], b = kf[hi];
  if (a.hold) return a.v;
  /* Discrete channels (text, colors, toggles) step at the next key. */
  if (typeof a.v !== 'number' || !Number.isFinite(a.v) ||
      typeof b.v !== 'number' || !Number.isFinite(b.v)) return a.v;
  const span = b.t - a.t;
  const u = span <= 0 ? 0 : (t - a.t) / span;
  if (a.spring) {
    try {
      const amount = Ease.spring(u * span, a.spring);
      if (Number.isFinite(amount)) return a.v + (b.v - a.v) * amount;
    } catch (e) { /* malformed spring data falls through to bezier */ }
  }
  const eo = Array.isArray(a.eo) && Number.isFinite(a.eo[0]) && Number.isFinite(a.eo[1]) ? a.eo : defaultHandles.eo;
  const ei = Array.isArray(b.ei) && Number.isFinite(b.ei[0]) && Number.isFinite(b.ei[1]) ? b.ei : defaultHandles.ei;
  try {
    const f = Ease.bezier(eo[0], eo[1], ei[0], ei[1]);
    const amount = f(u);
    if (Number.isFinite(amount)) return a.v + (b.v - a.v) * amount;
  } catch (e) { /* a broken easing implementation must not poison evaluation */ }
  return a.v + (b.v - a.v) * u;
}
PM.evalKfs = evalKfs;

/** Normalize persisted keyframes without discarding extension-owned fields. */
PM.normalizeKeyframes = (raw: any, fallbackValue: any, _fps?: any, minTime = 0) => {
  const expected = typeof fallbackValue;
  const validValue = (value: any) => {
    if (expected === 'number') return typeof value === 'number' && Number.isFinite(value);
    if (expected === 'string') return typeof value === 'string';
    if (expected === 'boolean') return typeof value === 'boolean';
    return (typeof value === 'number' && Number.isFinite(value)) ||
      typeof value === 'string' || typeof value === 'boolean';
  };
  const pair = (value: any, fallback: any) => Array.isArray(value) && value.length >= 2 &&
    Number.isFinite(value[0]) && Number.isFinite(value[1]) ? [value[0], value[1]] : [...fallback];
  const normalized = (Array.isArray(raw) ? raw : [])
    .map((key: any, index: number) => ({ key, index, time: Number(key?.t) }))
    .filter(({ key, time }: any) => key && typeof key === 'object' &&
      Number.isFinite(time) && time >= minTime && validValue(key.v))
    .map(({ key, index, time }: any) => ({
      ...key,
      t: time === 0 ? 0 : time,
      i: (typeof key.i === 'string' && key.i) || PM.uid('k'),
      eo: pair(key.eo, pair(defaultHandles.eo, [.33, 0])),
      ei: pair(key.ei, pair(defaultHandles.ei, [.67, 1])),
      hold: key.hold === true,
      __order: index,
    }))
    .sort((a: any, b: any) => a.t - b.t || a.__order - b.__order);
  const collapsed: any[] = [];
  const ids = new Set<string>();
  normalized.forEach((key: any) => {
    const previous = collapsed[collapsed.length - 1];
    const clean = { ...key };
    delete clean.__order;
    if (ids.has(clean.i)) clean.i = PM.uid('k');
    ids.add(clean.i);
    if (previous && Math.abs(previous.t - clean.t) <= 1e-6) {
      ids.delete(previous.i);
      collapsed[collapsed.length - 1] = clean;
    }
    else collapsed.push(clean);
  });
  return collapsed;
};

/* ── expressions ───────────────────────────────────────── */
PM.exprCache = new Map();
const EXPR_CACHE_MAX = 256;
function compile(src: any) {
  let c = PM.exprCache.get(src);
  if (c !== undefined) return c;
  const parsed = compileExpression(src);
  c = parsed ? {
    needsIdx: parsed.needsIdx,
    f: (t: number, T: number, fps: number, value: unknown, layer: any, comp: any,
      param: (name: string) => unknown, ch: string, idx: number) => parsed.evaluate({
        t, T, fps, value, layer, comp, param, ch, idx,
      }),
  } : null;
  if (PM.exprCache.size >= EXPR_CACHE_MAX) PM.exprCache.delete(PM.exprCache.keys().next().value);
  PM.exprCache.set(src, c);
  return c;
}
PM.compileExpr = compile;

/* ── channel evaluation ────────────────────────────────── */
function evalProp(prop: any, tLocal: any, ctx: any) {
  if (!prop || typeof prop !== 'object') return null;
  const keys = Array.isArray(prop.kf) ? prop.kf : [];
  let v = keys.length ? evalKfs(keys, tLocal) : prop.v;
  if (prop.expr) {
    const c = compile(prop.expr);
    if (c && c.f) {
      try {
        const r = c.f(tLocal, ctx.T, ctx.fps, v, ctx.layer, ctx.comp, ctx.param, ctx.key,
          c.needsIdx ? indexOfLayer(ctx.comp, ctx.layer) : 0);
        if (typeof r === 'number' && isFinite(r)) v = r;
        else if (typeof r === 'string') v = r;
      } catch (e) { /* keep base value */ }
    }
  }
  return v;
}

const paramGet = (name: any) => {
  const comp = PM.curComp();
  const p = comp.params && comp.params[name];
  return p ? p.value : 0;
};

/** Evaluate one channel of a layer at global time T (seconds). */
PM.ev = (L: any, key: any, T: any) => {
  const prop = L.p[key];
  if (!prop) return 0;
  const tl = T - L.from;
  const comp = PM.curComp();
  return evalProp(prop, tl, {
    T, fps: comp.fps || PM.proj.fps, layer: L, comp, param: paramGet, key,
  });
};
/** Evaluate an effect / uniform param (also layer-local). */
PM.evP = (L: any, prop: any, T: any, key: any) => {
  const comp = PM.curComp();
  return evalProp(prop, T - L.from, {
    T, fps: comp.fps || PM.proj.fps, layer: L, comp, param: paramGet, key,
  });
};

PM.active = (L: any, T: any) => {
  const start = Number(L?.from), duration = Number(L?.dur), time = Number(T);
  if (!L?.on || !Number.isFinite(start) || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(time)) return false;
  const end = start + duration;
  if (time < start - 1e-6) return false;
  const compEnd = Number(PM.curComp()?.dur);
  return time < end - 1e-6 ||
    (Number.isFinite(compEnd) && Math.abs(time - compEnd) <= 1e-6 && Math.abs(end - compEnd) <= 1e-6);
};

/* ── transform matrices (2x3, column-major [a,b,c,d,e,f]) ── */
function mul(m: any, n: any) {
  return [
    m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
    m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
    m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
  ];
}
PM.mul = mul;

PM.localMatrix = (L: any, T: any) => {
  const px = PM.ev(L, 'position.x', T), py = PM.ev(L, 'position.y', T);
  const ax = PM.ev(L, 'anchor.x', T), ay = PM.ev(L, 'anchor.y', T);
  const sx = PM.ev(L, 'scale.x', T) / 100, sy = PM.ev(L, 'scale.y', T) / 100;
  const r = PM.ev(L, 'rotation', T) * Math.PI / 180;
  const sk = Math.tan(PM.ev(L, 'skew', T) * Math.PI / 180);
  const c = Math.cos(r), s = Math.sin(r);
  // translate(p) * rotate * skew * scale * translate(-a)
  let m = [c, s, -s, c, px, py];
  m = mul(m, [1, 0, sk, 1, 0, 0]);
  m = mul(m, [sx, 0, 0, sy, 0, 0]);
  m = mul(m, [1, 0, 0, 1, -ax, -ay]);
  return m;
};

/* Cycle-safe parenting with per-timestamp memoization. Each query climbs the
   parent chain, stopping at any cached ancestor, then caches the cumulative
   result for EVERY node on the way back down — so a hierarchy costs one walk
   per frame total, not one walk per layer per frame. PM.touch() invalidates. */
const parentOf = (L: any) => {
  if (!L?.parent) return null;
  const comp = PM.curComp();
  if (!comp || typeof comp !== 'object') return PM.L(L.parent);
  const layers = Array.isArray(comp.layers) ? comp.layers : [];
  let cached = parentIndexes.get(comp);
  if (!cached || cached.layers !== layers || cached.length !== layers.length) {
    const byId = new Map<any, any>();
    for (const layer of layers) if (!byId.has(layer.id)) byId.set(layer.id, layer);
    cached = { layers, length: layers.length, byId };
    parentIndexes.set(comp, cached);
  }
  return cached.byId.get(L.parent) || null;
};

const indexOfLayer = (comp: any, layer: any) => {
  if (!comp || typeof comp !== 'object' || !Array.isArray(comp.layers)) return 0;
  const layers = comp.layers;
  let cached = parentIndexes.get(comp);
  if (!cached || cached.layers !== layers || cached.length !== layers.length) {
    const byId = new Map<any, any>();
    const indexes = new Map<any, number>();
    layers.forEach((candidate: any, index: number) => {
      if (!byId.has(candidate.id)) byId.set(candidate.id, candidate);
      indexes.set(candidate, index);
    });
    cached = { layers, length: layers.length, byId, indexes };
    parentIndexes.set(comp, cached);
  } else if (!cached.indexes) {
    cached.indexes = new Map(layers.map((candidate: any, index: number) => [candidate, index]));
  }
  return cached.indexes.get(layer) ?? 0;
};

/** Start a fresh evaluation window (called by the compositor per frame). */
PM.beginEval = (T: any) => {
  if (!Object.is(memoT, T)) { woMemo.clear(); wmMemo.clear(); }
  memoT = T;
};

PM.worldMatrix = (L: any, T: any) => {
  if (Object.is(memoT, T)) { const c = wmMemo.get(L); if (c) return c; }
  const chain: any[] = [];
  let cur = L, hit = false;
  while (cur && chain.length < 256) {
    if (chain.length > 8 && chain.includes(cur)) break;
    chain.push(cur);
    if (Object.is(memoT, T)) { const c = wmMemo.get(cur); if (c) { hit = c; break; } }
    cur = parentOf(cur);
  }
  /* fold from the topmost ancestor down to L */
  let m, i0;
  if (hit !== false) { m = hit; i0 = chain.length - 2; }
  else { m = PM.localMatrix(chain[chain.length - 1], T); i0 = chain.length - 2; }
  for (let i = i0; i >= 0; i--) {
    m = mul(m, PM.localMatrix(chain[i], T));
    if (Object.is(memoT, T) && i > 0) wmMemo.set(chain[i], m);
  }
  if (Object.is(memoT, T)) wmMemo.set(L, m);
  return m;
};

PM.worldOpacity = (L: any, T: any) => {
  if (Object.is(memoT, T)) { const c = woMemo.get(L); if (c !== undefined) return c; }
  const chain: any[] = [];
  let cur = L, hit = undefined;
  while (cur && chain.length < 256) {
    if (chain.length > 8 && chain.includes(cur)) break;
    chain.push(cur);
    if (Object.is(memoT, T)) { const c = woMemo.get(cur); if (c !== undefined) { hit = c; break; } }
    cur = parentOf(cur);
  }
  /* hit is the cached cumulative opacity of some ancestor — apply only the layers below it */
  let o, i0;
  if (hit !== undefined) { o = hit; i0 = chain.length - 2; }
  else { o = 1; i0 = chain.length - 1; }
  for (let i = i0; i >= 0; i--) {
    o *= clamp(PM.ev(chain[i], 'opacity', T) / 100, 0, 1);
    if (i > 0 && Object.is(memoT, T)) woMemo.set(chain[i], o);
  }
  o = clamp(o, 0, 1);
  if (Object.is(memoT, T)) woMemo.set(L, o);
  return o;
};

/* True when assigning parentId to L would create a parenting cycle. */
PM.wouldCycle = (L: any, parentId: any) => {
  if (!parentId) return false;
  if (parentId === L.id) return true;
  let cur = PM.L(parentId), guard = 0;
  while (cur && guard++ < 256) {
    if (cur.id === L.id) return true;
    cur = cur.parent ? PM.L(cur.parent) : null;
  }
  return false;
};

/* ── keyframe operations ───────────────────────────────── */
const sortKf = (p: any) => p.kf.sort((a: any, b: any) => a.t - b.t);

PM.setKey = (L: any, key: any, T: any, value: any, ease: any) => {
  const p = L.p[key] || (L.fxProp && L.fxProp[key]);
  if (!p) return null;
  return PM.setKeyOn(p, T - L.from, value, ease, PM.proj.fps);
};
PM.setKeyOn = (p: any, tLocal: any, value: any, ease: any = 'linear', fps: any = 30) => {
  if (!p || typeof p !== 'object') return null;
  p.kf = Array.isArray(p.kf) ? p.kf : [];
  const safeFps = Number.isFinite(Number(fps)) && Number(fps) > 0 ? Number(fps) : 30;
  const t = Math.max(0, PM.snapF(Number.isFinite(Number(tLocal)) ? Number(tLocal) : 0, safeFps));
  let k = p.kf.find((k: any) => Math.abs(k.t - t) < .5 / safeFps);
  if (k) { k.v = value; }
  else { k = PM.KF(t, value, ease); p.kf.push(k); sortKf(p); }
  PM.touch();
  return k;
};
PM.removeKey = (p: any, k: any) => { p.kf = p.kf.filter((x: any) => x !== k && x.i !== k.i); PM.touch(); };
PM.hasKeyAt = (L: any, p: any, T: any) => p.kf.find((k: any) => Math.abs(k.t - (T - L.from)) < .5 / PM.proj.fps) || null;

PM.toggleStopwatch = (L: any, key: any, T: any) => {
  const p = L.p[key]; if (!p) return;
  if (p.kf.length) { p.v = PM.ev(L, key, T); p.kf = []; }
  else PM.setKeyOn(p, T - L.from, p.v, 'linear', PM.proj.fps);
  PM.touch(); PM.invalidate();
};

PM.applyEaseTo = (keys: any, name: any) => {
  const { eo, ei } = Ease.handles(name);
  keys.forEach((k: any) => { k.eo = [...eo]; k.ei = [...ei]; k.hold = name === 'hold'; });
  PM.touch();
};

/** Build an animation from a spec: [{t, v, ease}] in layer-local seconds. */
PM.animate = (L: any, key: any, points: any, opt: any = {}) => {
  const p = L.p[key] || (opt.prop || null);
  if (!p) return false;
  if (!opt.keep) p.kf = [];
  points.forEach((pt: any) => PM.setKeyOn(p, pt.t, pt.v, pt.ease || opt.ease || 'linear', PM.proj.fps));
  PM.touch();
  return true;
};

/* Collect every animatable prop of a layer (transform + effects + masks + shader uniforms). */
PM.allProps = (L: any) => {
  const out = [];
  for (const k in L.p || {}) out.push({ key: k, prop: L.p[k], label: PM.CH[k] ? PM.CH[k].label : k, group: 'Transform' });
  (L.fx || []).forEach((fx: any) => { for (const k in fx.p || {}) out.push({ key: fx.id + '.' + k, prop: fx.p[k], label: k, group: fx.type }); });
  (L.masks || []).forEach((m: any, i: any) => {
    for (const k in m.p) out.push({ key: 'm.' + m.id + '.' + k, prop: m.p[k], label: k, group: 'Mask ' + (i + 1) });
  });
  if (L.type === 'shader') for (const k in L.d?.uniforms || {}) out.push({ key: 'u.' + k, prop: L.d.uniforms[k], label: k, group: 'Shader' });
  if (L.type === 'extension') {
    const definition = PM.layerDefinition?.(L.d?.definition);
    const labels = new Map((definition?.params || []).map((item: any) => [item.k, item.label]));
    for (const k in L.d?.params || {}) out.push({ key: 'x.' + k, prop: L.d.params[k], label: labels.get(k) || k, group: definition?.label || 'Extension' });
  }
  for (const [field, label] of [['transitionIn', 'Transition in'], ['transitionOut', 'Transition out']] as any) {
    const transition = L[field];
    for (const k in transition?.p || {}) out.push({ key: `${field}.p.${k}`, prop: transition.p[k], label: k, group: label });
  }
  return out;
};
PM.findProp = (L: any, key: any) => {
  if (L.p?.[key]) return L.p[key];
  const transition = /^(transitionIn|transitionOut)\.p\.([a-zA-Z][a-zA-Z0-9]*)$/.exec(String(key));
  if (transition) {
    const field = transition[1]!, param = transition[2]!;
    return L[field]?.p?.[param] || null;
  }
  if (key.startsWith('u.')) return L.d?.uniforms && L.d.uniforms[key.slice(2)];
  if (key.startsWith('x.')) return L.d?.params && L.d.params[key.slice(2)];
  if (key.startsWith('m.')) {
    const [, mid, mk] = key.split('.');
    const m = (L.masks || []).find((x: any) => x.id === mid);
    return m ? m.p[mk] : null;
  }
  const [fid, pk] = key.split('.');
  const fx = (L.fx || []).find((f: any) => f.id === fid);
  return fx ? fx.p[pk] : null;
};
}
