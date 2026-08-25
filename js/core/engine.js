/* No longer loaded — superseded by src/renderer/src/legacy/core/engine.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — playback transport. One rAF loop, adaptive quality, video sync. */
(() => {
const PM = window.PM;

PM.time = 0;
PM.playing = false;
PM.quality = 1;          // render scale
PM.loop = true;
PM.snap = true;

const E = { fps: 0, ms: 0, drops: 0, budget: 1000 / 60, auto: true };
PM.perf = E;

/* Video uses HTMLMediaElement.play(), which settles asynchronously. A pause can
   happen while a start is still pending, so remember the desired state and
   reassert pause when a late start completes. Audio has its own decoded-buffer
   scheduler in core/audio.js and never uses this race-prone path. */
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
  if (PM.playing) PM.Audio.seek(t);
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
  PM.Audio.start(PM.time);
  PM.bus.emit('transport');
  PM.invalidate('ui');
};
PM.pause = () => {
  const wasPlaying = PM.playing;
  PM.playing = false;
  PM.Audio.pause(); scrubVideos(PM.time);
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
      if (PM.loop) { t = ws; PM.Audio.seek(t); }
      else { PM.setTime(we); PM.pause(); return; }
    }
    PM.time = t;
    PM.Audio.tick(t);
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
