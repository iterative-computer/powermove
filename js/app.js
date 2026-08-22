/* Powermove — application bootstrap, project I/O, shell, autosave. */
(() => {
const PM = window.PM, h = PM.h, $ = PM.$;
const APP = { fileHandle: null, dirty: false, saveTimer: 0 };
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
      if (L.parent === L.id || (typeof L.parent === 'string' && !container.layers.some(o => o.id === L.parent))) L.parent = null;
      const fresh = PM.mkLayer(L.type || 'null', {}, container);
      Object.keys(fresh.p).forEach(k => {
        if (!L.p[k] || typeof L.p[k] !== 'object') L.p[k] = fresh.p[k];
        sanitizeProp(L.p[k], fresh.p[k]);
      });
      Object.keys(L.p).forEach(k => { if (!(k in fresh.p)) delete L.p[k]; });
      L.fx = L.fx.filter(f => f && typeof f === 'object' && PM.FX && PM.FX[f.type]);
      L.fx.forEach(f => { f.id = f.id || PM.uid('fx'); f.p = f.p || {}; f.on = f.on !== false; });
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

PM.proj = loadBootProject();
/* External media blobs cannot survive a browser restart; keep metadata but hide unresolved layers. */
PM.proj.layers.forEach(L => {
  if ((L.type === 'image' || L.type === 'video' || L.type === 'audio') && L.d.asset && !PM.assets.get(L.d.asset)) L.on = false;
});
PM.WS.init();
PM.selectLayers(PM.proj.layers.find(l => l.name === 'Powermove')?.id || []);
PM.setTime(.9, { raw: true, force: true });
PM.hist.clear();

/* ── shell ─────────────────────────────────────────────── */
function themeButton(button) {
  const b = button(PM.theme.current === 'dark' ? 'sun' : 'moon', 'Toggle light / dark appearance', () => PM.theme.toggle());
  PM.bus.on('layout', () => {
    const dark = PM.theme.current === 'dark';
    b.textContent = '';
    b.appendChild(PM.icon(dark ? 'sun' : 'moon'));
    b.title = dark ? 'Switch to light appearance' : 'Switch to dark appearance';
  });
  return b;
}
function buildTitlebar() {
  const tabs = $('#tabs'), right = $('#tb-right'), bar = $('#titlebar');
  /* Native window drag: WKWebView ignores -webkit-app-region, so forward pointerdown
     on empty titlebar regions to the AppKit drag bridge. Interactive children opt out. */
  const dragBridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.windowDrag;
  bar.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    if (e.target.closest('button, #tabs, input, a, .tb-right')) return;
    if (!dragBridge) return;
    e.preventDefault();
    /* Native-feel window drag: pin the arrow cursor, drop any hover state, and
       swallow the gesture so the web content never shows grab/hand feedback. */
    document.body.style.cursor = 'default';
    if (document.activeElement && document.activeElement.blur) document.activeElement.blur();
    const pin = () => { document.body.style.cursor = 'default'; };
    window.addEventListener('pointermove', pin, true);
    dragBridge.postMessage({ x: e.clientX, y: e.clientY });
    window.addEventListener('pointerup', () => {
      window.removeEventListener('pointermove', pin, true);
      document.body.style.cursor = '';
    }, { once: true });
  });
  bar.addEventListener('dblclick', (e) => {
    if (e.target.closest('button, #tabs, input, a, .tb-right')) return;
    /* mimic standard macOS titlebar double-click (zoom) */
    const zb = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.windowZoom;
    if (zb) zb.postMessage({});
  });
  const paintTabs = () => {
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
      const tab = h('div.project-doc' + (active ? '.on' : '') + (dirty ? '.dirty' : ''), {
        title: (meta && meta.name) || id, role: 'tab', tabindex: '0',
        'aria-selected': String(active),
      },
        h('i.project-doc-state', { 'aria-hidden': 'true' }),
        h('span.project-doc-label', (meta && meta.name) || 'Untitled'),
        h('button.project-doc-close', {
          title: 'Close project',
          onclick: (e) => { e.stopPropagation(); closeTab(id); },
        }, PM.icon('x')));
      tab.onclick = () => {
        if (PM.ProjectsScreen && PM.ProjectsScreen.isOpen) PM.ProjectsScreen.hide();
        if (!active) openTab(id);
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
    button('undo', 'Undo', () => PM.hist.undo()),
    button('redo', 'Redo', () => PM.hist.redo()),
    button('plus', 'New layer', e => addMenu(e)),
    button('wand', 'New shader layer', () => PM.cmd('newShader')),
    button('export', 'Export', () => PM.Export.dialog()),
    (() => {
      const b = button('panel', 'Workspace · ' + PM.WS.current.name, e => workspaceMenu(e, b));
      PM.bus.on('workspaces', () => { b.title = 'Workspace · ' + PM.WS.current.name; });
      return b;
    })(),
    themeButton(button),
    button('gear', 'Workspace definition', () => PM.WS.editJSON()),
  );
  PM.bus.on('workspaces', paintTabs); PM.bus.on('project', paintTabs); PM.bus.on('history', paintTabs);
  paintTabs();
}
function workspaceMenu(e, anchor) {
  e.preventDefault();
  PM.menu(anchor, [
    { header: 'Workspaces' },
    ...PM.WS.list().map(w => ({ label: w.name, on: w.id === PM.WS.current.id, run: () => PM.WS.activate(w.id) })),
    '-', { label: 'Save current as new…', run: () => PM.WS.saveAsNew() },
    { label: 'Edit workspace JSON…', run: () => PM.WS.editJSON() },
  ]);
}
function addMenu(e) {
  const a = e.currentTarget;
  PM.menu(a, [
    { header: 'New layer' },
    { label: 'Text', kb: '⌘T', run: () => PM.cmd('newText') },
    { label: 'Shape', kb: '⌘⇧Y', run: () => PM.cmd('newShape') },
    { label: 'Solid', kb: '⌘Y', run: () => PM.cmd('newSolid') },
    { label: 'Shader', kb: '⌘⇧G', run: () => PM.cmd('newShader') },
    { label: 'Null', run: () => PM.cmd('newNull') },
    '-', { label: 'Import media…', kb: '⌘I', run: () => PM.pickFiles() },
  ], { right: true });
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

/* ── AE-style tool toolbar, mounted as a fixed strip above #body ── */
function buildToolbar() {
  const def = PM.PANELS.toolbar; if (!def) return;
  const el = document.createElement('div');
  el.id = 'toolbar-strip';
  const body = document.createElement('div');
  el.appendChild(body);
  const bodyEl = document.getElementById('body');
  bodyEl.parentNode.insertBefore(el, bodyEl);
  try { def.build(body, {}); } catch (e) { console.error('toolbar', e); }
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
  PM.proj = hydrate(p);
  PM.Projects.markOpen(PM.proj.id);
  persistCurrent(false);
  PM.bus.emit('projects:tabs');
  PM.time = 0;
  PM.sel.layers = [];
  PM.sel.keys = [];
  PM.sel.chan = null;
  PM.hist.clear();
  PM.rasterClear();
  PM.assets.map.clear();
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
  PM.autosave();
}

/* ── media import ──────────────────────────────────────── */
PM.pickFiles = () => {
  const inp = h('input', { type: 'file', multiple: true, accept: 'image/*,video/*,audio/*,.pmv' });
  inp.onchange = () => PM.importFiles([...inp.files]); inp.click();
};
PM.importFiles = async (files) => {
  for (const f of files) {
    if (/\.pmv$/i.test(f.name)) { await openProjectFile(f); continue; }
    if (!/^(image|video|audio)\//.test(f.type)) { PM.toast('Unsupported file · ' + f.name); continue; }
    const a = await PM.assets.add(f);
    PM.cmd('addFromAsset', a.id);
  }
  PM.autosave(); PM.toast(files.length === 1 ? 'Imported ' + files[0].name : `Imported ${files.length} files`);
};
addEventListener('dragover', e => { if ([...e.dataTransfer.types].includes('Files')) e.preventDefault(); });
addEventListener('drop', e => {
  if (![...e.dataTransfer.types].includes('Files')) return;
  e.preventDefault(); PM.importFiles([...e.dataTransfer.files]);
});

function safeName(s) { return String(s || 'powermove').replace(/[\\/:*?"<>|]+/g, '-').trim() || 'powermove'; }
addEventListener('beforeunload', () => {
  clearTimeout(APP.saveTimer);
  persistCurrent(false);
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
