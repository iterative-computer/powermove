/* Powermove — playback engine. One rAF loop, adaptive quality, sample-accurate audio. */
(() => {
const PM = window.PM;

PM.time = 0;
PM.playing = false;
PM.quality = 1;          // render scale
PM.loop = true;
PM.snap = true;

const E = { fps: 0, ms: 0, drops: 0, budget: 1000 / 60, auto: true };
PM.perf = E;

/* ── audio ─────────────────────────────────────────────── */
const AU = { ctx: null, nodes: new Map() };
PM.audio = AU;
/* HTMLMediaElement.play() settles asynchronously. A pause can happen while a
   start is still pending, so remember the desired state and reassert pause
   when a late start completes. The pending flag also prevents a new play()
   request from being issued on every animation frame. */
const mediaState = new WeakMap();
function ensureMediaPlaying(el) {
  let state = mediaState.get(el);
  if (!state) { state = { desired: false, pending: false }; mediaState.set(el, state); }
  state.desired = true;
  if (state.pending || !el.paused) return;
  state.pending = true;
  let started;
  try { started = el.play(); }
  catch (e) { state.pending = false; state.desired = false; return; }
  Promise.resolve(started).then(() => {
    state.pending = false;
    if (!state.desired) { try { el.pause(); } catch (e) { } }
  }, () => {
    state.pending = false;
  });
}
function ensureMediaPaused(el) {
  let state = mediaState.get(el);
  if (!state) { state = { desired: false, pending: false }; mediaState.set(el, state); }
  const mustStopPendingStart = state.pending;
  state.desired = false;
  if (mustStopPendingStart || !el.paused) { try { el.pause(); } catch (e) { } }
}
function actx() {
  if (!AU.ctx) AU.ctx = new (window.AudioContext || window.webkitAudioContext)();
  if (AU.ctx.state === 'suspended') AU.ctx.resume();
  return AU.ctx;
}
function audioLayers() { return PM.proj.layers.filter(l => l.type === 'audio'); }
function disposeAudioNode(id, n) {
  try { n.el.pause(); n.el.src = ''; } catch (e) { }
  try { n.gain.disconnect(); } catch (e) { }
  try { n.src.disconnect(); } catch (e) { }
  AU.nodes.delete(id);
}
function audioNode(L, a) {
  let n = AU.nodes.get(L.id);
  /* Changing an existing layer's Source must change what is heard too. A
     MediaElementSource cannot be repointed safely, so rebuild just that node. */
  if (n && n.assetId !== L.d.asset) { disposeAudioNode(L.id, n); n = null; }
  if (n) return n;
  const el = a.el.cloneNode();
  if (!el.src && a.url) el.src = a.url;
  el.preload = 'auto'; el.playsInline = true; el.muted = false;
  const src = AU.ctx.createMediaElementSource(el);
  const gain = AU.ctx.createGain();
  src.connect(gain).connect(AU.ctx.destination);
  n = { el, gain, src, assetId: L.d.asset, active: false };
  AU.nodes.set(L.id, n);
  return n;
}
function gainAt(L, local) {
  const base = PM.clamp(L.d.gain == null ? 1 : L.d.gain, 0, 4);
  const fi = Math.max(0, Number(L.d.fadeIn) || 0);
  const fo = Math.max(0, Number(L.d.fadeOut) || 0);
  let envelope = 1;
  if (fi > 0) envelope = Math.min(envelope, PM.clamp(local / fi, 0, 1));
  if (fo > 0) envelope = Math.min(envelope, PM.clamp((L.dur - local) / fo, 0, 1));
  return base * envelope;
}
/* Keep timeline audio declaratively synchronized. This runs with the playback
   clock so a clip starts when the playhead enters it, stops at its out point,
   follows seeks/trim changes, and makes the inspector fades audible. */
function syncAudio(T, seek = false) {
  const layers = audioLayers();
  const soloOn = PM.proj.layers.some(L => L.on && L.solo);
  for (const L of layers) {
    let n = AU.nodes.get(L.id);
    const a = L.d.asset && PM.assets.get(L.d.asset);
    const local = T - L.from;
    const sourceTime = local + (Number(L.d.trim) || 0);
    const beforeAssetEnd = !a || !(a.dur > 0) || sourceTime < a.dur - 1e-3;
    const active = !!(PM.playing && L.on && L.d.asset && a && (!soloOn || L.solo)
      && local >= -1e-6 && local < L.dur - 1e-6 && sourceTime >= 0 && beforeAssetEnd);
    if (!active) {
      if (n) { n.active = false; ensureMediaPaused(n.el); }
      continue;
    }
    if (!AU.ctx) actx();
    n = audioNode(L, a);
    const maxTime = a.dur > 0 ? Math.max(0, a.dur - .001) : Math.max(0, sourceTime);
    const targetTime = PM.clamp(sourceTime, 0, maxTime);
    const drifted = Math.abs((Number(n.el.currentTime) || 0) - targetTime) > .12;
    if (!n.active || seek || drifted) {
      try { n.el.currentTime = targetTime; } catch (e) { }
    }
    n.active = true;
    n.gain.gain.value = gainAt(L, local);
    ensureMediaPlaying(n.el);
  }
  /* Disabled, deleted, or newly-soloed layers may no longer be in the active
     set, but their media element still needs an explicit pause. */
  const layerIds = new Set(layers.map(L => L.id));
  for (const [id, n] of AU.nodes) if (!layerIds.has(id)) ensureMediaPaused(n.el);
}
function startAudio(T) { actx(); syncAudio(T, true); }
function stopAudio() { AU.nodes.forEach(n => { n.active = false; ensureMediaPaused(n.el); }); }
/* Evict WebAudio nodes for layers that no longer exist — otherwise deleted/duplicated
   audio layers leak media-element sources and eventually exhaust the audio graph. */
function evictStaleAudio() {
  const live = new Set(PM.proj.layers.map(l => l.id));
  for (const [id, n] of AU.nodes) {
    if (!live.has(id)) disposeAudioNode(id, n);
  }
}
PM.bus.on('layers', () => { evictStaleAudio(); if (PM.playing) syncAudio(PM.time, true); });
PM.bus.on('assets', () => { if (PM.playing) syncAudio(PM.time, true); });
PM.bus.on('project', () => { stopAudio(); evictStaleAudio(); });
function scrubVideos(T) {
  for (const L of PM.proj.layers) {
    if (L.type !== 'video' || !L.d.asset) continue;
    const a = PM.assets.get(L.d.asset); if (!a) continue;
    const inRange = PM.active(L, T);
    /* playback position must respect layer speed, matching the compositor's vt math */
    const vt = PM.clamp((T - L.from) * (L.d.speed || 1) + (L.d.trim || 0), 0, Math.max(0, (a.dur || 0) - .04));
    if (PM.playing && inRange) { if (a.el.paused) { a.el.currentTime = vt; ensureMediaPlaying(a.el); } }
    else ensureMediaPaused(a.el);
    if (!PM.playing && Math.abs(a.el.currentTime - vt) > .02) { try { a.el.currentTime = vt; } catch (e) { } }
  }
}

/* ── transport ─────────────────────────────────────────── */
PM.setTime = (t, opt = {}) => {
  const p = PM.proj;
  t = PM.clamp(t, 0, p.dur);
  if (!opt.raw) t = PM.snapF(t, p.fps);
  if (t === PM.time && !opt.force) return;
  PM.time = t;
  if (PM.playing) syncAudio(t, true);
  PM.bus.emit('time', t);
  PM.invalidate('render'); PM.invalidate('timeline'); PM.invalidate('status');
  if (!PM.playing) PM.invalidate('ui');
};
PM.step = (frames) => PM.setTime(PM.time + frames / PM.proj.fps);

PM.play = () => {
  if (PM.playing) return;
  PM.playing = true;
  clock.last = performance.now();
  clock.base = PM.time;
  startAudio(PM.time);
  PM.bus.emit('transport');
  PM.invalidate('ui');
};
PM.pause = () => {
  const wasPlaying = PM.playing;
  PM.playing = false;
  stopAudio(); scrubVideos(PM.time);
  if (!wasPlaying) return;
  PM.bus.emit('transport');
  PM.invalidate();
};
PM.toggle = () => (PM.playing ? PM.pause() : PM.play());

const clock = { last: 0, base: 0, acc: 0, frames: 0, t0: 0 };

/* ── the single frame loop ─────────────────────────────── */
let needsDraw = true;
PM.bus.on('draw', () => { needsDraw = true; });

function frame(now) {
  requestAnimationFrame(frame);
  const p = PM.proj;
  if (PM.playing) {
    const dt = Math.min(.25, (now - clock.last) / 1000);
    clock.last = now;
    let t = PM.time + dt;
    const [ws, we] = p.work && p.work[1] > p.work[0] ? p.work : [0, p.dur];
    if (t >= we - 1e-6) {
      if (PM.loop) { t = ws; startAudio(t); }
      else { PM.setTime(we); PM.pause(); return; }
    }
    PM.time = t;
    syncAudio(t);
    PM.bus.emit('time', t);
    needsDraw = true;
    PM.invalidate('timeline');
    scrubVideos(t);
  }
  if (!needsDraw || !PM.GL.gl) return;
  needsDraw = false;
  const t0 = performance.now();
  PM.GL.render(PM.time, {
    mblur: true, mbSamples: PM.playing ? 6 : 12,
    shutter: p.shutter || .5, hideShy: false,
  });
  PM.bus.emit('overlay');
  const ms = performance.now() - t0;
  E.ms = E.ms * .85 + ms * .15;
  clock.frames++;
  if (now - clock.t0 > 500) { E.fps = Math.round(clock.frames * 1000 / (now - clock.t0)); clock.frames = 0; clock.t0 = now; PM.invalidate('status'); }
  /* adaptive quality while playing so scrubbing never stutters */
  if (E.auto && PM.playing) {
    if (E.ms > 22 && PM.quality > .5) { PM.quality = Math.max(.5, PM.quality - .25); PM.bus.emit('quality'); }
    else if (E.ms < 9 && PM.quality < 1) { PM.quality = Math.min(1, PM.quality + .25); PM.bus.emit('quality'); }
  }
}
requestAnimationFrame(frame);

/* Any structural change invalidates the frame. */
['layers', 'sel', 'project', 'assets', 'quality'].forEach(ev => PM.bus.on(ev, () => PM.invalidate()));

/* ── offscreen frame render (agent `look`, exporter, thumbnails) ── */
PM.renderFrameTo = (T, w, h) => {
  const cv = PM.GL.canvas;
  const ow = cv.width, oh = cv.height, oq = PM.quality;
  PM.GL.resize(w, h);
  PM.quality = 1;
  PM.GL.render(T, { mblur: true, mbSamples: 16, shutter: PM.proj.shutter || .5 });
  const out = document.createElement('canvas');
  out.width = w; out.height = h;
  out.getContext('2d').drawImage(cv, 0, 0);
  PM.quality = oq;
  PM.GL.resize(ow, oh);
  PM.invalidate('render');
  return out;
};
})();
