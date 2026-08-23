/* Powermove — viewer panel: GL stage, overlays, direct manipulation. */
(() => {
const PM = window.PM, h = PM.h, clamp = PM.clamp;

const V = { zoom: 1, fit: true, pan: [0, 0], el: null, ov: null, octx: null, inner: null, guides: null };
PM.Viewer = V;

PM.registerPanel('viewer', {
  title: 'Composition', flush: true, noscroll: true, persist: true, headless: true, moveSlot: '#viewer-foot',
  build(body) {
    const stage = h('div#stage');
    const inner = h('div#stage-inner');
    const gl = h('canvas#gl');
    const ov = h('canvas#overlay');
    inner.append(gl, ov);
    stage.appendChild(inner);
    const foot = h('div#viewer-foot');
    body.append(stage, foot);
    V.el = gl; V.ov = ov; V.octx = ov.getContext('2d'); V.inner = inner; V.stage = stage;
    if (!PM.GL.gl) PM.GL.init(gl);
    buildFoot(foot);
    bindStage(stage, inner);
    requestAnimationFrame(() => V.layout());
    new ResizeObserver(() => V.layout()).observe(stage);
  },
});

function buildFoot(foot) {
  const p = PM.proj;
  const aspect = h('button.chip', { onpointerdown: (e) => compMenu(e) });
  const zoom = h('button.chip', { onpointerdown: (e) => zoomMenu(e) });
  const q = h('button.chip', { onpointerdown: (e) => qualityMenu(e) });
  const tc = h('span.mono');
  const cmt = h('button.iconbtn', { title: 'Notes' }, PM.icon('code'));
  const exp = h('button.chip.solid', { onclick: () => PM.Export.dialog() }, PM.icon('export'), 'Export');
  foot.append(aspect, zoom, q, h('span', { style: { flex: 1 } }), cmt, tc, exp);
  const sync = () => {
    const p = PM.proj;
    const g = gcd(p.w, p.h);
    aspect.textContent = `${p.w / g}:${p.h / g}`;
    zoom.textContent = V.fit ? 'Fit' : Math.round(V.zoom * 100) + '%';
    q.textContent = PM.quality === 1 ? 'Full' : PM.quality === .5 ? 'Half' : PM.quality === .25 ? 'Quarter' : Math.round(PM.quality * 100) + '%';
    tc.textContent = PM.tc(PM.time, p.fps) + '  /  ' + PM.tc(p.dur, p.fps);
  };
  PM.bus.on('time', sync); PM.bus.on('project', sync); PM.bus.on('quality', sync);
  PM.bus.on('draw:ui', sync); PM.bus.on('draw:status', sync);
  sync();
}
const gcd = (a, b) => (b ? gcd(b, a % b) : a);

function compMenu(e) {
  e.preventDefault();
  const presets = [[1920, 1080, '1080p 16:9'], [3840, 2160, '4K UHD'], [1080, 1080, 'Square 1:1'], [1080, 1920, 'Vertical 9:16'], [1280, 720, '720p'], [2560, 1080, 'Cinemascope']];
  PM.menu(e.target, [
    { header: 'Composition size' },
    ...presets.map(([w, hh, l]) => ({
      label: l, on: PM.proj.w === w && PM.proj.h === hh,
      run: () => PM.hist.do('Comp size', () => { PM.proj.w = w; PM.proj.h = hh; PM.bus.emit('project'); V.layout(); }),
    })),
    '-',
    { header: 'Frame rate' },
    ...[24, 25, 30, 50, 60].map(f => ({
      label: f + ' fps', on: PM.proj.fps === f,
      run: () => PM.hist.do('Frame rate', () => { PM.proj.fps = f; PM.bus.emit('project'); }),
    })),
  ]);
}
function zoomMenu(e) {
  e.preventDefault();
  PM.menu(e.target, [{ label: 'Fit', on: V.fit, run: () => { V.fit = true; V.layout(); } },
  ...[.25, .5, 1, 2].map(z => ({ label: z * 100 + '%', on: !V.fit && V.zoom === z, run: () => { V.fit = false; V.zoom = z; V.layout(); } }))]);
}
function qualityMenu(e) {
  e.preventDefault();
  PM.menu(e.target, [
    { header: 'Preview resolution' },
    ...[[1, 'Full'], [.5, 'Half'], [.25, 'Quarter']].map(([q, l]) => ({
      label: l, on: PM.quality === q, run: () => { PM.perf.auto = false; PM.quality = q; V.layout(); PM.bus.emit('quality'); },
    })),
    { label: 'Adaptive', on: PM.perf.auto, run: () => { PM.perf.auto = true; PM.bus.emit('quality'); } },
  ]);
}

/* ── layout / sizing ───────────────────────────────────── */
V.layout = () => {
  if (!V.el || !V.stage) return;
  const p = PM.proj;
  const r = V.stage.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return;
  V._sw = r.width; V._sh = r.height;
  const pad = 8;
  let z = V.fit ? Math.min((r.width - pad * 2) / p.w, (r.height - pad * 2) / p.h) : V.zoom;
  z = clamp(z, .02, 8);
  V.shown = z;
  const dw = Math.round(p.w * z), dh = Math.round(p.h * z);
  V.inner.style.width = dw + 'px'; V.inner.style.height = dh + 'px';
  /* apply pan (AE Hand tool) */
  if (!V.fit && (V.pan[0] || V.pan[1])) V.inner.style.transform = `translate(${V.pan[0]}px, ${V.pan[1]}px)`;
  else V.inner.style.transform = '';
  const dpr = Math.min(devicePixelRatio || 1, 2);
  const rw = Math.max(2, Math.round(p.w * PM.quality)), rh = Math.max(2, Math.round(p.h * PM.quality));
  PM.GL.resize(rw, rh);
  V.ov.width = Math.round(dw * dpr); V.ov.height = Math.round(dh * dpr);
  V.ov.style.width = dw + 'px'; V.ov.style.height = dh + 'px';
  PM.invalidate();
};
PM.bus.on('quality', () => V.layout());
PM.bus.on('project', () => V.layout());
addEventListener('resize', () => V.layout());
PM.bus.on('layout:applied', () => V.layout());

/* comp px <-> screen px */
const toComp = (e) => {
  const r = V.ov.getBoundingClientRect();
  return [(e.clientX - r.left) / V.shown, (e.clientY - r.top) / V.shown];
};

/* ── overlay drawing ───────────────────────────────────── */
PM.bus.on('overlay', drawOverlay);
PM.bus.on('sel', () => PM.invalidate('render'));

function drawOverlay() {
  const c = V.octx; if (!c) return;
  /* Self-heal after layout rebuilds: if the stage no longer matches the size
     V.layout() last computed, re-run layout before drawing overlays. */
  if (V.stage) {
    const r = V.stage.getBoundingClientRect();
    if (r.width >= 8 && r.height >= 8 &&
        (Math.abs(r.width - (V._sw || 0)) > .5 || Math.abs(r.height - (V._sh || 0)) > .5)) {
      V.layout();
    }
  }
  const p = PM.proj, dpr = V.ov.width / (p.w * V.shown);
  c.setTransform(1, 0, 0, 1, 0, 0);
  c.clearRect(0, 0, V.ov.width, V.ov.height);
  const S = V.shown * dpr;
  c.save(); c.scale(S, S);
  c.lineWidth = 1 / S;

  drawSnapGuides(c, S, p);

  const sels = PM.selLayers().filter(L => PM.active(L, PM.time));
  for (const L of sels) {
    const b = PM.GL.bounds(L, PM.time);
    if (!b) continue;
    const m = PM.worldMatrix(L, PM.time);
    c.save();
    c.transform(m[0], m[1], m[2], m[3], m[4], m[5]);
    c.strokeStyle = 'rgba(255,107,26,.95)';
    c.lineWidth = 1.4 / S / Math.max(.05, Math.hypot(m[0], m[1]));
    c.strokeRect(b.x0, b.y0, b.w, b.h);
    /* handles */
    const hs = 5 / S / Math.max(.05, Math.hypot(m[0], m[1]));
    c.fillStyle = '#fff';
    c.strokeStyle = 'rgba(0,0,0,.45)';
    HANDLES.forEach(([hx, hy]) => {
      const x = b.x0 + b.w * hx, y = b.y0 + b.h * hy;
      c.fillRect(x - hs, y - hs, hs * 2, hs * 2);
      c.strokeRect(x - hs, y - hs, hs * 2, hs * 2);
    });
    c.restore();
  }
  c.restore();
}
const HANDLES = [[0, 0], [.5, 0], [1, 0], [1, .5], [1, 1], [.5, 1], [0, 1], [0, .5]];

function drawSnapGuides(c, S, p) {
  const guides = V.guides;
  if (!guides || (guides.x == null && guides.y == null)) return;
  c.save();
  c.strokeStyle = 'rgba(74,164,255,.96)';
  c.lineWidth = 1 / S;
  c.setLineDash([5 / S, 3 / S]);
  c.beginPath();
  if (guides.x != null) { c.moveTo(guides.x, 0); c.lineTo(guides.x, p.h); }
  if (guides.y != null) { c.moveTo(0, guides.y); c.lineTo(p.w, guides.y); }
  c.stroke();
  c.restore();
}

/* Screen-space alignment helpers. Bounds and guide coordinates stay in
   composition pixels, while the tolerance is converted from a constant
   on-screen distance by startMove(). */
function snapAxis(movingMarks, targetMarks, threshold) {
  let best = null;
  for (const moving of movingMarks || []) {
    const movingValue = typeof moving === 'number' ? moving : moving.value;
    if (!Number.isFinite(movingValue)) continue;
    for (const target of targetMarks || []) {
      const targetValue = typeof target === 'number' ? target : target.value;
      if (!Number.isFinite(targetValue)) continue;
      const delta = targetValue - movingValue;
      const distance = Math.abs(delta);
      if (distance > threshold || (best && distance >= best.distance - 1e-9)) continue;
      best = { delta, value: targetValue, distance };
    }
  }
  return best;
}

function worldBounds(L, T) {
  const b = PM.GL.bounds(L, T); if (!b) return null;
  const m = PM.worldMatrix(L, T);
  const points = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]]
    .map(([x, y]) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
  const xs = points.map(point => point[0]), ys = points.map(point => point[1]);
  const x0 = Math.min(...xs), x1 = Math.max(...xs), y0 = Math.min(...ys), y1 = Math.max(...ys);
  return { x0, x1, y0, y1, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

function unionBounds(layers, T) {
  const bounds = layers.map(L => worldBounds(L, T)).filter(Boolean);
  if (!bounds.length) return null;
  const x0 = Math.min(...bounds.map(b => b.x0)), x1 = Math.max(...bounds.map(b => b.x1));
  const y0 = Math.min(...bounds.map(b => b.y0)), y1 = Math.max(...bounds.map(b => b.y1));
  return { x0, x1, y0, y1, cx: (x0 + x1) / 2, cy: (y0 + y1) / 2 };
}

function hasSelectedAncestor(L, selectedIds) {
  let parentId = L.parent, guard = 0;
  while (parentId && guard++ < 256) {
    if (selectedIds.has(parentId)) return true;
    const parent = PM.L(parentId);
    parentId = parent && parent.parent;
  }
  return false;
}

function snapshotSnapTargets(T, selectedIds) {
  const x = [{ value: 0 }, { value: PM.proj.w / 2 }, { value: PM.proj.w }];
  const y = [{ value: 0 }, { value: PM.proj.h / 2 }, { value: PM.proj.h }];
  const soloOn = PM.proj.layers.some(L => L.solo);
  for (const L of PM.proj.layers) {
    if (selectedIds.has(L.id) || hasSelectedAncestor(L, selectedIds) || !PM.active(L, T) || (soloOn && !L.solo)) continue;
    const b = worldBounds(L, T); if (!b) continue;
    x.push({ value: b.x0 }, { value: b.cx }, { value: b.x1 });
    y.push({ value: b.y0 }, { value: b.cy }, { value: b.y1 });
  }
  return { x, y };
}

function alignmentSnap(bounds, targets, threshold, axes = { x: true, y: true }) {
  if (!bounds) return { dx: 0, dy: 0, x: null, y: null };
  const x = axes.x ? snapAxis([bounds.x0, bounds.cx, bounds.x1], targets.x, threshold) : null;
  const y = axes.y ? snapAxis([bounds.y0, bounds.cy, bounds.y1], targets.y, threshold) : null;
  return { dx: x ? x.delta : 0, dy: y ? y.delta : 0, x, y };
}

function worldDeltaToLocal(L, T, dx, dy) {
  if (!L.parent) return [dx, dy];
  const parent = PM.L(L.parent); if (!parent) return [dx, dy];
  const m = PM.worldMatrix(parent, T), det = m[0] * m[3] - m[1] * m[2];
  if (Math.abs(det) < 1e-9) return [0, 0];
  return [(dx * m[3] - dy * m[2]) / det, (dy * m[0] - dx * m[1]) / det];
}

Object.assign(V, { snapAxis, worldBounds, unionBounds, snapshotSnapTargets, alignmentSnap });

/* ── direct manipulation ───────────────────────────────── */
function bindStage(stage, inner) {
  inner.addEventListener('pointerdown', onDown);
  stage.addEventListener('wheel', (e) => {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      V.fit = false;
      V.zoom = clamp((V.shown || 1) * (1 - e.deltaY * .0035), .05, 8);
      V.layout();
    }
  }, { passive: false });
  inner.addEventListener('dblclick', (e) => {
    const [x, y] = toComp(e);
    const L = PM.GL.pick(x, y, PM.time);
    if (L && L.type === 'text') PM.Inspector.focusText(L);
  });
}

function startPan(e) {
  const sx = e.clientX, sy = e.clientY, start = [V.pan[0], V.pan[1]];
  V.fit = false;
  const move = (ev) => { V.pan = [start[0] + (ev.clientX - sx), start[1] + (ev.clientY - sy)]; V.layout(); };
  const up = () => { window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
}

function onDown(e) {
  if (e.button !== 0) return;

  /* AE-style tool override: Hand pans, Zoom zooms. Space/⌘ temporarily = Hand. */
  const tool = (PM.tool === 'hand' || PM.tool === 'zoom') ? PM.tool : (e.metaKey || e.altKey ? 'hand' : 'select');
  if (tool === 'hand') return startPan(e);
  if (tool === 'zoom') {
    const dir = e.shiftKey ? -1 : 1;
    V.fit = false;
    V.zoom = clamp((V.shown || 1) * (dir > 0 ? 1.25 : .8), .05, 8);
    V.layout();
    return;
  }

  const [x, y] = toComp(e);
  const T = PM.time;

  /* handle grab on the selected layer */
  const sel = PM.firstSel();
  if (sel && PM.active(sel, T)) {
    const hit = handleAt(sel, x, y, T);
    if (hit) return startTransform(e, sel, hit, T);
  }
  const L = PM.GL.pick(x, y, T);
  if (!L) { if (!e.shiftKey) PM.selectLayers([]); return; }
  PM.selectLayers(L.id, e.shiftKey);
  startMove(e, PM.selLayers().filter(l => !l.lock), T);
}

function handleAt(L, x, y, T) {
  const b = PM.GL.bounds(L, T); if (!b) return null;
  const m = PM.worldMatrix(L, T);
  const det = m[0] * m[3] - m[1] * m[2]; if (!det) return null;
  const dx = x - m[4], dy = y - m[5];
  const lx = (dx * m[3] - dy * m[2]) / det, ly = (dy * m[0] - dx * m[1]) / det;
  const tol = 9 / (V.shown * Math.max(.05, Math.hypot(m[0], m[1])));
  for (let i = 0; i < HANDLES.length; i++) {
    const hx = b.x0 + b.w * HANDLES[i][0], hy = b.y0 + b.h * HANDLES[i][1];
    if (Math.abs(lx - hx) < tol && Math.abs(ly - hy) < tol) return { i, corner: HANDLES[i] };
  }
  /* just outside a corner → rotate */
  for (const [hx0, hy0] of [[0, 0], [1, 0], [1, 1], [0, 1]]) {
    const hx = b.x0 + b.w * hx0, hy = b.y0 + b.h * hy0;
    if (Math.abs(lx - hx) < tol * 2.4 && Math.abs(ly - hy) < tol * 2.4) return { rotate: true };
  }
  return null;
}

function startMove(e, layers, T) {
  if (!layers.length) return;
  const start = layers.map(L => ({ L, x: PM.ev(L, 'position.x', T), y: PM.ev(L, 'position.y', T) }));
  const selectedIds = new Set(layers.map(L => L.id));
  const correctionRoots = start.filter(s => !hasSelectedAncestor(s.L, selectedIds));
  const targets = snapshotSnapTargets(T, selectedIds);
  const clearGuides = () => { V.guides = null; PM.invalidate('render'); };
  PM.Edit.begin('Move layer', { origin: 'canvas' });
  let moved = false;
  PM.drag(e, {
    move: (dx, dy, ev) => {
      moved = true;
      let ddx = dx / V.shown, ddy = dy / V.shown;
      const axes = { x: true, y: true };
      if (ev.shiftKey) {
        if (Math.abs(ddx) > Math.abs(ddy)) { ddy = 0; axes.y = false; }
        else { ddx = 0; axes.x = false; }
      }
      start.forEach(s => {
        setOrKey(s.L, 'position.x', s.x + ddx, T);
        setOrKey(s.L, 'position.y', s.y + ddy, T);
      });
      let snap = { dx: 0, dy: 0, x: null, y: null };
      if (PM.snap) snap = alignmentSnap(unionBounds(layers, T), targets, 8 / Math.max(.02, V.shown), axes);
      correctionRoots.forEach(s => {
        const [cx, cy] = worldDeltaToLocal(s.L, T, snap.dx, snap.dy);
        if (cx) setOrKey(s.L, 'position.x', s.x + ddx + cx, T);
        if (cy) setOrKey(s.L, 'position.y', s.y + ddy + cy, T);
      });
      V.guides = PM.snap && (snap.x || snap.y)
        ? { x: snap.x ? snap.x.value : null, y: snap.y ? snap.y.value : null }
        : null;
      PM.invalidate();
    },
    up: () => { clearGuides(); moved ? PM.Edit.commit('Move layer') : PM.Edit.cancel(); PM.Inspector.refresh(); },
    cancel: () => { clearGuides(); PM.Edit.cancel(); PM.Inspector.refresh(); },
  });
}

function startTransform(e, L, hit, T) {
  const b = PM.GL.bounds(L, T);
  const s0 = { sx: PM.ev(L, 'scale.x', T), sy: PM.ev(L, 'scale.y', T), r: PM.ev(L, 'rotation', T) };
  const m = PM.worldMatrix(L, T);
  const cx = m[4], cy = m[5];
  const r0 = V.ov.getBoundingClientRect();
  const a0 = Math.atan2((e.clientY - r0.top) / V.shown - cy, (e.clientX - r0.left) / V.shown - cx);
  PM.Edit.begin(hit.rotate ? 'Rotate layer' : 'Scale layer', { origin: 'canvas' });
  let moved = false;
  PM.drag(e, {
    cursor: hit.rotate ? 'grabbing' : 'nwse-resize',
    move: (dx, dy, ev) => {
      moved = true;
      if (hit.rotate) {
        const a = Math.atan2((ev.clientY - r0.top) / V.shown - cy, (ev.clientX - r0.left) / V.shown - cx);
        let deg = s0.r + (a - a0) * 180 / Math.PI;
        if (ev.shiftKey) deg = Math.round(deg / 15) * 15;
        setOrKey(L, 'rotation', PM.round(deg, 2), T);
      } else {
        const sgnX = hit.corner[0] === 0 ? -1 : hit.corner[0] === 1 ? 1 : 0;
        const sgnY = hit.corner[1] === 0 ? -1 : hit.corner[1] === 1 ? 1 : 0;
        const kx = sgnX ? 1 + (dx / V.shown) * sgnX / Math.max(1, b.w) * 2 : 1;
        const ky = sgnY ? 1 + (dy / V.shown) * sgnY / Math.max(1, b.h) * 2 : 1;
        let nx = s0.sx * kx, ny = s0.sy * ky;
        if (!ev.altKey) { const k = sgnX && sgnY ? Math.max(kx, ky) : (sgnX ? kx : ky); nx = s0.sx * k; ny = s0.sy * k; }
        setOrKey(L, 'scale.x', PM.round(nx, 2), T);
        setOrKey(L, 'scale.y', PM.round(ny, 2), T);
      }
      PM.invalidate();
    },
    up: () => { moved ? PM.Edit.commit() : PM.Edit.cancel(); PM.Inspector.refresh(); },
  });
}

/** Write a value: sets a keyframe when the channel is animated, otherwise the static value. */
function setOrKey(L, key, v, T) {
  return PM.Edit.dispatch({
    type: 'set_property', target: L.id, path: key, value: v, time: T,
    mode: 'auto', preserveHandEdits: false, markIntent: 'human',
  });
}
PM.setOrKey = setOrKey;
})();
