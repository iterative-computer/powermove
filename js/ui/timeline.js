/* Powermove — timeline: canvas-drawn tracks, keyframes, graph editor. */
(() => {
const PM = window.PM, h = PM.h, clamp = PM.clamp;

const T = {
  gut: 268, row: 30, ruler: 36, pps: 90, scrollT: 0, scrollY: 0,
  graph: false, rows: [], cv: null, ctx: null, w: 0, hgt: 0, dpr: 1,
  hover: null, marquee: null,
};
PM.TL = T;

PM.registerPanel('timeline', {
  title: 'Timeline', flush: true, noscroll: true, persist: true, headless: true, size: 300,
  build(body) {
    const wrap = h('div#tl-canvas-wrap');
    const cv = h('canvas#tl-canvas');
    wrap.appendChild(cv);
    const foot = h('div#tl-foot');
    body.append(wrap, foot);
    T.cv = cv; T.ctx = cv.getContext('2d', { alpha: false });
    buildTime(wrap);
    buildFoot(foot);
    bind(cv, wrap);
    new ResizeObserver(() => resize(wrap)).observe(wrap);
    requestAnimationFrame(() => resize(wrap));
  },
});

function buildTime(wrap) {
  const box = h('div#tl-timebox');
  const time = h('div#tl-time');
  const sub = h('div#tl-time-sub');
  box.append(time, sub);
  wrap.appendChild(box);
  T.timeEl = time; T.timeSub = sub;
  const sync = () => {
    if (time.classList.contains('edit')) return;
    const fps = PM.proj.fps;
    time.textContent = PM.tc(PM.time, fps);
    sub.textContent = Math.round(PM.time * fps) + ' (' + fps + ' fps)';
  };
  T.syncTime = sync;
  PM.bus.on('time', sync); PM.bus.on('project', sync); PM.bus.on('transport', sync); sync();
  box.addEventListener('pointerdown', (e) => {
    if (time.classList.contains('edit') || e.button !== 0) return;
    const start = PM.time;
    let moved = false;
    PM.drag(e, {
      cursor: 'ew-resize',
      move: (dx) => {
        if (!moved && Math.abs(dx) < 3) return;
        moved = true;
        PM.setTime(start + dx / 12 / PM.proj.fps);
      },
      up: () => { if (!moved) editTime(time); },
    });
  });
}
function editTime(el) {
  el.classList.add('edit');
  const inp = h('input', { value: PM.tc(PM.time, PM.proj.fps) });
  el.textContent = ''; el.appendChild(inp);
  inp.focus(); inp.select();
  let skip = false;
  const done = (ok) => {
    el.classList.remove('edit');
    if (ok) {
      const n = PM.parseTc(inp.value.replace(/;/g, ':'), PM.proj.fps);
      if (n != null && isFinite(n)) PM.setTime(n);
    }
    if (T.syncTime) T.syncTime();
  };
  inp.addEventListener('blur', () => done(!skip));
  inp.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { e.preventDefault(); inp.blur(); }
    if (e.key === 'Escape') { e.preventDefault(); skip = true; inp.blur(); }
  });
}

function buildFoot(foot) {
  const graph = h('button.iconbtn.sm' + (T.graph ? '.on' : ''), { title: 'Graph Editor (G)' }, PM.icon('graph'));
  graph.onclick = () => T.toggleGraph();
  T.graphBtn = graph;
  const meta = h('div#tl-foot-meta');
  T.footMeta = meta;
  const nav = h('div#tl-nav', h('div#tl-nav-thumb'));
  T.navEl = nav; T.navThumb = nav.firstChild;
  bindNav(nav);
  const zoom = h('input#tl-zoom', { type: 'range', min: 8, max: 900, value: T.pps, step: 1, title: 'Zoom' });
  T.zoomEl = zoom;
  zoom.addEventListener('input', () => setPps(+zoom.value, PM.time));
  const mtnS = h('button.tl-mtn.s', { title: 'Zoom out' }, PM.icon('mtnS'));
  const mtnL = h('button.tl-mtn.l', { title: 'Zoom in' }, PM.icon('mtnL'));
  mtnS.onclick = () => setPps(T.pps / 1.25, PM.time);
  mtnL.onclick = () => setPps(T.pps * 1.25, PM.time);
  foot.append(
    h('div#tl-foot-l', graph, meta),
    h('div#tl-foot-r', nav, mtnS, zoom, mtnL),
  );
}
T.toggleGraph = () => {
  T.graph = !T.graph;
  if (T.graphBtn) T.graphBtn.classList.toggle('on', T.graph);
  PM.invalidate('timeline');
};
function setPps(pps, anchorT) {
  const vis = Math.max(40, T.w - T.gut);
  const a = anchorT != null ? anchorT : T.scrollT + (vis / T.pps) / 2;
  const ax = t2x(a);
  T.pps = clamp(pps, 4, 4000);
  if (T.zoomEl) T.zoomEl.value = String(clamp(T.pps, 8, 900));
  T.scrollT = a - (ax - T.gut) / T.pps;
  T.scrollT = Math.max(-.4, T.scrollT);
  PM.invalidate('timeline');
}
function visDur() { return Math.max(.001, (T.w - T.gut) / T.pps); }
function updateNav() {
  const nav = T.navEl, th = T.navThumb;
  if (!nav || !th) return;
  const dur = Math.max(.001, PM.proj.dur);
  const vis = visDur();
  const nw = nav.clientWidth || 1;
  let x = (T.scrollT / dur) * nw;
  let w = (vis / dur) * nw;
  if (w >= nw - .5) { x = 0; w = nw; }
  x = clamp(x, 0, Math.max(0, nw - 8));
  w = clamp(w, 8, nw - x);
  th.style.left = x + 'px';
  th.style.width = w + 'px';
  if (T.zoomEl && document.activeElement !== T.zoomEl) T.zoomEl.value = String(clamp(T.pps, 8, 900));
  if (T.graphBtn) T.graphBtn.classList.toggle('on', !!T.graph);
  if (T.footMeta) {
    const ms = PM.perf && PM.perf.ms;
    T.footMeta.textContent = 'Frame Render Time: ' + (ms < 1 ? (ms || 0).toFixed(1) : Math.round(ms)) + 'ms';
  }
}
function bindNav(nav) {
  nav.addEventListener('pointermove', (e) => {
    const th = T.navThumb && T.navThumb.getBoundingClientRect();
    if (!th) return;
    const edge = 6;
    nav.style.cursor = (e.clientX >= th.left - 2 && e.clientX <= th.left + edge) ||
      (e.clientX >= th.right - edge && e.clientX <= th.right + 2) ? 'ew-resize' : 'grab';
  });
  nav.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    const r = nav.getBoundingClientRect();
    const dur = () => Math.max(.001, PM.proj.dur);
    const xOf = (ev) => clamp((ev.clientX - r.left) / Math.max(1, r.width), 0, 1);
    const th = T.navThumb.getBoundingClientRect();
    const edge = 6;
    let mode = 'jump';
    if (e.clientX >= th.left - 2 && e.clientX <= th.right + 2) {
      if (e.clientX <= th.left + edge) mode = 'in';
      else if (e.clientX >= th.right - edge) mode = 'out';
      else mode = 'pan';
    }
    const startT = T.scrollT, startVis = visDur(), startX = xOf(e);
    if (mode === 'jump') {
      T.scrollT = clamp(xOf(e) * dur() - visDur() / 2, 0, Math.max(0, dur() - visDur()));
      PM.invalidate('timeline');
      mode = 'pan';
    }
    PM.drag(e, {
      cursor: mode === 'pan' ? 'grabbing' : 'ew-resize',
      move: (dx, dy, ev) => {
        const x = xOf(ev), D = dur();
        if (mode === 'pan') {
          T.scrollT = clamp(startT + (x - startX) * D, -.4, Math.max(-.4, D - visDur() * .15));
        } else if (mode === 'in') {
          const end = startT + startVis;
          const nt = clamp(x * D, 0, end - .05);
          T.scrollT = nt;
          T.pps = clamp((T.w - T.gut) / (end - nt), 4, 4000);
        } else {
          const nt1 = clamp(x * D, startT + .05, D);
          T.pps = clamp((T.w - T.gut) / (nt1 - startT), 4, 4000);
        }
        PM.invalidate('timeline');
      },
    });
  });
  nav.addEventListener('dblclick', (e) => { e.preventDefault(); T.frameView(); });
}

function resize(wrap) {
  wrap = wrap || (T.cv && T.cv.parentElement);
  if (!wrap || !T.cv) return;
  const r = wrap.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return; /* detached / mid-remount */
  T.dpr = Math.min(devicePixelRatio || 1, 2);
  T.w = r.width; T.hgt = r.height;
  T.cv.width = Math.max(2, Math.round(r.width * T.dpr));
  T.cv.height = Math.max(2, Math.round(r.height * T.dpr));
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
      const shown = new Set();
      (PM.PAIRS || []).forEach(([label, keys]) => {
        const cells = keys.map(k => L.p[k] && { key: k, prop: L.p[k] }).filter(Boolean);
        if (!cells.length) return;
        rows.push({ kind: 'prop', L, label, cells });
        cells.forEach(c => shown.add(c.key));
        if (L._open && L._open[label] && cells.length > 1) {
          cells.forEach(c => rows.push({
            kind: 'prop', L, label: (PM.CH[c.key] && PM.CH[c.key].label) || c.key,
            cells: [c], child: true,
          }));
        }
      });
      PM.allProps(L).forEach(p => {
        if (shown.has(p.key)) return;
        if (p.prop.kf.length || p.prop.expr || PM.sel.chan === p.key || alwaysShow(L, p.key)) {
          rows.push({ kind: 'prop', L, label: p.label, cells: [{ key: p.key, prop: p.prop }] });
        }
      });
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
  theme = {
    accent: css('--accent') || '#F0580A', tx: css('--tx') || '#1C1C1F',
    tx2: css('--tx-2') || '#5D5D65', tx3: css('--tx-3') || '#8B8B93',
    panel: css('--bg-panel') || '#FCFCFD', sunken: css('--bg-sunken') || '#E4E4E7',
    line: css('--line') || 'rgba(15,15,20,.09)',
  };
};
PM.bus.on('layout', () => { refreshTheme(); refreshInk(); PM.invalidate('timeline'); });

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
  /* Re-resolve the live canvas every frame: workspace rebuilds can replace the
     panel element, and drawing into a detached canvas is the root cause of
     gutter/clip misalignment after layout changes. */
  const liveCv = PM.$('#tl-canvas');
  if (liveCv && liveCv !== T.cv) { T.cv = liveCv; T.ctx = liveCv.getContext('2d', { alpha: false }); }
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
  updateNav();
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
  /* markers */
  (p.markers || []).forEach(m => {
    const x = t2x(m.t);
    c.fillStyle = theme.accent;
    c.beginPath(); c.moveTo(x, T.ruler - 9); c.lineTo(x + 5, T.ruler - 4); c.lineTo(x, T.ruler); c.lineTo(x - 5, T.ruler - 4); c.fill();
  });
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
const layerLabel = (L) => (BADGE[L.type] ? BADGE[L.type] + ' ' : '') + L.name;

function drawClip(c, L, y) {
  const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
  if (x1 < T.gut || x0 > T.w) return;
  const hh = T.row - 7;
  const yy = y + 3.5;
  const sel = PM.sel.layers.includes(L.id);
  const r = 6;
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
  propCells(r).forEach(cell => {
    for (const k of cell.prop.kf) {
      const x = t2x(L.from + k.t);
      if (x < T.gut - 6 || x > T.w + 6) continue;
      const sel = PM.sel.keys.some(s => s.i === k.i);
      c.fillStyle = sel ? theme.accent : INK.key;
      if (k.hold) { c.fillRect(x - 3.4, cy - 3.4, 6.8, 6.8); }
      else {
        c.beginPath(); c.moveTo(x, cy - 4.4); c.lineTo(x + 4.4, cy); c.lineTo(x, cy + 4.4); c.lineTo(x - 4.4, cy); c.fill();
      }
    }
  });
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
      const L = r.L, sel = PM.sel.layers.includes(L.id), mid = y + T.row / 2;
      if (sel) { c.fillStyle = INK.over2; c.fillRect(0, y, T.gut, T.row); }
      /* A/V Features: Video · Audio · Solo · Lock  (AE order) */
      icoEye(c, SW.eye, mid, L.on);
      icoAudio(c, SW.audio, mid, L.audio !== false, L.type === 'audio' || L.type === 'video');
      icoSolo(c, SW.solo, mid, !!L.solo);
      icoLock(c, SW.lock, mid, L.lock);
      /* label color · # · twirl · name */
      c.fillStyle = L.color; roundRect(c, SW.label - 5, mid - 5, 10, 10, 2); c.fill();
      c.font = '500 10.5px ' + fmono();
      c.fillStyle = theme.tx3; c.textBaseline = 'middle'; c.textAlign = 'center';
      c.fillText(String(r.i + 1), SW.num, mid);
      c.textAlign = 'left';
      c.save();
      c.translate(SW.twirl, mid); c.rotate(L.collapsed ? 0 : Math.PI / 2);
      c.strokeStyle = theme.tx3; c.lineWidth = 1.4; c.beginPath();
      c.moveTo(-1.6, -3.4); c.lineTo(2, 0); c.lineTo(-1.6, 3.4); c.stroke();
      c.restore();
      c.font = (sel ? '560 ' : '450 ') + '11.5px ' + fui();
      c.fillStyle = sel ? theme.tx : theme.tx2;
      clipText(c, L.name, SW.name, mid, T.gut - SW.name - 8);
      if (L.parent) { c.fillStyle = theme.tx3; c.font = '400 9.5px ' + fmono(); c.fillText('↳', SW.name - 12, mid); }
    } else {
      const L = r.L, mid = y + T.row / 2;
      const cells = propCells(r);
      const sel = cells.some(c => PM.sel.chan === c.key);
      const keyed = cells.some(c => c.prop.kf.length);
      const nameX = r.child ? SW.name + 28 : SW.name + 16;
      if (!r.child) {
        c.save();
        c.translate(SW.twirl, mid);
        c.rotate((L._open && L._open[r.label]) ? Math.PI / 2 : 0);
        c.strokeStyle = cells.length > 1 ? theme.tx3 : INK.lo;
        c.lineWidth = 1.4; c.beginPath();
        c.moveTo(-1.6, -3.4); c.lineTo(2, 0); c.lineTo(-1.6, 3.4); c.stroke();
        c.restore();
        icoWatch(c, SW.name, mid, keyed);
      }
      c.font = '450 11px ' + fui();
      c.fillStyle = sel ? theme.accent : theme.tx3;
      clipText(c, r.label, nameX, mid, T.gut - cells.length * (VAL_W + 4) - nameX - 8);
      if (cells.some(c => c.prop.expr)) { c.fillStyle = theme.accent; c.fillText('ƒ', nameX - 12, mid); }
      cells.forEach((cell, ci) => {
        const box = valBox(i, ci, cells.length);
        if (T.editingVal && T.editingVal.row === i && T.editingVal.cell === ci) return;
        const hot = T.hoverVal && T.hoverVal.row === i && T.hoverVal.cell === ci;
        c.fillStyle = hot ? 'rgba(15,15,20,.10)' : 'rgba(15,15,20,.06)';
        roundRect(c, box.x, box.y, box.w, box.h, 4); c.fill();
        const v = PM.evP(L, cell.prop, PM.time, cell.key);
        c.font = '500 11px ' + fmono();
        c.fillStyle = cell.prop.kf.length ? theme.accent : theme.tx;
        c.textAlign = 'right';
        c.fillText(fmtVal(v), box.x + box.w - 5, y + T.row / 2);
        c.textAlign = 'left';
      });
    }
  }
  c.restore();
}
function clipText(c, s, x, y, max) {
  let t = s;
  if (c.measureText(t).width > max) { while (t.length > 3 && c.measureText(t + '…').width > max) t = t.slice(0, -1); t += '…'; }
  c.fillText(t, x, y);
}
/* AE A/V Features · Source Name columns */
const SW = { eye: 13, audio: 31, solo: 49, lock: 67, label: 84, num: 102, twirl: 120, name: 134 };
const LABEL_COLORS = ['#E23B3B', '#E8A23C', '#A89B76', '#4C8DFF', '#6C7BE8', '#E8E2CF', '#3FCF8E', '#FF6B1A', '#A9A9AE', '#6a6a70'];
function hitSwitch(x) {
  if (x < 22) return 'eye';
  if (x < 40) return 'audio';
  if (x < 58) return 'solo';
  if (x < 76) return 'lock';
  if (x < 94) return 'label';
  if (x < 114) return 'num';
  if (x < 130) return 'twirl';
  return 'name';
}
function icoEye(c, x, y, on) {
  c.strokeStyle = on ? INK.hi : INK.lo;
  c.lineWidth = 1.1; c.beginPath();
  c.ellipse(x, y, 5, 3.2, 0, 0, 7); c.stroke();
  if (on) { c.fillStyle = INK.hi; c.beginPath(); c.arc(x, y, 1.5, 0, 7); c.fill(); }
}
function icoAudio(c, x, y, on, has) {
  c.strokeStyle = on && has ? INK.hi : INK.lo;
  c.fillStyle = on && has ? INK.hi : INK.lo;
  c.lineWidth = 1.15;
  c.beginPath();
  c.moveTo(x - 3.4, y - 1.8); c.lineTo(x - 1.2, y - 1.8); c.lineTo(x + 2.2, y - 4.2);
  c.lineTo(x + 2.2, y + 4.2); c.lineTo(x - 1.2, y + 1.8); c.lineTo(x - 3.4, y + 1.8); c.closePath();
  if (on) c.fill(); else c.stroke();
  if (on && has) {
    c.beginPath(); c.arc(x + 2.4, y, 3.4, -.55, .55); c.stroke();
  }
}
function icoSolo(c, x, y, on) {
  c.strokeStyle = on ? theme.accent : INK.lo;
  c.fillStyle = on ? theme.accent : 'transparent';
  c.lineWidth = 1.15;
  c.beginPath(); c.arc(x, y, 4.1, 0, 7); on ? c.fill() : c.stroke();
  if (on) { c.fillStyle = '#fff'; c.beginPath(); c.arc(x, y, 1.5, 0, 7); c.fill(); }
}
function icoLock(c, x, y, on) {
  c.strokeStyle = on ? theme.accent : INK.lo;
  c.lineWidth = 1.1;
  c.strokeRect(x - 3.4, y - 1, 6.8, 5);
  c.beginPath(); c.arc(x, y - 1, 2.4, Math.PI, 0); c.stroke();
}
function icoWatch(c, x, y, on) {
  c.strokeStyle = on ? theme.accent : INK.lo;
  c.fillStyle = on ? theme.accent : 'transparent';
  c.lineWidth = 1.15;
  c.beginPath(); c.arc(x, y, 4.3, 0, 7); on ? c.fill() : c.stroke();
  c.strokeStyle = on ? '#fff' : INK.lo;
  c.beginPath(); c.moveTo(x, y); c.lineTo(x, y - 2.2); c.moveTo(x, y); c.lineTo(x + 1.7, y + .6); c.stroke();
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
  const rows = T.rows.filter(r => r.kind === 'prop' && propCells(r).some(c => PM.sel.chan === c.key || c.prop.kf.length));
  const targetRow = rows.find(r => propCells(r).some(c => c.key === PM.sel.chan)) || rows[0];
  c.save(); c.beginPath(); c.rect(T.gut, T.ruler, W - T.gut, H - T.ruler); c.clip();
  if (!targetRow) {
    c.fillStyle = theme.tx3; c.font = '400 11.5px ' + fui(); c.textAlign = 'center';
    c.fillText('Select an animated property to edit its curve', (W + T.gut) / 2, (H + T.ruler) / 2);
    c.textAlign = 'left'; c.restore(); return;
  }
  const cell = propCells(targetRow).find(c => c.key === PM.sel.chan && c.prop.kf.length)
    || propCells(targetRow).find(c => c.prop.kf.length) || propCells(targetRow)[0];
  const target = { L: targetRow.L, prop: cell.prop, key: cell.key, label: targetRow.label };
  const kf = target.prop.kf, L = target.L;
  let vmin, vmax;
  if (T._graphLock) {
    vmin = T._graphLock.vmin; vmax = T._graphLock.vmax;
  } else {
    vmin = Infinity; vmax = -Infinity;
    const grow = (v) => { if (v == null || !isFinite(v)) return; vmin = Math.min(vmin, v); vmax = Math.max(vmax, v); };
    kf.forEach((k, i) => {
      grow(k.v);
      const nx = kf[i + 1], pv = kf[i - 1];
      if (nx) grow(k.v + (nx.v - k.v) * k.eo[1]);
      if (pv) grow(pv.v + (k.v - pv.v) * k.ei[1]);
    });
    if (kf.length) {
      const t0 = kf[0].t, t1 = kf[kf.length - 1].t, span = Math.max(1e-4, t1 - t0);
      const n = Math.max(64, Math.min(240, Math.ceil(span * 80)));
      for (let i = 0; i <= n; i++) grow(PM.evalKfs(kf, t0 + span * i / n));
    }
    if (!isFinite(vmin)) { vmin = 0; vmax = 1; }
    if (vmax - vmin < 1e-6) { vmax = vmin + 1; }
    const padv = (vmax - vmin) * .08;
    vmin -= padv; vmax += padv;
  }
  const top = T.ruler + 28, bot = H - 16;
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

const VAL_W = 58;
function propCells(r) { return r.cells || (r.prop ? [{ key: r.key, prop: r.prop }] : []); }
function fmtVal(v) {
  if (typeof v !== 'number' || !isFinite(v)) return String(v).slice(0, 8);
  return String(PM.round(v, Math.abs(v) < 10 ? 2 : 1));
}
function valBox(rowIdx, cellIdx, n) {
  const y = rowY(rowIdx);
  const w = VAL_W;
  const x = T.gut - 6 - (n - cellIdx) * (w + 3);
  return { x, y: y + 5, w, h: T.row - 10 };
}
function hitVal(x, y) {
  const hr = hitRow(y);
  if (!hr || hr.row.kind !== 'prop') return null;
  const cells = propCells(hr.row);
  for (let ci = 0; ci < cells.length; ci++) {
    const b = valBox(hr.i, ci, cells.length);
    if (x >= b.x && x <= b.x + b.w && y >= b.y && y <= b.y + b.h) return { ...hr, cell: ci, cells };
  }
  return null;
}

function ensureEditEl(wrap) {
  wrap = wrap || (T.cv && T.cv.parentElement);
  if (!wrap) return null;
  let edit = wrap.querySelector('#tl-edit');
  if (edit) { T.editEl = edit; return edit; }
  edit = h('input#tl-edit', { spellcheck: 'false' });
  wrap.appendChild(edit);
  T.editEl = edit;
  edit.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter') { e.preventDefault(); edit.blur(); }
    if (e.key === 'Escape') { e.preventDefault(); T._editSkip = true; edit.blur(); }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      const meta = (T._editMeta) || {};
      const d = (e.key === 'ArrowUp' ? 1 : -1) * (meta.step || 1) * (e.shiftKey ? 10 : 1);
      edit.value = String(PM.round(parseFloat(edit.value || 0) + d, 4));
    }
  });
  edit.addEventListener('blur', () => commitEdit());
  return edit;
}

function bind(cv, wrap) {
  ensureEditEl(wrap);
  cv.addEventListener('pointerdown', onDown);
  cv.addEventListener('pointermove', onMove);
  cv.addEventListener('pointerleave', () => {
    if (T.hoverVal != null) { T.hoverVal = null; PM.invalidate('timeline'); }
    T.cv.style.cursor = 'default';
  });
  cv.addEventListener('dblclick', onDbl);
  cv.addEventListener('contextmenu', onCtx);
  cv.addEventListener('wheel', (e) => {
    if (T.editEl && T.editEl.style.display === 'block') T.editEl.blur();
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
  let hover = null;
  if (x < T.gut) {
    const hv = hitVal(x, y);
    if (hv && !hv.row.L.lock) { cur = 'ew-resize'; hover = { row: hv.i, cell: hv.cell }; }
  } else {
    const hr = hitRow(y);
    if (hr && hr.row.kind === 'layer') {
      const L = hr.row.L;
      const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
      if (Math.abs(x - x0) < 5 || Math.abs(x - x1) < 5) cur = 'ew-resize';
      else if (x > x0 && x < x1) cur = 'grab';
    }
    if (y < T.ruler) cur = 'ew-resize';
  }
  const same = T.hoverVal && hover && T.hoverVal.row === hover.row && T.hoverVal.cell === hover.cell;
  const bothNull = !T.hoverVal && !hover;
  if (!same && !bothNull) { T.hoverVal = hover; PM.invalidate('timeline'); }
  T.cv.style.cursor = cur;
}

function onDown(e) {
  const x = e.offsetX, y = e.offsetY;
  PM.closeMenus();
  if (y < T.ruler && x > T.gut) {
    /* drag work-area ends directly on the ruler (AE B/N handles) */
    const wa = PM.proj.work || [0, PM.proj.dur];
    const x0 = t2x(wa[0]), x1 = t2x(wa[1]);
    if (Math.abs(x - x0) < 6) return workAreaDrag(e, 0);
    if (Math.abs(x - x1) < 6) return workAreaDrag(e, 1);
    return scrub(e);
  }
  if (x < T.gut) return gutterDown(e, x, y);
  if (T.graph) return graphDown(e, x, y);
  const hr = hitRow(y);
  if (!hr) return marquee(e);
  const r = hr.row;
  if (r.kind === 'prop') return keyDown(e, r, x, y, hr.i);
  const L = r.L;
  const x0 = t2x(L.from), x1 = t2x(L.from + L.dur);
  if (Math.abs(x - x0) < 5 || Math.abs(x - x1) < 5 || (x > x0 && x < x1)) {
    if (!PM.sel.layers.includes(L.id)) PM.selectLayers(L.id, e.shiftKey || e.metaKey);
    else if (e.shiftKey) PM.selectLayers(L.id, true);
    if (L.lock) return;
    if (Math.abs(x - x0) < 5) return trim(e, 'in');
    if (Math.abs(x - x1) < 5) return trim(e, 'out');
    return slide(e);
  }
  return marquee(e);
}

function workAreaDrag(e, idx) {
  PM.hist.begin('Work area');
  PM.drag(e, {
    cursor: 'ew-resize',
    move: (dx, dy, ev) => {
      const r = T.cv.getBoundingClientRect();
      let t = Math.max(0, PM.snapF(x2t(ev.clientX - r.left), PM.proj.fps));
      const wa = PM.proj.work;
      wa[idx] = t;
      if (wa[0] > wa[1]) { const o = wa[0]; wa[0] = wa[1]; wa[1] = o; idx = 1 - idx; }
      PM.bus.emit('project'); PM.invalidate();
    },
    up: () => PM.hist.commit('Work area'),
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
  const hv = hitVal(x, y);
  if (hv) return scrubValue(e, hv);
  const hr = hitRow(y);
  if (!hr) return;
  const r = hr.row;
  if (r.kind !== 'layer') {
    const cells = propCells(r);
    const c0 = cells[0];
    if (c0) PM.sel.chan = c0.key;
    PM.selectLayers(r.L.id);
    if (!r.child && x >= 114 && x < 128 && cells.length > 1) {
      r.L._open = r.L._open || {};
      r.L._open[r.label] = !r.L._open[r.label];
      PM.invalidate('timeline');
      return;
    }
    if (!r.child && x >= 128 && x < 148) {
      PM.hist.do('Animate ' + r.label, () => {
        const any = cells.some(c => c.prop.kf.length);
        cells.forEach(c => {
          if (any) { c.prop.v = PM.evP(r.L, c.prop, PM.time, c.key); c.prop.kf = []; }
          else PM.setKeyOn(c.prop, PM.time - r.L.from, PM.evP(r.L, c.prop, PM.time, c.key), 'power', PM.proj.fps);
        });
        PM.touch();
      });
      PM.invalidate();
      return;
    }
    PM.invalidate('timeline'); return;
  }
  const L = r.L;
  const sw = hitSwitch(x);
  if (sw === 'eye') { PM.hist.do('Toggle visibility', () => { L.on = !L.on; }); PM.invalidate(); return; }
  if (sw === 'audio') { PM.hist.do('Toggle audio', () => { L.audio = L.audio === false; }); PM.invalidate(); return; }
  if (sw === 'solo') { PM.hist.do('Toggle solo', () => { L.solo = !L.solo; }); PM.invalidate(); return; }
  if (sw === 'lock') { PM.hist.do('Toggle lock', () => { L.lock = !L.lock; }); PM.invalidate(); return; }
  if (sw === 'label') {
    PM.hist.do('Label color', () => {
      const i = LABEL_COLORS.indexOf(L.color);
      L.color = LABEL_COLORS[(i + 1) % LABEL_COLORS.length];
    });
    PM.invalidate(); return;
  }
  if (sw === 'twirl') {
    L.collapsed = !L.collapsed;
    if (!L.collapsed) keepRowsVisible(hr.i, (PM.PAIRS || []).length);
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
        if (!done) { PM.hist.begin('Reorder layer'); done = true; }
        PM.proj.layers.splice(PM.proj.layers.indexOf(L), 1);
        PM.proj.layers.splice(to, 0, L);
        PM.invalidate('timeline');
      }
    },
    up: () => { if (done) { PM.hist.commit('Reorder layer'); PM.bus.emit('layers'); } },
  });
}

function scrubValue(e, hr) {
  const cell = hr.cells[hr.cell];
  const L = hr.row.L, key = cell.key, prop = cell.prop;
  PM.selectLayers(L.id);
  PM.sel.chan = key;
  PM.invalidate('timeline');
  if (L.lock) return;
  const meta = PM.CH[key] || {};
  const read = () => {
    const v = PM.evP(L, prop, PM.time, key);
    return typeof v === 'number' && isFinite(v) ? v : 0;
  };
  const write = (v) => {
    if (meta.min != null) v = Math.max(meta.min, v);
    if (meta.max != null) v = Math.min(meta.max, v);
    PM.setOrKey(L, key, PM.round(v, 3), PM.time);
    PM.invalidate();
  };
  const start = read();
  let moved = false;
  PM.hist.begin(hr.row.label);
  PM.drag(e, {
    cursor: 'ew-resize',
    move: (dx, dy, ev) => {
      if (!moved && Math.abs(dx) < 3) return;
      moved = true;
      const mult = ev.shiftKey ? 10 : ev.altKey ? .1 : 1;
      write(start + dx * (meta.step || 1) * mult * .5);
    },
    up: () => {
      if (!moved) { PM.hist.cancel(); editValue(hr); }
      else PM.hist.commit(hr.row.label);
    },
  });
}

function editValue(hr) {
  const el = ensureEditEl(); if (!el) return;
  const cell = hr.cells[hr.cell];
  const L = hr.row.L, key = cell.key, prop = cell.prop;
  const v = PM.evP(L, prop, PM.time, key);
  const box = valBox(hr.i, hr.cell, hr.cells.length);
  T.editingVal = { row: hr.i, cell: hr.cell };
  T._editSkip = false;
  T._editHr = hr;
  T._editMeta = PM.CH[key] || {};
  el.style.display = 'block';
  el.style.left = box.x + 'px';
  el.style.top = box.y + 'px';
  el.style.width = box.w + 'px';
  el.style.height = box.h + 'px';
  el.value = typeof v === 'number' ? String(PM.round(v, 3)) : String(v);
  PM.invalidate('timeline');
  el.focus(); el.select();
}
function commitEdit() {
  const el = T.editEl;
  const hr = T._editHr;
  const skip = T._editSkip;
  T.editingVal = null; T._editHr = null; T._editSkip = false;
  if (el) el.style.display = 'none';
  if (!skip && hr && el) {
    const cell = hr.cells[hr.cell];
    const cur = PM.evP(hr.row.L, cell.prop, PM.time, cell.key);
    const raw = el.value.trim();
    let n = NaN;
    if (/^[-+*/]/.test(raw) && typeof cur === 'number') {
      try { n = Function('"use strict";return (' + cur + raw + ')')(); } catch { n = NaN; }
    } else if (/^[-+*/(). 0-9e]+$/i.test(raw)) {
      try { n = Function('"use strict";return (' + raw + ')')(); } catch { n = NaN; }
    }
    if (isFinite(n)) {
      const meta = PM.CH[cell.key] || {};
      if (meta.min != null) n = Math.max(meta.min, n);
      if (meta.max != null) n = Math.min(meta.max, n);
      PM.hist.do(hr.row.label, () => PM.setOrKey(hr.row.L, cell.key, PM.round(n, 4), PM.time));
    }
  }
  PM.invalidate();
}

function slide(e) {
  const layers = PM.selLayers().filter(l => !l.lock);
  const start = layers.map(L => ({ L, from: L.from }));
  PM.hist.begin('Move clip');
  let moved = false;
  PM.drag(e, {
    cursor: 'grabbing',
    move: (dx) => {
      moved = true;
      let dt = dx / T.pps;
      if (PM.snap) dt = snapDelta(start, dt);
      start.forEach(s => { s.L.from = Math.max(0, PM.snapF(s.from + dt, PM.proj.fps)); });
      PM.invalidate();
    },
    up: () => { moved ? PM.hist.commit('Move clip') : PM.hist.cancel(); },
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
  const start = layers.map(L => ({ L, from: L.from, dur: L.dur }));
  PM.hist.begin('Trim clip');
  let moved = false;
  PM.drag(e, {
    cursor: 'ew-resize',
    move: (dx) => {
      moved = true;
      const dt = PM.snapF(dx / T.pps, PM.proj.fps);
      start.forEach(s => {
        if (side === 'in') {
          const nf = clamp(s.from + dt, 0, s.from + s.dur - 1 / PM.proj.fps);
          s.L.from = nf; s.L.dur = s.dur + (s.from - nf);
        } else s.L.dur = Math.max(1 / PM.proj.fps, s.dur + dt);
      });
      PM.invalidate();
    },
    up: () => { moved ? PM.hist.commit('Trim clip') : PM.hist.cancel(); },
  });
}

function keyDown(e, r, x, y, rowIdx) {
  const cells = propCells(r);
  const cell = cells.find(c => c.prop.kf.some(k => Math.abs(t2x(r.L.from + k.t) - x) < 6)) || cells[0];
  if (cell) PM.sel.chan = cell.key;
  const hit = cell && cell.prop.kf.find(k => Math.abs(t2x(r.L.from + k.t) - x) < 6);
  if (!hit) return marquee(e);
  if (!PM.sel.layers.includes(r.L.id)) {
    const keep = [...PM.sel.keys];
    PM.selectLayers(r.L.id, true);
    if (!e.shiftKey) PM.sel.keys = keep;
  }
  if (e.shiftKey) {
    if (!PM.sel.keys.some(k => k.i === hit.i)) PM.sel.keys.push(hit);
  } else if (!PM.sel.keys.some(k => k.i === hit.i)) {
    PM.sel.keys = [hit];
  }
  const keys = PM.sel.keys.length ? PM.sel.keys : [hit];
  const start = keys.map(k => ({ k, t: k.t }));
  PM.hist.begin('Move keyframe');
  let moved = false;
  PM.drag(e, {
    move: (dx) => {
      moved = true;
      const dt = PM.snapF(dx / T.pps, PM.proj.fps);
      start.forEach(s => { s.k.t = Math.max(0, s.t + dt); });
      PM.proj.layers.forEach(L => PM.allProps(L).forEach(p => p.prop.kf.sort((a, b) => a.t - b.t)));
      PM.touch(); PM.invalidate();
    },
    up: () => { moved ? PM.hist.commit('Move keyframe') : PM.hist.cancel(); PM.invalidate('timeline'); },
  });
}

function lockGraph() {
  const g = T._graph;
  if (g && !T._graphLock) T._graphLock = { vmin: g.vmin, vmax: g.vmax };
}
function unlockGraph() { T._graphLock = null; PM.invalidate('timeline'); }

function graphDown(e, x, y) {
  const g = T._graph; if (!g) return;
  const kf = g.target.prop.kf, L = g.target.L;
  for (const k of kf) {
    if (k._ho && Math.hypot(x - k._ho[0], y - k._ho[1]) < 7) return dragHandle(e, k, 'eo', g, kf, L);
    if (k._hi && Math.hypot(x - k._hi[0], y - k._hi[1]) < 7) return dragHandle(e, k, 'ei', g, kf, L);
  }
  const hit = kf.find(k => k._pt && Math.hypot(x - k._pt[0], y - k._pt[1]) < 8);
  if (!hit) return marquee(e, 'graph');
  PM.sel.keys = e.shiftKey ? [...PM.sel.keys, hit] : [hit];
  lockGraph();
  const y2v = g.y2v;
  PM.hist.begin('Edit curve');
  PM.drag(e, {
    move: (dx, dy, ev) => {
      const rec = T.cv.getBoundingClientRect();
      hit.t = Math.max(0, PM.snapF(x2t(ev.clientX - rec.left) - L.from, PM.proj.fps));
      hit.v = PM.round(y2v(ev.clientY - rec.top), 3);
      kf.sort((a, b) => a.t - b.t);
      PM.touch(); PM.invalidate();
    },
    up: () => { PM.hist.commit('Edit curve'); unlockGraph(); },
  });
}
function dragHandle(e, k, which, g, kf, L) {
  const i = kf.indexOf(k);
  const other = which === 'eo' ? kf[i + 1] : kf[i - 1];
  if (!other) return;
  lockGraph();
  const self = { x: t2x(L.from + k.t), y: g.v2y(k.v) };
  const oth = { x: t2x(L.from + other.t), y: g.v2y(other.v) };
  PM.hist.begin('Adjust easing');
  PM.drag(e, {
    move: (dx, dy, ev) => {
      const rec = T.cv.getBoundingClientRect();
      const px = ev.clientX - rec.left, py = ev.clientY - rec.top;
      let nx, ny;
      if (which === 'eo') {
        nx = (px - self.x) / (oth.x - self.x || 1);
        ny = (py - self.y) / (oth.y - self.y || 1);
      } else {
        nx = (px - oth.x) / (self.x - oth.x || 1);
        ny = (py - oth.y) / (self.y - oth.y || 1);
      }
      k[which] = [PM.round(clamp(nx, 0, 1), 3), PM.round(ny, 3)];
      PM.touch(); PM.invalidate();
    },
    up: () => { PM.hist.commit('Adjust easing'); unlockGraph(); },
  });
}

function keysInBox(m) {
  const out = [];
  T.rows.forEach((row, i) => {
    if (row.kind !== 'prop') return;
    const cy = rowY(i) + T.row / 2;
    if (cy < m.y0 - 4 || cy > m.y1 + 4) return;
    propCells(row).forEach(cell => {
      cell.prop.kf.forEach(k => {
        const x = t2x(row.L.from + k.t);
        if (x + 5 >= m.x0 && x - 5 <= m.x1) out.push(k);
      });
    });
  });
  return out;
}
function graphKeysInBox(m) {
  const g = T._graph; if (!g) return [];
  return g.target.prop.kf.filter(k => k._pt && k._pt[0] + 5 >= m.x0 && k._pt[0] - 5 <= m.x1 && k._pt[1] + 5 >= m.y0 && k._pt[1] - 5 <= m.y1);
}
function layersInBox(m) {
  const out = [];
  T.rows.forEach((row, i) => {
    if (row.kind !== 'layer') return;
    const y = rowY(i);
    if (y + T.row < m.y0 || y > m.y1) return;
    const a = t2x(row.L.from), b = t2x(row.L.from + row.L.dur);
    if (b > m.x0 && a < m.x1) out.push(row.L.id);
  });
  return out;
}
function ownersOfKeys(keys) {
  const ids = [];
  T.rows.forEach(row => {
    if (row.kind !== 'prop') return;
    if (propCells(row).some(cell => cell.prop.kf.some(k => keys.some(s => s.i === k.i))) && !ids.includes(row.L.id)) ids.push(row.L.id);
  });
  return ids;
}

function marquee(e, mode) {
  const r = T.cv.getBoundingClientRect();
  const x0 = e.clientX - r.left, y0 = e.clientY - r.top;
  const add = e.shiftKey;
  const startKeys = add ? [...PM.sel.keys] : [];
  const startLayers = add ? [...PM.sel.layers] : [];
  let dragged = false;
  PM.drag(e, {
    move: (dx, dy) => {
      if (!dragged && Math.hypot(dx, dy) < 3) return;
      dragged = true;
      T.marquee = { x0: Math.min(x0, x0 + dx), y0: Math.min(y0, y0 + dy), x1: Math.max(x0, x0 + dx), y1: Math.max(y0, y0 + dy) };
      const m = T.marquee;
      const keys = mode === 'graph' ? graphKeysInBox(m) : keysInBox(m);
      const layers = mode === 'graph' ? [] : layersInBox(m);
      if (add) {
        const seen = new Set(startKeys.map(k => k.i));
        PM.sel.keys = startKeys.concat(keys.filter(k => !seen.has(k.i)));
        const have = new Set(startLayers);
        PM.sel.layers = startLayers.concat(layers.filter(id => !have.has(id)));
      } else {
        PM.sel.keys = keys;
        const next = layers.length ? layers : ownersOfKeys(keys);
        PM.sel.layers = next;
      }
      PM.invalidate('timeline');
    },
    up: () => {
      if (!dragged && !add) {
        PM.sel.keys = [];
        if (!hitRow(y0)) PM.sel.layers = [];
      }
      T.marquee = null;
      PM.bus.emit('sel');
      PM.invalidate('timeline');
    },
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
      position: 'absolute', left: SW.name + 'px', top: (rowY(rowIdx) + 5) + 'px', width: (T.gut - SW.name - 8) + 'px',
      height: '20px', background: '#000', border: '1px solid var(--accent)', borderRadius: '4px',
      color: 'var(--tx)', fontSize: '11.5px', padding: '0 5px', zIndex: 9,
    },
  });
  wrap.appendChild(inp); inp.focus(); inp.select();
  const done = (ok) => {
    if (ok && inp.value.trim()) PM.hist.do('Rename layer', () => { L.name = inp.value.trim(); });
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
    const cells = propCells(r);
    const cell = cells.find(c => c.prop.kf.some(k => Math.abs(t2x(r.L.from + k.t) - x) < 7)) || cells[0];
    const key = cell && cell.prop.kf.find(k => Math.abs(t2x(r.L.from + k.t) - x) < 7);
    if (key) {
      items.push({ header: 'Keyframe' });
      ['power', 'linear', 'easeOut', 'easeInOut', 'expoOut', 'backOut', 'snap', 'glide'].forEach(n =>
        items.push({ label: 'Ease · ' + n, run: () => PM.hist.do('Ease', () => PM.applyEaseTo([key], n)) }));
      items.push({ label: key.hold ? 'Remove hold' : 'Toggle hold', run: () => PM.hist.do('Hold', () => { key.hold = !key.hold; }) });
      items.push('-', { label: 'Delete keyframe', run: () => PM.hist.do('Delete keyframe', () => PM.removeKey(cell.prop, key)) });
    } else if (cell) {
      items.push({ label: 'Add keyframe here', run: () => PM.hist.do('Add keyframe', () => PM.setKeyOn(cell.prop, x2t(x) - r.L.from, PM.evP(r.L, cell.prop, x2t(x), cell.key), 'power', PM.proj.fps)) });
      items.push({ label: 'Clear all keyframes', run: () => PM.hist.do('Clear keys', () => { cells.forEach(c => { c.prop.kf = []; }); PM.touch(); }) });
    }
  } else if (hr && hr.row.kind === 'layer') {
    const L = hr.row.L;
    items.push({ header: L.name },
      { label: 'Duplicate', kb: '⌘D', run: () => PM.cmd('duplicate') },
      { label: 'Split at playhead', kb: '⌘⇧D', run: () => PM.cmd('split') },
      { label: 'Trim in to playhead', run: () => PM.hist.do('Trim', () => { const d = PM.time - L.from; L.dur -= d; L.from = PM.time; }) },
      { label: 'Trim out to playhead', run: () => PM.hist.do('Trim', () => { L.dur = Math.max(1 / PM.proj.fps, PM.time - L.from); }) },
      '-',
      { label: L.mblur ? 'Motion blur off' : 'Motion blur on', run: () => PM.hist.do('Motion blur', () => { L.mblur = !L.mblur; }) },
      { label: 'Fit to composition', run: () => PM.hist.do('Fit', () => { L.from = 0; L.dur = PM.proj.dur; }) },
      '-',
      { header: 'Parent to' },
      { label: 'None', on: !L.parent, run: () => PM.hist.do('Parent', () => { L.parent = null; }) },
      ...PM.proj.layers.filter(o => o.id !== L.id).map(o => ({
        label: o.name, on: L.parent === o.id, run: () => PM.hist.do('Parent', () => { L.parent = o.id; }),
      })),
      '-',
      { label: 'Delete', run: () => PM.cmd('delete') });
  } else {
    items.push({ label: 'Add marker at playhead', run: () => PM.hist.do('Marker', () => { PM.proj.markers.push({ t: PM.time, name: 'M' + (PM.proj.markers.length + 1) }); }) },
      { label: 'Set work area start', run: () => PM.hist.do('Work area', () => { PM.proj.work[0] = PM.time; }) },
      { label: 'Set work area end', run: () => PM.hist.do('Work area', () => { PM.proj.work[1] = PM.time; }) },
      { label: 'Reset work area', run: () => PM.hist.do('Work area', () => { PM.proj.work = [0, PM.proj.dur]; }) });
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
  PM.proj.markers.forEach(m => e.add(m.t));
  return [...e].sort((a, b) => a - b);
}
function nextEdge() { const e = edges(); return e.find(t => t > PM.time + 1e-4) ?? PM.proj.dur; }
function prevEdge() { const e = edges(); return [...e].reverse().find(t => t < PM.time - 1e-4) ?? 0; }
T.nextEdge = nextEdge; T.prevEdge = prevEdge;
T.frameView = () => { T.scrollT = 0; T.pps = clamp((T.w - T.gut - 40) / Math.max(.5, PM.proj.dur), 4, 4000); if (T.zoomEl) T.zoomEl.value = String(clamp(T.pps, 8, 900)); PM.invalidate('timeline'); };
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
