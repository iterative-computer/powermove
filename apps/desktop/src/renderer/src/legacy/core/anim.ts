import { structuredProperties } from './vector-paths';
/* Ported from js/core/anim.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { canAnimateContent, contentLabel, isProperty, resolveContent } from './content-properties';
import { compileExpression } from './expression';
import { temporalKeys, temporalValue, temporalEvaluate } from './temporal-bridge';

export function install(PM: PMRegistry): void {
const Ease = PM.Ease, clamp = PM.clamp;
let defaultHandles: any = { eo: [.33, 0], ei: [.67, 1] };
try { defaultHandles = Ease.handles('power') || defaultHandles; } catch (e) { }

let version = 0;
let parentIndexes = new WeakMap<object, any>();
PM.touch = () => {
  version++;
  PM.ProjectIndex?.invalidateKeyframes?.();
  /* hierarchy memos must never outlive an edit */
  woMemo.clear(); wmMemo.clear(); lmMemo.clear(); memoT = null;
  parentIndexes = new WeakMap();
};
PM.animVersion = () => version;
let memoT: any = null;
const woMemo = new Map(), wmMemo = new Map(), lmMemo = new Map();

/* ── keyframe evaluation ───────────────────────────────── */
function evalKfs(kf: any, t: any, interpolateColor = false) {
  const n = Array.isArray(kf) ? kf.length : 0;
  if (n === 0) return null;
  if (typeof t !== 'number' || !Number.isFinite(t)) return kf[0]?.v ?? null;
  if (typeof kf[0].v === 'number' && typeof kf[n-1].v === 'number') return temporalEvaluate(kf,t,version,Ease.spring);
  if (t <= kf[0].t) return kf[0].v;
  if (t >= kf[n - 1].t) return kf[n - 1].v;
  let lo = 0, hi = n - 1;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; kf[m].t <= t ? (lo = m) : (hi = m); }
  temporalKeys(kf,version);
  const a = kf[lo], b = kf[hi];
  if (a.hold) return a.v;
  const colors = interpolateColor && /^#[0-9a-f]{6}$/i.test(a.v) && /^#[0-9a-f]{6}$/i.test(b.v);
  // Text and toggles remain discrete, including text that looks like a hex color.
  if (!colors && (typeof a.v !== 'number' || !Number.isFinite(a.v) ||
      typeof b.v !== 'number' || !Number.isFinite(b.v))) return a.v;
  const mix = (amount: number) => colors
    ? '#' + [1, 3, 5].map(index => {
      const start = parseInt(a.v.slice(index, index + 2), 16), end = parseInt(b.v.slice(index, index + 2), 16);
      return Math.round(clamp(start + (end - start) * amount, 0, 255)).toString(16).padStart(2, '0');
    }).join('')
    : a.v + (b.v - a.v) * amount;
  const span = b.t - a.t;
  const u = span <= 0 ? 0 : (t - a.t) / span;
  if (a.spring) {
    try {
      const amount = Ease.spring(u * span, a.spring);
      if (Number.isFinite(amount)) return mix(amount);
    } catch (e) { /* malformed spring data falls through to bezier */ }
  }
  if (!colors) return temporalValue(kf,lo,t);
  const eo = Array.isArray(a.eo) && Number.isFinite(a.eo[0]) && Number.isFinite(a.eo[1]) ? a.eo : defaultHandles.eo;
  const ei = Array.isArray(b.ei) && Number.isFinite(b.ei[0]) && Number.isFinite(b.ei[1]) ? b.ei : defaultHandles.ei;
  try {
    const f = Ease.bezier(eo[0], eo[1], ei[0], ei[1]);
    const amount = f(u);
    if (Number.isFinite(amount)) return mix(amount);
  } catch (e) { /* a broken easing implementation must not poison evaluation */ }
  return mix(u);
}
PM.evalKfs = evalKfs;
PM.resolveContent = (layer: any, time = PM.time) => resolveContent(PM, layer, time);

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
  return temporalKeys(collapsed);
};

/* ── expressions ───────────────────────────────────────── */
PM.exprCache = new Map();
PM.expressionErrors = new WeakMap();
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
  let v = keys.length ? evalKfs(keys, tLocal, ctx.key === 'c.color' || ctx.key === 'c.strokeColor') : prop.v;
  if (prop.expr) {
    const c = compile(prop.expr);
    if (c && c.f) {
      try {
        const r = c.f(tLocal, ctx.T, ctx.fps, v, ctx.layer, ctx.comp, ctx.param, ctx.key,
          c.needsIdx ? indexOfLayer(ctx.comp, ctx.layer) : 0);
        if ((typeof r === 'number' && isFinite(r)) || typeof r === 'string' || typeof r === 'boolean') { v = r; PM.expressionErrors.delete(prop); }
        else PM.expressionErrors.set(prop,'Expression did not return a finite value.');
      } catch (e) { PM.expressionErrors.set(prop, e instanceof Error ? e.message : 'Expression failed.'); }
    } else PM.expressionErrors.set(prop,'Expression syntax is invalid.');
  } else {
    PM.expressionErrors.delete(prop);
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
  if (!prop.expr) {
    PM.expressionErrors.delete(prop);
    return prop.kf?.length ? evalKfs(prop.kf, tl, key === 'c.color' || key === 'c.strokeColor') : prop.v;
  }
  const comp = PM.curComp();
  return evalProp(prop, tl, {
    T, fps: comp.fps || PM.proj.fps, layer: L, comp, param: paramGet, key,
  });
};
/** Evaluate an effect / uniform param (also layer-local). */
PM.evP = (L: any, prop: any, T: any, key: any) => {
  if (!prop || typeof prop !== 'object') return null;
  if (!prop.expr) {
    PM.expressionErrors.delete(prop);
    return prop.kf?.length ? evalKfs(prop.kf, T - L.from, key === 'c.color' || key === 'c.strokeColor') : prop.v;
  }
  const comp = PM.curComp();
  return evalProp(prop, T - L.from, {
    T, fps: comp.fps || PM.proj.fps, layer: L, comp, param: paramGet, key,
  });
};

PM.active = (L: any, T: any) => {
  if (L?.group && PM.groupAncestors?.(L).some((group: any) => !(isProperty(group.on) ? PM.evP(group, group.on, T, 'l.on') : group.on))) return false;
  const start = Number(L?.from), duration = Number(L?.dur), time = Number(T);
  if (!(isProperty(L?.on) ? PM.evP(L, L.on, T, 'l.on') : L?.on) || !Number.isFinite(start) || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(time)) return false;
  if (L.type === 'group') return true;
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
  if (Object.is(memoT, T)) { const cached = lmMemo.get(L); if (cached) return cached; }
  const px = PM.ev(L, 'position.x', T), py = PM.ev(L, 'position.y', T);
  const ax = PM.ev(L, 'anchor.x', T), ay = PM.ev(L, 'anchor.y', T);
  const sx = PM.ev(L, 'scale.x', T) / 100, sy = PM.ev(L, 'scale.y', T) / 100;
  const r = PM.ev(L, 'rotation', T) * Math.PI / 180;
  const sk = Math.tan(PM.ev(L, 'skew', T) * Math.PI / 180);
  const c = Math.cos(r), s = Math.sin(r);
  // Compose the affine transform directly, without three intermediate matrices per layer.
  const a=c*sx,b=s*sx,cc=(c*sk-s)*sy,d=(s*sk+c)*sy;
  const matrix = [a,b,cc,d,px-a*ax-cc*ay,py-b*ax-d*ay];
  if (Object.is(memoT, T)) lmMemo.set(L, matrix);
  return matrix;
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
  if (!Object.is(memoT, T)) { woMemo.clear(); wmMemo.clear(); lmMemo.clear(); }
  memoT = T;
};

/** Compose group transforms once, even when several members share a parent rig.
 * Group membership changes the surrounding coordinate space, never the parent link. */
PM.transformParentMatrix = (L: any, T: any, parent = parentOf(L)) => {
  const groups = new Set<string>();
  let matrix = [1,0,0,1,0,0];
  const appendParents = (layer: any, firstParent = parentOf(layer)) => {
    const chain: any[] = [], seen = new Set<any>([layer]);
    for (let cur = firstParent; cur && !seen.has(cur) && chain.length < 256; cur = parentOf(cur)) {
      seen.add(cur); chain.push(cur); appendGroups(cur);
    }
    for (let i = chain.length - 1; i >= 0; i--) matrix = mul(matrix, PM.localMatrix(chain[i], T));
  };
  const appendGroups = (layer: any) => {
    for (const group of (PM.groupAncestors?.(layer) || []).slice().reverse()) {
      if (groups.has(group.id)) continue;
      groups.add(group.id);
      appendParents(group);
      matrix = mul(matrix, PM.localMatrix(group, T));
    }
  };
  appendGroups(L);
  appendParents(L, parent);
  return matrix;
};

PM.worldMatrix = (L: any, T: any) => {
  if (Object.is(memoT, T)) { const c = wmMemo.get(L); if (c) return c; }
  const chain: any[] = [];
  let cur = L, hit = false;
  while (cur && chain.length < 256) {
    if (chain.length > 8 && chain.includes(cur)) break;
    chain.push(cur);
    if (Object.is(memoT, T) && !L.group && !cur.group) { const c = wmMemo.get(cur); if (c) { hit = c; break; } }
    cur = parentOf(cur);
  }
  if (chain.some(layer => layer.group)) {
    const matrix = mul(PM.transformParentMatrix(L, T), PM.localMatrix(L, T));
    if (Object.is(memoT, T)) wmMemo.set(L, matrix);
    return matrix;
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

PM.worldOpacity = (L: any, T: any) => (PM.groupAncestors?.(L) || []).reduce((opacity: number, group: any) => opacity * clamp(PM.ev(group, 'opacity', T) / 100, 0, 1), clamp(PM.ev(L, 'opacity', T) / 100, 0, 1));

/* True when assigning parentId to L would create a parenting or group-space cycle. */
PM.wouldCycle = (L: any, parentId: any) => {
  if (!parentId) return false;
  if (parentId === L.id) return true;
  if ((PM.groupAncestors?.(L) || []).some((group: any) => group.id === parentId)) return true;
  let cur = PM.L(parentId), guard = 0;
  while (cur && guard++ < 256) {
    if (cur.id === L.id) return true;
    if (L.type === 'group' && (PM.groupAncestors?.(cur) || []).some((group: any) => group.id === L.id)) return true;
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
  temporalKeys(p.kf);
  PM.touch();
  return k;
};
PM.removeKey = (p: any, k: any) => {
  if (!k) return;
  const remaining = p.kf.filter((x: any) => x !== k && x.i !== k.i);
  // A single remaining key evaluates to its value everywhere. Keep that value
  // when its removal turns this channel back into a static property.
  if (p.kf.length && !remaining.length) p.v = p.kf[0].v;
  p.kf = remaining; PM.touch();
};
PM.hasKeyAt = (L: any, p: any, T: any) => p.kf.find((k: any) => Math.abs(k.t - (T - L.from)) < .5 / PM.proj.fps) || null;

PM.toggleStopwatch = (L: any, key: any, T: any) => {
  const p = L.p[key]; if (!p) return;
  if (p.kf.length) { p.v = PM.ev(L, key, T); p.kf = []; }
  else PM.setKeyOn(p, T - L.from, p.v, 'linear', PM.proj.fps);
  PM.touch(); PM.invalidate();
};

PM.applyEaseTo = (keys: any, name: any) => {
  const selected=new Set(keys);
  for(const layer of PM.ProjectIndex?.allLayers?.() || PM.proj?.layers || [])for(const {prop} of PM.allProps(layer))if(prop.kf.some((key:any)=>selected.has(key)))temporalKeys(prop.kf);
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
  for (const [key, prop] of Object.entries(L.d || {})) {
    if (canAnimateContent(L, key) && isProperty(prop)) out.push({ key: 'c.' + key, prop, label: contentLabel(key), group: 'Content' });
  }
  for (const key of ['blend', 'mblur', 'matteMode']) if (isProperty(L[key])) out.push({ key: 'l.' + key, prop: L[key], label: key === 'blend' ? 'Blend mode' : 'Motion blur', group: 'Layer' });
  (L.fx || []).forEach((fx: any) => { if (isProperty(fx.on)) out.push({ key: fx.id + '.$enabled', prop: fx.on, label: 'Enabled', group: fx.type }); for (const k in fx.p || {}) out.push({ key: fx.id + '.' + k, prop: fx.p[k], label: k, group: fx.type }); });
  (L.masks || []).forEach((m: any, i: any) => {
    for (const key of ['shape', 'mode', 'on']) if (isProperty(m[key])) out.push({ key: `m.${m.id}.${key}`, prop: m[key], label: key, group: 'Mask ' + (i + 1) });
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
  out.push(...structuredProperties(L));
  return out;
};
PM.findProp = (L: any, key: any) => {
  if (/^(g|mp|ta|ts)\./.test(key)) return structuredProperties(L).find(p=>p.key===key)?.prop ?? null;
  if (L.p?.[key]) return L.p[key];
  if (key === 'l.blend' || key === 'l.mblur' || key === 'l.matteMode') return isProperty(L[key.slice(2)]) ? L[key.slice(2)] : null;
  if (key.startsWith('c.')) {
    const value = L.d?.[key.slice(2)];
    return isProperty(value) ? value : null;
  }
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
    return m ? m.p[mk] ?? (isProperty(m[mk]) ? m[mk] : null) : null;
  }
  const [fid, pk] = key.split('.');
  const fx = (L.fx || []).find((f: any) => f.id === fid);
  return fx ? pk === '$enabled' ? (isProperty(fx.on) ? fx.on : null) : fx.p[pk] : null;
};
}
