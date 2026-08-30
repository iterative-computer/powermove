/* Ported from js/core/engine.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

const ENGINE_RUNTIME = Symbol.for('powermove.engine.runtime');

export function install(PM: PMRegistry): void {

// Renderer bootstrap can be evaluated again while the same PM registry stays
// alive. A second animation loop advances the shared playhead a second time on
// every display refresh, making clips race past while the preview appears
// frozen or black. Keep the playback clock a singleton for this registry.
if ((PM as any)[ENGINE_RUNTIME]?.active) return;
Object.defineProperty(PM, ENGINE_RUNTIME, {
  value: { active: true },
  configurable: true,
});

PM.time = 0;
PM.playing = false;
PM.quality = 1;          // render scale
PM.loop = true;

const E = { fps: 0, ms: 0, drops: 0, budget: 1000 / 60, auto: true };
PM.perf = E;

/* Video uses HTMLMediaElement.play(), which settles asynchronously. A pause can
   happen while a start is still pending, so remember the desired state and
   reassert pause when a late start completes. Audio has its own decoded-buffer
   scheduler in core/audio.js and never uses this race-prone path. */
const mediaState = new WeakMap<any, any>();
const VIDEO_DRIFT_SECONDS = 0.12;
let videoSeekGeneration = 0;
function ensureMediaPlaying(el: any, expectedTime: number, playbackRate: number) {
  let state = mediaState.get(el);
  if (!state) {
    state = { desired: false, pending: false, expectedTime, playbackRate, seekGeneration: -1 };
    mediaState.set(el, state);
  }
  state.desired = true;
  state.expectedTime = expectedTime;
  state.playbackRate = playbackRate;
  try {
    if (Number.isFinite(playbackRate) && playbackRate > 0 && el.playbackRate !== playbackRate) {
      el.playbackRate = playbackRate;
    }
  } catch (e) { }
  if (!el.paused && !state.pending) {
    // A real timeline jump needs one seek. Ordinary decoder drift does not:
    // repeatedly assigning currentTime flushes queued 4K frames and turns a
    // small clock difference into visibly low-frame-rate playback.
    if (state.seekGeneration !== videoSeekGeneration) {
      try { el.currentTime = expectedTime; state.seekGeneration = videoSeekGeneration; } catch (e) { }
    }
    return;
  }
  if (state.pending) return;
  try { el.currentTime = expectedTime; state.seekGeneration = videoSeekGeneration; } catch (e) { }
  state.pending = true;
  let started;
  try { started = el.play(); }
  catch (e) { state.pending = false; state.desired = false; return; }
  Promise.resolve(started).then(() => {
    state.pending = false;
    if (!state.desired) { try { el.pause(); } catch (e) { } }
    else if (Math.abs(Number(el.currentTime || 0) - state.expectedTime) > VIDEO_DRIFT_SECONDS) {
      // Starting a cold 4K decoder can take several frames. Rejoin the editor
      // clock once the play promise settles instead of carrying that lag for
      // the rest of the clip.
      try { el.currentTime = state.expectedTime; state.seekGeneration = videoSeekGeneration; } catch (e) { }
    }
  }, () => {
    state.pending = false;
  });
}
function ensureMediaPaused(el: any) {
  let state = mediaState.get(el);
  if (!state) { state = { desired: false, pending: false }; mediaState.set(el, state); }
  const mustStopPendingStart = state.pending;
  state.desired = false;
  if (mustStopPendingStart || !el.paused) { try { el.pause(); } catch (e) { } }
}
function scrubVideos(T: any) {
  const videos = PM.ProjectIndex?.layersOfType?.('video', PM.proj) || PM.proj.layers.filter((layer: any) => layer.type === 'video');
  for (const L of videos) {
    if (!L.d.asset) continue;
    const a = PM.assets.get(L.d.asset); if (!a) continue;
    const inRange = PM.active(L, T);
    /* playback position must respect layer speed, matching the compositor's vt math */
    const vt = PM.clamp((T - L.from) * (L.d.speed || 1) + (L.d.trim || 0), 0, Math.max(0, (a.dur || 0) - .04));
    if (PM.playing && inRange) ensureMediaPlaying(a.el, vt, Math.max(.0001, Number(L.d.speed) || 1));
    else ensureMediaPaused(a.el);
    if (!PM.playing && Math.abs(a.el.currentTime - vt) > .02) { try { a.el.currentTime = vt; } catch (e) { } }
  }
}

/* ── transport ─────────────────────────────────────────── */
PM.setTime = (t: any, opt: any = {}) => {
  const p = PM.proj;
  t = PM.clamp(t, 0, p.dur);
  if (!opt.raw) t = PM.snapF(t, p.fps);
  if (t === PM.time && !opt.force) return;
  PM.time = t;
  if (PM.playing) { videoSeekGeneration++; PM.Audio.seek(t); }
  PM.bus.emit('time', t);
  PM.invalidate('render'); PM.invalidate('timeline'); PM.invalidate('status');
  if (!PM.playing) PM.invalidate('ui');
};
PM.step = (frames: any) => PM.setTime(PM.time + frames / PM.proj.fps);

PM.play = () => {
  if (PM.playing) return;
  PM.playing = true;
  clock.last = window.performance.now();
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

function frame(now: any) {
  window.requestAnimationFrame(frame);
  const p = PM.proj;
  if (PM.playing) {
    const dt = Math.min(.25, (now - clock.last) / 1000);
    clock.last = now;
    let t = PM.time + dt;
    const [ws, we] = p.work && p.work[1] > p.work[0] ? p.work : [0, p.dur];
    if (t >= we - 1e-6) {
      if (PM.loop) { t = ws; videoSeekGeneration++; PM.Audio.seek(t); }
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
  const t0 = window.performance.now();
  PM.GL.render(PM.time, {
    mblur: true, mbSamples: PM.playing ? 6 : 12,
    shutter: p.shutter || .5, hideShy: false,
  });
  PM.bus.emit('overlay');
  const ms = window.performance.now() - t0;
  E.ms = E.ms * .85 + ms * .15;
  clock.frames++;
  if (now - clock.t0 > 500) { E.fps = Math.round(clock.frames * 1000 / (now - clock.t0)); clock.frames = 0; clock.t0 = now; PM.invalidate('status'); }
  /* adaptive quality while playing so scrubbing never stutters */
  if (E.auto && PM.playing) {
    if (E.ms > 22 && PM.quality > .5) { PM.quality = Math.max(.5, PM.quality - .25); PM.bus.emit('quality'); }
    else if (E.ms < 9 && PM.quality < 1) { PM.quality = Math.min(1, PM.quality + .25); PM.bus.emit('quality'); }
  }
}
window.requestAnimationFrame(frame);

/* Any structural change invalidates the frame. */
['layers', 'sel', 'project', 'assets', 'quality'].forEach(ev => PM.bus.on(ev, () => PM.invalidate()));

/* ── offscreen frame render (agent `look`, exporter, thumbnails) ── */
PM.renderFrameTo = (T: any, w: any, h: any, options: any = {}) => {
  const cv = PM.GL.canvas;
  const ow = cv.width, oh = cv.height, oq = PM.quality;
  PM.GL.resize(w, h);
  PM.quality = 1;
  const motionBlur = options.mblur !== false;
  PM.GL.render(T, { mblur: motionBlur, mbSamples: motionBlur ? Math.max(1, Number(options.mbSamples) || 16) : 1, shutter: PM.proj.shutter || .5 });
  const out = window.document.createElement('canvas');
  out.width = w; out.height = h;
  (out.getContext('2d') as any).drawImage(cv, 0, 0);
  PM.quality = oq;
  PM.GL.resize(ow, oh);
  PM.invalidate('render');
  return out;
};
}
