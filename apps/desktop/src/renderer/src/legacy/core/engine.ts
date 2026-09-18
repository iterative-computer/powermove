import { videoClipsAt } from './video-timeline';
import { previewVideoElement } from './video-preview';
import { layerVideoElement, pruneVideoInstances } from './video-instances';
import { cancelPreviewVideoSeek, seekPreviewVideo } from './video-seek';
import { sequencePlaybackTime } from '../../../../shared/image-sequence';
import { prepareFrame } from './frame-preparation';
import { installPreviewCache } from './preview-cache';
import { sourceTime } from './retiming';
import { viewerService } from './services';
/* Ported from js/core/engine.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

const ENGINE_RUNTIME = Symbol.for('powermove.engine.runtime');

export function install(PM: PMRegistry): void {
PM.prepareFrame=(time:number)=>prepareFrame(PM,time);

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
/* The rate the compositor actually draws at, which is what a playing decoder
   has to be placed against. */
const drawnFps = () => Math.max(1, Number(PM.previewFps || PM.proj?.fps) || 30);
const VIDEO_DRIFT_SECONDS = 0.12;
let videoSeekGeneration = 0;
function ensureMediaPlaying(el: any, expectedTime: number, playbackRate: number) {
  cancelPreviewVideoSeek(el);
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
    // small clock difference into visibly low-frame-rate playback. The element
    // sweeps continuously through a frame the target only steps between, so
    // any tolerance near one frame trips on healthy playback as well.
    if (state.seekGeneration !== videoSeekGeneration) {
      try { el.currentTime = expectedTime; state.seekGeneration = videoSeekGeneration; } catch (e) { }
    }
    return;
  }
  if (state.pending) return;
  try {
    // Do not flush a frame already decoded by lookahead at the cut.
    if (Math.abs(el.currentTime - expectedTime) > 1 / drawnFps()) el.currentTime = expectedTime;
    state.seekGeneration = videoSeekGeneration;
  } catch (e) { }
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
let playingVideos = new Set<any>();
let decoderAssets = new Set<any>();
const frameVideoSeeks = new Map<any, number>();
function flushFrameVideoSeeks() {
  for (const [el, target] of frameVideoSeeks) seekPreviewVideo(el, target, .0005);
  frameVideoSeeks.clear();
}
function scrubVideos(T: any) {
  frameVideoSeeks.clear();
  const retained = new Map<any, Set<string>>();
  const retain = (asset: any, id: string) => {
    let ids = retained.get(asset);
    if (!ids) retained.set(asset, ids = new Set());
    ids.add(id);
  };
  const videos = PM.ProjectIndex?.layersOfType?.('video', PM.proj) || PM.proj.layers.filter((layer: any) => layer.type === 'video');
  /* The playhead runs continuously but the compositor draws whole frames, so a
     decoder chasing the raw clock is aimed between two source frames. Place a
     playing one against the frame actually being drawn. */
  const drawn = PM.playing ? Math.floor(T * drawnFps()) / drawnFps() : T;
  // Resolve only active clips, so inactive copies never claim or pause a
  // decoder owned by a visible clip.
  const owners = new Map<any, any>();
  for (const clip of videoClipsAt(PM, drawn)) {
    const { layer, asset } = clip;
    const source = previewVideoElement(PM, asset);
    const el = layerVideoElement(PM, asset, clip.id);
    retain(asset, clip.id);
    const unused = source === asset.el ? asset.preview?.el : asset.el;
    if (unused) { cancelPreviewVideoSeek(unused); ensureMediaPaused(unused); }
    owners.set(el, { layer, asset, el, clip });
  }
  for (const el of playingVideos) if (!owners.has(el)) { cancelPreviewVideoSeek(el); ensureMediaPaused(el); }
  playingVideos = new Set(owners.keys());
  // Decode the next cut while it is still offscreen. Limit lookahead so a
  // long timeline does not eagerly allocate every clip's decoder.
  if (PM.playing) for (const layer of videos) {
    if (layer.from <= drawn || layer.from > drawn + .5 || !PM.active(layer, layer.from)) continue;
    const asset = PM.assets.get(layer.d.asset);
    if (!asset?.el) continue;
    const el = layerVideoElement(PM, asset, layer.id);
    retain(asset, layer.id);
    ensureMediaPaused(el);
    const at = sourceTime(PM, layer, layer.from);
    seekPreviewVideo(el, sequencePlaybackTime(asset, at) ?? PM.clamp(at, 0, Math.max(0, (asset.dur || 0) - .04)), .0005);
  }

  // Prepare the loop entrance only if it owns a different decoder. Seeking an
  // active clip early would interrupt the tail of the current loop.
  const [start, end] = PM.proj.work?.[1] > PM.proj.work?.[0] ? PM.proj.work : [0, PM.proj.dur];
  if (PM.playing && PM.loop && end - drawn <= .5) for (const clip of videoClipsAt(PM, start)) {
    const el = layerVideoElement(PM, clip.asset, clip.id);
    retain(clip.asset, clip.id);
    if (owners.has(el)) continue;
    ensureMediaPaused(el); seekPreviewVideo(el, clip.at, .0005);
  }

  // Do not retain a decoder (and its decoded frame buffers) for every clip
  // ever visited. Only visible clips and the bounded lookahead stay allocated.
  const empty = new Set<string>();
  for (const asset of new Set([...decoderAssets, ...retained.keys()])) {
    pruneVideoInstances(asset, retained.get(asset) ?? empty, true);
  }
  decoderAssets = new Set(retained.keys());

  for (const { layer: L, asset: a, el, clip } of owners.values()) {
    const previous = mediaState.get(el);
    if (previous && previous.layer !== clip.id) previous.seekGeneration = -1;

    /* playback position must respect layer speed, matching the compositor's vt math */
    const vt = clip.at;
    /* A still is snapped to the frame's first millisecond, which is where a
       seek belongs. A decoder running on its own clock always joins a little
       late, and on that leading edge the lag presents the frame before — which
       is what held a clip's first frame over two composition frames. Aim it at
       the middle of the frame being drawn, which absorbs the lag. Only a
       sequence publishes the frame grid that needs; other media keeps the
       continuous position it has always used. */
    const streamAt = clip.streamAt ?? vt;
    if (PM.playing && clip.rate != null) ensureMediaPlaying(el, streamAt, Math.max(.0001, clip.rate));
    else ensureMediaPaused(el);
    const state = mediaState.get(el); if (state) state.layer = clip.id;
    if (!PM.playing) seekPreviewVideo(el, vt, .0005);
    else if (clip.rate == null) {
      // Present the completed decode before starting another seek. Starting it
      // before render drops readyState and starves the texture of every frame.
      frameVideoSeeks.set(el, vt);
    }
  }
}

for (const event of ['layers', 'project', 'assets']) PM.bus.on(event, () => {
  const layers = PM.ProjectIndex?.allLayers?.() || PM.proj.layers;
  const byAsset = new Map<string, Set<string>>();
  for (const layer of layers) {
    if (layer.type !== 'video') continue;
    let ids = byAsset.get(layer.d.asset);
    if (!ids) byAsset.set(layer.d.asset, ids = new Set());
    ids.add(layer.id);
  }
  const empty = new Set<string>();
  for (const asset of PM.assets.map?.values() ?? []) pruneVideoInstances(asset, byAsset.get(asset.id) ?? empty);
});

/* ── transport ─────────────────────────────────────────── */
PM.setTime = (t: any, opt: any = {}) => {
  const p = PM.proj;
  t = PM.clamp(t, 0, p.dur);
  if (!opt.raw) t = PM.snapF(t, p.fps);
  if (t === PM.time && !opt.force) return;
  PM.preparedVideoFrames=null;
  PM.time = t;
  if (PM.playing) { clock.base=t;clock.origin=window.performance.now();clock.last=clock.origin;clock.cycle=0;videoSeekGeneration++; PM.Audio.seek(t); }
  PM.bus.emit('time', t);
  PM.invalidate('render'); PM.invalidate('timeline'); PM.invalidate('status');
  if (!PM.playing) PM.invalidate('ui');
};
PM.step = (frames: any) => PM.setTime(PM.time + frames / PM.proj.fps);

PM.play = () => {
  if (PM.playing) return;
  PM.preparedVideoFrames=null;
  PM.playing = true;
  lastRenderTime = NaN;
  // AudioContext construction can block the first Play on some devices. Do
  // that setup before starting the transport clock so it cannot skip frames.
  PM.Audio.start(PM.time);
  clock.last = window.performance.now();
  // Paused redraws are demand-driven. Start a new measurement window here so
  // idle time and the previous playback session cannot depress the FPS readout.
  clock.t0 = clock.last; clock.frames = 0; E.fps = 0;
  PM.invalidate('status');
  clock.base = PM.time;
  clock.origin=clock.last;clock.cycle=0;
  PM.bus.emit('transport');
  PM.invalidate('ui');
};
PM.pause = () => {
  const wasPlaying = PM.playing;
  PM.playing = false;
  E.fps = 0; clock.frames = 0;
  PM.Audio.pause(); scrubVideos(PM.time);
  if (!wasPlaying) return;
  PM.bus.emit('transport');
  PM.invalidate();
};
PM.toggle = () => (PM.Preview?.active ? PM.Preview.stop() : PM.playing ? PM.pause() : PM.play());
installPreviewCache(PM);

const clock = { last: 0, base: 0, origin:0, cycle:0, acc: 0, frames: 0, t0: 0 };

/* ── the single frame loop ─────────────────────────────── */
let needsDraw = true;
let contentGeneration = 0, renderedGeneration = -1;
let lastRenderTime = NaN, lastProject: any = null;
let lastRenderFailure = -Infinity;
let interactionUntil = 0, refinePending = false, lastQualityChange = -Infinity;
let inputUntil = 0;
PM.interactionActive = () => window.performance.now() < Math.max(inputUntil, interactionUntil);
for (const event of ['pointerdown', 'pointermove', 'wheel', 'keydown']) window.document?.addEventListener?.(event, () => { inputUntil = window.performance.now() + 500; }, { passive: true });
PM.bus.on('time', () => { if (!PM.playing) { interactionUntil = window.performance.now() + 150; refinePending = true; } });
for (const event of ['layers', 'project', 'assets', 'quality']) PM.bus.on(event, () => { contentGeneration++; });

PM.bus.on('draw', () => { needsDraw = true; });

function frame(now: any) {
  window.requestAnimationFrame(frame);
  // Offline renderers own the shared video decoders and canvas until done.
  // Redrawing the preview here seeks them back to the editor playhead between
  // export frames, producing repeated/frozen frames in the encoded video.
  if (PM.agentFrameCapture || PM.Export?.busy || PM.Preview?.preparing || PM.Preview?.active) return;
  const p = PM.proj;
  if (PM.playing) {
    if (now - clock.t0 > 500) { E.fps = Math.round(clock.frames * 1000 / (now - clock.t0)); clock.frames = 0; clock.t0 = now; PM.invalidate('status'); }
    const gap = (now-clock.last)/1000;
    clock.last = now;
    let t = clock.base+Math.max(0,(now-clock.origin)/1000);
    const [ws,we]=p.work&&p.work[1]>p.work[0]?p.work:[0,p.dur];
    let cycle=0;
    if(t>=we-1e-6){
      if(!PM.loop){PM.setTime(we);PM.pause();return;}
      const span=we-ws;cycle=Math.max(1,Math.floor((t-ws)/span));t=ws+((t-ws)%span+span)%span;
    }
    if(cycle!==clock.cycle||gap>.25){videoSeekGeneration++;PM.Audio.seek(t);clock.cycle=cycle;}
    PM.time = t;
    PM.Audio.tick(t);
    PM.bus.emit('time', t);
    needsDraw = true;
    PM.invalidate('timeline');
    scrubVideos(t);
  }
  const interactive = !PM.playing && now < interactionUntil;
  if (refinePending && !interactive && !PM.playing) { needsDraw = true; refinePending = false; }
  if (!needsDraw || !PM.GL.gl) return;
  // During a paused zoom gesture the viewer can transform valid presented
  // pixels immediately. Keep the redraw pending until refinement is needed;
  // playback and content/time changes always bypass this navigation-only path.
  if (!PM.playing && viewerService(PM)?.deferNavigationRender(now)) return;
  const renderTime=PM.playing ? Math.floor(PM.time*(PM.previewFps||p.fps))/(PM.previewFps||p.fps) : PM.time;
  if (PM.playing && renderTime === lastRenderTime && p === lastProject && renderedGeneration === contentGeneration) {
    PM.bus.emit('overlay');
    return;
  }
  if (!PM.playing && !interactive && PM.quality < 1 && E.auto) { PM.quality = 1; PM.bus.emit('quality'); }
  needsDraw = false;
  const t0 = window.performance.now();
  let presented = true;
  try {
    presented = PM.GL.render(renderTime, {
      mblur: !interactive, mbSamples: PM.playing ? 6 : 12,
      shutter: p.shutter || .5, hideShy: false,
    }) !== false;
  } catch (error) {
    /* A throw here used to escape the animation frame with the redraw flag
       already cleared, leaving the last frame (or black) on screen until the
       next invalidate threw again. Report it and keep the loop alive. */
    if (now - lastRenderFailure > 2000) {
      lastRenderFailure = now;
      console.error('[engine] frame render failed', error);
      PM.toast?.('The preview could not be rendered. See the console for details.');
    }
  }
  flushFrameVideoSeeks();
  if (presented) { lastRenderTime = renderTime; lastProject = p; renderedGeneration = contentGeneration; }
  else needsDraw = true;
  PM.bus.emit('overlay');
  const ms = window.performance.now() - t0;
  E.ms = E.ms * .85 + ms * .15;
  if (PM.playing) {
    clock.frames++;
  }
  /* adaptive quality while playing so scrubbing never stutters */
  if (E.auto && (PM.playing || interactive) && now - lastQualityChange > 750) {
    if (E.ms > 22 && PM.quality > .5) { PM.quality = Math.max(.5, PM.quality - .25); lastQualityChange = now; PM.bus.emit('quality'); }
    else if (E.ms < 9 && PM.quality < 1) { PM.quality = Math.min(1, PM.quality + .25); lastQualityChange = now; PM.bus.emit('quality'); }
  }
}
window.requestAnimationFrame(frame);
const resumeView=()=>{if(window.document?.visibilityState==='hidden')return;videoSeekGeneration++;viewerService(PM)?.layout();needsDraw=true;PM.invalidate();};
window.addEventListener?.('focus',resumeView);
window.document?.addEventListener?.('visibilitychange',resumeView);

/* Any structural change invalidates the frame. */
['layers', 'sel', 'project', 'assets', 'quality'].forEach(ev => PM.bus.on(ev, () => PM.invalidate()));

/* ── offscreen frame render (agent `look`, exporter, thumbnails) ── */
PM.renderFrameTo = (T: any, w: any, h: any, options: any = {}) => {
  const quality = PM.quality;
  try {
    PM.quality = 1;
    const motionBlur = options.mblur !== false;
    // Capture into an offscreen target. Resizing the visible canvas used to
    // destroy its large preview buffers twice for every small thumbnail.
    const pixels = PM.GL.renderToPixels(T, w, h, {
      mblur: motionBlur, mbSamples: motionBlur ? Math.max(1, Number(options.mbSamples) || 16) : 1,
      shutter: PM.proj.shutter || .5, opaque: true,
    });
    const out = window.document.createElement('canvas');
    out.width = w; out.height = h;
    const context = out.getContext('2d')!;
    const image = context.createImageData(w, h);
    for (let y = 0; y < h; y++) {
      image.data.set(pixels.subarray((h - y - 1) * w * 4, (h - y) * w * 4), y * w * 4);
    }
    // The established capture API is opaque, with transparency over black,
    // just like copying the alpha:false WebGL presentation canvas.
    for (let i = 3; i < image.data.length; i += 4) image.data[i] = 255;
    context.putImageData(image, 0, 0);
    return out;
  } finally {
    PM.quality = quality;
  }
};
}
