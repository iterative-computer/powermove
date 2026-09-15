import { IMAGE_SEQUENCE_ACCEPT, sequenceCandidate } from '../../../shared/image-sequence';
import { chooseSequence, convertImageSequence } from './core/image-sequence';
import { canAnimateContent, isProperty } from './core/content-properties';
/* Ported from js/app.js — behavior-preserving. */
import { normalizeExportDefaults, type ExportDefaults } from '../core/export-defaults';
import { compactEditLog } from '../core/edit-log';
import type { PMRegistry } from './registry';
import { packProjectFile, restoreProjectFileMedia, unpackProjectFile } from './core/project-file';
import { projectFingerprint } from './core/project-fingerprint';
import { stringifyAsync } from './core/serialize-async';
import { createNewProjectForm } from './ui/project-settings';
import { inspectorService, shaderHooks, timelineService, viewerService } from './core/services';

export function install(PM: PMRegistry): void {
const h = PM.h;
const APP: any = { fileHandle: null, dirty: false, saveTimer: 0, importQueue: Promise.resolve() };
const document = window.document;
type FileState = { path?: string; savedHash?: string; baselineHash?: string; baselineReady?: Promise<void>; dirty: boolean; handle?: any };
const fileStates = new Map<string, FileState>();
const comparisonVersions = new Map<string, number>();
let activeSave: Promise<boolean> | null = null;
function fileState(id = PM.proj.id): FileState {
  let state = fileStates.get(id);
  if (!state) {
    const saved = PM.Projects.getState(id, { history: false })?.file;
    state = { path: saved?.path, savedHash: saved?.savedHash, baselineHash: saved?.baselineHash, dirty: true };
    fileStates.set(id, state);
    // A local project can be unchanged without ever having been saved to a
    // file. Keep its original baseline across recovery and tab switches.
    const project = id === PM.proj?.id ? PM.proj : PM.Projects.get(id);
    if (!state.savedHash && !state.baselineHash && project) {
      const initialState = state;
      initialState.baselineReady = projectFingerprint(JSON.stringify(project)).then(hash => {
        initialState.baselineHash = hash;
        rememberFile(id, initialState);
      });
    }
  }
  return state;
}
function fileUI() {
  APP.dirty = fileState().dirty;
  PM.invalidate('status'); PM.bus.emit('projects:tabs');
  document.title = `${APP.dirty ? '• ' : ''}${PM.proj.name} — Powermove`;
}
async function refreshFileDirty(project = PM.proj): Promise<boolean> {
  const state = fileState(project.id);
  const json = await stringifyAsync(project);
  const version = (comparisonVersions.get(project.id) || 0) + 1;
  comparisonVersions.set(project.id, version);
  await state.baselineReady;
  const hash = await projectFingerprint(json);
  // A newer edit wins over an in-flight comparison.
  if (await stringifyAsync(project) !== json) return true;
  const baseline = state.savedHash || state.baselineHash;
  const dirty = !baseline || baseline !== hash;
  // Concurrent comparisons of the same content are not edits. Only the
  // latest comparison may publish state, but both can answer Close correctly.
  if (comparisonVersions.get(project.id) !== version) return dirty;
  state.dirty = dirty;
  if (PM.proj.id === project.id) fileUI();
  return state.dirty;
}
function rememberFile(id: string, state: FileState) {
  PM.Projects.putState(id, { ...PM.Projects.getState(id, { history: false }), file: { path: state.path, savedHash: state.savedHash, baselineHash: state.baselineHash } });
}
PM.projectFileState = (id: string) => fileState(id);
let saveGeneration = 0;
PM.app = APP;

/* ── appearance (system default; light/dark are explicit overrides) ── */
PM.theme = (() => {
  const root = window.document.documentElement;
  const media = window.matchMedia?.('(prefers-color-scheme: dark)');
  let mode: any = 'system';
  const resolved = () => (mode === 'system' ? (media?.matches ? 'dark' : 'light') : mode);
  const syncNative = () => {
    try {
      (window as any).webkit?.messageHandlers?.pmTheme?.postMessage(mode);
    } catch (e) { }
  };
  const render = () => {
    if (resolved() === 'dark') root.dataset.theme = 'dark'; else delete root.dataset.theme;
    /* layout event re-resolves CSS-token caches (timeline canvas) */
    PM.bus.emit('layout');
    PM.invalidate();
  };
  const apply = (t: any) => {
    mode = ['light', 'dark', 'system'].includes(t) ? t : 'system';
    PM.store.set('themeMode', mode);
    PM.store.set('theme', mode);
    /* Legacy owns the preference and its persistence; the kernel owns the
       active theme's tokens. Push the resolved scheme across so a registered
       theme repaints its dark/light variant with the switch. */
    if (PM.Kernel?.theme) {
      PM.Kernel.theme.scheme = mode;
      PM.Kernel.events?.emit?.('theme:changed', { id: PM.Kernel.theme.activeId, scheme: resolved() });
    }
    syncNative();
    render();
  };
  /* Follow OS appearance changes live while in system mode. */
  media?.addEventListener?.('change', () => { if (mode === 'system') render(); });
  apply(PM.store.get('themeMode', 'system'));
  return {
    get current() { return root.dataset.theme === 'dark' ? 'dark' : 'light'; },
    get mode() { return mode; },
    apply,
    toggle() { apply(this.current === 'dark' ? 'light' : 'dark'); },
  };
})();

/* ── project boot / migration ───────────────────────────── */
let homeProjectId: string | null = null;
function emptyHomeProject() {
  const project = PM.mkProject();
  homeProjectId = project.id;
  return project;
}
function loadBootProject() {
  const raw = PM.Projects.pickBoot({ legacy: PM.store.get('autosave', null) });
  if (!raw) return emptyHomeProject();
  try { return hydrate(raw); }
  catch (e) { console.warn('Saved project could not be loaded', raw.id, e); return emptyHomeProject(); }
}
function hydrate(p: any) {
  const base = PM.mkProject({ name: p.name, w: p.w, h: p.h, fps: p.fps, dur: p.dur, bg: p.bg });
  Object.assign(base, p);
  base.edits = compactEditLog(Array.isArray(p.edits) ? p.edits : []);
  base.backgroundFill = PM.normalizeFill(p.backgroundFill, p.bg || '#000000');
  base.bg = base.backgroundFill.stops[0].color;
  base.layers = Array.isArray(p.layers) ? p.layers : [];
  base.assets = p.assets || {};
  base.markers = p.markers || [];
  base.work = p.work && p.work.length === 2 ? p.work : [0, base.dur];
  const num = (x: any, fb?: any) => { const n = Number(x); return Number.isFinite(n) ? n : fb; };
  const sanitizeParams = (params: any) => {
    if (!params || typeof params !== 'object' || Array.isArray(params)) return {};
    const out: any = {};
    Object.entries(params).forEach(([key, src]) => {
      if (!src || typeof src !== 'object' || Array.isArray(src)) return;
      const q: any = { ...src };
      q.name = typeof q.name === 'string' && q.name ? q.name : key;
      q.label = typeof q.label === 'string' && q.label ? q.label : q.name;
      q.control = ['num', 'color', 'toggle', 'select'].includes(q.control) ? q.control : 'num';
      if (q.control === 'num') {
        q.min = num(q.min, 0); q.max = num(q.max, Math.max(q.min, 1));
        if (q.max < q.min) [q.min, q.max] = [q.max, q.min];
        q.value = PM.clamp(num(q.value, q.min), q.min, q.max);
      } else if (q.control === 'toggle') q.value = !!q.value;
      else if (q.control === 'color') q.value = typeof q.value === 'string' && /^#[0-9a-f]{6}$/i.test(q.value) ? q.value : '#FF6B1A';
      else {
        q.options = Array.isArray(q.options) ? q.options.filter((o: any) => o && typeof o === 'object' && 'v' in o) : [];
        if (q.value === undefined) q.value = q.options.length ? q.options[0].v : '';
      }
      out[key] = q;
    });
    return out;
  };
  base.params = sanitizeParams(p.params);
  base.shutter = p.shutter == null ? .5 : p.shutter;
  base.exportDefaults = normalizeExportDefaults(p.exportDefaults, base.fps);
  /* Production rule: a saved project must never poison the renderer. Every channel,
     keyframe, effect and layer field is normalized here so malformed data degrades
     to a static value instead of NaN transforms or a broken keyframe search.
     Applied recursively to nested compositions as well. */
  const keyIds = new Set<string>();
  const sanitizeProp = (prop: any, fresh: any, minTime = 0) => {
    const fallback = fresh?.v;
    const valid = (value: any) => {
      if (typeof fallback === 'number') return typeof value === 'number' && Number.isFinite(value);
      if (typeof fallback === 'string') return typeof value === 'string';
      if (typeof fallback === 'boolean') return typeof value === 'boolean';
      return value !== undefined && value !== null;
    };
    prop.v = valid(prop.v) ? prop.v : fallback;
    prop.kf = PM.normalizeKeyframes(prop.kf, fallback, base.fps, minTime);
    prop.kf.forEach((key: any) => {
      if (keyIds.has(key.i)) key.i = PM.uid('k');
      keyIds.add(key.i);
    });
    prop.expr = typeof prop.expr === 'string' && prop.expr.trim() ? prop.expr : null;
  };
  const sanitizeLooseParams = (params: any, minTime = 0) => {
    const source = params && typeof params === 'object' && !Array.isArray(params) ? params : {};
    Object.keys(source).forEach((key: any) => {
      const saved = source[key];
      const prop = saved && typeof saved === 'object' && !Array.isArray(saved) ? saved : { v: saved };
      const fallback = (typeof prop.v === 'number' && Number.isFinite(prop.v)) ||
        typeof prop.v === 'string' || typeof prop.v === 'boolean' ? prop.v : 0;
      sanitizeProp(prop, { v: fallback }, minTime);
      source[key] = prop;
    });
    return source;
  };
  const sanitizeJson = (value: any, seen = new WeakSet<object>(), depth = 0): any => {
    if (value == null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'number') return Number.isFinite(value) ? value : null;
    if (typeof value !== 'object' || depth > 32 || seen.has(value)) return null;
    seen.add(value);
    if (Array.isArray(value)) return value.slice(0, 10_000).map((item) => sanitizeJson(item, seen, depth + 1));
    const out: any = {};
    for (const [key, item] of Object.entries(value).slice(0, 10_000)) {
      if (key === '__proto__' || key === 'prototype' || key === 'constructor') continue;
      out[key] = sanitizeJson(item, seen, depth + 1);
    }
    return out;
  };
  const sanitizeTransition = (value: any, minTime = 0) => {
    if (value == null) return null;
    if (!value || typeof value !== 'object' || Array.isArray(value) || typeof value.type !== 'string' || !value.type) return null;
    const duration = PM.clamp(num(value.dur, 0.5), 0.02, 600);
    const savedParams = value.p && typeof value.p === 'object' && !Array.isArray(value.p) ? value.p : {};
    const definition = PM.transitionDef?.(value.type);
    if (!definition) return { ...value, type: value.type, dur: duration, p: sanitizeLooseParams(savedParams, minTime), missing: true };
    const transition = PM.mkTransition(value.type);
    if (!transition) return { ...value, type: value.type, dur: duration, p: savedParams, missing: true };
    transition.dur = duration;
    definition.params.forEach((param: any) => {
      const fallback = param.def;
      const saved = savedParams[param.k];
      const source = saved && typeof saved === 'object' && !Array.isArray(saved) ? saved : { v: saved };
      const validValue = (raw: any) => {
        if (param.type === 'color') return typeof raw === 'string' && /^#[0-9a-f]{6}$/i.test(raw) ? raw : fallback;
        if (param.type === 'toggle') return typeof raw === 'boolean' ? raw : fallback;
        return num(raw, fallback);
      };
      const prop = { ...source, v: validValue(source.v) };
      prop.kf = (Array.isArray(source.kf) ? source.kf : []).map((key: any) => ({ ...key, v: validValue(key?.v) }));
      sanitizeProp(prop, { v: fallback }, minTime);
      transition.p[param.k] = prop;
    });
    return transition;
  };
  const sanitizeLayers = (layers: any, container: any) => {
    layers.forEach((L: any, li: any) => {
      L.id = typeof L.id === 'string' && L.id ? L.id : PM.uid('L');
      if (!L.name || typeof L.name !== 'string') L.name = 'Layer ' + (li + 1);
      L.from = L.type === 'group' ? num(L.from, 0) : Math.max(0, num(L.from, 0));
      L.dur = Math.max(.01, num(L.dur, 5));
      if (L.type != null && !PM.TYPE_META[L.type]) L.type = 'null';
      L.on = isProperty(L.on) ? L.on : L.on !== false; L.lock = !!L.lock; L.solo = !!L.solo; L.shy = !!L.shy;
      L.collapsed = L.collapsed !== false; L.fx = Array.isArray(L.fx) ? L.fx : []; L.p = L.p && typeof L.p === 'object' ? L.p : {}; L.d = L.d && typeof L.d === 'object' ? L.d : {};
      L.locked_intent = L.locked_intent || {}; L.blend = L.blend || 'normal'; L.mblur = isProperty(L.mblur) ? L.mblur : !!L.mblur;
      if (L.type === 'audio') PM.Audio.normalizeLayer(L);
      if (L.parent === L.id || (typeof L.parent === 'string' && !container.layers.some((o: any) => o.id === L.parent))) L.parent = null;
      const fresh = PM.mkLayer(L.type || 'null', {}, container);
      if (L.type === 'text') for (const key of ['boxWidth', 'boxHeight']) if (!isProperty(L.d[key])) L.d[key] = fresh.d[key];
      for (const [key, value] of Object.entries(L.d)) if (canAnimateContent(L, key) && isProperty(value)) sanitizeProp(value, { v: isProperty(fresh.d[key]) ? fresh.d[key].v : fresh.d[key] ?? (value as any).v }, -L.from);
      for (const key of ['blend', 'mblur', 'on']) if (isProperty(L[key])) sanitizeProp(L[key], { v: key === 'blend' ? 'normal' : key === 'on' }, -L.from);
      Object.keys(fresh.p).forEach(k => {
        if (!L.p[k] || typeof L.p[k] !== 'object') L.p[k] = fresh.p[k];
        sanitizeProp(L.p[k], fresh.p[k], -L.from);
      });
      Object.keys(L.p).forEach(k => { if (!(k in fresh.p)) delete L.p[k]; });
      /* An effect whose type is not registered right now is kept as a marked
         placeholder rather than deleted: the extension that provides it may
         load later (or be re-enabled), and silently dropping the effect would
         lose the user's keyframes on save. The compositor skips `missing`. */
      L.fx = L.fx
        .filter((f: any) => f && typeof f === 'object' && typeof f.type === 'string')
        .map((f: any) => (PM.FX && PM.FX[f.type] ? (f.missing ? (({ missing, ...rest }: any) => rest)(f) : f) : { ...f, missing: true }));
      L.fx.forEach((f: any) => {
        f.id = f.id || PM.uid('fx'); f.p = sanitizeLooseParams(f.p, -L.from); f.on = isProperty(f.on) ? f.on : f.on !== false;
        if (isProperty(f.on)) sanitizeProp(f.on, { v: true }, -L.from);
        const definition = PM.FX?.[f.type];
        for (const param of definition?.params || []) {
          if (!f.p[param.k] || typeof f.p[param.k] !== 'object') f.p[param.k] = PM.P(param.def);
          sanitizeProp(f.p[param.k], { v: param.def }, -L.from);
        }
      });
      L.transitionIn = sanitizeTransition(L.transitionIn, -L.from);
      L.transitionOut = sanitizeTransition(L.transitionOut, -L.from);
      if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].effects === false) L.fx = [];
      /* masks: validate shape/mode and every animatable channel */
      L.masks = Array.isArray(L.masks) ? L.masks.filter((m: any) => m && typeof m === 'object' && m.p && typeof m.p === 'object') : [];
      L.masks.forEach((m: any) => {
        m.id = typeof m.id === 'string' && m.id ? m.id : PM.uid('K');
        if (isProperty(m.shape)) sanitizeProp(m.shape, { v: 'rect' }, -L.from);
        else if (!PM.MASK_SHAPES.includes(m.shape)) m.shape = 'rect';
        if (isProperty(m.mode)) sanitizeProp(m.mode, { v: 'add' }, -L.from);
        else if (m.mode !== 'subtract') m.mode = 'add';
        m.on = isProperty(m.on) ? m.on : m.on !== false;
        if (isProperty(m.on)) sanitizeProp(m.on, { v: true }, -L.from);
        const freshM = PM.mkMask(m.shape, container);
        Object.keys(freshM.p).forEach(k => {
          if (!m.p[k] || typeof m.p[k] !== 'object') m.p[k] = freshM.p[k];
          sanitizeProp(m.p[k], freshM.p[k], -L.from);
        });
        Object.keys(m.p).forEach(k => { if (!(k in freshM.p)) delete m.p[k]; });
      });
      if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].masks === false) L.masks = [];
      if (L.type === 'shader') {
        L.d.uniforms = sanitizeLooseParams(L.d.uniforms, -L.from);
        shaderHooks(PM)?.syncShaderUniforms(L);
      }
      if (L.type === 'extension') {
        L.d.definition = typeof L.d.definition === 'string' ? L.d.definition : '';
        L.d.version = Math.max(1, Math.round(num(L.d.version, 1)));
        if (!isProperty(L.d.w)) L.d.w = PM.clamp(num(L.d.w, container.w), 1, 16384);
        if (!isProperty(L.d.h)) L.d.h = PM.clamp(num(L.d.h, container.h), 1, 16384);
        L.d.params = sanitizeLooseParams(L.d.params, -L.from);
        L.d.data = sanitizeJson(L.d.data) || {};
      }
    });
  };
  base.comps = p.comps && typeof p.comps === 'object' ? p.comps : {};
  (Object.entries(base.comps) as any).forEach(([cid, c]: any) => {
    if (!c || typeof c !== 'object') { delete base.comps[cid]; return; }
    c.id = cid;
    c.name = typeof c.name === 'string' && c.name ? c.name : 'Precomp';
    c.w = Math.max(2, num(c.w, base.w)); c.h = Math.max(2, num(c.h, base.h));
    c.fps = Math.max(1, num(c.fps, base.fps)); c.dur = Math.max(.04, num(c.dur, base.dur));
    c.bg = typeof c.bg === 'string' ? c.bg : '#000000';
    c.params = sanitizeParams(c.params);
    c.layers = Array.isArray(c.layers) ? c.layers : [];
    c.edits = compactEditLog(Array.isArray(c.edits) ? c.edits : []);
    sanitizeLayers(c.layers, c);
  });
  sanitizeLayers(base.layers, base);
  /* precomp layers whose referenced comp failed to load degrade to empty layers */
  base.layers.forEach((L: any) => { if (L.type === 'precomp' && !(L.d && L.d.comp && base.comps[L.d.comp])) L.d.comp = null; });
  return base;
}
/* Shared project boundary for import/open flows and deterministic regression tests. */
PM.hydrateProject = hydrate;
/* Boot hygiene: repeated launches can leave several untouched "Untitled" projects
   in the registry and the tab strip. Keep at most one, and none at all while real
   projects exist. */
(function pruneEmptyUntitled() {
  const metas = PM.Projects.list();
  const empties = metas.filter((m: any) => m.name === 'Untitled');
  if (!empties.length) return;
  const hasReal = metas.some((m: any) => {
    if (m.name !== 'Untitled') return true;
    const raw = PM.Projects.get(m.id);
    return !!(raw && Array.isArray(raw.layers) && raw.layers.length);
  });
  empties.forEach((m: any, i: any) => {
    if (hasReal || i > 0) {
      const raw = PM.Projects.get(m.id);
      if (!raw || (Array.isArray(raw.layers) && raw.layers.length === 0)) PM.Projects.remove(m.id);
    }
  });
  /* A registry card without a slot is never recoverable from the registry and
     otherwise lingers as a project that cannot open (for example, an interrupted
     duplicate). Project files exported by the user are unaffected. */
  PM.Projects.list().forEach((m: any) => { if (!PM.Projects.get(m.id)) PM.Projects.remove(m.id); });
})();

async function restoreProjectAssets(project: any, warn: any = true) {
  const result = await PM.assets.restoreProject(project);
  if (result.stale || PM.proj !== project) return result;
  PM.bus.emit('assets');
  inspectorService(PM)?.refresh();
  PM.invalidate('all');
  if (warn && result.missing.length) {
    const count = result.missing.length;
    PM.toast(count === 1
      ? 'One media file is missing · import it again to relink it'
      : `${count} media files are missing · import them again to relink them`, 5000);
  }
  return result;
}

PM.proj = loadBootProject();

/* Extension boot happens after project hydration. Revalidate placeholders on
   registry changes so a saved effect/transition becomes live as soon as its
   provider activates, and becomes safely disabled again if that provider is
   turned off. Saved parameters and keyframes remain on the same objects. */
function revalidateContributionPlaceholders() {
  let changed = false;
  const containers = [PM.proj, ...Object.values(PM.proj?.comps || {})] as any[];
  for (const container of containers) {
    for (const layer of container?.layers || []) {
      for (const effect of layer.fx || []) {
        if (!effect || typeof effect.type !== 'string') continue;
        const missing = !PM.FX?.[effect.type];
        if (missing && !effect.missing) { effect.missing = true; changed = true; }
        else if (!missing && effect.missing) { delete effect.missing; changed = true; }
        if (!missing) {
          effect.p = effect.p && typeof effect.p === 'object' ? effect.p : {};
          for (const param of PM.FX[effect.type].params || []) {
            const prop = effect.p[param.k];
            if (!prop || typeof prop !== 'object') {
              effect.p[param.k] = PM.P(param.def);
              changed = true;
            } else {
              prop.kf = PM.normalizeKeyframes(prop.kf, param.def, PM.proj.fps, -layer.from);
              if (typeof prop.expr !== 'string') prop.expr = null;
              const sameType = typeof prop.v === typeof param.def;
              if (!sameType || (typeof prop.v === 'number' && !Number.isFinite(prop.v))) prop.v = param.def;
            }
          }
        }
      }
      for (const edge of ['transitionIn', 'transitionOut']) {
        const transition = layer[edge];
        if (!transition || typeof transition.type !== 'string') continue;
        const missing = !PM.transitionDef?.(transition.type);
        if (missing && !transition.missing) { transition.missing = true; changed = true; }
        else if (!missing && transition.missing) { delete transition.missing; changed = true; }
        if (!missing) {
          transition.p = transition.p && typeof transition.p === 'object' ? transition.p : {};
          for (const param of PM.transitionDef(transition.type).params || []) {
            const prop = transition.p[param.k];
            if (!prop || typeof prop !== 'object') {
              transition.p[param.k] = PM.P(param.def);
              changed = true;
            } else {
              prop.kf = PM.normalizeKeyframes(prop.kf, param.def, PM.proj.fps, -layer.from);
              if (typeof prop.expr !== 'string') prop.expr = null;
              const sameType = typeof prop.v === typeof param.def;
              if (!sameType || (typeof prop.v === 'number' && !Number.isFinite(prop.v))) prop.v = param.def;
            }
          }
        }
      }
    }
  }
  if (changed) {
    inspectorService(PM)?.refresh();
    PM.invalidate();
  }
}
PM.Kernel?.effects?.onChange?.(revalidateContributionPlaceholders);
PM.Kernel?.transitions?.onChange?.(revalidateContributionPlaceholders);

PM.WS.init();
const bootSession = PM.Projects.getState(PM.proj.id);
const bootWorkspace = PM.store.get(`projectWorkspace.${PM.proj.id}`, null) || bootSession?.workspace;
if (bootWorkspace) PM.WS.restoreSnapshot(bootWorkspace);
PM.selectLayers((bootSession?.selection?.layers || []).filter((id: any) => PM.L(id)).length
  ? bootSession.selection.layers.filter((id: any) => PM.L(id))
  : PM.proj.layers.find((l: any) => l.name === 'Powermove')?.id || []);
PM.sel.keys = [...new Set((bootSession?.selection?.keys || []).filter((key: any) => typeof key === 'string'))];
PM.sel.keys = PM.resolveSelectedKeys().map((key: any) => key.i);
PM.setTime(Number.isFinite(bootSession?.time) ? bootSession.time : .9, { raw: true, force: true });
const bootTimeline = timelineService(PM);
if (bootSession?.timeline && bootTimeline) {
  bootTimeline.pps = Number.isFinite(bootSession.timeline.pps) ? bootSession.timeline.pps : bootTimeline.pps;
  bootTimeline.scrollT = Number.isFinite(bootSession.timeline.scrollT) ? bootSession.timeline.scrollT : bootTimeline.scrollT;
  bootTimeline.scrollY = Number.isFinite(bootSession.timeline.scrollY) ? bootSession.timeline.scrollY : bootTimeline.scrollY;
  bootTimeline.graph = !!bootSession.timeline.graph;
}
PM.hist.import?.(bootSession?.history);
restoreProjectAssets(PM.proj);

/* ── shell ─────────────────────────────────────────────── */
/** Export defaults live on the project, so every write is an ordinary edit. */
function readExportDefaults(): ExportDefaults {
  return normalizeExportDefaults(PM.proj?.exportDefaults, PM.proj?.fps);
}
function writeExportDefaults(patch: Partial<ExportDefaults>) {
  if (!PM.proj) return;
  const next = normalizeExportDefaults({ ...readExportDefaults(), ...patch }, PM.proj.fps);
  PM.hist.do('Export settings', () => { PM.proj.exportDefaults = next; });
}
PM.exportDefaults = { read: readExportDefaults, write: writeExportDefaults };


/* ── persistence ───────────────────────────────────────── */
/* Save the document immediately; thumbnail GPU readback waits for navigation
   to settle. A five-second rate limit alone still allowed mid-gesture stalls. */
let lastThumbAt = 0;
let thumbRetry = 0;
const thumbnailBlocked = () => PM.interactionActive?.() || viewerService(PM)?.isNavigating?.() || PM.playing
  || PM.agentFrameCapture || PM.Export?.busy || PM.Preview?.preparing || PM.Preview?.active;
function retryThumbnail(project: any) {
  window.clearTimeout(thumbRetry);
  thumbRetry = window.setTimeout(() => {
    thumbRetry = 0;
    // Retry only the preview. The document was already saved; rewriting it
    // every half second would keep serialization work running during playback.
    if (PM.proj === project) projectThumb();
  }, 500);
}
function canFinishThumbnail(project: any) {
  if (PM.proj !== project) return false;
  if (!thumbnailBlocked()) return true;
  lastThumbAt = 0;
  retryThumbnail(project);
  return false;
}
function projectThumb() {
  const now = Date.now();
  if (now - lastThumbAt < 5000) return undefined;
  if (thumbnailBlocked()) {
    retryThumbnail(PM.proj);
    return undefined;
  }
  window.clearTimeout(thumbRetry); thumbRetry = 0;
  lastThumbAt = now;
  // Never render a second composition or synchronously readPixels for recovery.
  // Capture the already-presented canvas asynchronously; metadata can arrive later.
  const canvas = PM.GL?.canvas, project = PM.proj;
  if (canvas?.toBlob && typeof createImageBitmap === 'function') {
    canvas.toBlob((blob: Blob | null) => {
      if (!blob || !canFinishThumbnail(project)) return;
      void createImageBitmap(blob, { resizeWidth: 320, resizeHeight: Math.max(1, Math.round(320 * canvas.height / canvas.width)) }).then(bitmap => {
        try {
          // Playback may have started while readback or bitmap resizing ran.
          if (!canFinishThumbnail(project)) return;
          const thumbnail = document.createElement('canvas'); thumbnail.width = bitmap.width; thumbnail.height = bitmap.height;
          thumbnail.getContext('2d')!.drawImage(bitmap, 0, 0);
          PM.Projects.upsertMeta({ id: project.id, name: project.name, at: Date.now(), thumb: thumbnail.toDataURL('image/jpeg', .7) });
        } finally { bitmap.close(); }
      }).catch(() => undefined);
    }, 'image/jpeg', .7);
  }
  return undefined;
}
function persistCurrent(withThumb: any) {
  if (PM.proj.id === homeProjectId) return true;
  try {
    const thumb = withThumb ? projectThumb() : undefined;
    if (withThumb && PM.Projects.recover) PM.Projects.recover(PM.proj, thumb);
    else PM.Projects.put(PM.proj, thumb);
    return true;
  } catch (e) { APP.dirty = true; console.warn('Project save failed', e); return false; }
}

function captureProjectSession() {
  if (!PM.proj?.id || PM.proj.id === homeProjectId) return true;
  PM.bus.emit('project:flush-edits');
  const saved = persistCurrent(false);
  const timeline = timelineService(PM);
  PM.Projects.putState(PM.proj.id, {
    ...PM.Projects.getState(PM.proj.id, { history: false }),
    lastActiveAt: Date.now(),
    history: PM.hist.export?.({ copy: false }),
    workspace: PM.WS.snapshot(), time: PM.time,
    selection: { layers: [...PM.sel.layers], keys: PM.sel.keys.filter((key: any) => typeof key === 'string'), chan: PM.sel.chan },
    timeline: timeline
      ? { pps: timeline.pps, scrollT: timeline.scrollT, scrollY: timeline.scrollY, graph: timeline.graph }
      : undefined,
  });
  return saved;
}

// Native close/quit calls this while the document and its IPC channel are
// still alive. beforeunload alone runs after the main process's quit flush.
PM.flushProject = async () => {
  await APP.importQueue;
  PM.AgentUI?.flushThreads?.();
  window.clearTimeout(APP.saveTimer);
  if (!captureProjectSession()) throw new Error('Project storage is unavailable or full');
  await PM.store.flush?.();
};

function closeProjectTransients() {
  PM.LibraryUI?.close?.();
  PM.SpatialAssistant?.cancel?.();
  PM.closeMenus?.();
  if (PM.WS.editing) PM.WS.cancelEdit();
}
PM.autosave = () => {
  if (PM.proj.id === homeProjectId) return;
  APP.dirty = true;
  fileState().dirty = true;
  comparisonVersions.set(PM.proj.id, (comparisonVersions.get(PM.proj.id) || 0) + 1);
  fileUI();
  const generation = ++saveGeneration, projectId = PM.proj.id;
  window.clearTimeout(APP.saveTimer);
  APP.saveTimer = window.setTimeout(async () => {
    try {
      if (!persistCurrent(true)) throw new Error('Project storage is unavailable or full');
      PM.Projects.putState(PM.proj.id, { ...PM.Projects.getState(PM.proj.id, { history: false }), history: PM.hist.export?.({ copy: false }) });
      await PM.store.flush?.();
      if (generation !== saveGeneration || projectId !== PM.proj.id) return;
      // Local recovery is not the user's .pmv file and must never mark it saved.
      await refreshFileDirty();
      PM.bus.emit('project:recovered');
    } catch (error) {
      APP.dirty = true;
      PM.toast('Could not save this project. Your edits are still open; try Save again.', 6000);
      console.warn('Project save failed', error);
    }
    PM.invalidate('status'); PM.bus.emit('projects:tabs');
  }, 550);
};
PM.bus.on('storage:error', () => { APP.dirty = true; PM.invalidate('status'); });
['layers','project','assets','library'].forEach(ev => PM.bus.on(ev, PM.autosave));

async function saveProject({ saveAs = false, projectId = PM.proj.id }: any = {}): Promise<boolean> {
  APP.saving = true; PM.invalidate('status');
  try {
    (window.document.activeElement as HTMLElement | null)?.blur?.();
    PM.bus.emit('project:flush-edits');
    await APP.importQueue;
    const project = projectId === PM.proj.id ? PM.proj : PM.Projects.get(projectId);
    if (!project) throw new Error('This project is no longer available.');
    let projectJSON: string, snapshot: any, serialized: string;
    // Edits can arrive between serialization slices. Retry a changed snapshot
    // instead of writing a mixture of two document revisions to disk.
    for (;;) {
      const generation = comparisonVersions.get(projectId) || 0;
      const editVersion = PM.animVersion?.();
      const current = projectId === PM.proj.id ? PM.proj : PM.Projects.get(projectId) || project;
      snapshot = { v: PM.version, proj: current,
        history: projectId === PM.proj.id ? PM.hist.export?.({ copy: false }) : PM.Projects.getState(projectId)?.history,
        ws: projectId === PM.proj.id ? PM.WS.snapshot() : PM.Projects.getState(projectId)?.workspace };
      projectJSON = await stringifyAsync(current);
      const historyJSON = await stringifyAsync(snapshot.history ?? null);
      serialized = `{"v":${JSON.stringify(PM.version ?? null)},"proj":${projectJSON},"history":${historyJSON},"ws":${JSON.stringify(snapshot.ws ?? null)}}`;
      if (editVersion === PM.animVersion?.() && generation === (comparisonVersions.get(projectId) || 0) && (projectId !== PM.proj.id || current === PM.proj)) break;
    }
    const state = fileState(projectId);
    const suggestedName = safeName(project.name) + '.pmv';
    const data = await packProjectFile(snapshot, PM.MediaStore, serialized);
    const finish = async (path?: string) => {
      state.path = path || state.path;
      state.savedHash = await projectFingerprint(projectJSON);
      rememberFile(projectId, state);
      await refreshFileDirty(projectId === PM.proj.id ? PM.proj : PM.Projects.get(projectId) || project);
      if (projectId === PM.proj.id) captureProjectSession();
      await PM.store.flush?.();
      PM.bus.emit('project:saved');
      PM.toast('Saved ' + (state.path?.split(/[\\/]/).pop() || suggestedName));
      return true;
    };
    if (typeof window.powermove?.saveFile === 'function') {
      const metadata = { name: suggestedName, projectId, saveAs };
      const upload = window.powermove.fileUpload;
      const result = await (async () => {
        if (!upload || data.length <= 4 * 1024 * 1024) return window.powermove!.saveFile({ ...metadata, data });
        const token = await upload.begin(data.length);
        try {
          for (let offset = 0; offset < data.length; offset += 1024 * 1024) {
            // A subarray still owns the complete backing buffer. Electron's
            // context bridge would clone the entire project for every slice.
            await upload.chunk(token, data.slice(offset, offset + 1024 * 1024));
          }
          return await upload.finish(token, metadata);
        } finally { await upload.abort(token).catch(() => undefined); }
      })();
      if (result.ok) return await finish(result.path);
      if (!result.cancelled) throw new Error(result.error || 'Save failed');
      return false;
    }
    let handle = saveAs ? null : state.handle;
    if (!handle && (window as any).showSaveFilePicker) {
      handle = await (window as any).showSaveFilePicker({ suggestedName, types: [{ description: 'Powermove Project', accept: { 'application/json': ['.pmv'] } }] });
    }
    if (handle?.createWritable) {
      const w = await handle.createWritable(); await w.write(data); await w.close();
      state.handle = handle;
      return await finish(handle.name);
    }
    PM.download(new window.Blob([new Uint8Array(data)], { type: 'application/x-powermove' }), safeName(PM.proj.name) + '.pmv');
    PM.toast('Download started'); return false;
  } catch (error: any) {
    if (error.name === 'AbortError') return false;
    PM.toast('Could not save project: ' + (error.message || 'Save failed'), 6000); return false;
  }
}
PM.saveProject = (options: any = {}) => {
  if (PM.proj.id === homeProjectId) return Promise.resolve(false);
  if (activeSave) return Promise.resolve(false);
  const save = saveProject(options);
  activeSave = save;
  void save.finally(() => {
    if (activeSave === save) {
      activeSave = null;
      APP.saving = false;
    }
  });
  return save;
};
PM.openProject = async () => {
  if (window.powermove?.openProjectFile) {
    const result = await window.powermove.openProjectFile();
    if (result.ok) await openProjectFile({ name: result.path.split(/[\\/]/).pop(), arrayBuffer: async () => result.data.buffer.slice(result.data.byteOffset, result.data.byteOffset + result.data.byteLength) }, result);
    else if (!result.cancelled) PM.toast('Could not open project: ' + result.error, 6000);
    return;
  }
  const inp = h('input', { type: 'file', accept: '.pmv,.json,application/json' });
  inp.onchange = async () => { const f = inp.files[0]; if (f) await openProjectFile(f); };
  inp.click();
};
async function openProjectFile(file: any, association?: { path: string; projectId: string }) {
  try {
    const input = file.arrayBuffer ? await file.arrayBuffer() : await file.text();
    const o = unpackProjectFile(input);
    const source = o.proj || o;
    if (!source || !Array.isArray(source.layers) || !Number.isFinite(source.w) || !Number.isFinite(source.h)) throw new Error('This is not a Powermove project.');
    source.id = association?.projectId || PM.uid('project');
    const project = hydrate(source);
    await restoreProjectFileMedia(o, PM.MediaStore);
    // File opens receive a fresh document ID to keep independent copies safe.
    // Recover the most recent layout for this file, rather than reverting to
    // the older workspace embedded the last time its content was saved.
    const filePath = association?.path;
    const rememberedWorkspace = filePath && fileState().path === filePath
      ? PM.WS.snapshot()
      : filePath ? PM.Projects.list()
        .map((item: any) => ({ ...PM.Projects.getState(item.id, { history: false }), workspace: PM.store.get(`projectWorkspace.${item.id}`, null) || PM.Projects.getState(item.id, { history: false })?.workspace }))
        .filter((session: any) => session?.file?.path === filePath && session?.workspace?.layout?.docks)
        .sort((a: any, b: any) => (b.lastActiveAt || 0) - (a.lastActiveAt || 0))[0]?.workspace
        : undefined;
    // Native file opens intentionally get a fresh document identity. Carry the
    // latest local conversation archive forward only for this exact file path.
    // Flush first so opening the active file also retains a just-typed draft.
    PM.AgentUI?.flushThreads?.();
    const previousProjectId = filePath && fileState().path === filePath
      ? PM.proj.id
      : filePath ? PM.Projects.list()
        .map((item: any) => ({ id: item.id, session: PM.Projects.getState(item.id, { history: false }) }))
        .filter((item: any) => item.session?.file?.path === filePath)
        .sort((a: any, b: any) => (b.session.lastActiveAt || 0) - (a.session.lastActiveAt || 0))[0]?.id
        : undefined;
    const archive = previousProjectId && PM.store.get(`agentThreads.${previousProjectId}`, null);
    if (archive && PM.store.set(`agentThreads.${project.id}`, archive) === false) {
      throw new Error('Could not restore agent threads: project storage is unavailable or full');
    }
    switchProject(project, o.history || { version: 1, index: -1, entries: [] });
    const state = fileState(project.id);
    state.path = association?.path;
    state.savedHash = association ? await projectFingerprint(JSON.stringify(PM.proj)) : undefined;
    rememberFile(project.id, state);
    await refreshFileDirty();
    const workspace = rememberedWorkspace || o.ws;
    if (workspace?.layout?.docks) PM.WS.restoreSnapshot(workspace);
    captureProjectSession();
    PM.toast('Opened ' + file.name);
  } catch (e: any) { PM.toast('Could not open project: ' + e.message, 4500); }
}
PM.newProject = () => {
  /* New projects start from the current one's format: the next composition is
     usually a sibling of the one already open. */
  const form = createNewProjectForm(PM.proj
    ? { w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur, bg: PM.proj.bg }
    : {}, {
      backgroundField: (get, set) => PM.colorField(get, set, { label: 'Background', local: true })
    });
  PM.modal({ title: 'New composition', body: form.element, width: 420, actions: [
    { label: 'Cancel' },
    { label: 'Create', pri: true, run: () => {
      const values = form.values();
      switchProject(PM.mkProject({
        ...values,
        exportDefaults: { ...(PM.proj ? readExportDefaults() : {}), fps: values.fps },
      }));
      PM.ProjectsScreen?.hide?.();
    } },
  ] });
  window.setTimeout(() => form.focus(), 30);
};
function switchProject(p: any, history?: any) {
  PM.pause();
  if (PM.proj?.id && PM.proj.id !== p.id) captureProjectSession();
  closeProjectTransients();
  PM.proj = hydrate(p);
  PM.Projects.markOpen(PM.proj.id);
  const session = PM.Projects.getState(PM.proj.id);
  PM.Projects.putState(PM.proj.id, { ...session, lastActiveAt: Date.now() });
  const projectWorkspace = PM.store.get(`projectWorkspace.${PM.proj.id}`, null) || session?.workspace;
  if (projectWorkspace) PM.WS.restoreSnapshot(projectWorkspace);
  else PM.WS.activate('design', true);
  PM.hist.import?.(history ?? session?.history);
  persistCurrent(false);
  PM.bus.emit('projects:tabs');
  PM.time = Number.isFinite(session?.time) ? PM.clamp(session.time, 0, PM.proj.dur) : 0;
  PM.sel.layers = (session?.selection?.layers || []).filter((id: any) => PM.L(id));
  PM.sel.keys = [...new Set((session?.selection?.keys || []).filter((key: any) => typeof key === 'string'))];
  PM.sel.keys = PM.resolveSelectedKeys().map((key: any) => key.i);
  PM.sel.chan = session?.selection?.chan || null;
  const timeline = timelineService(PM);
  if (timeline) {
    timeline.pps = Number.isFinite(session?.timeline?.pps) ? session.timeline.pps : 90;
    timeline.scrollT = Number.isFinite(session?.timeline?.scrollT) ? session.timeline.scrollT : 0;
    timeline.scrollY = Number.isFinite(session?.timeline?.scrollY) ? session.timeline.scrollY : 0;
    timeline.graph = !!session?.timeline?.graph;
  }
  PM.rasterClear();
  PM.assets.clear();
  APP.fileHandle = null;
  APP.dirty = fileState().dirty;
  PM.bus.emit('project');
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  PM.bus.emit('assets');
  inspectorService(PM)?.refresh();
  viewerService(PM)?.layout();
  PM.invalidate('all');
  PM.invalidate('status');
  restoreProjectAssets(PM.proj);
  PM.autosave();
}

PM.confirmCloseProject = async (id: string) => {
  if (id === homeProjectId) return true;
  PM.bus.emit('project:flush-edits');
  await APP.importQueue;
  // Closing is a continuation of the user's current action. If a save is
  // already running, join it and then re-check the document instead of making
  // them retry Close after the file dialog or write completes.
  if (activeSave) await activeSave;
  const project = id === PM.proj.id ? PM.proj : PM.Projects.get(id);
  if (!project || !await refreshFileDirty(project)) return true;
  if (!window.powermove?.confirmProjectClose) return window.confirm?.('Close without saving a project file?') ?? false;
  const decision = await window.powermove.confirmProjectClose(project.name || 'Untitled');
  if (decision === 'cancel') return false;
  if (decision === 'save') {
    if (!await PM.saveProject({ projectId: id })) return false;
    return !await refreshFileDirty(id === PM.proj.id ? PM.proj : PM.Projects.get(id) || project);
  }
  return true;
};
PM.prepareToClose = async () => {
  const ids = [...new Set([PM.proj.id, ...PM.Projects.tabs()])];
  for (const id of ids) if (!await PM.confirmCloseProject(id)) return false;
  return true;
};

/* ── media import ──────────────────────────────────────── */
PM.pickFiles = (sequence = false) => {
  const targetProject = PM.proj;
  const inp = h('input', {
    // Chromium's video/* picker omits codecs it cannot decode natively.
    // These containers also support the native playback conversion path.
    type: 'file', multiple: true, accept: sequence ? IMAGE_SEQUENCE_ACCEPT : 'image/*,.svg,video/*,.mov,.mp4,.m4v,.webm,audio/*,.obj,.pmv',
    style: { position: 'fixed', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none' },
  });
  const cleanup = () => { inp.onchange = null; inp.remove(); };
  inp.onchange = async () => {
    const files = [...inp.files];
    try { if (files.length) await PM.importFiles(files, { project: targetProject, sequence: sequence || undefined }); }
    finally { cleanup(); }
  };
  inp.addEventListener('cancel', cleanup, { once: true });
  window.document.body.appendChild(inp);
  inp.click();
};
/* `placement` decides what happens after decode:
   - undefined → layers at the playhead (pickers, window-level drops)
   - null      → assets only, no layers (drop on the Media panel)
   - {at,index}→ layers at a time and layer-stack position (drop on the timeline) */
async function importFiles(files: any, placement?: { at: number; index?: number } | null, sequence?: boolean) {
  const targetProject = PM.proj;
  const importAt = placement ? placement.at : PM.time;
  const assertCurrent = () => {
    if (PM.proj !== targetProject) throw new Error('Import stopped because you switched projects · import again in the intended project');
  };
  if (sequence === true || (sequence !== false && sequenceCandidate(files))) {
    const choice = await chooseSequence(PM, files, sequence === true);
    if (choice === null) return;
    try {
      assertCurrent();
      files = choice.files;
      if (choice.fps !== null) {
        PM.toast(`Preparing ${files.length} image frames…`, 30_000);
        files = [await convertImageSequence(files, choice.fps, assertCurrent)];
        assertCurrent();
      }
    } catch (error: any) { PM.toast(error.message || 'Could not import image sequence', 6000); return; }
  }
  const mediaFiles: any = [];
  for (const f of files) {
    if (/\.pmv$/i.test(f.name)) { await openProjectFile(f); continue; }
    if (!PM.assetKind(f)) { PM.toast('Unsupported file · ' + f.name); continue; }
    mediaFiles.push(f);
  }
  if (!mediaFiles.length) return;
  if (mediaFiles.length > 1) PM.toast(`Preparing ${mediaFiles.length} media files…`, 2400);
  const results = await PM.assets.importBatch(mediaFiles, {
    onProgress: (progress: any) => PM.bus.emit('import:progress', progress),
  });
  const failures = results.filter((result: any) => result.status === 'failed');
  failures.forEach((result: any) => PM.toast(result.error?.message || ('Could not import ' + result.file?.name), 5000));
  const layerResults = results.filter((result: any) => result.status === 'created' || result.status === 'reused');
  const commands = placement === null ? [] : layerResults.map((result: any, n: number) => {
    const command = PM.commandForAsset(result.asset.id, importAt);
    if (command && placement?.index != null) command.index = placement.index + n;
    return command;
  }).filter(Boolean);
  if (commands.length) {
    PM.Edit.apply(commands, {
      label: commands.length === 1 ? 'Import file' : `Import ${commands.length} files`,
      origin: 'import',
    });
  }
  const relinked = results.filter((result: any) => result.status === 'relinked');
  const volatile = results.filter((result: any) => result.status !== 'failed' && !result.persisted);
  if (layerResults.length || relinked.length) {
    PM.autosave();
    if (placement === null) PM.bus.emit('assets');
    const parts: any = [];
    if (layerResults.length) {
      const editableSvg = layerResults.length === 1 && layerResults[0].asset.format === 'svg' && layerResults[0].asset.svg?.paths?.length;
      parts.push(layerResults.length === 1
        ? `Imported ${layerResults[0].asset.name}${editableSvg ? ' as editable paths' : ''}`
        : `Imported ${layerResults.length} files`);
    }
    if (relinked.length) parts.push(`relinked ${relinked.length} missing ${relinked.length === 1 ? 'asset' : 'assets'}`);
    const svgWarnings = layerResults.flatMap((result: any) => result.asset.svg?.warnings || []);
    if (svgWarnings.length) parts.push(`${svgWarnings.length} SVG ${svgWarnings.length === 1 ? 'feature needs' : 'features need'} review`);
    if (volatile.length) parts.push('durable storage unavailable');
    PM.toast(parts.join(' · '), volatile.length ? 6000 : 3400);
  }
}
/* File pickers and drag/drop can fire while an earlier batch is still decoding.
   Preserve user order and project identity by serializing batches; each batch
   still performs its expensive work through the bounded parallel pool. */
PM.importFiles = (files: any, { project = PM.proj, placement, sequence }: any = {}) => {
  const run = () => {
    if (PM.proj !== project) {
      PM.toast('Import stopped because you switched projects · import again in the intended project', 5000);
      return [];
    }
    return importFiles(Array.from(files || []), placement, sequence);
  };
  APP.importQueue = APP.importQueue.then(run, run);
  return APP.importQueue;
};
window.addEventListener('dragover', (e: any) => { if ([...e.dataTransfer.types].includes('Files')) e.preventDefault(); });
window.addEventListener('drop', (e: any) => {
  if (![...e.dataTransfer.types].includes('Files')) return;
  e.preventDefault(); PM.importFiles([...e.dataTransfer.files]);
});

function safeName(s: any) { return String(s || 'powermove').replace(/[\\/:*?"<>|]+/g, '-').trim() || 'powermove'; }
window.addEventListener('beforeunload', () => {
  window.clearTimeout(APP.saveTimer);
  captureProjectSession();
});

window.addEventListener('pm-open-project', (e: any) => {
  const p = e.detail;
  if (p && typeof p === 'object') switchProject(p);
});

/* A fresh profile has no document tabs or recovery entries. */
if (PM.proj.id === homeProjectId) {
  PM.ProjectsScreen.show('recents');
  document.title = 'Powermove';
} else {
  PM.Projects.markOpen(PM.proj.id);
  persistCurrent(false);
  void refreshFileDirty();
}
for (const id of PM.Projects.tabs()) {
  if (id === PM.proj.id) continue;
  const project = PM.Projects.get(id);
  if (project) void refreshFileDirty(project);
}
PM.bus.on('project:saved', () => PM.bus.emit('projects:tabs'));

/* first full frame after persistent panels have measured */
window.requestAnimationFrame(() => window.requestAnimationFrame(() => {
  viewerService(PM)?.layout();
  timelineService(PM)?.frameView();
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  inspectorService(PM)?.refresh();
  PM.invalidate('all');
  PM.invalidate('status');
  if (PM.SpatialAssistant) PM.SpatialAssistant.init();
  /* A brand-new registry entry has no card preview yet. Capture only after the
     first real composition frame has painted; capturing during boot produces a
     black placeholder even though the viewer becomes healthy a moment later. */
  window.requestAnimationFrame(() => {
    lastThumbAt = 0;
    persistCurrent(true);
    PM.bus.emit('projects:tabs');
  });
}));
}
