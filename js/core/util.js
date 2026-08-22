/* Powermove — util: dom, bus, math, time, persistence. */
const PM = (window.PM = window.PM || {});

PM.version = '1.0.0';
PM.bootVersion = 20260816;

/* ── dom ───────────────────────────────────────────────── */
const h = (tag, attrs, ...kids) => {
  const parts = tag.split(/([.#])/);
  const el = document.createElement(parts[0] || 'div');
  for (let i = 1; i < parts.length; i += 2) {
    if (parts[i] === '.') el.classList.add(parts[i + 1]);
    else el.id = parts[i + 1];
  }
  if (attrs && (attrs.nodeType || typeof attrs === 'string')) { kids.unshift(attrs); attrs = null; }
  if (attrs) for (const k in attrs) {
    const v = attrs[k];
    if (v == null || v === false) continue;
    if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k === 'class') el.className += (el.className ? ' ' : '') + v;
    else if (k === 'html') el.innerHTML = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2).toLowerCase(), v);
    else if (k === 'data') for (const d in v) el.dataset[d] = v[d];
    else el.setAttribute(k, v === true ? '' : v);
  }
  const add = (k) => {
    if (k == null || k === false) return;
    if (Array.isArray(k)) return k.forEach(add);
    el.appendChild(k.nodeType ? k : document.createTextNode(k));
  };
  kids.forEach(add);
  return el;
};
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];
PM.h = h; PM.$ = $; PM.$$ = $$;

/* Native log bridge: surface webview errors on the app's stderr so headless
   debugging works without Safari Web Inspector. */
const __pmLog = (tag, args) => {
  try {
    const mh = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.pmLog;
    if (mh) mh.postMessage(tag + ' ' + args.map(a => {
      try { return a instanceof Error ? (a.message + '\n' + (a.stack || '')) : (typeof a === 'object' ? JSON.stringify(a) : String(a)); } catch { return String(a); }
    }).join(' '));
  } catch (e) { }
};
const __cerr = console.error.bind(console), __cwarn = console.warn.bind(console);
console.error = (...a) => { __pmLog('[error]', a); __cerr(...a); };
console.warn = (...a) => { __pmLog('[warn]', a); __cwarn(...a); };
addEventListener('error', (e) => __pmLog('[uncaught]', [e.message + ' @ ' + (e.filename || '') + ':' + (e.lineno || 0)]));
addEventListener('unhandledrejection', (e) => __pmLog('[rejection]', [String(e.reason)]));

PM.svg = (d, box = 24) => {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', `0 0 ${box} ${box}`);
  s.innerHTML = d;
  return s;
};
PM.icon = (name) => PM.svg(PM.ICONS[name] || PM.ICONS.dot);
PM.ICONS = {
  dot: '<circle cx="12" cy="12" r="3"/>',
  play: '<path d="M7 4.5v15l12-7.5z" fill="currentColor" stroke="none"/>',
  pause: '<path d="M8 5v14M16 5v14"/>',
  prev: '<path d="M18 5v14L8 12zM6 5v14"/>',
  next: '<path d="M6 5v14l10-7zM18 5v14"/>',
  home: '<path d="M4 11.5 12 4l8 7.5V20H4z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12H9"/>',
  redo: '<path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h5"/>',
  panel: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>',
  panelL: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
  grip: '<circle cx="9" cy="6" r="1.1" fill="currentColor" stroke="none"/><circle cx="15" cy="6" r="1.1" fill="currentColor" stroke="none"/><circle cx="9" cy="12" r="1.1" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="1.1" fill="currentColor" stroke="none"/><circle cx="9" cy="18" r="1.1" fill="currentColor" stroke="none"/><circle cx="15" cy="18" r="1.1" fill="currentColor" stroke="none"/>',
  up: '<path d="M12 19V5M5 12l7-7 7 7"/>',
  export: '<path d="M12 15V3M8 7l4-4 4 4"/><path d="M4 15v4a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-4"/>',
  clock: '<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>',
  chev: '<path d="m9 6 6 6-6 6"/>',
  chevD: '<path d="m6 9 6 6 6-6"/>',
  eye: '<path d="M2 12s3.6-6 10-6 10 6 10 6-3.6 6-10 6-10-6-10-6z"/><circle cx="12" cy="12" r="2.6"/>',
  lock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
  link: '<path d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1"/><path d="M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1"/>',
  magnet: '<path d="M6 4v7a6 6 0 0 0 12 0V4h-4v7a2 2 0 0 1-4 0V4z"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5z"/><path d="m3 13 9 5 9-5"/>',
  wand: '<path d="M15 4V2M15 10V8M12 6h-2M20 6h-2M17.5 3.5 16 5M17.5 8.5 16 7M12.5 3.5 14 5M4 20l9-9"/>',
  grid: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
  cursor: '<path d="M5 3l14 7-6 2-3 6z"/>',
  hand: '<path d="M8 11V5a1.5 1.5 0 0 1 3 0v5V4a1.5 1.5 0 0 1 3 0v6V5.5a1.5 1.5 0 0 1 3 0V13a7 7 0 0 1-7 7h-1c-2.5 0-4-1-5.2-3.4L3 13.5c-.7-1.2.9-2.4 1.9-1.4L8 15z"/>',
  zoom: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.5-4.5M8 11h6M11 8v6"/>',
  type: '<path d="M5 5h14M12 5v14M9 19h6"/>',
  shape: '<rect x="4" y="4" width="10" height="10" rx="1"/><circle cx="16" cy="16" r="4"/>',
  solid: '<rect x="4" y="4" width="16" height="16" rx="2" fill="currentColor" stroke="none" opacity=".9"/>',
  frame: '<path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4"/>',
  code: '<path d="m8 8-4 4 4 4M16 8l4 4-4 4"/>',
  cam: '<path d="M3 7h11v10H3z"/><path d="m14 11 7-4v10l-7-4z"/>',
  graph: '<path d="M3 20c6 0 6-16 18-16"/><circle cx="3" cy="20" r="1.6" fill="currentColor"/><circle cx="21" cy="4" r="1.6" fill="currentColor"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/>',
  sparkle: '<path d="m12 3 1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z"/><path d="M18.5 15.5 19 17l1.5.5L19 18l-.5 1.5L18 18l-1.5-.5L18 17z"/>',
  sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M19.1 4.9l-1.8 1.8M6.7 17.3l-1.8 1.8"/>',
  moon: '<path d="M20.5 14.5A8.5 8.5 0 1 1 9.5 3.5a7 7 0 0 0 11 11z"/>',
  eyeoff: '<path d="M4 4l16 16"/><path d="M9.9 5.3A9.8 9.8 0 0 1 12 5c6.4 0 10 6 10 6a17 17 0 0 1-3.4 3.9M6.6 7.6C3.9 9.2 2 12 2 12s3.6 6 10 6a10 10 0 0 0 3.6-.7"/>',
};

/* ── math ──────────────────────────────────────────────── */
const clamp = (v, a, b) => (v < a ? a : v > b ? b : v);
const lerp = (a, b, t) => a + (b - a) * t;
const round = (v, n = 2) => { const p = 10 ** n; return Math.round(v * p) / p; };
PM.clamp = clamp; PM.lerp = lerp; PM.round = round;
PM.uid = (p = 'l') => p + Math.random().toString(36).slice(2, 9);

PM.hex2rgb = (hex) => {
  const s = hex.replace('#', '');
  const n = parseInt(s.length === 3 ? s.split('').map(c => c + c).join('') : s, 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
};
PM.rgb2hex = (r, g, b) => '#' + [r, g, b].map(v => clamp(Math.round(v * 255), 0, 255).toString(16).padStart(2, '0')).join('');

/* ── time ──────────────────────────────────────────────── */
PM.tc = (sec, fps = 30, showFrames = true) => {
  const neg = sec < 0; sec = Math.abs(sec);
  let f = Math.round(sec * fps);
  const ff = f % fps; f = Math.floor(f / fps);
  const ss = f % 60; const mm = Math.floor(f / 60) % 60; const hh = Math.floor(f / 3600);
  const p = (n, w = 2) => String(n).padStart(w, '0');
  const base = (hh ? p(hh) + ':' : '') + p(mm) + ':' + p(ss);
  return (neg ? '-' : '') + base + (showFrames ? ':' + p(ff) : '');
};
PM.parseTc = (str, fps = 30) => {
  const parts = String(str).trim().split(':').map(Number);
  if (parts.some(isNaN)) return null;
  let s = 0;
  if (parts.length === 4) s = parts[0] * 3600 + parts[1] * 60 + parts[2] + parts[3] / fps;
  else if (parts.length === 3) s = parts[0] * 60 + parts[1] + parts[2] / fps;
  else if (parts.length === 2) s = parts[0] + parts[1] / fps;
  else s = parts[0];
  return s;
};
PM.snapF = (t, fps) => Math.round(t * fps) / fps;

/* ── event bus ─────────────────────────────────────────── */
const bus = { m: new Map() };
bus.on = (ev, fn) => { (bus.m.get(ev) || bus.m.set(ev, new Set()).get(ev)).add(fn); return () => bus.off(ev, fn); };
bus.off = (ev, fn) => { const s = bus.m.get(ev); if (s) s.delete(fn); };
bus.emit = (ev, a, b) => { const s = bus.m.get(ev); if (s) for (const fn of [...s]) { try { fn(a, b); } catch (e) { console.error('[bus]', ev, e); } } };
PM.bus = bus;

/* Coalesced invalidation — everything redraws on one rAF. */
const dirty = new Set();
let rafId = 0;
PM.invalidate = (what = 'all') => {
  if (what === 'all') { dirty.add('render'); dirty.add('ui'); dirty.add('timeline'); }
  else dirty.add(what);
  if (!rafId) rafId = requestAnimationFrame(flush);
};
function flush() {
  rafId = 0;
  const d = new Set(dirty); dirty.clear();
  if (d.has('render')) bus.emit('draw');
  if (d.has('timeline')) bus.emit('draw:timeline');
  if (d.has('ui')) bus.emit('draw:ui');
  if (d.has('status')) bus.emit('draw:status');
}

/* ── persistence ───────────────────────────────────────── */
PM.store = {
  get(k, d) { try { const v = localStorage.getItem('pm.' + k); return v == null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem('pm.' + k, JSON.stringify(v)); } catch (e) { console.warn('store', e); } },
  del(k) { localStorage.removeItem('pm.' + k); },
};

/* ── toasts ────────────────────────────────────────────── */
PM.toast = (msg, ms = 2200) => {
  const el = h('div.toast', msg);
  $('#toasts').appendChild(el);
  setTimeout(() => { el.style.opacity = '0'; el.style.transition = 'opacity .25s'; setTimeout(() => el.remove(), 260); }, ms);
};

/* ── floating menus ────────────────────────────────────── */
PM.menu = (anchor, items, opt = {}) => {
  PM.closeMenus();
  const el = h('div.drop');
  for (const it of items) {
    if (it === '-') { el.appendChild(h('div.sep')); continue; }
    if (it.header) { el.appendChild(h('div.hd', it.header)); continue; }
    el.appendChild(h('div.di' + (it.on ? '.on' : ''), {
      onclick: (e) => { e.stopPropagation(); PM.closeMenus(); it.run && it.run(); }
    }, it.on ? '✓ ' : '', it.label, it.kb ? h('span', { style: { marginLeft: 'auto', fontFamily: 'var(--f-mono)', fontSize: '10.5px', color: 'var(--tx-4)' } }, it.kb) : null));
  }
  document.body.appendChild(el);
  const r = anchor.getBoundingClientRect();
  const w = el.offsetWidth, ht = el.offsetHeight;
  let x = opt.x != null ? opt.x : (opt.right ? r.right - w : r.left);
  let y = opt.y != null ? opt.y : r.bottom + 5;
  el.style.left = clamp(x, 6, innerWidth - w - 6) + 'px';
  el.style.top = clamp(y, 6, innerHeight - ht - 6) + 'px';
  setTimeout(() => document.addEventListener('pointerdown', PM.closeMenus, { once: true }), 0);
  return el;
};
PM.closeMenus = () => $$('.drop').forEach(e => e.remove());

/* ── drag helper ───────────────────────────────────────── */
/* Returns { cancel }. cancel() and a native pointercancel both end the drag
   without calling up(), so interrupted gestures can never wedge the cursor
   or leave ghost UI behind. */
PM.drag = (e, { move, up, cancel, cursor }) => {
  e.preventDefault();
  const sx = e.clientX, sy = e.clientY;
  const prevCur = document.body.style.cursor;
  if (cursor) document.body.style.cursor = cursor;
  let done = false;
  const stop = () => {
    if (done) return false;
    done = true;
    window.removeEventListener('pointermove', mv);
    window.removeEventListener('pointerup', fin);
    window.removeEventListener('pointercancel', pc);
    document.body.style.cursor = prevCur;
    return true;
  };
  const mv = (ev) => { if (!done && move) move(ev.clientX - sx, ev.clientY - sy, ev); };
  const fin = (ev) => { if (stop() && up) up(ev.clientX - sx, ev.clientY - sy, ev); };
  const pc = () => { if (stop() && cancel) cancel(); };
  window.addEventListener('pointermove', mv);
  window.addEventListener('pointerup', fin);
  window.addEventListener('pointercancel', pc);
  return { cancel: () => { if (stop() && cancel) cancel(); } };
};

/* ── modal ─────────────────────────────────────────────── */
PM.modal = ({ title, body, actions = [], width = 460, onClose }) => {
  const scrim = $('#scrim'); scrim.classList.add('on');
  const el = h('div.modal', { style: { width: width + 'px', left: '50%', top: '18vh', transform: 'translateX(-50%)', maxHeight: '68vh' } });
  if (title) el.appendChild(h('h3', title));
  const mb = h('div.mb'); if (body) mb.appendChild(body); el.appendChild(mb);
  const close = () => { el.remove(); scrim.classList.remove('on'); scrim.onclick = null; onClose && onClose(); };
  if (actions.length) {
    el.appendChild(h('div.mf', ...actions.map(a =>
      h('button.btn' + (a.pri ? '.pri' : ''), { onclick: () => { if (a.run && a.run() === false) return; close(); } }, a.label))));
  }
  scrim.onclick = close;
  document.body.appendChild(el);
  return { el, close, body: mb };
};

PM.download = (blob, name) => {
  /* Native WKWebView has no browser download shelf — route through the AppKit save bridge. */
  const bridge = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.saveFile;
  if (bridge) {
    const reader = new FileReader();
    reader.onload = () => {
      const res = String(reader.result || '');
      const comma = res.indexOf(',');
      const data = comma >= 0 ? res.slice(comma + 1) : res;
      bridge.postMessage({ name: String(name || 'powermove.bin'), data });
    };
    reader.onerror = () => { if (PM.toast) PM.toast('Save failed'); };
    reader.readAsDataURL(blob);
    return;
  }
  const a = h('a', { href: URL.createObjectURL(blob), download: name });
  document.body.appendChild(a); a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 4000);
};
