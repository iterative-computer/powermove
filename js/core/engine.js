/* Powermove — playback engine. One rAF loop, adaptive quality, sample-accurate audio. */
(() => {
const PM = window.PM;

PM.time = 0;
PM.playing = false;
PM.quality = 1;          // render scale
PM.loop = true;
PM.mblurOn = true;
PM.snap = true;
PM.guides = true;

const E = { fps: 0, ms: 0, drops: 0, budget: 1000 / 60, auto: true };
PM.perf = E;

/* ── audio ─────────────────────────────────────────────── */
const AU = { ctx: null, nodes: new Map() };
PM.audio = AU;
function actx() {
  if (!AU.ctx) AU.ctx = new (window.AudioContext || window.webkitAudioContext)();
  if (AU.ctx.state === 'suspended') AU.ctx.resume();
  return AU.ctx;
}
function audioLayers() { return PM.proj.layers.filter(l => l.type === 'audio' && l.on && l.d.asset); }

function startAudio(T) {
  actx();
  for (const L of audioLayers()) {
    const a = PM.assets.get(L.d.asset); if (!a) continue;
    const local = T - L.from;
    if (local < -.05 || local >= L.dur) continue;
    let n = AU.nodes.get(L.id);
    if (!n) {
      const el = a.el.cloneNode();
      const src = AU.ctx.createMediaElementSource(el);
      const gain = AU.ctx.createGain();
      src.connect(gain).connect(AU.ctx.destination);
      n = { el, gain }; AU.nodes.set(L.id, n);
    }
    n.gain.gain.value = PM.clamp(L.d.gain == null ? 1 : L.d.gain, 0, 4);
    try { n.el.currentTime = PM.clamp(local + (L.d.trim || 0), 0, a.dur || 0); } catch (e) { }
    n.el.play().catch(() => { });
  }
}
function stopAudio() { AU.nodes.forEach(n => n.el.pause()); }
function scrubVideos(T) {
  for (const L of PM.proj.layers) {
    if (L.type !== 'video' || !L.d.asset) continue;
    const a = PM.assets.get(L.d.asset); if (!a) continue;
    const inRange = PM.active(L, T);
    if (PM.playing && inRange) { if (a.el.paused) { a.el.currentTime = PM.clamp(T - L.from + (L.d.trim || 0), 0, a.dur); a.el.play().catch(() => { }); } }
    else if (!a.el.paused) a.el.pause();
  }
}

/* ── transport ─────────────────────────────────────────── */
PM.setTime = (t, opt = {}) => {
  const p = PM.proj;
  t = PM.clamp(t, 0, p.dur);
  if (!opt.raw) t = PM.snapF(t, p.fps);
  if (t === PM.time && !opt.force) return;
  PM.time = t;
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
  if (!PM.playing) return;
  PM.playing = false;
  stopAudio(); scrubVideos(PM.time);
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
    PM.bus.emit('time', t);
    needsDraw = true;
    PM.invalidate('timeline');
    scrubVideos(t);
  }
  if (!needsDraw || !PM.GL.gl) return;
  needsDraw = false;
  const t0 = performance.now();
  PM.GL.render(PM.time, {
    mblur: PM.mblurOn, mbSamples: PM.playing ? 6 : 12,
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
  PM.GL.render(T, { mblur: PM.mblurOn, mbSamples: 16, shutter: PM.proj.shutter || .5 });
  const out = document.createElement('canvas');
  out.width = w; out.height = h;
  out.getContext('2d').drawImage(cv, 0, 0);
  PM.quality = oq;
  PM.GL.resize(ow, oh);
  PM.invalidate('render');
  return out;
};
})();
