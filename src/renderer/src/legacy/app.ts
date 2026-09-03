/* Ported from js/app.js — behavior-preserving. */
import { normalizeExportDefaults, type ExportDefaults } from '../core/export-defaults';
import type { PMRegistry } from './registry';
import { packProjectFile, restoreProjectFileMedia, unpackProjectFile } from './core/project-file';
import { projectFingerprint } from './core/project-fingerprint';
import { createExtensionSettingsControl } from './ui/extension-settings';
import { createGeneralSettingsControl } from './ui/general-settings';
import {
  createNewProjectForm,
  createProjectSettingsControl,
  type CompositionPatch
} from './ui/project-settings';
import { createSettingsTabs } from './ui/settings-tabs';

export function install(PM: PMRegistry): void {
const h = PM.h;
const APP: any = { fileHandle: null, dirty: false, saveTimer: 0, importQueue: Promise.resolve() };
const document = window.document;
type FileState = { path?: string; savedHash?: string; dirty: boolean; handle?: any };
const fileStates = new Map<string, FileState>();
const comparisonVersions = new Map<string, number>();
function fileState(id = PM.proj.id): FileState {
  let state = fileStates.get(id);
  if (!state) {
    const saved = PM.Projects.getState(id)?.file;
    state = { path: saved?.path, savedHash: saved?.savedHash, dirty: true };
    fileStates.set(id, state);
  }
  return state;
}
function fileUI() {
  APP.dirty = fileState().dirty;
  PM.invalidate('status'); PM.bus.emit('projects:tabs');
  document.title = `${APP.dirty ? '• ' : ''}${PM.proj.name} — Powermove`;
}
async function refreshFileDirty(project = PM.proj): Promise<boolean> {
  const state = fileState(project.id), json = JSON.stringify(project);
  const version = (comparisonVersions.get(project.id) || 0) + 1;
  comparisonVersions.set(project.id, version);
  const hash = await projectFingerprint(json);
  // A newer edit wins over an in-flight comparison.
  if (comparisonVersions.get(project.id) !== version || JSON.stringify(project) !== json) return true;
  state.dirty = !state.savedHash || state.savedHash !== hash;
  if (PM.proj.id === project.id) fileUI();
  return state.dirty;
}
function rememberFile(id: string, state: FileState) {
  PM.Projects.putState(id, { ...PM.Projects.getState(id), file: { path: state.path, savedHash: state.savedHash } });
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
function loadBootProject() {
  const raw = PM.Projects.pickBoot({ legacy: PM.store.get('autosave', null) });
  if (!raw) return demo();
  try { return hydrate(raw); }
  catch (e) { console.warn('Saved project could not be loaded', raw.id, e); return demo(); }
}
function hydrate(p: any) {
  const base = PM.mkProject({ name: p.name, w: p.w, h: p.h, fps: p.fps, dur: p.dur, bg: p.bg });
  Object.assign(base, p);
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
      L.from = Math.max(0, num(L.from, 0)); L.dur = Math.max(.01, num(L.dur, 5));
      if (L.type != null && !PM.TYPE_META[L.type]) L.type = 'null';
      L.on = L.on !== false; L.lock = !!L.lock; delete L.solo; L.shy = !!L.shy;
      L.collapsed = L.collapsed !== false; L.fx = Array.isArray(L.fx) ? L.fx : []; L.p = L.p && typeof L.p === 'object' ? L.p : {}; L.d = L.d && typeof L.d === 'object' ? L.d : {};
      L.locked_intent = L.locked_intent || {}; L.blend = L.blend || 'normal'; L.mblur = !!L.mblur;
      if (L.type === 'audio') PM.Audio.normalizeLayer(L);
      if (L.parent === L.id || (typeof L.parent === 'string' && !container.layers.some((o: any) => o.id === L.parent))) L.parent = null;
      const fresh = PM.mkLayer(L.type || 'null', {}, container);
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
        f.id = f.id || PM.uid('fx'); f.p = sanitizeLooseParams(f.p, -L.from); f.on = f.on !== false;
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
        if (!PM.MASK_SHAPES.includes(m.shape)) m.shape = 'rect';
        if (m.mode !== 'subtract') m.mode = 'add';
        m.on = m.on !== false;
        const freshM = PM.mkMask(m.shape, container);
        Object.keys(freshM.p).forEach(k => {
          if (!m.p[k] || typeof m.p[k] !== 'object') m.p[k] = freshM.p[k];
          sanitizeProp(m.p[k], freshM.p[k], -L.from);
        });
        Object.keys(m.p).forEach(k => { if (!(k in freshM.p)) delete m.p[k]; });
      });
      if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].masks === false) L.masks = [];
      if (L.type === 'text') {
        for (const key of ['boxWidth', 'boxHeight']) {
          if (!L.d[key] || typeof L.d[key] !== 'object' || Array.isArray(L.d[key])) L.d[key] = fresh.d[key];
          sanitizeProp(L.d[key], fresh.d[key], -L.from);
        }
      }
      if (L.type === 'shader') {
        L.d.uniforms = sanitizeLooseParams(L.d.uniforms, -L.from);
        PM.syncShaderUniforms?.(L);
      }
      if (L.type === 'extension') {
        L.d.definition = typeof L.d.definition === 'string' ? L.d.definition : '';
        L.d.version = Math.max(1, Math.round(num(L.d.version, 1)));
        L.d.w = PM.clamp(num(L.d.w, container.w), 1, 16384);
        L.d.h = PM.clamp(num(L.d.h, container.h), 1, 16384);
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
    sanitizeLayers(c.layers, c);
  });
  sanitizeLayers(base.layers, base);
  /* precomp layers whose referenced comp failed to load degrade to empty layers */
  base.layers.forEach((L: any) => { if (L.type === 'precomp' && !(L.d && L.d.comp && base.comps[L.d.comp])) L.d.comp = null; });
  return base;
}
/* Shared project boundary for import/open flows and deterministic regression tests. */
PM.hydrateProject = hydrate;
function demo() {
  const p = PM.mkProject({ name: 'Velocity Study', w: 1920, h: 1080, fps: 30, dur: 8, bg: '#080809' });
  p.shutter = .5;
  const add = (L: any) => { p.layers.push(L); return L; };
  const key = (L: any, ch: any, list: any) => { const prop = L.p[ch]; list.forEach(([t,v,e='power']: any) => prop.kf.push(PM.KF(t,v,e))); };

  const eyebrow = add(PM.mkLayer('text', { name: 'Kicker', from: .35, dur: 6.1, d: {
    text: 'DESIGN  /  MOTION  /  SYSTEM', font: 'SF Mono', weight: 560, size: 28, tracking: 8, leading: 1, color: '#FF8A47', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 404 } }, p));
  key(eyebrow, 'opacity', [[0,0,'power'],[.45,100,'power'],[5.3,100,'easeIn'],[5.8,0,'easeIn']]);
  key(eyebrow, 'position.y', [[0,430,'power'],[.58,404,'power']]);

  const title = add(PM.mkLayer('text', { name: 'Powermove', from: .45, dur: 6, d: {
    text: 'Make the move.', font: 'SF Pro Display', weight: 650, size: 164, tracking: -7, leading: 1, color: '#F1F0EC', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 535 } }, p));
  key(title, 'opacity', [[0,0,'power'],[.6,100,'power'],[5.05,100,'easeIn'],[5.62,0,'easeIn']]);
  key(title, 'position.y', [[0,630,'power'],[.72,535,'power'],[5.05,535,'easeIn'],[5.62,475,'easeIn']]);
  key(title, 'scale.x', [[0,94,'glide'],[.8,100,'glide']]);
  key(title, 'scale.y', [[0,94,'glide'],[.8,100,'glide']]);

  const sub = add(PM.mkLayer('text', { name: 'Descriptor', from: 1.1, dur: 5, d: {
    text: 'A design-aware motion instrument.', font: 'SF Pro Display', weight: 430, size: 42, tracking: -.4, leading: 1.1, color: '#9C9A97', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 660 } }, p));
  key(sub, 'opacity', [[0,0,'glide'],[.65,100,'glide'],[4.3,100,'easeIn'],[4.85,0,'easeIn']]);

  const signal = add(PM.mkLayer('shape', { name: 'Signal', from: .15, dur: 7.85, d: {
    shape: 'ellipse', color: '#FF6B1A', w: 110, h: 110, radius: 0, stroke: 0, strokeColor: '#FFFFFF', points: 5,
  }, p: { 'position.x': 960, 'position.y': 540 } }, p));
  signal.blend = 'screen'; signal.mblur = true;
  key(signal, 'scale.x', [[0,0,'power'],[.7,100,'backOut'],[5.2,100,'glide'],[7.1,1800,'expoIn']]);
  key(signal, 'scale.y', [[0,0,'power'],[.7,100,'backOut'],[5.2,100,'glide'],[7.1,1800,'expoIn']]);
  key(signal, 'opacity', [[0,0,'power'],[.25,100,'power'],[5.65,100,'linear'],[7.2,92,'linear']]);
  /* Built-in effects activate after the legacy app hydrates. Keep the demo's
     glow as a recoverable placeholder during that short boot window. */
  const glow = PM.mkEffect('glow') || {
    id: PM.uid('f'), type: 'glow', on: true, missing: true,
    p: { threshold: PM.P(18), radius: PM.P(120), intensity: PM.P(165) }
  };
  glow.p.threshold.v = 18; glow.p.radius.v = 120; glow.p.intensity.v = 165; signal.fx.push(glow);

  const bg = add(PM.mkLayer('shader', { name: 'Atmosphere', from: 0, dur: 8, d: {
    code: `uniform float uSpeed; // @param 0.16 0 2
uniform float uScale; // @param 2.1 0.4 8
uniform vec3 uEmber; // @param #FF6B1A
uniform float uEnergy; // @param 0.75 0 2
void main(){
  vec2 p=(uv-.5)*vec2(iResolution.x/iResolution.y,1.);
  float t=iTime*uSpeed;
  float a=fbm(p*uScale+vec2(t,-t*.34));
  float b=fbm(rot(.82)*p*uScale*1.6+vec2(-t*.5,t*.72)+13.7);
  float field=smoothstep(.42,.83,a*.72+b*.4);
  vec3 col=mix(vec3(.015,.014,.017),uEmber,field*.52*uEnergy);
  col+=uEmber*pow(field,8.)*.26;
  float halo=exp(-length(p-vec2(.0,.05))*2.7);
  col+=uEmber*halo*.035;
  col*=1.-dot(p,p)*.3;
  fragColor=vec4(col,1.);
}`, w: 1920, h: 1080, uniforms: {},
  } }, p));
  PM.syncShaderUniforms?.(bg);
  p.markers = [{ t: .45, name: 'Reveal' }, { t: 5.55, name: 'Expansion' }, { t: 7.25, name: 'Resolve' }];
  p.notes = 'One continuous signal becomes the field. Keep the hierarchy singular, restrained, and physical.';
  return p;
}

/* Dev safety: while the app is being built, a malformed persisted snapshot can boot
   into a blank/off-looking project. Bump PM.bootVersion to force a one-time reset to
   the known-good demo on next launch. This never discards real saved projects (⌘S),
   only the volatile in-progress autosave. */
(function resetIfStale() {
  const bv = PM.store.get('bootVersion', 0);
  if (bv !== PM.bootVersion) {
    PM.store.del('autosave');
    PM.store.set('bootVersion', PM.bootVersion);
  }
})();

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
  PM.Inspector?.refresh?.();
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
    PM.Inspector?.refresh?.();
    PM.invalidate();
  }
}
PM.Kernel?.effects?.onChange?.(revalidateContributionPlaceholders);
PM.Kernel?.transitions?.onChange?.(revalidateContributionPlaceholders);

PM.WS.init();
const bootSession = PM.Projects.getState(PM.proj.id);
if (bootSession?.workspace) PM.WS.restoreSnapshot(bootSession.workspace);
PM.selectLayers((bootSession?.selection?.layers || []).filter((id: any) => PM.L(id)).length
  ? bootSession.selection.layers.filter((id: any) => PM.L(id))
  : PM.proj.layers.find((l: any) => l.name === 'Powermove')?.id || []);
PM.sel.keys = [...new Set((bootSession?.selection?.keys || []).filter((key: any) => typeof key === 'string'))];
PM.sel.keys = PM.resolveSelectedKeys().map((key: any) => key.i);
PM.setTime(Number.isFinite(bootSession?.time) ? bootSession.time : .9, { raw: true, force: true });
if (bootSession?.timeline && PM.TL) {
  PM.TL.pps = Number.isFinite(bootSession.timeline.pps) ? bootSession.timeline.pps : PM.TL.pps;
  PM.TL.scrollT = Number.isFinite(bootSession.timeline.scrollT) ? bootSession.timeline.scrollT : PM.TL.scrollT;
  PM.TL.scrollY = Number.isFinite(bootSession.timeline.scrollY) ? bootSession.timeline.scrollY : PM.TL.scrollY;
  PM.TL.graph = !!bootSession.timeline.graph;
}
PM.hist.clear();
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

function projectSettingsBridge() {
  return {
    composition: () => ({
      name: PM.proj.name, w: PM.proj.w, h: PM.proj.h, fps: PM.proj.fps, dur: PM.proj.dur
    }),
    applyComposition: (patch: CompositionPatch) => {
      /* Renaming goes through the project registry so the tab strip and the
         stored project slot stay in step, exactly like an inline tab rename. */
      if (patch.name != null) { PM.Projects.rename(PM.proj.id, patch.name); return; }
      try {
        PM.Edit.apply({ type: 'set_composition', patch }, { label: 'Project settings', origin: 'interface' });
      } catch (error: any) {
        PM.toast('Could not change the project: ' + (error?.message || 'invalid value'), 4500);
        return;
      }
      if (PM.time > PM.proj.dur) PM.setTime(PM.proj.dur);
      PM.rasterClear?.();
      PM.Viewer?.layout?.();
      PM.invalidate('all');
    },
    exportDefaults: readExportDefaults,
    applyExportDefaults: writeExportDefaults,
    backgroundField: () => PM.fillField(
      () => PM.normalizeFill(PM.proj.backgroundFill, PM.proj.bg),
      (value: any) => {
        PM.proj.backgroundFill = PM.normalizeFill(value, PM.proj.bg);
        PM.proj.bg = PM.proj.backgroundFill.stops[0].color;
      },
      { label: 'Background', command: (value: any) => ({ type: 'set_composition', patch: { backgroundFill: value } }) }
    )
  };
}

/* Settings is a singleton: the menu item, the ⌘, shortcut, and the titlebar
   button all lead to the one dialog. A second request re-uses the open one and
   switches it to the asked-for tab instead of stacking another copy. */
let settingsSession: { dialog: any; show(tab: 'general' | 'project' | 'extensions'): void } | null = null;

function openSettings(initialTab: 'general' | 'project' | 'extensions' = 'general') {
  if (settingsSession) {
    settingsSession.show(initialTab);
    return settingsSession.dialog;
  }
  const general = createGeneralSettingsControl(PM.theme);
  const extensions = createExtensionSettingsControl();
  /* The Project tab only makes sense with a project open; without one it is
     left out entirely rather than shown empty. */
  const project = PM.proj ? createProjectSettingsControl(projectSettingsBridge()) : null;
  const offProject = project ? PM.bus.on('project', () => project.refresh()) : null;
  const tabs = createSettingsTabs([
    { id: 'general', label: 'General', panel: h('section', general.element) },
    ...(project ? [{ id: 'project', label: 'Project', panel: h('section', project.element) }] : []),
    { id: 'extensions', label: 'Extensions', panel: h('section', extensions.element) }
  ], project || initialTab !== 'project' ? initialTab : 'general');
  const dialog = PM.modal({
    title: 'Settings',
    body: h('div.settings-view', tabs.element),
    width: 620,
    fill: true,
    actions: [{ label: 'Done', pri: true }],
    onClose: () => {
      settingsSession = null;
      offProject?.(); general.destroy(); project?.destroy(); extensions.destroy();
    }
  });
  const show = (tab: 'general' | 'project' | 'extensions') => {
    const target = tab === 'project' && !project ? 'general' : tab;
    tabs.select(target);
    window.setTimeout(() => {
      if (target === 'general') general.focus();
      else if (target === 'project') project?.focus();
    }, 30);
  };
  settingsSession = { dialog, show };
  show(initialTab);
  return dialog;
}
PM.SettingsUI = { open: openSettings };

/* ── persistence ───────────────────────────────────────── */
/* Thumbnails are captured at most once per 5s so autosave never janks. */
let lastThumbAt = 0;
function projectThumb() {
  const now = Date.now();
  if (now - lastThumbAt < 5000) return undefined;
  lastThumbAt = now;
  try { return PM.Export.snapshot(PM.time, 320); } catch (e) { return undefined; }
}
function persistCurrent(withThumb: any) {
  try {
    const thumb = withThumb ? projectThumb() : undefined;
    if (withThumb && PM.Projects.recover) PM.Projects.recover(PM.proj, thumb);
    else PM.Projects.put(PM.proj, thumb);
    return true;
  } catch (e) { APP.dirty = true; console.warn('Project save failed', e); return false; }
}

function captureProjectSession() {
  if (!PM.proj?.id) return true;
  PM.bus.emit('project:flush-edits');
  const saved = persistCurrent(false);
  PM.Projects.putState(PM.proj.id, {
    ...PM.Projects.getState(PM.proj.id),
    lastActiveAt: Date.now(),
    workspace: PM.WS.snapshot(), time: PM.time,
    selection: { layers: [...PM.sel.layers], keys: PM.sel.keys.filter((key: any) => typeof key === 'string'), chan: PM.sel.chan },
    timeline: PM.TL
      ? { pps: PM.TL.pps, scrollT: PM.TL.scrollT, scrollY: PM.TL.scrollY, graph: PM.TL.graph }
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
  APP.dirty = true;
  fileState().dirty = true;
  comparisonVersions.set(PM.proj.id, (comparisonVersions.get(PM.proj.id) || 0) + 1);
  fileUI();
  const generation = ++saveGeneration, projectId = PM.proj.id;
  window.clearTimeout(APP.saveTimer);
  APP.saveTimer = window.setTimeout(async () => {
    try {
      if (!persistCurrent(true)) throw new Error('Project storage is unavailable or full');
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

PM.saveProject = async ({ saveAs = false, projectId = PM.proj.id }: any = {}) => {
  if (APP.saving) return false;
  APP.saving = true;
  try {
    (window.document.activeElement as HTMLElement | null)?.blur?.();
    PM.bus.emit('project:flush-edits');
    await APP.importQueue;
    const project = projectId === PM.proj.id ? PM.proj : PM.Projects.get(projectId);
    if (!project) throw new Error('This project is no longer available.');
    const projectJSON = JSON.stringify(project);
    const snapshot = { v: PM.version, proj: project,
      ws: projectId === PM.proj.id ? PM.WS.current : PM.Projects.getState(projectId)?.workspace };
    const state = fileState(projectId);
    const suggestedName = safeName(project.name) + '.pmv';
    const data = await packProjectFile(snapshot, PM.MediaStore);
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
    if (window.powermove?.saveFile) {
      const result = await window.powermove.saveFile({ name: suggestedName, projectId, saveAs, data });
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
  } finally { APP.saving = false; }
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
    switchProject(project);
    const state = fileState(project.id);
    state.path = association?.path;
    state.savedHash = association ? await projectFingerprint(JSON.stringify(PM.proj)) : undefined;
    rememberFile(project.id, state);
    await refreshFileDirty();
    if (o.ws?.layout?.docks) PM.WS.restoreSnapshot(o.ws);
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
    } },
  ] });
  window.setTimeout(() => form.focus(), 30);
};
function switchProject(p: any) {
  PM.pause();
  if (PM.proj?.id && PM.proj.id !== p.id) captureProjectSession();
  closeProjectTransients();
  PM.proj = hydrate(p);
  PM.Projects.markOpen(PM.proj.id);
  const session = PM.Projects.getState(PM.proj.id);
  PM.Projects.putState(PM.proj.id, { ...session, lastActiveAt: Date.now() });
  if (session?.workspace) PM.WS.restoreSnapshot(session.workspace);
  else PM.WS.activate('design', true);
  persistCurrent(false);
  PM.bus.emit('projects:tabs');
  PM.time = Number.isFinite(session?.time) ? PM.clamp(session.time, 0, PM.proj.dur) : 0;
  PM.sel.layers = (session?.selection?.layers || []).filter((id: any) => PM.L(id));
  PM.sel.keys = [...new Set((session?.selection?.keys || []).filter((key: any) => typeof key === 'string'))];
  PM.sel.keys = PM.resolveSelectedKeys().map((key: any) => key.i);
  PM.sel.chan = session?.selection?.chan || null;
  if (PM.TL) {
    PM.TL.pps = Number.isFinite(session?.timeline?.pps) ? session.timeline.pps : 90;
    PM.TL.scrollT = Number.isFinite(session?.timeline?.scrollT) ? session.timeline.scrollT : 0;
    PM.TL.scrollY = Number.isFinite(session?.timeline?.scrollY) ? session.timeline.scrollY : 0;
    PM.TL.graph = !!session?.timeline?.graph;
  }
  PM.hist.clear();
  PM.rasterClear();
  PM.assets.clear();
  APP.fileHandle = null;
  APP.dirty = fileState().dirty;
  PM.bus.emit('project');
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  PM.bus.emit('assets');
  PM.Inspector?.refresh?.();
  PM.Viewer?.layout?.();
  PM.invalidate('all');
  PM.invalidate('status');
  restoreProjectAssets(PM.proj);
  PM.autosave();
}

PM.confirmCloseProject = async (id: string) => {
  PM.bus.emit('project:flush-edits');
  await APP.importQueue;
  if (APP.saving) { PM.toast('Please wait for the current save to finish.'); return false; }
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
PM.pickFiles = () => {
  const targetProject = PM.proj;
  const inp = h('input', {
    type: 'file', multiple: true, accept: 'image/*,video/*,audio/*,.obj,.pmv',
    style: { position: 'fixed', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none' },
  });
  const cleanup = () => { inp.onchange = null; inp.remove(); };
  inp.onchange = async () => {
    const files = [...inp.files];
    try { if (files.length) await PM.importFiles(files, { project: targetProject }); }
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
async function importFiles(files: any, placement?: { at: number; index?: number } | null) {
  const mediaFiles: any = [];
  for (const f of files) {
    if (/\.pmv$/i.test(f.name)) { await openProjectFile(f); continue; }
    if (!PM.assetKind(f)) { PM.toast('Unsupported file · ' + f.name); continue; }
    mediaFiles.push(f);
  }
  if (!mediaFiles.length) return;
  const importAt = placement ? placement.at : PM.time;
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
    if (layerResults.length) parts.push(layerResults.length === 1 ? `Imported ${layerResults[0].asset.name}` : `Imported ${layerResults.length} files`);
    if (relinked.length) parts.push(`relinked ${relinked.length} missing ${relinked.length === 1 ? 'asset' : 'assets'}`);
    if (volatile.length) parts.push('durable storage unavailable');
    PM.toast(parts.join(' · '), volatile.length ? 6000 : 3400);
  }
}
/* File pickers and drag/drop can fire while an earlier batch is still decoding.
   Preserve user order and project identity by serializing batches; each batch
   still performs its expensive work through the bounded parallel pool. */
PM.importFiles = (files: any, { project = PM.proj, placement }: any = {}) => {
  const run = () => {
    if (PM.proj !== project) {
      PM.toast('Import stopped because you switched projects · import again in the intended project', 5000);
      return [];
    }
    return importFiles(Array.from(files || []), placement);
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

/* boot registration: the startup project becomes the first tab */
PM.Projects.markOpen(PM.proj.id);
persistCurrent(false);
void refreshFileDirty();
for (const id of PM.Projects.tabs()) {
  if (id === PM.proj.id) continue;
  const project = PM.Projects.get(id);
  if (project) void refreshFileDirty(project);
}
PM.bus.on('project:saved', () => PM.bus.emit('projects:tabs'));

/* first full frame after persistent panels have measured */
window.requestAnimationFrame(() => window.requestAnimationFrame(() => {
  PM.Viewer?.layout?.();
  PM.TL?.frameView?.();
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  PM.Inspector?.refresh?.();
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
