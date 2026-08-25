/* No longer loaded — superseded by src/renderer/src/legacy/app.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — application bootstrap, project I/O, shell, autosave. */
(() => {
const PM = window.PM, h = PM.h, $ = PM.$;
const APP = { fileHandle: null, dirty: false, saveTimer: 0, importQueue: Promise.resolve() };
PM.app = APP;

/* ── appearance (light default, dark alternate) ────────── */
PM.theme = (() => {
  const root = document.documentElement;
  const syncNative = () => {
    try {
      window.webkit && window.webkit.messageHandlers.pmTheme &&
        window.webkit.messageHandlers.pmTheme.postMessage(root.dataset.theme || 'light');
    } catch (e) { }
  };
  const apply = (t) => {
    if (t === 'dark') root.dataset.theme = 'dark'; else delete root.dataset.theme;
    PM.store.set('theme', t);
    syncNative();
    /* layout event re-resolves CSS-token caches (timeline canvas) */
    PM.bus.emit('layout');
    PM.invalidate();
  };
  const saved = PM.store.get('theme', 'light');
  apply(saved === 'dark' ? 'dark' : 'light');
  return {
    get current() { return root.dataset.theme === 'dark' ? 'dark' : 'light'; },
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
function hydrate(p) {
  const base = PM.mkProject({ name: p.name, w: p.w, h: p.h, fps: p.fps, dur: p.dur, bg: p.bg });
  Object.assign(base, p);
  base.backgroundFill = PM.normalizeFill(p.backgroundFill, p.bg || '#000000');
  base.bg = base.backgroundFill.stops[0].color;
  base.layers = Array.isArray(p.layers) ? p.layers : [];
  base.assets = p.assets || {};
  base.markers = p.markers || [];
  base.work = p.work && p.work.length === 2 ? p.work : [0, base.dur];
  const num = (x, fb) => { const n = Number(x); return Number.isFinite(n) ? n : fb; };
  const sanitizeParams = (params) => {
    if (!params || typeof params !== 'object' || Array.isArray(params)) return {};
    const out = {};
    Object.entries(params).forEach(([key, src]) => {
      if (!src || typeof src !== 'object' || Array.isArray(src)) return;
      const q = { ...src };
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
        q.options = Array.isArray(q.options) ? q.options.filter(o => o && typeof o === 'object' && 'v' in o) : [];
        if (q.value === undefined) q.value = q.options.length ? q.options[0].v : '';
      }
      out[key] = q;
    });
    return out;
  };
  base.params = sanitizeParams(p.params);
  base.shutter = p.shutter == null ? .5 : p.shutter;
  /* Production rule: a saved project must never poison the renderer. Every channel,
     keyframe, effect and layer field is normalized here so malformed data degrades
     to a static value instead of NaN transforms or a broken keyframe search.
     Applied recursively to nested compositions as well. */
  const sanitizeProp = (prop, fresh) => {
    prop.kf = Array.isArray(prop.kf) ? prop.kf : [];
    /* sanitize keyframes: finite t/v only, sorted, near-duplicates collapsed */
    prop.kf = prop.kf
      .filter(q => q && typeof q === 'object' && Number.isFinite(num(q.t)) && Number.isFinite(num(q.v)))
      .map(q => ({ ...q, t: num(q.t), v: num(q.v), i: q.i || PM.uid('k'), hold: !!q.hold }))
      .sort((a, b) => a.t - b.t)
      .filter((q, i, arr) => i === 0 || q.t - arr[i - 1].t > 1e-6);
    prop.expr = typeof prop.expr === 'string' && prop.expr.trim() ? prop.expr : null;
    prop.v = num(prop.v, fresh.v);
  };
  const sanitizeLayers = (layers, container) => {
    layers.forEach((L, li) => {
      L.id = typeof L.id === 'string' && L.id ? L.id : PM.uid('L');
      if (!L.name || typeof L.name !== 'string') L.name = 'Layer ' + (li + 1);
      L.from = Math.max(0, num(L.from, 0)); L.dur = Math.max(.01, num(L.dur, 5));
      if (L.type != null && !PM.TYPE_META[L.type]) L.type = 'null';
      L.on = L.on !== false; L.lock = !!L.lock; L.solo = !!L.solo; L.shy = !!L.shy;
      L.collapsed = L.collapsed !== false; L.fx = Array.isArray(L.fx) ? L.fx : []; L.p = L.p && typeof L.p === 'object' ? L.p : {}; L.d = L.d && typeof L.d === 'object' ? L.d : {};
      L.locked_intent = L.locked_intent || {}; L.blend = L.blend || 'normal'; L.mblur = !!L.mblur;
      if (L.type === 'audio') PM.Audio.normalizeLayer(L);
      if (L.parent === L.id || (typeof L.parent === 'string' && !container.layers.some(o => o.id === L.parent))) L.parent = null;
      const fresh = PM.mkLayer(L.type || 'null', {}, container);
      Object.keys(fresh.p).forEach(k => {
        if (!L.p[k] || typeof L.p[k] !== 'object') L.p[k] = fresh.p[k];
        sanitizeProp(L.p[k], fresh.p[k]);
      });
      Object.keys(L.p).forEach(k => { if (!(k in fresh.p)) delete L.p[k]; });
      L.fx = L.fx.filter(f => f && typeof f === 'object' && PM.FX && PM.FX[f.type]);
      L.fx.forEach(f => { f.id = f.id || PM.uid('fx'); f.p = f.p || {}; f.on = f.on !== false; });
      if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].effects === false) L.fx = [];
      /* masks: validate shape/mode and every animatable channel */
      L.masks = Array.isArray(L.masks) ? L.masks.filter(m => m && typeof m === 'object' && m.p && typeof m.p === 'object') : [];
      L.masks.forEach(m => {
        m.id = typeof m.id === 'string' && m.id ? m.id : PM.uid('K');
        if (!PM.MASK_SHAPES.includes(m.shape)) m.shape = 'rect';
        if (m.mode !== 'subtract') m.mode = 'add';
        m.on = m.on !== false;
        const freshM = PM.mkMask(m.shape, container);
        Object.keys(freshM.p).forEach(k => {
          if (!m.p[k] || typeof m.p[k] !== 'object') m.p[k] = freshM.p[k];
          sanitizeProp(m.p[k], freshM.p[k]);
        });
        Object.keys(m.p).forEach(k => { if (!(k in freshM.p)) delete m.p[k]; });
      });
      if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].masks === false) L.masks = [];
      if (L.type === 'shader') PM.syncShaderUniforms(L);
    });
  };
  base.comps = p.comps && typeof p.comps === 'object' ? p.comps : {};
  Object.entries(base.comps).forEach(([cid, c]) => {
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
  base.layers.forEach(L => { if (L.type === 'precomp' && !(L.d && L.d.comp && base.comps[L.d.comp])) L.d.comp = null; });
  return base;
}
function demo() {
  const p = PM.mkProject({ name: 'Velocity Study', w: 1920, h: 1080, fps: 30, dur: 8, bg: '#080809' });
  p.shutter = .5;
  const add = L => { p.layers.push(L); return L; };
  const key = (L, ch, list) => { const prop = L.p[ch]; list.forEach(([t,v,e='power']) => prop.kf.push(PM.KF(t,v,e))); };

  const eyebrow = add(PM.mkLayer('text', { name: 'Kicker', from: .35, dur: 6.1, d: {
    text: 'DESIGN  /  MOTION  /  SYSTEM', font: 'Geist Mono', weight: 560, size: 28, tracking: 8, leading: 1, color: '#FF8A47', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 404 } }, p));
  key(eyebrow, 'opacity', [[0,0,'power'],[.45,100,'power'],[5.3,100,'easeIn'],[5.8,0,'easeIn']]);
  key(eyebrow, 'position.y', [[0,430,'power'],[.58,404,'power']]);

  const title = add(PM.mkLayer('text', { name: 'Powermove', from: .45, dur: 6, d: {
    text: 'Make the move.', font: 'Geist', weight: 650, size: 164, tracking: -7, leading: 1, color: '#F1F0EC', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 535 } }, p));
  key(title, 'opacity', [[0,0,'power'],[.6,100,'power'],[5.05,100,'easeIn'],[5.62,0,'easeIn']]);
  key(title, 'position.y', [[0,630,'power'],[.72,535,'power'],[5.05,535,'easeIn'],[5.62,475,'easeIn']]);
  key(title, 'scale.x', [[0,94,'glide'],[.8,100,'glide']]);
  key(title, 'scale.y', [[0,94,'glide'],[.8,100,'glide']]);

  const sub = add(PM.mkLayer('text', { name: 'Descriptor', from: 1.1, dur: 5, d: {
    text: 'A design-aware motion instrument.', font: 'Geist', weight: 430, size: 42, tracking: -.4, leading: 1.1, color: '#9C9A97', align: 'center',
  }, p: { 'position.x': 960, 'position.y': 660 } }, p));
  key(sub, 'opacity', [[0,0,'glide'],[.65,100,'glide'],[4.3,100,'easeIn'],[4.85,0,'easeIn']]);

  const signal = add(PM.mkLayer('shape', { name: 'Signal', from: .15, dur: 7.85, d: {
    shape: 'ellipse', color: '#FF6B1A', w: 110, h: 110, radius: 0, stroke: 0, strokeColor: '#FFFFFF', points: 5,
  }, p: { 'position.x': 960, 'position.y': 540 } }, p));
  signal.blend = 'screen'; signal.mblur = true;
  key(signal, 'scale.x', [[0,0,'power'],[.7,100,'backOut'],[5.2,100,'glide'],[7.1,1800,'expoIn']]);
  key(signal, 'scale.y', [[0,0,'power'],[.7,100,'backOut'],[5.2,100,'glide'],[7.1,1800,'expoIn']]);
  key(signal, 'opacity', [[0,0,'power'],[.25,100,'power'],[5.65,100,'linear'],[7.2,92,'linear']]);
  const glow = PM.mkEffect('glow'); glow.p.threshold.v = 18; glow.p.radius.v = 120; glow.p.intensity.v = 165; signal.fx.push(glow);

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
  PM.syncShaderUniforms(bg);
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
  const empties = metas.filter(m => m.name === 'Untitled');
  if (!empties.length) return;
  const hasReal = metas.some(m => {
    if (m.name !== 'Untitled') return true;
    const raw = PM.Projects.get(m.id);
    return !!(raw && Array.isArray(raw.layers) && raw.layers.length);
  });
  empties.forEach((m, i) => {
    if (hasReal || i > 0) {
      const raw = PM.Projects.get(m.id);
      if (!raw || (Array.isArray(raw.layers) && raw.layers.length === 0)) PM.Projects.remove(m.id);
    }
  });
  /* A registry card without a slot is never recoverable from the registry and
     otherwise lingers as a project that cannot open (for example, an interrupted
     duplicate). Project files exported by the user are unaffected. */
  PM.Projects.list().forEach(m => { if (!PM.Projects.get(m.id)) PM.Projects.remove(m.id); });
})();

async function restoreProjectAssets(project, warn = true) {
  const result = await PM.assets.restoreProject(project);
  if (result.stale || PM.proj !== project) return result;
  PM.bus.emit('assets');
  PM.Inspector.refresh();
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
PM.WS.init();
const bootSession = PM.Projects.getState(PM.proj.id);
if (bootSession?.workspace) PM.WS.restoreSnapshot(bootSession.workspace);
PM.selectLayers((bootSession?.selection?.layers || []).filter(id => PM.L(id)).length
  ? bootSession.selection.layers.filter(id => PM.L(id))
  : PM.proj.layers.find(l => l.name === 'Powermove')?.id || []);
PM.sel.keys = [...new Set((bootSession?.selection?.keys || []).filter(key => typeof key === 'string'))];
PM.sel.keys = PM.resolveSelectedKeys().map(key => key.i);
PM.setTime(Number.isFinite(bootSession?.time) ? bootSession.time : .9, { raw: true, force: true });
if (bootSession?.timeline) {
  PM.TL.pps = Number.isFinite(bootSession.timeline.pps) ? bootSession.timeline.pps : PM.TL.pps;
  PM.TL.scrollT = Number.isFinite(bootSession.timeline.scrollT) ? bootSession.timeline.scrollT : PM.TL.scrollT;
  PM.TL.scrollY = Number.isFinite(bootSession.timeline.scrollY) ? bootSession.timeline.scrollY : PM.TL.scrollY;
  PM.TL.graph = !!bootSession.timeline.graph;
}
PM.hist.clear();
restoreProjectAssets(PM.proj);

/* ── shell ─────────────────────────────────────────────── */
function openSettings() {
  const appearance = h('select.settings-appearance', { 'aria-label': 'Appearance' },
    h('option', { value: 'light' }, 'Light'), h('option', { value: 'dark' }, 'Dark'));
  appearance.value = PM.theme.current;
  appearance.onchange = () => PM.theme.apply(appearance.value);
  const body = h('div.settings-view',
    h('div.settings-row', h('div.settings-copy', h('b', 'Appearance'), h('span', 'Choose how Powermove looks.')), appearance),
    h('p.settings-note', 'Undo and Redo remain available from the Edit menu and keyboard shortcuts.'));
  const dialog = PM.modal({ title: 'Settings', body, width: 440, actions: [{ label: 'Done', pri: true }] });
  setTimeout(() => appearance.focus(), 30);
  return dialog;
}
PM.SettingsUI = { open: openSettings };
function buildTitlebar() {
  const tabs = $('#tabs'), right = $('#tb-right'), bar = $('#titlebar');
  /* Native window drag: WKWebView ignores -webkit-app-region, so forward pointerdown
     on empty titlebar regions to the AppKit drag bridge. Interactive children opt out. */
  const dragBridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.windowDrag;
  bar.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    if (e.target.closest('button, #tabs, #toolbar-strip, input, a, .tb-right')) return;
    if (!dragBridge) return;
    e.preventDefault();
    /* Native-feel window drag: pin the arrow cursor, drop any hover state, and
       swallow the gesture so the web content never shows grab/hand feedback. */
    document.body.style.cursor = 'default';
    if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
    const pin = () => { document.body.style.cursor = 'default'; };
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      window.removeEventListener('pointermove', pin, true);
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
      document.body.style.cursor = '';
    };
    window.addEventListener('pointermove', pin, true);
    window.addEventListener('pointerup', release, true);
    window.addEventListener('pointercancel', release, true);
    dragBridge.postMessage({ x: e.clientX, y: e.clientY });
  });
  bar.addEventListener('dblclick', (e) => {
    if (e.target.closest('button, #tabs, #toolbar-strip, input, a, .tb-right')) return;
    /* mimic standard macOS titlebar double-click (zoom) */
    const zb = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.windowZoom;
    if (zb) zb.postMessage({});
  });
  const beginProjectRename = (tab, id, currentName) => {
    if (tab.classList.contains('renaming')) return;
    const label = tab.querySelector('.project-doc-label');
    if (!label) return;
    const input = h('input.project-doc-input', {
      value: currentName, maxlength: 120, 'aria-label': 'Rename project', spellcheck: 'false',
    });
    let finished = false;
    const finish = (commit) => {
      if (finished) return;
      finished = true;
      if (commit) PM.Projects.rename(id, input.value);
      tab.classList.remove('renaming');
      PM.bus.emit('projects:tabs');
      if (commit) PM.bus.emit('project');
    };
    input.onpointerdown = (e) => e.stopPropagation();
    input.onclick = (e) => e.stopPropagation();
    input.onkeydown = (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); finish(true); }
      else if (e.key === 'Escape') { e.preventDefault(); finish(false); }
    };
    input.onblur = () => finish(true);
    tab.classList.add('renaming');
    tab.setAttribute('aria-label', 'Rename ' + currentName);
    label.replaceWith(input);
    requestAnimationFrame(() => { input.focus(); input.select(); });
  };
  const paintTabs = () => {
    /* Autosave and background project events may refresh the strip. Never tear
       down a live rename field before the user commits or cancels it. */
    if (tabs.querySelector('.project-doc.renaming')) return;
    tabs.textContent = '';
    tabs.setAttribute('role', 'tablist');
    tabs.setAttribute('aria-label', 'Open projects');
    const homeOpen = !!(PM.ProjectsScreen && PM.ProjectsScreen.isOpen);
    const home = h('button.project-strip-btn.project-home' + (homeOpen ? '.on' : ''), {
      title: 'Projects', 'aria-label': 'Projects', 'aria-selected': String(homeOpen),
      onclick: () => PM.ProjectsScreen.show(),
    }, PM.icon('home'));
    tabs.append(home, h('i.project-strip-divider'));
    PM.Projects.tabs().forEach(id => {
      const meta = PM.Projects.list().find(m => m.id === id);
      const current = id === PM.proj.id;
      const active = current && !homeOpen;
      const dirty = current && APP.dirty;
      const tabName = (meta && meta.name) || 'Untitled';
      const tab = h('div.project-doc' + (active ? '.on' : '') + (dirty ? '.dirty' : ''), {
        title: (meta && meta.name) || id, role: 'tab', tabindex: '0',
        'aria-selected': String(active), 'aria-label': tabName + (dirty ? ', unsaved' : ''),
      },
        h('span.project-doc-label', tabName),
        h('button.project-doc-close', {
          title: 'Close project',
          onclick: (e) => { e.stopPropagation(); closeTab(id); },
        }, PM.icon('x')));
      tab.onclick = (e) => {
        if (tab.classList.contains('renaming') || e.target.closest('input')) return;
        /* The second click is the most reliable cross-WebKit double-click signal,
           including when the first click activates a previously inactive tab. */
        if (e.detail > 1) { e.preventDefault(); beginProjectRename(tab, id, tabName); return; }
        if (PM.ProjectsScreen && PM.ProjectsScreen.isOpen) PM.ProjectsScreen.hide();
        if (!active) openTab(id);
      };
      tab.ondblclick = (e) => {
        if (e.target.closest('.project-doc-close')) return;
        e.preventDefault(); e.stopPropagation();
        beginProjectRename(tab, id, tabName);
      };
      tab.onkeydown = (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); tab.click(); }
      };
      tab.onauxclick = (e) => { if (e.button === 1) { e.preventDefault(); closeTab(id); } };
      tabs.appendChild(tab);
    });
    const plus = h('button.project-strip-btn.project-new', { title: 'New project', 'aria-label': 'New project', onclick: () => {
      if (PM.ProjectsScreen && PM.ProjectsScreen.isOpen) PM.ProjectsScreen.hide();
      PM.newProject();
    } }, PM.icon('plus'));
    tabs.appendChild(plus);
    requestAnimationFrame(() => {
      const selected = tabs.querySelector('.project-doc.on');
      if (selected) selected.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    });
  };
  PM.bus.on('projects:tabs', paintTabs); PM.bus.on('projects:screen', paintTabs);
  right.textContent = '';
  const button = (icon, title, run) => h('button.iconbtn', { title, onclick: run }, PM.icon(icon));
  right.append(
    button('wand', 'Ask Powermove agent (⌘⇧K)', () => PM.SpatialAssistant?.open?.()),
    button('grid', 'Library · Sections and Workspaces', () => PM.LibraryUI.open()),
    button('gear', 'Settings', openSettings),
  );
  PM.bus.on('workspaces', paintTabs); PM.bus.on('project', paintTabs); PM.bus.on('history', paintTabs);
  paintTabs();
}
function buildStatus() {
  const s = $('#status');
  const paint = () => {
    const keys = PM.proj.layers.reduce((n,L) => n + PM.allProps(L).reduce((m,x) => m + x.prop.kf.length, 0), 0);
    s.textContent = '';
    s.append(
      h('span', APP.dirty ? 'UNSAVED' : 'SAVED'),
      h('span', `${PM.proj.layers.length} layers`), h('span', `${keys} keys`),
      h('span.sp'),
      h('span', PM.GL.gl ? [h('b.live', '●'), ' WebGL2'] : 'WebGL unavailable'),
      h('span', `${PM.perf.fps || '—'} fps`),
      h('span', `${PM.round(PM.perf.ms || 0, 1)} ms`),
      h('span', `${PM.proj.w}×${PM.proj.h} · ${PM.proj.fps} fps`),
    );
  };
  ['draw:status','layers','project','history','assets'].forEach(ev => PM.bus.on(ev, paint));
  setInterval(() => PM.invalidate('status'), 1000);
  paint();
}
buildTitlebar(); buildStatus();

/* ── Compact tool strip, integrated with the native macOS titlebar ── */
function buildToolbar() {
  const def = PM.PANELS.toolbar; if (!def) return;
  const el = document.createElement('div');
  el.id = 'toolbar-strip';
  const body = document.createElement('div');
  el.appendChild(body);
  const bar = document.getElementById('titlebar');
  /* The flexible drag region pushes this compact tool group to the right,
     immediately before the global actions. */
  bar.insertBefore(el, bar.querySelector('#tb-right'));
  try { def.build(body, {}); } catch (e) { console.error('toolbar', e); }
  const syncVisibility = () => { el.hidden = !!(PM.ProjectsScreen && PM.ProjectsScreen.isOpen); };
  PM.bus.on('projects:screen', syncVisibility);
  syncVisibility();
}
buildToolbar();

/* ── persistence ───────────────────────────────────────── */
/* Thumbnails are captured at most once per 5s so autosave never janks. */
let lastThumbAt = 0;
function projectThumb() {
  const now = Date.now();
  if (now - lastThumbAt < 5000) return undefined;
  lastThumbAt = now;
  try { return PM.Export.snapshot(PM.time, 320); } catch (e) { return undefined; }
}
function persistCurrent(withThumb) {
  try {
    PM.Projects.put(PM.proj, withThumb ? projectThumb() : undefined);
    APP.dirty = false;
  } catch (e) { console.warn('Project save failed', e); }
}

function captureProjectSession() {
  if (!PM.proj?.id) return;
  persistCurrent(false);
  PM.Projects.putState(PM.proj.id, {
    workspace: PM.WS.snapshot(), time: PM.time,
    selection: { layers: [...PM.sel.layers], keys: PM.sel.keys.filter(key => typeof key === 'string'), chan: PM.sel.chan },
    timeline: { pps: PM.TL.pps, scrollT: PM.TL.scrollT, scrollY: PM.TL.scrollY, graph: PM.TL.graph },
    detached: PM.Popout?.openIds?.() || [],
  });
}

function closeProjectTransients() {
  PM.LibraryUI?.close?.();
  PM.SpatialAssistant?.cancel?.();
  PM.closeMenus?.();
  if (PM.WS.editing) PM.WS.cancelEdit();
  PM.Popout?.closeAll?.();
}
PM.autosave = () => {
  APP.dirty = true;
  clearTimeout(APP.saveTimer);
  APP.saveTimer = setTimeout(() => {
    persistCurrent(true);
    PM.invalidate('status'); PM.bus.emit('project:saved'); PM.bus.emit('projects:tabs');
  }, 550);
};
['layers','project','assets','library'].forEach(ev => PM.bus.on(ev, PM.autosave));

PM.saveProject = async () => {
  const text = PM.serialize();
  if (APP.fileHandle && APP.fileHandle.createWritable) {
    try {
      const w = await APP.fileHandle.createWritable(); await w.write(text); await w.close();
      APP.dirty = false; PM.toast('Project saved'); PM.invalidate('status'); return;
    } catch (e) { console.warn(e); }
  }
  if (window.showSaveFilePicker) {
    try {
      APP.fileHandle = await showSaveFilePicker({ suggestedName: safeName(PM.proj.name) + '.pmv', types: [{ description: 'Powermove Project', accept: { 'application/json': ['.pmv'] } }] });
      const w = await APP.fileHandle.createWritable(); await w.write(text); await w.close();
      APP.dirty = false; PM.toast('Project saved'); PM.invalidate('status'); return;
    } catch (e) { if (e.name === 'AbortError') return; }
  }
  PM.download(new Blob([text], { type: 'application/json' }), safeName(PM.proj.name) + '.pmv');
  APP.dirty = false; PM.toast('Project downloaded'); PM.invalidate('status');
};
PM.openProject = () => {
  const inp = h('input', { type: 'file', accept: '.pmv,.json,application/json' });
  inp.onchange = async () => { const f = inp.files[0]; if (f) await openProjectFile(f); };
  inp.click();
};
async function openProjectFile(file) {
  try {
    const o = JSON.parse(await file.text());
    switchProject(hydrate(o.proj || o));
    PM.toast('Opened ' + file.name);
  } catch (e) { PM.toast('Could not open project: ' + e.message, 4500); }
}
PM.newProject = () => {
  const name = h('input', { value: 'Untitled' });
  PM.modal({ title: 'New composition', body: h('div.field', name), width: 400, actions: [
    { label: 'Cancel' }, { label: 'Create', pri: true, run: () => switchProject(PM.mkProject({ name: name.value.trim() || 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' })) },
  ] });
  setTimeout(() => { name.focus(); name.select(); }, 30);
};
function switchProject(p) {
  PM.pause();
  if (PM.proj?.id && PM.proj.id !== p.id) captureProjectSession();
  closeProjectTransients();
  PM.proj = hydrate(p);
  PM.Projects.markOpen(PM.proj.id);
  const session = PM.Projects.getState(PM.proj.id);
  if (session?.workspace) PM.WS.restoreSnapshot(session.workspace);
  else PM.WS.activate('design', true);
  persistCurrent(false);
  PM.bus.emit('projects:tabs');
  PM.time = Number.isFinite(session?.time) ? PM.clamp(session.time, 0, PM.proj.dur) : 0;
  PM.sel.layers = (session?.selection?.layers || []).filter(id => PM.L(id));
  PM.sel.keys = [...new Set((session?.selection?.keys || []).filter(key => typeof key === 'string'))];
  PM.sel.keys = PM.resolveSelectedKeys().map(key => key.i);
  PM.sel.chan = session?.selection?.chan || null;
  PM.TL.pps = Number.isFinite(session?.timeline?.pps) ? session.timeline.pps : 90;
  PM.TL.scrollT = Number.isFinite(session?.timeline?.scrollT) ? session.timeline.scrollT : 0;
  PM.TL.scrollY = Number.isFinite(session?.timeline?.scrollY) ? session.timeline.scrollY : 0;
  PM.TL.graph = !!session?.timeline?.graph;
  PM.hist.clear();
  PM.rasterClear();
  PM.assets.clear();
  APP.fileHandle = null;
  APP.dirty = true;
  PM.bus.emit('project');
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  PM.bus.emit('assets');
  PM.Inspector.refresh();
  PM.Viewer.layout();
  PM.invalidate('all');
  PM.invalidate('status');
  restoreProjectAssets(PM.proj);
  requestAnimationFrame(() => (session?.detached || []).forEach(id => PM.Popout.open(id)));
  PM.autosave();
}

/* ── media import ──────────────────────────────────────── */
PM.pickFiles = () => {
  const targetProject = PM.proj;
  const inp = h('input', {
    type: 'file', multiple: true, accept: 'image/*,video/*,audio/*,.pmv',
    style: { position: 'fixed', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none' },
  });
  const cleanup = () => { inp.onchange = null; inp.remove(); };
  inp.onchange = async () => {
    const files = [...inp.files];
    try { if (files.length) await PM.importFiles(files, { project: targetProject }); }
    finally { cleanup(); }
  };
  inp.addEventListener('cancel', cleanup, { once: true });
  document.body.appendChild(inp);
  inp.click();
};
async function importFiles(files) {
  const mediaFiles = [];
  for (const f of files) {
    if (/\.pmv$/i.test(f.name)) { await openProjectFile(f); continue; }
    if (!PM.assetKind(f)) { PM.toast('Unsupported file · ' + f.name); continue; }
    mediaFiles.push(f);
  }
  if (!mediaFiles.length) return;
  const importAt = PM.time;
  if (mediaFiles.length > 1) PM.toast(`Preparing ${mediaFiles.length} media files…`, 2400);
  const results = await PM.assets.importBatch(mediaFiles, {
    onProgress: progress => PM.bus.emit('import:progress', progress),
  });
  const failures = results.filter(result => result.status === 'failed');
  failures.forEach(result => PM.toast(result.error?.message || ('Could not import ' + result.file?.name), 5000));
  const layerResults = results.filter(result => result.status === 'created' || result.status === 'reused');
  const commands = layerResults.map(result => PM.commandForAsset(result.asset.id, importAt)).filter(Boolean);
  if (commands.length) {
    PM.Edit.apply(commands, {
      label: commands.length === 1 ? 'Import media' : `Import ${commands.length} media files`,
      origin: 'import',
    });
  }
  const relinked = results.filter(result => result.status === 'relinked');
  const volatile = results.filter(result => result.status !== 'failed' && !result.persisted);
  if (commands.length || relinked.length) {
    PM.autosave();
    const parts = [];
    if (commands.length) parts.push(commands.length === 1 ? `Imported ${layerResults[0].asset.name}` : `Imported ${commands.length} files`);
    if (relinked.length) parts.push(`relinked ${relinked.length} missing ${relinked.length === 1 ? 'asset' : 'assets'}`);
    if (volatile.length) parts.push('durable storage unavailable');
    PM.toast(parts.join(' · '), volatile.length ? 6000 : 3400);
  }
}
/* File pickers and drag/drop can fire while an earlier batch is still decoding.
   Preserve user order and project identity by serializing batches; each batch
   still performs its expensive work through the bounded parallel pool. */
PM.importFiles = (files, { project = PM.proj } = {}) => {
  const run = () => {
    if (PM.proj !== project) {
      PM.toast('Import stopped because you switched projects · import again in the intended project', 5000);
      return [];
    }
    return importFiles(Array.from(files || []));
  };
  APP.importQueue = APP.importQueue.then(run, run);
  return APP.importQueue;
};
addEventListener('dragover', e => { if ([...e.dataTransfer.types].includes('Files')) e.preventDefault(); });
addEventListener('drop', e => {
  if (![...e.dataTransfer.types].includes('Files')) return;
  e.preventDefault(); PM.importFiles([...e.dataTransfer.files]);
});

function safeName(s) { return String(s || 'powermove').replace(/[\\/:*?"<>|]+/g, '-').trim() || 'powermove'; }
addEventListener('beforeunload', () => {
  clearTimeout(APP.saveTimer);
  captureProjectSession();
});

/* ── open-project tabs ─────────────────────────────────── */
function openTab(id) {
  if (!id || id === PM.proj.id) return;
  const raw = PM.Projects.get(id);
  if (!raw) { PM.toast('That project could not be found'); PM.Projects.markClosed(id); PM.bus.emit('projects:tabs'); return; }
  switchProject(raw);
}
function closeTab(id) {
  if (id === PM.proj.id) persistCurrent(false);
  PM.Projects.markClosed(id);
  const rest = PM.Projects.tabs();
  if (id === PM.proj.id) {
    if (rest.length) openTab(rest[0]);
    else switchProject(PM.mkProject({ name: 'Untitled', dur: 10, w: 1920, h: 1080, fps: 30, bg: '#09090A' }));
  }
  PM.bus.emit('projects:tabs');
}
window.addEventListener('pm-open-project', (e) => {
  const p = e.detail;
  if (p && typeof p === 'object') switchProject(p);
});

/* boot registration: the startup project becomes the first tab */
PM.Projects.markOpen(PM.proj.id);
persistCurrent(false);
PM.bus.on('project:saved', () => PM.bus.emit('projects:tabs'));

/* first full frame after persistent panels have measured */
requestAnimationFrame(() => requestAnimationFrame(() => {
  PM.Viewer.layout();
  PM.TL.frameView();
  PM.bus.emit('layers');
  PM.bus.emit('sel');
  PM.Inspector.refresh();
  PM.invalidate('all');
  PM.invalidate('status');
  if (PM.SpatialAssistant) PM.SpatialAssistant.init();
  /* A brand-new registry entry has no card preview yet. Capture only after the
     first real composition frame has painted; capturing during boot produces a
     black placeholder even though the viewer becomes healthy a moment later. */
  requestAnimationFrame(() => {
    lastThumbAt = 0;
    persistCurrent(true);
    PM.bus.emit('projects:tabs');
  });
}));
})();
