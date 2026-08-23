/* Powermove — timeline: canvas-drawn tracks, keyframes, graph editor. */
(() => {
const PM = window.PM, h = PM.h, clamp = PM.clamp;

const T = {
  gut: 214, row: 30, ruler: 26, pps: 90, scrollT: 0, scrollY: 0,
  graph: false, rows: [], cv: null, ctx: null, w: 0, hgt: 0, dpr: 1,
  hover: null, marquee: null,
  style: { clipRadius: 6, keyframeSize: 8.8, showLayerNumbers: true, showTypeBadges: true, toolbarDensity: 'normal' },
};
PM.TL = T;

PM.registerPanel('timeline', {
  title: 'Timeline', flush: true, noscroll: true, persist: true, headless: true, size: 300, moveSlot: '#tl-head',
  build(body) {
    const head = h('div#tl-head');
    const wrap = h('div#tl-canvas-wrap');
    const cv = h('canvas#tl-canvas');
    wrap.appendChild(cv);
    body.append(head, wrap);
    /* Keep the backing store transparent while a host resize is in flight.
       An opaque 2D canvas is cleared to black as soon as its bitmap changes,
       which made the whole timeline flash/stick black while a section was
       being adjusted. The timeline still paints its own solid background. */
    T.cv = cv; T.ctx = cv.getContext('2d');
    refreshTimelineManifest();
    buildHead(head);
    bind(cv, wrap);
    new ResizeObserver(() => resize(wrap)).observe(wrap);
    requestAnimationFrame(() => resize(wrap));
  },
});

function buildHead(head) {
  const btn = (icon, fn, title) => h('button.iconbtn', { title, onclick: fn }, PM.icon(icon));
  const playBtn = btn('play', () => PM.toggle(), 'Play / Pause (Space)');
  const time = h('div#tl-time');
  const zoom = h('input', { type: 'range', min: 8, max: 900, value: T.pps, step: 1 });
  zoom.addEventListener('input', () => { T.pps = +zoom.value; PM.invalidate('timeline'); });
  const snap = h('button.iconbtn' + (PM.snap ? '.on' : ''), { title: 'Snapping (S)' }, PM.icon('magnet'));
  snap.onclick = () => { PM.snap = !PM.snap; snap.classList.toggle('on', PM.snap); };
  const graph = h('button.iconbtn' + (T.graph ? '.on' : ''), { title: 'Graph editor (G)' }, PM.icon('graph'));
  graph.onclick = () => { T.graph = !T.graph; graph.classList.toggle('on', T.graph); PM.invalidate('timeline'); };
  const loop = h('button.iconbtn' + (PM.loop ? '.on' : ''), { title: 'Loop' }, PM.icon('undo'));
  loop.onclick = () => { PM.loop = !PM.loop; loop.classList.toggle('on', PM.loop); };

  head.append(
    btn('prev', () => PM.setTime(prevEdge()), 'Previous edge'),
    playBtn,
    btn('next', () => PM.setTime(nextEdge()), 'Next edge'),
    time,
    h('span', { style: { flex: 1 } }),
    h('div.zoomrow', h('span', '−'), zoom, h('span', '+')),
    btn('frame', () => T.frameView(), 'Frame entire composition (⇧F)'),
    loop, snap, graph,
  );
  const sync = () => {
    time.textContent = PM.tc(PM.time, PM.proj.fps);
    playBtn.textContent = '';
    playBtn.appendChild(PM.icon(PM.playing ? 'pause' : 'play'));
  };
  PM.bus.on('time', sync); PM.bus.on('transport', sync); sync();
  time.addEventListener('pointerdown', (e) => {
    PM.drag(e, { cursor: 'ew-resize', move: (dx) => PM.setTime(PM.time + dx / 12 / PM.proj.fps) });
  });
}

function refreshTimelineManifest() {
  const raw = PM.WS?.current?.chrome?.timeline || {};
  const config = PM.WS?.normalizeTimelineChrome?.(raw) || {
    rowHeight: 30, gutterWidth: 214, rulerHeight: 26, clipRadius: 6,
    keyframeSize: 8.8, showLayerNumbers: true, showTypeBadges: true, toolbarDensity: 'normal',
  };
  T.row = config.rowHeight; T.gut = config.gutterWidth; T.ruler = config.rulerHeight;
  T.style = config;
  const head = PM.$('#tl-head');
  if (head) head.dataset.density = config.toolbarDensity;
  return config;
}

function resize(wrap) {
  wrap = wrap || (T.cv && T.cv.parentElement);
  if (!wrap || !T.cv) return;
  const r = wrap.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return; /* detached / mid-remount */
  T.dpr = Math.min(devicePixelRatio || 1, 2);
  T.w = r.width; T.hgt = r.height;
  const width = Math.max(2, Math.round(r.width * T.dpr));
  const height = Math.max(2, Math.round(r.height * T.dpr));
  const changed = T.cv.width !== width || T.cv.height !== height;
  if (T.cv.width !== width) T.cv.width = width;
  if (T.cv.height !== height) T.cv.height = height;
  /* Paint synchronously after the bitmap is cleared. ResizeObserver can run
     after an already-requested animation frame, so relying only on the shared
     invalidation queue can expose the canvas's cleared backing store. */
  if (changed) draw();
  PM.invalidate('timeline');
}
addEventListener('resize', () => resize());
PM.bus.on('layout:applied', () => {
  resize();
  /* Second pass after flex settles: the first layout:applied can fire while the
     dock is still animating to its final size (workspace switch, boot with a
     stale saved size), leaving the canvas showing a stale placeholder frame
     until the user nudges a splitter. */
  requestAnimationFrame(() => resize());
  setTimeout(() => resize(), 120);
});

/* ── row model ─────────────────────────────────────────── */
function buildRows() {
  const rows = [];
  const layers = PM.proj.layers;
  for (let i = 0; i < layers.length; i++) {
    const L = layers[i];
    if (L.shy) continue;
    rows.push({ kind: 'layer', L, i });
    if (!L.collapsed) {
      const props = PM.allProps(L).filter(p => p.prop.kf.length || p.prop.expr || PM.sel.chan === p.key || alwaysShow(L, p.key));
      props.forEach(p => rows.push({ kind: 'prop', L, ...p }));
    }
  }
  T.rows = rows;
  return rows;
}
function alwaysShow(L, key) { return L._reveal && L._reveal.includes(key); }

const x2t = (x) => (x - T.gut) / T.pps + T.scrollT;
const t2x = (t) => T.gut + (t - T.scrollT) * T.pps;
const rowY = (idx) => Math.round(T.ruler + idx * T.row - T.scrollY);

/* ── draw ──────────────────────────────────────────────── */
PM.bus.on('draw:timeline', draw);
PM.bus.on('layers', () => PM.invalidate('timeline'));
PM.bus.on('sel', () => PM.invalidate('timeline'));

function css(v) { return getComputedStyle(document.documentElement).getPropertyValue(v).trim(); }
let theme = null;
const refreshTheme = () => {
  const reversed = document.documentElement.dataset.timelineSurfaces === 'reversed';
  theme = {
    accent: css('--accent') || '#F0580A', tx: css('--tx') || '#1C1C1F',
    tx2: css('--tx-2') || '#5D5D65', tx3: css('--tx-3') || '#8B8B93',
    panel: css(reversed ? '--bg-sunken' : '--bg-panel') || '#FCFCFD',
    sunken: css(reversed ? '--bg-panel' : '--bg-sunken') || '#E4E4E7',
    line: css('--line') || 'rgba(15,15,20,.09)',
  };
};
PM.bus.on('layout', () => { refreshTimelineManifest(); refreshTheme(); refreshInk(); PM.invalidate('timeline'); });

/* Ink-on-paper colors for canvas chrome. Light theme uses black alpha;
   dark theme uses white alpha — resolved on every theme refresh. */
const ink = (a) => (document.documentElement.dataset.theme === 'dark' ? 'rgba(255,255,255,' : 'rgba(15,15,20,') + a + ')';
const INK = { over:'', over2:'', grid:'', tick:'', sub:'', hi:'', lo:'', thumb:'', key:'', handle:'', inv:'' };
function refreshInk() {
  const dark = document.documentElement.dataset.theme === 'dark';
  const a = (x) => (dark ? 'rgba(255,255,255,' : 'rgba(15,15,20,') + x + ')';
  INK.over = a('.045'); INK.over2 = a('.06'); INK.grid = a('.05'); INK.tick = a('.13');
  INK.sub = a('.32'); INK.hi = a('.72'); INK.lo = a('.16'); INK.thumb = a('.14');
  INK.key = dark ? '#d7d7db' : '#54545c';
  INK.handle = dark ? '#8f8f96' : '#8B8B93';
  INK.inv = dark ? '#fff' : '#1C1C1F';
}
refreshInk();

function draw() {
  try { drawInner(); } catch (e) { console.error('[timeline draw]', e, e.stack); }
}
function drawInner() {
  refreshTimelineManifest();
  /* Re-resolve the live canvas every frame: workspace rebuilds can replace the
     panel element, and drawing into a detached canvas is the root cause of
     gutter/clip misalignment after layout changes. */
  const liveCv = PM.$('#tl-canvas');
  if (liveCv && liveCv !== T.cv) { T.cv = liveCv; T.ctx = liveCv.getContext('2d'); }
  const c = T.ctx; if (!c) return;
  /* Unconditional size sync: measure the wrap every draw so the bitmap always
     matches the laid-out size, regardless of missed observer/RAF frames. */
  const host = T.cv && T.cv.parentElement;
  if (host) {
    const r = host.getBoundingClientRect();
    if (r.width >= 8 && r.height >= 8) {
      const bw = Math.max(2, Math.round(r.width * T.dpr));
      const bh = Math.max(2, Math.round(r.height * T.dpr));
      if (T.cv.width !== bw || T.cv.height !== bh || T.w !== r.width || T.hgt !== r.height) {
        T.w = r.width; T.hgt = r.height; T.cv.width = bw; T.cv.height = bh;
      }
    }
  }
  if (!theme) refreshTheme();
  const p = PM.proj;
  const W = T.w, H = T.hgt;
  c.setTransform(T.dpr, 0, 0, T.dpr, 0, 0);
  c.fillStyle = theme.panel; c.fillRect(0, 0, W, H);
  buildRows();

  const maxScroll = Math.max(0, T.rows.length * T.row - (H - T.ruler));
  T.scrollY = clamp(T.scrollY, 0, maxScroll);

  /* keep playhead in view while playing (AE follow) */
  if (PM.playing) {
    const px = t2x(PM.time);
    if (px > W - 50) T.scrollT = PM.time - (W - T.gut - 50) / T.pps;
    else if (px < T.gut) T.scrollT = PM.time - 60 / T.pps;
    T.scrollT = Math.max(-.4, T.scrollT);
  }

  drawTracksBg(c, W, H);
  if (T.graph) drawGraph(c, W, H);
  else drawClips(c, W, H);
  drawGutter(c, W, H);
  drawRuler(c, W, H);
  drawPlayhead(c, W, H);
  drawScrollThumb(c, W, H, maxScroll);
  if (window.__tlDebug) {
    const px = t2x(PM.time);
    const msg = '[tl] t=' + PM.time.toFixed(3) + ' px=' + (isFinite(px) ? px.toFixed(1) : String(px)) + ' gut=' + T.gut + ' W=' + W + ' sT=' + T.scrollT.toFixed(3) + ' pps=' + T.pps + ' rows=' + T.rows.length + ' graph=' + T.graph;
    try { window.webkit.messageHandlers.pmLog.postMessage(msg); } catch (e) { }
    console.log(msg);
  }
  if (T.marquee) {
    c.strokeStyle = theme.accent; c.fillStyle = 'rgba(255,107,26,.10)';
    const m = T.marquee;
    c.fillRect(m.x0, m.y0, m.x1 - m.x0, m.y1 - m.y0);
    c.strokeRect(m.x0, m.y0, m.x1 - m.x0, m.y1 - m.y0);
  }
}

/* Thin scrollbar on the right edge of the track area when rows overflow. */
function drawScrollThumb(c, W, H, maxScroll) {
  if (maxScroll <= 0) return;
  const trackH = H - T.ruler;
  const total = T.rows.length * T.row;
  const th = Math.max(18, trackH * trackH / total);
  const ty = T.ruler + (trackH - th) * (T.scrollY / maxScroll);
  c.fillStyle = INK.thumb;
  roundRect(c, W - 5, ty, 3, th, 1.5); c.fill();
}

/* After expanding/collapsing a layer, scroll just enough to keep the toggled
   row and its newly revealed children inside the viewport (AE behavior). */
function keepRowsVisible(rowIdx, childCount) {
  const viewH = T.hgt - T.ruler;
  if (viewH <= 0) return;
  const top = rowIdx * T.row;
  const bottom = top + (1 + childCount) * T.row;
  if (bottom - T.scrollY > viewH) T.scrollY = bottom - viewH;
  if (top < T.scrollY) T.scrollY = top;
  T.scrollY = Math.max(0, T.scrollY);
}

function drawTracksBg(c, W, H) {
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  c.fillStyle = theme.sunken; c.fillRect(T.gut, T.ruler, W - T.gut, H - T.ruler);
  /* work area */
  const p = PM.proj;
  const wa = p.work || [0, p.dur];
  c.fillStyle = INK.over;
  c.fillRect(t2x(wa[0]), T.ruler, (wa[1] - wa[0]) * T.pps, H - T.ruler);
  /* second gridlines */
  const step = niceStep(T.pps);
  c.strokeStyle = INK.grid; c.lineWidth = 1;
  c.beginPath();
  for (let t = Math.floor(T.scrollT / step) * step; t2x(t) < W; t += step) {
    const x = Math.round(t2x(t)) + .5;
    if (x < T.gut) continue;
    c.moveTo(x, T.ruler); c.lineTo(x, H);
  }
  c.stroke();
  /* row stripes */
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    if (r.kind === 'layer' && PM.sel.layers.includes(r.L.id)) {
      c.fillStyle = INK.over; c.fillRect(T.gut, y, W - T.gut, T.row);
    }
  }
  c.restore();
}

function niceStep(pps) {
  const targets = [1 / 30, 1 / 10, .2, .5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
  for (const s of targets) if (s * pps > 62) return s;
  return 600;
}

function drawRuler(c, W, H) {
  const p = PM.proj;
  c.fillStyle = theme.panel; c.fillRect(0, 0, W, T.ruler);
  c.strokeStyle = theme.line; c.beginPath(); c.moveTo(0, T.ruler - .5); c.lineTo(W, T.ruler - .5); c.stroke();
  c.save(); c.beginPath(); c.rect(T.gut, 0, W - T.gut, T.ruler); c.clip();
  const step = niceStep(T.pps);
  c.font = '500 10px ' + fmono();
  c.fillStyle = theme.tx3; c.textBaseline = 'middle';
  c.strokeStyle = INK.tick;
  c.beginPath();
  for (let t = Math.floor(T.scrollT / step) * step; t2x(t) < W; t += step) {
    const x = Math.round(t2x(t)) + .5;
    if (x < T.gut - 1) continue;
    c.moveTo(x, T.ruler - 6); c.lineTo(x, T.ruler);
    c.fillText(fmtRuler(t, step, p.fps), x + 4, T.ruler / 2 - 1);
  }
  c.stroke();
  /* work area handles */
  const wa = p.work || [0, p.dur];
  c.fillStyle = INK.sub;
  c.fillRect(t2x(wa[0]) - 1, 2, 3, 8); c.fillRect(t2x(wa[1]) - 1, 2, 3, 8);
  c.restore();
}
function fmtRuler(t, step, fps) {
  if (step < 1) return PM.tc(t, fps).slice(-5);
  const m = Math.floor(t / 60), s = Math.round(t % 60);
  return (m ? m + ':' : '0:') + String(s).padStart(2, '0');
}
const fui = () => '"Geist",-apple-system,system-ui,sans-serif';
const fmono = () => '"Geist Mono",ui-monospace,Menlo,monospace';

function drawClips(c, W, H) {
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    if (r.kind === 'layer') drawClip(c, r.L, y);
    else drawPropKeys(c, r, y);
  }
  c.restore();
}

function rgba(hex, a) { const [r, g, b] = PM.hex2rgb(hex); return `rgba(${r * 255 | 0},${g * 255 | 0},${b * 255 | 0},${a})`; }

const BADGE = { text: 'T', shape: 'S', solid: 'S', shader: 'fx', null: 'N', image: 'img', video: 'vid', audio: 'aud' };
const layerLabel = (L) => (T.style.showTypeBadges && BADGE[L.type] ? BADGE[L.type] + ' ' : '') + L.name;

function drawClip(c, L, y) {
  const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
  if (x1 < T.gut || x0 > T.w) return;
  const hh = T.row - 7;
  const yy = y + 3.5;
  const sel = PM.sel.layers.includes(L.id);
  const r = T.style.clipRadius;
  /* Label contrast from actual chip luminance, so it works in both themes:
     dark text on light chips (cream/sand/yellow/gray), white text on dark chips. */
  const [cr, cg, cb] = PM.hex2rgb(L.color);
  const lum = .2126 * cr + .7152 * cg + .0722 * cb;   /* 0..1 */
  const darkText = lum > .55;
  c.save();
  roundRect(c, x0, yy, Math.max(4, x1 - x0), hh, r);
  c.fillStyle = L.on ? L.color : rgba(L.color, .38);
  c.fill();
  if (sel) { c.strokeStyle = INK.inv; c.lineWidth = 1.4; c.stroke(); }
  c.clip();
  /* waveform-ish texture for audio */
  if (L.type === 'audio') {
    c.fillStyle = 'rgba(255,255,255,.4)';
    const step = 3;
    for (let x = Math.max(x0, T.gut); x < x1; x += step) {
      const n = Math.abs(Math.sin(x * .21) * Math.sin(x * .073) * Math.sin(x * .0131));
      const bh = 3 + n * (hh - 8);
      c.fillRect(x, yy + (hh - bh) / 2, 1.4, bh);
    }
  }
  c.fillStyle = darkText ? 'rgba(20,20,24,.92)' : 'rgba(255,255,255,.96)';
  c.font = '560 11.5px ' + fui();
  c.textBaseline = 'middle';
  const label = layerLabel(L);
  c.fillText(label, Math.max(x0, T.gut) + 9, yy + hh / 2);
  c.restore();
}

function drawPropKeys(c, r, y) {
  const L = r.L, cy = y + T.row / 2;
  c.strokeStyle = INK.grid;
  c.beginPath(); c.moveTo(T.gut, cy); c.lineTo(T.w, cy); c.stroke();
  const kf = r.prop.kf;
  const keyRadius = T.style.keyframeSize / 2;
  for (const k of kf) {
    const x = t2x(L.from + k.t);
    if (x < T.gut - 6 || x > T.w + 6) continue;
    const sel = PM.sel.keys.some(s => s.i === k.i);
  c.fillStyle = sel ? theme.accent : INK.key;
    if (k.hold) { c.fillRect(x - keyRadius, cy - keyRadius, keyRadius * 2, keyRadius * 2); }
    else {
      c.beginPath(); c.moveTo(x, cy - keyRadius); c.lineTo(x + keyRadius, cy); c.lineTo(x, cy + keyRadius); c.lineTo(x - keyRadius, cy); c.fill();
    }
  }
}

function drawGutter(c, W, H) {
  c.fillStyle = theme.panel;
  c.fillRect(0, T.ruler, T.gut, H - T.ruler);
  c.strokeStyle = theme.line; c.beginPath();
  c.moveTo(T.gut - .5, 0); c.lineTo(T.gut - .5, H); c.stroke();
  c.save(); c.beginPath(); c.rect(0, T.ruler, T.gut, H - T.ruler); c.clip();
  for (let i = 0; i < T.rows.length; i++) {
    const y = rowY(i);
    if (y + T.row < T.ruler || y > H) continue;
    const r = T.rows[i];
    if (r.kind === 'layer') {
      const L = r.L, sel = PM.sel.layers.includes(L.id);
      if (sel) { c.fillStyle = INK.over2; c.fillRect(0, y, T.gut, T.row); }
      c.font = '500 11px ' + fmono();
      c.fillStyle = theme.tx3; c.textBaseline = 'middle';
      if (T.style.showLayerNumbers) c.fillText(String(r.i + 1).padStart(2, '0'), 8, y + T.row / 2);
      /* eye / lock */
      icoEye(c, 30, y + T.row / 2, L.on);
      icoLock(c, 48, y + T.row / 2, L.lock);
      if (L.solo) { c.fillStyle = theme.accent; c.beginPath(); c.arc(64, y + T.row / 2, 3, 0, 7); c.fill(); }
      /* twirl */
      c.save();
      c.translate(76, y + T.row / 2); c.rotate(L.collapsed ? 0 : Math.PI / 2);
      c.strokeStyle = theme.tx3; c.lineWidth = 1.4; c.beginPath();
      c.moveTo(-1.6, -3.4); c.lineTo(2, 0); c.lineTo(-1.6, 3.4); c.stroke();
      c.restore();
      /* color chip + name */
      c.fillStyle = L.color; roundRect(c, 86, y + T.row / 2 - 6, 3, 12, 1.5); c.fill();
      c.font = (sel ? '560 ' : '450 ') + '11.5px ' + fui();
      c.fillStyle = sel ? theme.tx : theme.tx2;
      clipText(c, L.name, 96, y + T.row / 2, T.gut - 130);
      if (L.parent) { c.fillStyle = theme.tx3; c.font = '400 9.5px ' + fmono(); c.fillText('↳', T.gut - 24, y + T.row / 2); }
      if (L.mblur) { c.fillStyle = theme.accent; c.font = '500 8.5px ' + fmono(); c.fillText('MB', T.gut - 15, y + T.row / 2); }
    } else {
      const L = r.L;
      c.font = '450 11px ' + fui();
      c.fillStyle = PM.sel.chan === r.key ? theme.accent : theme.tx3;
      clipText(c, r.label, 112, y + T.row / 2, T.gut - 150);
      /* value at playhead */
      const v = PM.evP(L, r.prop, PM.time, r.key);
      c.font = '400 10px ' + fmono();
      c.fillStyle = INK.sub;
      c.textAlign = 'right';
      c.fillText(typeof v === 'number' ? PM.round(v, 1) : String(v).slice(0, 8), T.gut - 8, y + T.row / 2);
      c.textAlign = 'left';
      if (r.prop.expr) { c.fillStyle = theme.accent; c.fillText('ƒ', 100, y + T.row / 2); }
    }
  }
  c.restore();
}
function clipText(c, s, x, y, max) {
  let t = s;
  if (c.measureText(t).width > max) { while (t.length > 3 && c.measureText(t + '…').width > max) t = t.slice(0, -1); t += '…'; }
  c.fillText(t, x, y);
}
function icoEye(c, x, y, on) {
  c.strokeStyle = on ? INK.hi : INK.lo;
  c.lineWidth = 1.1; c.beginPath();
  c.ellipse(x, y, 5, 3.2, 0, 0, 7); c.stroke();
  if (on) { c.fillStyle = INK.hi; c.beginPath(); c.arc(x, y, 1.5, 0, 7); c.fill(); }
}
function icoLock(c, x, y, on) {
  c.strokeStyle = on ? theme.accent : INK.lo;
  c.lineWidth = 1.1;
  c.strokeRect(x - 3.4, y - 1, 6.8, 5);
  c.beginPath(); c.arc(x, y - 1, 2.4, Math.PI, 0); c.stroke();
}
function roundRect(c, x, y, w, hh, r) {
  r = Math.min(r, Math.abs(w) / 2, Math.abs(hh) / 2);
  c.beginPath();
  c.moveTo(x + r, y); c.arcTo(x + w, y, x + w, y + hh, r);
  c.arcTo(x + w, y + hh, x, y + hh, r); c.arcTo(x, y + hh, x, y, r);
  c.arcTo(x, y, x + w, y, r); c.closePath();
}

function drawPlayhead(c, W, H) {
  const x = t2x(PM.time);
  if (x < T.gut) return;
  c.save(); c.beginPath(); c.rect(T.gut, 0, W - T.gut, H); c.clip();
  c.strokeStyle = '#3E7BFA'; c.lineWidth = 1.5;
  c.beginPath(); c.moveTo(x, 0); c.lineTo(x, H); c.stroke();
  c.fillStyle = '#3E7BFA';
  roundRect(c, x - 5, 0, 10, 9, 2); c.fill();
  c.restore();
}

/* ── graph editor ──────────────────────────────────────── */
function drawGraph(c, W, H) {
  const rows = T.rows.filter(r => r.kind === 'prop' && (PM.sel.chan === r.key || r.prop.kf.length));
  const target = rows.find(r => PM.sel.chan === r.key) || rows[0];
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  if (!target) {
    c.fillStyle = theme.tx3; c.font = '400 11.5px ' + fui(); c.textAlign = 'center';
    c.fillText('Select an animated property to edit its curve', (W + T.gut) / 2, (H + T.ruler) / 2);
    c.textAlign = 'left'; c.restore(); return;
  }
  const kf = target.prop.kf, L = target.L;
  let vmin = Infinity, vmax = -Infinity;
  kf.forEach(k => { vmin = Math.min(vmin, k.v); vmax = Math.max(vmax, k.v); });
  if (!isFinite(vmin)) { vmin = 0; vmax = 1; }
  if (vmax - vmin < 1e-6) { vmax = vmin + 1; }
  const padv = (vmax - vmin) * .22;
  vmin -= padv; vmax += padv;
  const top = T.ruler + 16, bot = H - 16;
  const v2y = (v) => bot - (v - vmin) / (vmax - vmin) * (bot - top);
  T._graph = { target, vmin, vmax, v2y, y2v: (y) => vmin + (bot - y) / (bot - top) * (vmax - vmin) };

  /* value gridlines */
  c.strokeStyle = INK.grid; c.font = '400 9.5px ' + fmono(); c.fillStyle = theme.tx3;
  for (let i = 0; i <= 4; i++) {
    const v = vmin + (vmax - vmin) * i / 4, y = Math.round(v2y(v)) + .5;
    c.beginPath(); c.moveTo(T.gut, y); c.lineTo(W, y); c.stroke();
    c.fillText(PM.round(v, 1), T.gut + 5, y - 4);
  }
  /* curve */
  c.strokeStyle = theme.accent; c.lineWidth = 1.8;
  c.beginPath();
  const x0 = Math.max(T.gut, t2x(L.from)), x1 = Math.min(W, t2x(L.from + L.dur));
  for (let x = x0; x <= x1; x += 1.5) {
    const tl = x2t(x) - L.from;
    const v = PM.evalKfs(kf, tl);
    const y = v2y(v == null ? 0 : v);
    x === x0 ? c.moveTo(x, y) : c.lineTo(x, y);
  }
  c.stroke();
  /* handles + keys */
  kf.forEach((k, i) => {
    const x = t2x(L.from + k.t), y = v2y(k.v);
    const nx = kf[i + 1], pv = kf[i - 1];
    c.strokeStyle = INK.sub; c.lineWidth = 1;
    if (nx) {
      const hx = x + (t2x(L.from + nx.t) - x) * k.eo[0];
      const hy = y + (v2y(nx.v) - y) * k.eo[1];
      c.beginPath(); c.moveTo(x, y); c.lineTo(hx, hy); c.stroke();
      c.fillStyle = INK.handle; c.beginPath(); c.arc(hx, hy, 3, 0, 7); c.fill();
      k._ho = [hx, hy];
    }
    if (pv) {
      const px = t2x(L.from + pv.t), py = v2y(pv.v);
      const hx = px + (x - px) * k.ei[0], hy = py + (y - py) * k.ei[1];
      c.beginPath(); c.moveTo(x, y); c.lineTo(hx, hy); c.stroke();
      c.fillStyle = INK.handle; c.beginPath(); c.arc(hx, hy, 3, 0, 7); c.fill();
      k._hi = [hx, hy];
    }
    const sel = PM.sel.keys.some(s => s.i === k.i);
    c.fillStyle = sel ? theme.accent : INK.inv;
    c.beginPath(); c.arc(x, y, 4.2, 0, 7); c.fill();
    k._pt = [x, y];
  });
  c.fillStyle = theme.tx2; c.font = '500 11px ' + fui();
  c.fillText(L.name + ' · ' + target.label, T.gut + 10, T.ruler + 12);
  c.restore();
}

/* ── interaction ───────────────────────────────────────── */
function hitRow(y) {
  if (y < T.ruler) return null;
  const i = Math.floor((y - T.ruler + T.scrollY) / T.row);
  return T.rows[i] ? { row: T.rows[i], i } : null;
}

function bind(cv, wrap) {
  cv.addEventListener('pointerdown', onDown);
  cv.addEventListener('pointermove', onMove);
  cv.addEventListener('dblclick', onDbl);
  cv.addEventListener('contextmenu', onCtx);
  cv.addEventListener('wheel', (e) => {
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      const tAt = x2t(e.offsetX);
      T.pps = clamp(T.pps * (1 - e.deltaY * .004), 4, 4000);
      T.scrollT = tAt - (e.offsetX - T.gut) / T.pps;
    } else if (e.shiftKey) {
      T.scrollT += e.deltaY / T.pps;
    } else {
      T.scrollY += e.deltaY;
      T.scrollT += e.deltaX / T.pps;
    }
    T.scrollT = Math.max(-.4, T.scrollT);
    PM.invalidate('timeline');
  }, { passive: false });
}

function onMove(e) {
  const x = e.offsetX, y = e.offsetY;
  let cur = 'default';
  if (x > T.gut) {
    const hr = hitRow(y);
    if (hr && hr.row.kind === 'layer') {
      const L = hr.row.L;
      const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
      if (Math.abs(x - x0) < 5 || Math.abs(x - x1) < 5) cur = 'ew-resize';
      else if (x > x0 && x < x1) cur = 'grab';
    }
    if (y < T.ruler) cur = 'ew-resize';
  }
  T.cv.style.cursor = cur;
}

function onDown(e) {
  const x = e.offsetX, y = e.offsetY;
  PM.closeMenus();
  if (y < T.ruler && x > T.gut) {
    const wa = PM.proj.work || [0, PM.proj.dur];
    const x0 = t2x(wa[0]), x1 = t2x(wa[1]);
    if (Math.abs(x - x0) < 6) return workAreaDrag(e, 0);
    if (Math.abs(x - x1) < 6) return workAreaDrag(e, 1);
    return scrub(e);
  }
  if (x < T.gut) return gutterDown(e, x, y);
  if (T.graph) return graphDown(e, x, y);
  const hr = hitRow(y);
  if (!hr) { PM.selectLayers([]); return marquee(e); }
  const r = hr.row;
  if (r.kind === 'prop') return keyDown(e, r, x, y, hr.i);
  const L = r.L;
  const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
  if (!PM.sel.layers.includes(L.id)) PM.selectLayers(L.id, e.shiftKey || e.metaKey);
  else if (e.shiftKey) PM.selectLayers(L.id, true);
  if (L.lock) return;
  if (Math.abs(x - x0) < 5) return trim(e, 'in');
  if (Math.abs(x - x1) < 5) return trim(e, 'out');
  if (x > x0 && x < x1) return slide(e);
  scrub(e);
}

function workAreaDrag(e, idx) {
  PM.Edit.begin('Work area', { origin: 'timeline' });
  PM.drag(e, {
    cursor: 'ew-resize',
    move: (dx, dy, ev) => {
      const r = T.cv.getBoundingClientRect();
      let t = Math.max(0, PM.snapF(x2t(ev.clientX - r.left), PM.proj.fps));
      const wa = [...PM.proj.work];
      wa[idx] = t;
      if (wa[0] > wa[1]) { const o = wa[0]; wa[0] = wa[1]; wa[1] = o; idx = 1 - idx; }
      if (wa[1] > wa[0]) PM.Edit.dispatch({ type: 'set_composition', patch: { workArea: wa } });
    },
    up: () => PM.Edit.commit('Work area'),
    cancel: () => PM.Edit.cancel(),
  });
}

function scrub(e) {
  const set = (ev) => {
    const r = T.cv.getBoundingClientRect();
    PM.setTime(x2t(ev.clientX - r.left));
  };
  set(e);
  PM.drag(e, { move: (dx, dy, ev) => set(ev) });
}

function gutterDown(e, x, y) {
  const hr = hitRow(y);
  if (!hr) return;
  const r = hr.row;
  if (r.kind !== 'layer') { PM.sel.chan = r.key; PM.selectLayers(r.L.id); PM.invalidate('timeline'); return; }
  const L = r.L;
  if (x < 22) { }
  else if (x < 40) { PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { visible: !L.on } }, { label: 'Toggle visibility', origin: 'timeline' }); return; }
  else if (x < 58) { PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { locked: !L.lock } }, { label: 'Toggle lock', origin: 'timeline' }); return; }
  else if (x < 72) { PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { solo: !L.solo } }, { label: 'Toggle solo', origin: 'timeline' }); return; }
  else if (x < 86) {
    L.collapsed = !L.collapsed;
    if (!L.collapsed) {
      const kids = PM.allProps(L).filter(pp => pp.prop.kf.length || pp.prop.expr || PM.sel.chan === pp.key || alwaysShow(L, pp.key)).length;
      keepRowsVisible(hr.i, kids);
    }
    PM.invalidate('timeline');
    return;
  }
  PM.selectLayers(L.id, e.shiftKey || e.metaKey);
  /* drag to reorder */
  const startIdx = PM.proj.layers.indexOf(L);
  let done = false;
  PM.drag(e, {
    move: (dx, dy) => {
      const delta = Math.round(dy / T.row);
      const to = clamp(startIdx + delta, 0, PM.proj.layers.length - 1);
      if (to !== PM.proj.layers.indexOf(L)) {
        if (!done) { PM.Edit.begin('Reorder layer', { origin: 'timeline' }); done = true; }
        PM.Edit.dispatch({ type: 'reorder_layer', target: L.id, index: to });
      }
    },
    up: () => { if (done) PM.Edit.commit('Reorder layer'); },
    cancel: () => { if (done) PM.Edit.cancel(); },
  });
}

function slide(e) {
  const layers = PM.selLayers().filter(l => !l.lock);
  const start = layers.map(L => ({ L, from: L.from }));
  PM.Edit.begin('Move clip', { origin: 'timeline' });
  let moved = false;
  PM.drag(e, {
    cursor: 'grabbing',
    move: (dx) => {
      moved = true;
      let dt = dx / T.pps;
      if (PM.snap) dt = snapDelta(start, dt);
      start.forEach(s => PM.Edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { from: Math.max(0, PM.snapF(s.from + dt, PM.proj.fps)) } }));
    },
    up: () => { moved ? PM.Edit.commit('Move clip') : PM.Edit.cancel(); },
    cancel: () => PM.Edit.cancel(),
  });
}
function snapDelta(start, dt) {
  const pts = [PM.time, 0, PM.proj.dur, ...(PM.proj.work || []), ...PM.proj.markers.map(m => m.t)];
  PM.proj.layers.forEach(L => { if (!start.some(s => s.L === L)) { pts.push(L.from, L.from + L.dur); } });
  const tol = 8 / T.pps;
  let best = dt, bd = tol;
  for (const s of start) for (const edge of [s.from + dt, s.from + s.L.dur + dt]) {
    for (const p of pts) { const d = Math.abs(edge - p); if (d < bd) { bd = d; best = dt + (p - edge); } }
  }
  return best;
}

function trim(e, side) {
  const layers = PM.selLayers().filter(l => !l.lock);
  const start = layers.map(L => ({ L, from: L.from, dur: L.dur, trim: Number(L.d && L.d.trim) || 0 }));
  PM.Edit.begin('Trim clip', { origin: 'timeline' });
  let moved = false;
  PM.drag(e, {
    cursor: 'ew-resize',
    move: (dx) => {
      moved = true;
      const dt = PM.snapF(dx / T.pps, PM.proj.fps);
      start.forEach(s => {
        if (side === 'in') {
          let nf = clamp(s.from + dt, 0, s.from + s.dur - 1 / PM.proj.fps);
          if (PM.MediaTiming.isTimed(s.L)) nf = Math.max(nf, PM.MediaTiming.earliestStart({ ...s.L, from: s.from, d: { ...s.L.d, trim: s.trim } }));
          PM.Edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { from: nf, duration: s.dur + (s.from - nf) } });
          if (PM.MediaTiming.isTimed(s.L)) {
            const rate = PM.MediaTiming.rate(s.L);
            PM.Edit.dispatch({ type: 'set_content', target: s.L.id, patch: { trim: Math.max(0, s.trim + (nf - s.from) * rate) } });
          }
        } else PM.Edit.dispatch({ type: 'set_layer', target: s.L.id, patch: { duration: Math.max(1 / PM.proj.fps, s.dur + dt) } });
      });
    },
    up: () => { moved ? PM.Edit.commit('Trim clip') : PM.Edit.cancel(); },
    cancel: () => PM.Edit.cancel(),
  });
}

function trimInCommands(L, from) {
  const commands = [{ type: 'set_layer', target: L.id, patch: { from, duration: L.dur - (from - L.from) } }];
  if (PM.MediaTiming.isTimed(L)) commands.push({ type: 'set_content', target: L.id, patch: { trim: PM.MediaTiming.trimAtStart(L, from) } });
  return commands;
}

function keyDown(e, r, x, y, rowIdx) {
  PM.sel.chan = r.key;
  PM.selectLayers(r.L.id);
  const hit = r.prop.kf.find(k => Math.abs(t2x(r.L.from + k.t) - x) < 6);
  if (!hit) { PM.sel.keys = []; PM.invalidate('timeline'); return scrub(e); }
  if (e.shiftKey) PM.sel.keys.push(hit);
  else if (!PM.sel.keys.some(k => k.i === hit.i)) PM.sel.keys = [hit];
  const keys = PM.sel.keys.length ? PM.sel.keys : [hit];
  const start = keys.map(k => ({ k, t: k.t }));
  PM.hist.begin('Move keyframe');
  let moved = false;
  PM.drag(e, {
    move: (dx) => {
      moved = true;
      const dt = PM.snapF(dx / T.pps, PM.proj.fps);
      start.forEach(s => { s.k.t = Math.max(0, s.t + dt); });
      r.prop.kf.sort((a, b) => a.t - b.t);
      PM.touch(); PM.invalidate();
    },
    up: () => { moved ? PM.hist.commit('Move keyframe') : PM.hist.cancel(); PM.invalidate('timeline'); },
    cancel: () => { PM.hist.cancel(); PM.invalidate('timeline'); },
  });
}

function graphDown(e, x, y) {
  const g = T._graph; if (!g) return;
  const kf = g.target.prop.kf, L = g.target.L;
  for (const k of kf) {
    if (k._ho && Math.hypot(x - k._ho[0], y - k._ho[1]) < 7) return dragHandle(e, k, 'eo', g, kf, L);
    if (k._hi && Math.hypot(x - k._hi[0], y - k._hi[1]) < 7) return dragHandle(e, k, 'ei', g, kf, L);
  }
  const hit = kf.find(k => k._pt && Math.hypot(x - k._pt[0], y - k._pt[1]) < 8);
  if (!hit) return scrub(e);
  PM.sel.keys = e.shiftKey ? [...PM.sel.keys, hit] : [hit];
  const start = { t: hit.t, v: hit.v };
  PM.hist.begin('Edit curve');
  PM.drag(e, {
    move: (dx, dy) => {
      hit.t = Math.max(0, PM.snapF(start.t + dx / T.pps, PM.proj.fps));
      hit.v = PM.round(start.v + (g.y2v(0) - g.y2v(dy)) * -1, 3);
      kf.sort((a, b) => a.t - b.t);
      PM.touch(); PM.invalidate();
    },
    up: () => PM.hist.commit('Edit curve'),
    cancel: () => PM.hist.cancel(),
  });
}
function dragHandle(e, k, which, g, kf, L) {
  const i = kf.indexOf(k);
  const other = which === 'eo' ? kf[i + 1] : kf[i - 1];
  if (!other) return;
  const ax = t2x(L.from + k.t), ay = g.v2y(k.v);
  const bx = t2x(L.from + other.t), by = g.v2y(other.v);
  const start = [...k[which]];
  PM.hist.begin('Adjust easing');
  PM.drag(e, {
    move: (dx, dy) => {
      const nx = clamp(start[0] + dx / (bx - ax || 1), 0, 1);
      const ny = start[1] + dy / (by - ay || 1);
      k[which] = [PM.round(nx, 3), PM.round(ny, 3)];
      PM.touch(); PM.invalidate();
    },
    up: () => PM.hist.commit('Adjust easing'),
    cancel: () => PM.hist.cancel(),
  });
}

function marquee(e) {
  const r = T.cv.getBoundingClientRect();
  const x0 = e.clientX - r.left, y0 = e.clientY - r.top;
  PM.drag(e, {
    move: (dx, dy) => {
      T.marquee = { x0: Math.min(x0, x0 + dx), y0: Math.min(y0, y0 + dy), x1: Math.max(x0, x0 + dx), y1: Math.max(y0, y0 + dy) };
      PM.invalidate('timeline');
    },
    up: () => {
      if (T.marquee) {
        const m = T.marquee;
        const picked = [];
        T.rows.forEach((row, i) => {
          const y = rowY(i);
          if (y + T.row < m.y0 || y > m.y1) return;
          if (row.kind === 'layer') {
            const a = t2x(row.L.from), b = t2x(row.L.from + row.L.dur);
            if (b > m.x0 && a < m.x1) picked.push(row.L.id);
          }
        });
        if (picked.length) PM.selectLayers(picked);
      }
      T.marquee = null; PM.invalidate('timeline');
    },
    cancel: () => { T.marquee = null; PM.invalidate('timeline'); },
  });
}

function onDbl(e) {
  const x = e.offsetX, y = e.offsetY;
  if (x < T.gut && y > T.ruler) {
    const hr = hitRow(y);
    if (hr && hr.row.kind === 'layer' && x > 90) renameLayer(hr.row.L, hr.i);
  }
}
function renameLayer(L, rowIdx) {
  const wrap = PM.$('#tl-canvas-wrap');
  const inp = h('input', {
    value: L.name,
    style: {
      position: 'absolute', left: '94px', top: (rowY(rowIdx) + 5) + 'px', width: (T.gut - 110) + 'px',
      height: '20px', background: '#000', border: '1px solid var(--accent)', borderRadius: '4px',
      color: 'var(--tx)', fontSize: '11.5px', padding: '0 5px', zIndex: 9,
    },
  });
  wrap.appendChild(inp); inp.focus(); inp.select();
  const done = (ok) => {
    if (ok && inp.value.trim()) PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { name: inp.value.trim() } }, { label: 'Rename layer', origin: 'timeline' });
    inp.remove(); PM.invalidate();
  };
  inp.onblur = () => done(true);
  inp.onkeydown = (ev) => { ev.stopPropagation(); if (ev.key === 'Enter') done(true); if (ev.key === 'Escape') done(false); };
}

function onCtx(e) {
  e.preventDefault();
  const x = e.offsetX, y = e.offsetY;
  const hr = hitRow(y);
  const items = [];
  if (hr && hr.row.kind === 'prop') {
    const r = hr.row;
    const key = r.prop.kf.find(k => Math.abs(t2x(r.L.from + k.t) - x) < 7);
    if (key) {
      items.push({ header: 'Keyframe' });
      ['power', 'linear', 'easeOut', 'easeInOut', 'expoOut', 'backOut', 'snap', 'glide'].forEach(n =>
        items.push({ label: 'Ease · ' + n, run: () => PM.hist.do('Ease', () => PM.applyEaseTo([key], n)) }));
      items.push({ label: key.hold ? 'Remove hold' : 'Toggle hold', run: () => PM.hist.do('Hold', () => { key.hold = !key.hold; }) });
      items.push('-', { label: 'Delete keyframe', run: () => PM.hist.do('Delete keyframe', () => PM.removeKey(r.prop, key)) });
    } else {
      items.push({ label: 'Add keyframe here', run: () => PM.hist.do('Add keyframe', () => PM.setKeyOn(r.prop, x2t(x) - r.L.from, PM.evP(r.L, r.prop, x2t(x), r.key), 'power', PM.proj.fps)) });
      items.push({ label: 'Clear all keyframes', disabled: !r.prop.kf.length, run: () => PM.hist.do('Clear keys', () => { r.prop.kf = []; PM.touch(); }) });
    }
  } else if (hr && hr.row.kind === 'layer') {
    const L = hr.row.L;
    if (!PM.sel.layers.includes(L.id)) PM.selectLayers(L.id);
    const inside = PM.time > L.from && PM.time < L.from + L.dur;
    items.push({ header: L.name },
      { label: 'Duplicate', kb: '⌘D', run: () => PM.cmd('duplicate') },
      { label: 'Precompose', kb: '⌘⇧C', run: () => PM.cmd('precompose') },
      { label: 'Split at playhead', kb: '⌘⇧D', disabled: !inside, run: () => PM.cmd('split') },
      { label: 'Trim in to playhead', disabled: !inside, run: () => PM.Edit.apply(trimInCommands(L, PM.time), { label: 'Trim', origin: 'timeline' }) },
      { label: 'Trim out to playhead', disabled: !inside, run: () => PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { duration: Math.max(1 / PM.proj.fps, PM.time - L.from) } }, { label: 'Trim', origin: 'timeline' }) },
      '-',
      { label: L.mblur ? 'Motion blur off' : 'Motion blur on', run: () => PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { motionBlur: !L.mblur } }, { label: 'Motion blur', origin: 'timeline' }) },
      { label: 'Fit to composition', run: () => PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { from: 0, duration: PM.proj.dur } }, { label: 'Fit', origin: 'timeline' }) },
      '-',
      { header: 'Parent to' },
      { label: 'None', on: !L.parent, run: () => PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { parent: null } }, { label: 'Parent', origin: 'timeline' }) },
      ...PM.proj.layers.filter(o => o.id !== L.id && !PM.wouldCycle(L, o.id)).map(o => ({
        label: o.name, on: L.parent === o.id, run: () => PM.Edit.apply({ type: 'set_layer', target: L.id, patch: { parent: o.id } }, { label: 'Parent', origin: 'timeline' }),
      })),
      '-',
      { label: 'Delete', run: () => PM.cmd('delete') });
  } else {
    items.push({ label: 'Set work area start', run: () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [Math.min(PM.time, PM.proj.work[1] - 1 / PM.proj.fps), PM.proj.work[1]] } }, { label: 'Work area', origin: 'timeline' }) },
      { label: 'Set work area end', run: () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [PM.proj.work[0], Math.max(PM.time, PM.proj.work[0] + 1 / PM.proj.fps)] } }, { label: 'Work area', origin: 'timeline' }) },
      { label: 'Reset work area', run: () => PM.Edit.apply({ type: 'set_composition', patch: { workArea: [0, PM.proj.dur] } }, { label: 'Work area', origin: 'timeline' }) });
  }
  PM.menu(document.body, items, { x: e.clientX, y: e.clientY });
}

/* ── edge navigation ───────────────────────────────────── */
function edges() {
  const e = new Set([0, PM.proj.dur]);
  PM.proj.layers.forEach(L => {
    e.add(PM.round(L.from, 4)); e.add(PM.round(L.from + L.dur, 4));
    PM.allProps(L).forEach(p => p.prop.kf.forEach(k => e.add(PM.round(L.from + k.t, 4))));
  });
  return [...e].sort((a, b) => a - b);
}
function nextEdge() { const e = edges(); return e.find(t => t > PM.time + 1e-4) ?? PM.proj.dur; }
function prevEdge() { const e = edges(); return [...e].reverse().find(t => t < PM.time - 1e-4) ?? 0; }
T.nextEdge = nextEdge; T.prevEdge = prevEdge;
T.frameView = () => { T.scrollT = 0; T.pps = clamp((T.w - T.gut - 40) / Math.max(.5, PM.proj.dur), 4, 4000); PM.invalidate('timeline'); };
T.reveal = (L, keys) => {
  L.collapsed = false; L._reveal = keys;
  buildRows();
  const idx = T.rows.findIndex(r => r.kind === 'layer' && r.L === L);
  if (idx >= 0) {
    const kids = (keys && keys.length) || PM.allProps(L).filter(pp => pp.prop.kf.length || pp.prop.expr).length;
    keepRowsVisible(idx, kids);
  }
  PM.invalidate('timeline');
};
})();
