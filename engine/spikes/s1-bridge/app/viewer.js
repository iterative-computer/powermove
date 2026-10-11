'use strict';
// Spike S1 viewer, main world (plan section 18 steps 3-5). Receives engine frames as
// VideoFrames from the preload, holds playback frames in a jitter buffer until their
// presentation time, draws them into a WebGPU canvas, and returns each pool slot to
// the engine once the GPU work that read it has completed. Input goes straight to
// the engine over its MessagePort.
//
// Every timestamp the viewer logs is performance.now() milliseconds (the renderer's
// TimeTicks since timeOrigin). `clock.offsetUs` maps them to the engine's
// CLOCK_UPTIME_RAW microseconds; a performance.mark maps them to the trace clock.

const FLAG_GESTURE = 1;
const GPUBufferUsage = window.GPUBufferUsage;
const GPUTextureUsage = window.GPUTextureUsage;
const GPUMapMode = window.GPUMapMode;

const S = {
  cfg: null,
  port: null,
  clock: { offsetUs: 0, rttUs: 0, samples: 0 },
  device: null,
  context: null,
  canvasFormat: null,
  W: 3840,
  H: 2160,
  mode: 'idle',
  queue: [], // playback frames waiting for their time, sorted by pts
  gestureFrame: null, // newest gesture frame waiting for the next rAF
  displayed: null,
  hold: false,
  rafTimes: [],
  draws: [],
  viewerDrops: [],
  arrivals: [],
  inputs: [],
  integrity: { checked: 0, ok: 0, bad: [], skipped: 0 },
  releasesSent: 0,
  rev: 0,
  keySeq: 0,
  baselineRev: 0,
  baselineDrawnRev: 0,
  pointer: [0, 0],
  recording: false,
  periodUs: 16_667,
  qualityWaiter: null
};

const log = m => window.s1.send('s1:log', String(m));
const uptimeUs = ms => ms * 1000 + S.clock.offsetUs;

// --- Engine port and clock ------------------------------------------------------------------

const portReady = new Promise(resolve => {
  window.addEventListener('message', e => {
    if (e.source === window && e.data && e.data.s1 === 'engine-port' && e.ports[0]) {
      S.port = e.ports[0];
      S.port.onmessage = onEngineMessage;
      resolve();
    }
  });
});

const pongs = new Map();
function onEngineMessage(e) {
  const m = e.data;
  if (m.t === 'pong') {
    const w = pongs.get(m.id);
    if (w) {
      pongs.delete(m.id);
      w(m.now);
    }
  }
}

async function syncClock(n = 64) {
  let best = null;
  for (let i = 0; i < n; i++) {
    const id = `${performance.now()}-${i}`;
    const t0 = performance.now();
    const now = await new Promise(resolve => {
      pongs.set(id, resolve);
      S.port.postMessage({ t: 'ping', id });
    });
    const t1 = performance.now();
    const rtt = (t1 - t0) * 1000;
    const offset = now - ((t0 + t1) / 2) * 1000;
    if (!best || rtt < best.rtt) best = { rtt, offset };
  }
  S.clock = { offsetUs: best.offset, rttUs: best.rtt, samples: n };
  return S.clock;
}

const post = m => S.port.postMessage(m);

// --- WebGPU -------------------------------------------------------------------------------------

const VS = /* wgsl */ `
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
  return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const PRESENT_2D = /* wgsl */ `${VS}
@group(0) @binding(0) var src: texture_2d<f32>;
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  return textureLoad(src, vec2i(pos.xy), 0);
}`;

const PRESENT_EXT = /* wgsl */ `${VS}
@group(0) @binding(0) var src: texture_external;
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  return textureLoad(src, vec2u(pos.xy));
}
// 64 x 1 target: the frame-code cell centers (row 16, every 32 px).
@fragment fn fs_code(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  return textureLoad(src, vec2u(u32(pos.x) * 32u + 16u, 16u));
}`;

// "Today": the renderer draws the viewer content itself on the next frame.
const BASELINE = /* wgsl */ `${VS}
struct U { marker: vec4f };
@group(0) @binding(0) var<uniform> u: U;
@fragment fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
  let x = floor(pos.x);
  let y = floor(pos.y);
  var c = select(vec3f(0.18), vec3f(0.3), (x % 120.0) < 2.0 || (y % 120.0) < 2.0);
  let m = u.marker;
  if (x >= m.x && x < m.x + m.z && y >= m.y && y < m.y + m.w) { c = vec3f(1.0, 0.5, 0.0); }
  return vec4f(c, 1.0);
}`;

const G = {};

async function initGpu({ hdrCanvas, format }) {
  const adapter = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
  if (!adapter) throw new Error('no WebGPU adapter');
  const device = await adapter.requestDevice();
  device.lost.then(info => log(`WebGPU device lost: ${info.message}`));
  device.addEventListener('uncapturederror', e => log(`WebGPU error: ${e.error.message}`));
  S.device = device;
  const canvas = document.getElementById('view');
  S.context = canvas.getContext('webgpu');
  configureCanvas({ hdrCanvas, format });

  const mod2d = device.createShaderModule({ code: PRESENT_2D });
  const modExt = device.createShaderModule({ code: PRESENT_EXT });
  const modBase = device.createShaderModule({ code: BASELINE });
  const pipe = (module, entry, fmt) =>
    device.createRenderPipeline({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: entry, targets: [{ format: fmt }] }, primitive: { topology: 'triangle-list' } });
  G.pipes = new Map();
  G.pipe = (kind, fmt) => {
    const key = `${kind}:${fmt}`;
    if (!G.pipes.has(key)) {
      const module = kind === '2d' ? mod2d : kind === 'base' ? modBase : modExt;
      const entry = kind === 'code' ? 'fs_code' : 'fs';
      G.pipes.set(key, pipe(module, entry, fmt));
    }
    return G.pipes.get(key);
  };
  G.frameTex = device.createTexture({
    size: [S.W, S.H],
    format: 'rgba16float',
    usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_SRC
  });
  G.frameView = G.frameTex.createView();
  G.codeTex = device.createTexture({ size: [64, 1], format: 'rgba16float', usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
  G.baseUbo = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
  G.readPool = Array.from({ length: 12 }, () => device.createBuffer({ size: 2048 * 8, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ }));
  G.readFree = [...G.readPool];
  // Compile every pipeline up front: a first-use compile stalls the GPU process for
  // hundreds of milliseconds on a busy machine and would show up as latency.
  const t0 = performance.now();
  await Promise.all(
    [['2d', S.canvasFormat], ['ext', S.canvasFormat], ['base', S.canvasFormat], ['2d', 'rgba16float'], ['ext', 'rgba16float'], ['code', 'rgba16float']].map(async ([kind, fmt]) => {
      const module = kind === '2d' ? mod2d : kind === 'base' ? modBase : modExt;
      const entry = kind === 'code' ? 'fs_code' : 'fs';
      const p = await device.createRenderPipelineAsync({ layout: 'auto', vertex: { module, entryPoint: 'vs' }, fragment: { module, entryPoint: entry, targets: [{ format: fmt }] }, primitive: { topology: 'triangle-list' } });
      G.pipes.set(`${kind}:${fmt}`, p);
    })
  );
  G.pipelineCompileMs = performance.now() - t0;
  return { pipelineCompileMs: +G.pipelineCompileMs.toFixed(1), adapter: adapter.info ? { vendor: adapter.info.vendor, architecture: adapter.info.architecture, description: adapter.info.description } : null };
}

function configureCanvas({ hdrCanvas, format }) {
  // rgbaf16 pools present through an rgba16float canvas; extended tone mapping lets
  // values above 1.0 reach an EDR display. bgra pools present through bgra8unorm.
  S.canvasFormat = format === 'rgbaf16' || hdrCanvas ? 'rgba16float' : navigator.gpu.getPreferredCanvasFormat();
  S.context.configure({
    device: S.device,
    format: S.canvasFormat,
    alphaMode: 'opaque',
    colorSpace: 'srgb',
    toneMapping: { mode: hdrCanvas ? 'extended' : 'standard' },
    usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC
  });
}

function pass(enc, view, pipeline, bindGroup, size) {
  const p = enc.beginRenderPass({ colorAttachments: [{ view, loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 1 } }] });
  if (size) p.setViewport(0, 0, size[0], size[1], 0, 1);
  p.setPipeline(pipeline);
  p.setBindGroup(0, bindGroup);
  p.draw(3);
  p.end();
}

function f16(h) {
  const s = h & 0x8000 ? -1 : 1;
  const e = (h >> 10) & 0x1f;
  const m = h & 0x3ff;
  if (e === 0) return s * m * 2 ** -24;
  if (e === 31) return m ? NaN : s * Infinity;
  return s * (1 + m / 1024) * 2 ** (e - 15);
}

/**
 * Draws one engine frame into the canvas. Returns after submission; the frame is
 * closed at once and its pool slot is released to the engine when the GPU work that
 * read it has completed (queue.onSubmittedWorkDone).
 */
function drawFrame(item) {
  const { device } = S;
  const enc = device.createCommandEncoder();
  const target = S.context.getCurrentTexture().createView();
  let check = G.readFree.pop() ?? null;
  if (S.cfg.drawPath === 'copy') {
    device.queue.copyExternalImageToTexture({ source: item.frame }, { texture: G.frameTex }, [S.W, S.H]);
    const bg = device.createBindGroup({ layout: G.pipe('2d', S.canvasFormat).getBindGroupLayout(0), entries: [{ binding: 0, resource: G.frameView }] });
    pass(enc, target, G.pipe('2d', S.canvasFormat), bg);
    if (check) enc.copyTextureToBuffer({ texture: G.frameTex, origin: { x: 0, y: 16 } }, { buffer: check, bytesPerRow: 2048 * 8 }, { width: 2048, height: 1 });
  } else {
    const ext = device.importExternalTexture({ source: item.frame });
    const bg = device.createBindGroup({ layout: G.pipe('ext', S.canvasFormat).getBindGroupLayout(0), entries: [{ binding: 0, resource: ext }] });
    pass(enc, target, G.pipe('ext', S.canvasFormat), bg);
    if (check) {
      const bgc = device.createBindGroup({ layout: G.pipe('code', 'rgba16float').getBindGroupLayout(0), entries: [{ binding: 0, resource: ext }] });
      pass(enc, G.codeTex.createView(), G.pipe('code', 'rgba16float'), bgc);
      enc.copyTextureToBuffer({ texture: G.codeTex }, { buffer: check, bytesPerRow: 512 }, { width: 64, height: 1 });
    }
  }
  device.queue.submit([enc.finish()]);
  const submitMs = performance.now();
  item.frame.close();
  const { poolIndex, seq, revision } = item.desc;
  device.queue.onSubmittedWorkDone().then(() => {
    post({ t: 'release', pool: poolIndex, seq });
    S.releasesSent += 1;
  });
  if (check) verifyCode(check, seq, revision);
  else S.integrity.skipped += 1;
  return submitMs;
}

function verifyCode(buf, seq, rev) {
  buf.mapAsync(GPUMapMode.READ).then(
    () => {
      const u16 = new Uint16Array(buf.getMappedRange());
      const stride = S.cfg.drawPath === 'copy' ? 32 : 1; // texels between cell centers
      const first = S.cfg.drawPath === 'copy' ? 16 : 0;
      let s = 0;
      let r = 0;
      for (let c = 0; c < 64; c++) {
        const bit = f16(u16[(first + c * stride) * 4]) > 0.5 ? 1 : 0;
        if (c < 32) s |= bit << c;
        else r |= bit << (c - 32);
      }
      buf.unmap();
      G.readFree.push(buf);
      S.integrity.checked += 1;
      if (s >>> 0 === seq % 2 ** 32 && r >>> 0 === rev % 2 ** 32) S.integrity.ok += 1;
      else if (S.integrity.bad.length < 50) S.integrity.bad.push({ seq, rev, got: [s >>> 0, r >>> 0] });
    },
    () => G.readFree.push(buf)
  );
}

function releaseUndrawn(item, reason) {
  item.frame.close();
  post({ t: 'release', pool: item.desc.poolIndex, seq: item.desc.seq });
  S.releasesSent += 1;
  if (S.recording) S.viewerDrops.push([item.desc.seq, item.desc.ptsUs, reason, performance.now()]);
}

// --- Frame intake -------------------------------------------------------------------------------

window.s1.onFrame((frame, desc, recvMs) => {
  const arriveMs = performance.now();
  const item = { frame, desc, recvMs, arriveMs };
  if (S.recording) S.arrivals.push([desc.seq, desc.ptsUs, desc.gpuDoneUs, desc.bridgeRecvUs, desc.mainJsUs, recvMs, arriveMs, desc.flags]);
  if (desc.flags & FLAG_GESTURE) {
    if (S.gestureFrame) releaseUndrawn(S.gestureFrame, 'superseded');
    S.gestureFrame = item;
    return;
  }
  if (S.mode !== 'playback' || S.hold) {
    releaseUndrawn(item, 'not-playing');
    return;
  }
  // Keep the queue sorted by pts (arrivals are in order; this guards re-grids).
  let i = S.queue.length;
  while (i > 0 && S.queue[i - 1].desc.ptsUs > desc.ptsUs) i--;
  S.queue.splice(i, 0, item);
});

// --- The frame loop -----------------------------------------------------------------------------

function fitVsync(r) {
  const d = [];
  for (let i = Math.max(1, r.length - 120); i < r.length; i++) d.push(r[i] - r[i - 1]);
  d.sort((a, b) => a - b);
  const median = d[d.length >> 1];
  const idx = [0];
  for (let i = 1; i < r.length; i++) idx.push(idx[i - 1] + Math.max(1, Math.round((r[i] - r[i - 1]) / median)));
  const n = r.length;
  let sx = 0;
  let sy = 0;
  for (let i = 0; i < n; i++) {
    sx += idx[i];
    sy += r[i] - r[0];
  }
  const mx = sx / n;
  const my = sy / n;
  let sxy = 0;
  let sxx = 0;
  let res = 0;
  for (let i = 0; i < n; i++) {
    sxy += (idx[i] - mx) * (r[i] - r[0] - my);
    sxx += (idx[i] - mx) ** 2;
  }
  const periodMs = sxy / sxx;
  const intercept = r[0] + my - periodMs * mx;
  for (let i = 0; i < n; i++) res = Math.max(res, Math.abs(r[i] - (intercept + periodMs * idx[i])));
  return { periodMs, lastMs: intercept + periodMs * idx[n - 1], maxResidualMs: res, samples: n };
}

function snapToVsync(ms) {
  const f = S.vsyncFit;
  if (!f) return ms;
  return f.lastMs + Math.round((ms - f.lastMs) / f.periodMs) * f.periodMs;
}

let lastTimingPost = 0;
function frameLoop(rafMs) {
  requestAnimationFrame(frameLoop);
  const rt = S.rafTimes;
  if (S.recording) rt.push(rafMs);
  S.lastRaf = rafMs;
  // Display timing for the engine's PTS grid: a recent vsync and the refresh period.
  // rAF timestamps are BeginFrame times coarsened to 0.1 ms and, depending on the
  // display link, jittered by up to about 1 ms, so a single interval (or the median)
  // is not precise enough: an error of 0.03 ms drifts the grid by 4 ms per second.
  // Each sample gets a vsync index (skipped vsyncs advance it by more than one) and a
  // least-squares line through (index, time) gives the period and a de-jittered
  // phase for the latest vsync.
  S.recentRaf = S.recentRaf || [];
  S.recentRaf.push(rafMs);
  if (S.recentRaf.length > 1200) S.recentRaf.shift();
  if (S.port && rafMs - lastTimingPost > 500 && S.recentRaf.length > 60) {
    const fit = fitVsync(S.recentRaf);
    S.periodUs = fit.periodMs * 1000;
    S.vsyncFit = fit;
    post({ t: 'timing', anchor: uptimeUs(fit.lastMs), period: fit.periodMs * 1000 });
    lastTimingPost = rafMs;
  }
  if (S.hold || !S.device) return;

  if (S.mode === 'playback') {
    // Decide on the fitted vsync grid, not the raw rAF timestamp, which can be
    // jittered by milliseconds when the display link is a software timer.
    const t = uptimeUs(snapToVsync(rafMs));
    let pick = -1;
    for (let i = 0; i < S.queue.length; i++) if (S.queue[i].desc.ptsUs <= t) pick = i;
    if (pick < 0) return;
    for (const old of S.queue.splice(0, pick)) releaseUndrawn(old, 'late');
    const item = S.queue.shift();
    if (S.qualityWaiter) {
      const w = S.qualityWaiter;
      S.qualityWaiter = null;
      w(item, rafMs);
      return;
    }
    const submitMs = drawFrame(item);
    S.displayed = item.desc;
    if (S.recording) S.draws.push([rafMs, item.desc.seq, item.desc.revision, item.desc.ptsUs, item.arriveMs, submitMs, item.desc.flags]);
  } else if (S.mode === 'baseline') {
    if (S.baselineRev === S.baselineDrawnRev) return;
    const rev = S.baselineRev;
    const [x, y] = S.pointer;
    S.device.queue.writeBuffer(G.baseUbo, 0, new Float32Array([x - 48, y - 48, 96, 96]));
    const enc = S.device.createCommandEncoder();
    const bg = S.device.createBindGroup({ layout: G.pipe('base', S.canvasFormat).getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: G.baseUbo } }] });
    pass(enc, S.context.getCurrentTexture().createView(), G.pipe('base', S.canvasFormat), bg);
    S.device.queue.submit([enc.finish()]);
    S.baselineDrawnRev = rev;
    if (S.recording) S.draws.push([rafMs, -1, rev, 0, rafMs, performance.now(), 0]);
  } else if (S.gestureFrame) {
    const item = S.gestureFrame;
    S.gestureFrame = null;
    const submitMs = drawFrame(item);
    S.displayed = item.desc;
    if (S.recording) S.draws.push([rafMs, item.desc.seq, item.desc.revision, item.desc.ptsUs, item.arriveMs, submitMs, item.desc.flags]);
  }
}

// --- Input --------------------------------------------------------------------------------------

const decodeSeq = (x, y) => (y - 300) * 1200 + (x - 200);

window.addEventListener('pointermove', e => {
  const events = e.getCoalescedEvents ? e.getCoalescedEvents() : [];
  const seqs = (events.length ? events : [e]).map(c => decodeSeq(Math.round(c.clientX), Math.round(c.clientY)));
  const rev = ++S.rev;
  const x = e.clientX * 2;
  const y = e.clientY * 2;
  S.pointer = [x, y];
  if (S.mode === 'baseline') S.baselineRev = rev;
  else if (S.mode === 'gesture' || S.mode === 'orbit') post({ t: 'input', rev, x, y, ts: uptimeUs(e.timeStamp), kind: 0 });
  if (S.recording) S.inputs.push([rev, seqs, e.timeStamp, performance.now()]);
});

window.addEventListener('keydown', e => {
  if (e.repeat) return;
  const rev = ++S.rev;
  const seq = S.keySeq++;
  if (S.mode === 'walk') post({ t: 'input', rev, x: S.pointer[0], y: S.pointer[1], ts: uptimeUs(e.timeStamp), kind: 1 });
  if (S.recording) S.inputs.push([rev, [seq], e.timeStamp, performance.now()]);
});

// --- Commands from main -------------------------------------------------------------------------

function resetLogs() {
  S.rafTimes = [];
  S.draws = [];
  S.viewerDrops = [];
  S.arrivals = [];
  S.inputs = [];
  S.integrity = { checked: 0, ok: 0, bad: [], skipped: 0 };
  S.keySeq = 0;
}

function setMode(mode) {
  // Leaving a mode returns every held frame to the engine.
  for (const item of S.queue.splice(0)) releaseUndrawn(item, 'mode');
  if (S.gestureFrame) releaseUndrawn(S.gestureFrame, 'mode');
  S.gestureFrame = null;
  S.mode = mode;
  const engineMode = mode === 'baseline' ? 'idle' : mode;
  post({ t: 'mode', mode: engineMode });
}

const commands = {
  async init(args) {
    S.cfg = args;
    S.W = args.width;
    S.H = args.height;
    await portReady;
    const clock = await syncClock();
    const gpu = await initGpu(args);
    performance.mark('s1-origin');
    requestAnimationFrame(frameLoop);
    await new Promise(r => setTimeout(r, 800));
    return { clock, gpu, canvasFormat: S.canvasFormat, dpr: devicePixelRatio, periodUs: S.periodUs, float16Array: typeof Float16Array !== 'undefined', userAgent: navigator.userAgent };
  },
  async mode({ mode }) {
    setMode(mode);
    return { mode };
  },
  async playback({ durationMs }) {
    resetLogs();
    const clockBefore = await syncClock(32);
    S.recording = true;
    performance.mark('s1-playback-start');
    await new Promise(r => setTimeout(r, durationMs));
    S.recording = false;
    performance.mark('s1-playback-end');
    const clockAfter = await syncClock(32);
    return collect({ clockBefore, clockAfter });
  },
  async 'latency-report'() {
    S.recording = false;
    const r = collect({ clock: await syncClock(32) });
    resetLogs();
    S.recording = true;
    return r;
  },
  async quality() {
    return qualityProbe();
  },
  async hold({ hold }) {
    S.hold = hold;
    return { hold };
  },
  async held() {
    return S.displayed;
  },
  async 'receiver-stats'() {
    return { ...window.s1.receiverStats(), releasesSent: S.releasesSent };
  }
};

// Latency runs record from the first mode switch.
const origMode = commands.mode;
commands.mode = async args => {
  const r = await origMode(args);
  if (args.mode !== 'playback' && args.mode !== 'idle') {
    resetLogs();
    performance.mark(`s1-mode-${args.mode}`);
    S.recording = true;
  }
  return r;
};

function collect(extra) {
  return {
    ...extra,
    clock: extra.clock ?? S.clock,
    marks: performance.getEntriesByType('mark').map(m => [m.name, m.startTime]),
    rafTimes: S.rafTimes,
    draws: S.draws,
    viewerDrops: S.viewerDrops,
    arrivals: S.arrivals,
    inputs: S.inputs,
    integrity: S.integrity,
    releasesSent: S.releasesSent,
    periodUs: S.periodUs,
    vsyncFit: S.vsyncFit ?? null
  };
}

window.s1.on('s1:cmd', async ({ id, cmd, args }) => {
  let value;
  try {
    value = await commands[cmd](args);
  } catch (err) {
    value = { error: String(err && err.stack ? err.stack : err) };
    log(`command ${cmd} failed: ${value.error}`);
  }
  window.s1.send('s1:result', { id, value });
});

// --- Quality probe: ramp banding and values above 1.0 through both import paths ---------------

async function readRows(texture, y0, rows, width = S.W) {
  const bpr = Math.ceil((width * 8) / 256) * 256;
  const buf = S.device.createBuffer({ size: bpr * rows, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
  const enc = S.device.createCommandEncoder();
  enc.copyTextureToBuffer({ texture, origin: { x: 0, y: y0 } }, { buffer: buf, bytesPerRow: bpr }, { width, height: rows });
  S.device.queue.submit([enc.finish()]);
  await buf.mapAsync(GPUMapMode.READ);
  const u16 = new Uint16Array(buf.getMappedRange().slice(0));
  buf.destroy();
  return (x, row, ch = 0) => f16(u16[(row * bpr) / 2 + x * 4 + ch]);
}

function rampMetrics(get, rows, label) {
  const W = S.W;
  // Per-column mean over the rows, then 10-bit statistics.
  const mean = new Float64Array(W);
  for (let x = 0; x < W; x++) {
    let s = 0;
    for (let r = 0; r < rows; r++) s += get(x, r);
    mean[x] = s / rows;
  }
  const codes = new Set();
  let maxStep10 = 0;
  let monotonic = true;
  let maxErr10 = 0;
  for (let x = 0; x < W; x++) {
    const v = get(x, 0);
    codes.add(Math.round(v * 1023));
    const ideal = (x + 0.5) / W;
    if (x > 0) {
      const step = (get(x, 0) - get(x - 1, 0)) * 1023;
      maxStep10 = Math.max(maxStep10, step);
      if (step < 0) monotonic = false;
    }
    maxErr10 = Math.max(maxErr10, Math.abs(v - ideal) * 1023);
  }
  // Local mean over 4-column blocks x rows (narrower than one 8-bit step).
  let localErr10 = 0;
  for (let bx = 0; bx < W / 4; bx++) {
    let s = 0;
    for (let x = bx * 4; x < bx * 4 + 4; x++) s += mean[x];
    const ideal = (bx * 4 + 2) / W;
    localErr10 = Math.max(localErr10, Math.abs(s / 4 - ideal) * 1023);
  }
  return { label, rows, distinct10BitCodesRow0: codes.size, maxStep10BitRow0: +maxStep10.toFixed(3), monotonicRow0: monotonic, maxAbsErr10BitRow0: +maxErr10.toFixed(3), localMeanMaxErr10Bit: +localErr10.toFixed(3) };
}

async function measureTexture(texture, label) {
  const dith = await readRows(texture, 64, 64);
  const plain = await readRows(texture, 320, 64);
  const patches = await readRows(texture, 700, 1);
  const code = await readRows(texture, 16, 1);
  let seq = 0;
  for (let c = 0; c < 32; c++) seq |= (code(c * 32 + 16, 0) > 0.5 ? 1 : 0) << c;
  return {
    label,
    codeSeq: seq >>> 0,
    dithered: rampMetrics(dith, 64, 'rows 64-127 (dithered on bgra)'),
    undithered: rampMetrics(plain, 64, 'rows 320-383 (never dithered)'),
    patches: Array.from({ length: 8 }, (_, i) => +patches(Math.floor((i + 0.5) * (S.W / 8)), 0).toFixed(5)),
    patchesWanted: [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 4]
  };
}

async function qualityProbe() {
  const item = await new Promise(resolve => (S.qualityWaiter = resolve));
  const { device } = S;
  const out = { seq: item.desc.seq, format: S.cfg.format, colorSpace: item.frame.colorSpace ? item.frame.colorSpace.toJSON() : null, frameFormat: item.frame.format, codedSize: [item.frame.codedWidth, item.frame.codedHeight] };
  // Path 1: copyExternalImageToTexture into rgba16float.
  device.queue.copyExternalImageToTexture({ source: item.frame }, { texture: G.frameTex }, [S.W, S.H]);
  out.copy = await measureTexture(G.frameTex, 'copyExternalImageToTexture -> rgba16float');
  // Path 2: importExternalTexture sampled 1:1 into rgba16float.
  const scratch = device.createTexture({ size: [S.W, S.H], format: 'rgba16float', usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
  {
    const ext = device.importExternalTexture({ source: item.frame });
    const enc = device.createCommandEncoder();
    const bg = device.createBindGroup({ layout: G.pipe('ext', 'rgba16float').getBindGroupLayout(0), entries: [{ binding: 0, resource: ext }] });
    pass(enc, scratch.createView(), G.pipe('ext', 'rgba16float'), bg);
    device.queue.submit([enc.finish()]);
  }
  out.external = await measureTexture(scratch, 'importExternalTexture -> rgba16float');
  // Path 3: the presented canvas, configured rgba16float with extended tone mapping,
  // read back through copyExternalImageToTexture(canvas).
  const prev = { hdrCanvas: S.cfg.hdrCanvas, format: S.cfg.format };
  configureCanvas({ hdrCanvas: true, format: S.cfg.format });
  {
    const enc = device.createCommandEncoder();
    const bg = device.createBindGroup({ layout: G.pipe('2d', S.canvasFormat).getBindGroupLayout(0), entries: [{ binding: 0, resource: G.frameView }] });
    pass(enc, S.context.getCurrentTexture().createView(), G.pipe('2d', S.canvasFormat), bg);
    device.queue.submit([enc.finish()]);
  }
  const canvasRow = device.createTexture({ size: [S.W, 1], format: 'rgba16float', usage: GPUTextureUsage.COPY_DST | GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
  try {
    device.queue.copyExternalImageToTexture({ source: document.getElementById('view'), origin: { x: 0, y: 700 } }, { texture: canvasRow }, [S.W, 1]);
    const row = await readRows(canvasRow, 0, 1);
    out.canvasExtended = { canvasFormat: S.canvasFormat, toneMapping: 'extended', patches: Array.from({ length: 8 }, (_, i) => +row(Math.floor((i + 0.5) * (S.W / 8)), 0).toFixed(5)) };
  } catch (err) {
    out.canvasExtended = { error: String(err) };
  }
  // Hold this frame on screen (extended canvas) so main can read EDR headroom and capture.
  S.displayed = item.desc;
  S.qualityCanvas = prev;
  scratch.destroy();
  canvasRow.destroy();
  item.frame.close();
  await device.queue.onSubmittedWorkDone();
  post({ t: 'release', pool: item.desc.poolIndex, seq: item.desc.seq });
  S.releasesSent += 1;
  return out;
}
