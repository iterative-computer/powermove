import { createPathTiming, type PathTiming } from '../../core/anim/motion-path';
import { structuredProperties } from './vector-paths';
import { sceneProperties } from '../../core/scene3d/schema';
/* Ported from js/core/anim.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { canAnimateContent, contentLabel, isProperty, resolveContent } from './content-properties';
import { compileExpression } from './expression';
import { temporalKeys, temporalValue, temporalEvaluate } from './temporal-bridge';
import { applyUnitCurve, hasNativeEase } from '../../core/anim/temporal-ease';
import { derivedId } from './derived-id';

/** Composition settings no layer, keyframe or evaluation depends on. */
export const LAYER_FREE_SETTINGS: ReadonlySet<string> = new Set(['name', 'work', 'audioGain', 'bg', 'backgroundFill', 'shutter', 'colorDepth']);
/** Of those, the ones no rendered pixel depends on either. */
export const UNRENDERED_SETTINGS: ReadonlySet<string> = new Set(['name', 'work', 'audioGain']);
/** Whether every path (as writers or history give them: `['work']` or
 *  `['comps', id, 'work']`) is one of `settings` or the edit's provenance. */
export function settingsOnly(paths: unknown, settings: ReadonlySet<string>): boolean {
  if (!Array.isArray(paths) || !paths.length) return false;
  return paths.every(path => {
    if (!Array.isArray(path)) return false;
    if (path.length === 1) return path[0] === 'revision' || path[0] === 'edits' || settings.has(String(path[0]));
    return path.length === 3 && path[0] === 'comps' && settings.has(String(path[2]));
  });
}

export function install(PM: PMRegistry): void {
const Ease = PM.Ease, clamp = PM.clamp;
let defaultHandles: any = { eo: [.33, 0], ei: [.67, 1] };
try { defaultHandles = Ease.handles('power') || defaultHandles; } catch (e) { }

/* `version` advances on every document write (PM.animVersion: "did anything
   change"); `evaluation` only on writes that can change what layers and
   keyframes evaluate to. A composition setting (the work area, the name, the
   background) that no expression reads keeps every prepared track and hierarchy memo: re-preparing
   them walks every keyframe of the document. */
let version = 0;
let evaluation = 0;
let parentIndexes = new WeakMap<object, any>();
let staticTransforms = new WeakMap<object, { values: any[]; matrix: number[] }>();
/* An expression's `comp` is its composition object, every setting included
   (`comp.name`, `comp.work`): while one reads it, a setting is an edit like
   any other. Walked once per evaluation version and document. */
let compositionReaders: { project: unknown; evaluation: number; read: boolean } | null = null;
function expressionsReadComposition(): boolean {
  const project = PM.proj, cached = compositionReaders;
  if (cached && cached.project === project && cached.evaluation === evaluation) return cached.read;
  const seen = new Set<unknown>();
  const reads = (comp: any): boolean => {
    if (!comp || typeof comp !== 'object' || seen.has(comp)) return false;
    seen.add(comp);
    for (const layer of Array.isArray(comp.layers) ? comp.layers : []) {
      for (const { prop } of PM.allProps(layer)) if (typeof prop?.expr === 'string' && /\bcomp\b/.test(prop.expr)) return true;
    }
    return Object.values(comp.comps || {}).some(reads);
  };
  const read = reads(project);
  compositionReaders = { project, evaluation, read };
  return read;
}
PM.touch = (paths?: unknown) => {
  version++;
  const setting = settingsOnly(paths, LAYER_FREE_SETTINGS);
  if (setting && !expressionsReadComposition()) {
    if (!settingsOnly(paths, UNRENDERED_SETTINGS)) PM._scene3dRevision=(PM._scene3dRevision || 0)+1;
    return;
  }
  PM._scene3dRevision=(PM._scene3dRevision || 0)+1;
  evaluation++;
  // A setting writes no expression: the walk's answer stands.
  if (setting && compositionReaders) compositionReaders.evaluation = evaluation;
  PM.ProjectIndex?.invalidateKeyframes?.();
  /* hierarchy memos must never outlive an edit */
  woMemo.clear(); wmMemo.clear(); lmMemo.clear(); groupSpaceMemo.clear(); memoT = null;
  parentIndexes = new WeakMap();
  staticTransforms = new WeakMap();
};
PM.animVersion = () => version;
PM.evaluationVersion = () => evaluation;
/** settingsOnly, and no expression can read those settings. */
PM.settingsOnly = (paths: unknown, settings: ReadonlySet<string>) => settingsOnly(paths, settings) && !expressionsReadComposition();
let memoT: any = null;
const woMemo = new Map(), wmMemo = new Map(), lmMemo = new Map(), groupSpaceMemo = new Map();

/* ── keyframe evaluation ───────────────────────────────── */
function evalKfs(kf: any, t: any, interpolateColor = false) {
  const n = Array.isArray(kf) ? kf.length : 0;
  if (n === 0) return null;
  if (typeof t !== 'number' || !Number.isFinite(t)) return kf[0]?.v ?? null;
  if (typeof kf[0].v === 'number' && typeof kf[n-1].v === 'number') return temporalEvaluate(kf,t,evaluation,Ease.spring);
  if (t <= kf[0].t) return kf[0].v;
  if (t >= kf[n - 1].t) return kf[n - 1].v;
  let lo = 0, hi = n - 1;
  while (hi - lo > 1) { const m = (lo + hi) >> 1; kf[m].t <= t ? (lo = m) : (hi = m); }
  temporalKeys(kf,evaluation);
  const a = kf[lo], b = kf[hi];
  if (a.hold) return a.v;
  const hexColor = (v: any) => typeof v === 'string' && /^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i.test(v);
  const colors = interpolateColor && hexColor(a.v) && hexColor(b.v);
  // Text and toggles remain discrete, including text that looks like a hex color.
  if (!colors && (typeof a.v !== 'number' || !Number.isFinite(a.v) ||
      typeof b.v !== 'number' || !Number.isFinite(b.v))) return a.v;
  // A key without alpha digits is opaque; alpha is written only while it is below FF.
  const channel = (v: string, index: number) => index === 7 && v.length === 7 ? 255 : parseInt(v.slice(index, index + 2), 16);
  const mix = (amount: number) => {
    if (!colors) return a.v + (b.v - a.v) * amount;
    const bytes = [1, 3, 5, 7].map(index => {
      const start = channel(a.v, index), end = channel(b.v, index);
      return Math.round(clamp(start + (end - start) * amount, 0, 255));
    });
    if (bytes[3] === 255) bytes.pop();
    return '#' + bytes.map(byte => byte.toString(16).padStart(2, '0')).join('');
  };
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

/** Normalize persisted keyframes without discarding extension-owned fields.
 *  `stable`: a missing or repeated id is replaced deterministically (opening a document). */
PM.normalizeKeyframes = (raw: any, fallbackValue: any, _fps?: any, minTime = 0, options: { stable?: boolean } = {}) => {
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
  const valid = (Array.isArray(raw) ? raw : [])
    .map((key: any, index: number) => ({ key, index, time: Number(key?.t) }))
    .filter(({ key, time }: any) => key && typeof key === 'object' &&
      Number.isFinite(time) && time >= minTime && validValue(key.v))
    .sort((a: any, b: any) => a.time - b.time || a.index - b.index);
  /* A numeric track's keys are adopted below (temporalKeys): a key already
     in the native ease model gets `eo`/`ei`/`hold` as views of that model,
     so legacy copies made here would only be replaced (and the replacement
     leaves the JSON, which made every opened track look rewritten). Each key
     is one object literal, never edited with `delete`: that keeps it in
     V8's fast mode, sharing its shape with the other keys (a dictionary-mode
     key costs several times the memory, at a million keys per project). */
  const adopted = valid.length > 0 && valid.every(({ key }: any) => typeof key.v === 'number');
  const normalized = valid.map(({ key, index, time }: any) => {
    const t = time === 0 ? 0 : time;
    const i = (typeof key.i === 'string' && key.i) || (options.stable ? `k${index}` : PM.uid('k'));
    if (adopted && hasNativeEase(key)) {
      // Adoption makes these views of the native ease; no stored copy is kept.
      const { __order: _order, eo: _eo, ei: _ei, hold: _hold, ...fields } = key;
      return { ...fields, t, i };
    }
    const { __order: _order, ...fields } = key;
    return {
      ...fields, t, i,
      eo: pair(key.eo, pair(defaultHandles.eo, [.33, 0])),
      ei: pair(key.ei, pair(defaultHandles.ei, [.67, 1])),
      hold: key.hold === true,
    };
  });
  const collapsed: any[] = [];
  const ids = new Set<string>();
  normalized.forEach((clean: any) => {
    const previous = collapsed[collapsed.length - 1];
    if (ids.has(clean.i)) clean.i = options.stable ? derivedId(clean.i, ids) : PM.uid('k');
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
// Layer identity and source revision invalidate this non-serializable timing
// cache on edits, undo, and project reload; playback reuses it across axes.
const pathTimings = new WeakMap<object, Map<string, { version: number; timing: PathTiming }>>();
function motionPathValue(comp: any, layerId: string, channel: string, progress: number, start: number, end: number): number {
  if (!/^position\.[xyz]$/.test(channel) || ![progress, start, end].every(Number.isFinite) || !(end > start)) throw new Error('Invalid motion path request');
  const layer = comp.layers?.find((item: any) => item.id === layerId);
  if (!layer) throw new Error('Motion path layer is unavailable');
  const axes = ['position.x', 'position.y', 'position.z'].filter(axis => layer.p?.[axis]);
  const index = axes.indexOf(channel);
  if (index < 0) throw new Error('Motion path axis is unavailable');
  let cache = pathTimings.get(layer);
  if (!cache) { cache = new Map(); pathTimings.set(layer, cache); }
  const key = `${start}:${end}`;
  let entry = cache.get(key);
  if (!entry || entry.version !== evaluation) {
    const channels = axes.map(axis => layer.p[axis]);
    const sample = (time: number) => channels.map(prop => prop.kf?.length ? Number(evalKfs(prop.kf, time)) : Number(prop.v));
    const timing = createPathTiming(sample, channels.flatMap(prop => prop.kf.map((k: any) => k.t)), start, end);
    if (cache.size >= 16) cache.delete(cache.keys().next().value!);
    entry = { version: evaluation, timing }; cache.set(key, entry);
  }
  return entry.timing.position(progress)[index]!;
}

function compile(src: any) {
  let c = PM.exprCache.get(src);
  if (c !== undefined) return c;
  const parsed = compileExpression(src);
  c = parsed ? {
    needsIdx: parsed.needsIdx,
    f: (t: number, T: number, fps: number, value: unknown, layer: any, comp: any,
      param: (name: string) => unknown, ch: string, idx: number) => parsed.evaluate({
        t, T, fps, value, layer, comp, param, ch, idx,
        motionPath: (layerId, channel, progress, start, end) => motionPathValue(comp, layerId, channel, progress, start, end),
        sample: (layerId, channel, localTime) => {
          if (!Number.isFinite(localTime)) throw new Error('Sample time must be finite');
          const sourceLayer = (comp.layers as any[])?.find(item => item.id === layerId);
          const source = sourceLayer && PM.findProp(sourceLayer, channel);
          if (!source) throw new Error('Sampled channel is unavailable');
          return source.kf?.length ? evalKfs(source.kf, localTime) : source.v;
        },
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
  const start = Number(L?.from), duration = Number(L?.dur), time = Number(T);
  if (!Number.isFinite(start) || !Number.isFinite(duration) || duration <= 0 || !Number.isFinite(time)) return false;
  // Most layers in a long edit are outside the visible frame. Reject them
  // before walking group visibility or evaluating expression/keyframe switches.
  // Groups govern their children independently of the group's own timeline bar.
  if (L.type !== 'group') {
    if (time < start - 1e-6) return false;
    const end = start + duration;
    if (time >= end - 1e-6) {
      const compEnd = Number(PM.curComp()?.dur);
      if (!(Number.isFinite(compEnd) && Math.abs(time - compEnd) <= 1e-6 && Math.abs(end - compEnd) <= 1e-6)) return false;
    }
  }
  if (!(isProperty(L.on) ? PM.evP(L, L.on, T, 'l.on') : L.on)) return false;
  if (L.group && PM.groupAncestors?.(L).some((group: any) => !(isProperty(group.on) ? PM.evP(group, group.on, T, 'l.on') : group.on))) return false;
  return true;
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

const transformChannels = ['position.x', 'position.y', 'anchor.x', 'anchor.y', 'scale.x', 'scale.y', 'rotation', 'skew'];
PM.localMatrix = (L: any, T: any) => {
  if (Object.is(memoT, T)) { const cached = lmMemo.get(L); if (cached) return cached; }
  const previous = staticTransforms.get(L);
  let isStatic = true, unchanged = !!previous;
  // Static artwork does not need new trigonometry or matrix allocations every
  // frame. Check actual values as well as animation flags so direct inspector
  // writes and property replacement cannot reuse a stale transform.
  for (let i = 0; i < transformChannels.length; i++) {
    const prop = L.p[transformChannels[i]!];
    if (prop?.expr || prop?.kf?.length) { isStatic = false; break; }
    if (previous && !Object.is(previous.values[i], prop ? prop.v : 0)) unchanged = false;
  }
  if (isStatic && unchanged) {
    if (Object.is(memoT, T)) lmMemo.set(L, previous!.matrix);
    return previous!.matrix;
  }
  const px = PM.ev(L, 'position.x', T), py = PM.ev(L, 'position.y', T);
  const ax = PM.ev(L, 'anchor.x', T), ay = PM.ev(L, 'anchor.y', T);
  const sx = PM.ev(L, 'scale.x', T) / 100, sy = PM.ev(L, 'scale.y', T) / 100;
  const r = PM.ev(L, 'rotation', T) * Math.PI / 180;
  const sk = Math.tan(PM.ev(L, 'skew', T) * Math.PI / 180);
  const c = Math.cos(r), s = Math.sin(r);
  // Compose the affine transform directly, without three intermediate matrices per layer.
  const a=c*sx,b=s*sx,cc=(c*sk-s)*sy,d=(s*sk+c)*sy;
  const matrix = [a,b,cc,d,px-a*ax-cc*ay,py-b*ax-d*ay];
  if (isStatic) staticTransforms.set(L, { matrix, values: transformChannels.map(key => L.p[key] ? L.p[key].v : 0) });
  else if (previous) staticTransforms.delete(L);
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
  if (!Object.is(memoT, T)) { woMemo.clear(); wmMemo.clear(); lmMemo.clear(); groupSpaceMemo.clear(); }
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
  if (!L.parent && !L.group) {
    const matrix = PM.localMatrix(L, T);
    if (Object.is(memoT, T)) wmMemo.set(L, matrix);
    return matrix;
  }
  if (!L.parent && L.group && Object.is(memoT, T)) {
    const groups = PM.groupAncestors?.(L) || [];
    // Ordinary nested groups share one surrounding space across all siblings.
    // Parent rigs and incomplete/cyclic memberships retain the general path.
    if (groups.length && !groups[groups.length - 1].group && groups.every((group: any) => !group.parent)) {
      let space: number[] | undefined, start = groups.length - 1;
      for (let i = 0; i < groups.length; i++) {
        const cached = groupSpaceMemo.get(groups[i]);
        if (cached) { space = cached; start = i - 1; break; }
      }
      space ||= [1,0,0,1,0,0];
      for (let i = start; i >= 0; i--) {
        space = mul(space, PM.localMatrix(groups[i], T));
        groupSpaceMemo.set(groups[i], space);
      }
      const matrix = mul(space, PM.localMatrix(L, T));
      wmMemo.set(L, matrix);
      return matrix;
    }
  }
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
const setKeyOn = (p: any, tLocal: any, value: any, ease: any = 'linear', fps: any = 30, prepare = true) => {
  if (!p || typeof p !== 'object') return null;
  p.kf = Array.isArray(p.kf) ? p.kf : [];
  const safeFps = Number.isFinite(Number(fps)) && Number(fps) > 0 ? Number(fps) : 30;
  const t = Math.max(0, PM.snapF(Number.isFinite(Number(tLocal)) ? Number(tLocal) : 0, safeFps));
  let k = p.kf.find((k: any) => Math.abs(k.t - t) < .5 / safeFps);
  if (k) { k.v = value; }
  else { k = PM.KF(t, value, ease); p.kf.push(k); sortKf(p); }
  if (prepare) temporalKeys(p.kf);
  PM.touch();
  return k;
};
PM.setKeyOn = setKeyOn;
/** Convert presets only after every key has its final segment duration/value. */
PM.setKeysOn = (p: any, points: any[], fps: any = 30) => {
  const edits = new Map<any, any>();
  for (const point of points) {
    const key = setKeyOn(p, point.t, point.v, point.ease || 'linear', fps, false);
    if (key) edits.set(key, point);
  }
  temporalKeys(p.kf);
  p.kf.forEach((key: any, index: number) => {
    const point = edits.get(key);
    if (!point) return;
    if (point.hold != null) key.hold = !!point.hold;
    const { eo, ei } = Ease.handles(point.ease || 'linear');
    if (typeof key.v === 'number') applyUnitCurve(p.kf, index, [eo[0], eo[1], ei[0], ei[1]]);
    else { key.eo = [...eo]; key.ei = [...ei]; }
  });
  PM.touch();
};
PM.removeKey = (p: any, k: any) => {
  if (!k) return;
  const remaining = p.kf.filter((x: any) => x !== k && x.i !== k.i);
  // A single remaining key evaluates to its value everywhere. Keep that value
  // when its removal turns this channel back into a static property.
  if (p.kf.length && !remaining.length) p.v = p.kf[0].v;
  p.kf = remaining;
  // Legacy ease accessors resolve neighbours from the array they were built
  // for; rebind them so the surviving keys see their new neighbours.
  temporalKeys(p.kf);
  PM.touch();
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
  PM.setKeysOn(p, points.map((pt: any) => ({ ...pt, ease: pt.ease || opt.ease || 'linear' })), PM.proj.fps);
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
  out.push(...sceneProperties(L));
  return out;
};
PM.findProp = (L: any, key: any) => {
  if (/^(?:m\.[^.]+$|(?:o|light|camera|environment)\.)/.test(key)) return sceneProperties(L).find(p=>p.key===key)?.prop ?? null;
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
