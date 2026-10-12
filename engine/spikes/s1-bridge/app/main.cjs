'use strict';
// Spike S1 main process (plan section 18). Main owns the bridge: it registers the
// per-launch bootstrap name, spawns the engine in a utilityProcess, verifies its one
// check-in, imports every pool surface once with sharedTexture.importSharedTexture,
// and forwards each frame descriptor to the viewer with sendSharedTexture. It also
// drives the measurements and applies the main-process load.
//
// Configuration comes from S1_CONFIG (JSON). See scripts/run.mjs for the scenarios.

const { app, BrowserWindow, utilityProcess, MessageChannelMain, sharedTexture, ipcMain, contentTracing, screen } = require('electron');
const fs = require('node:fs');
const path = require('node:path');
const { performance } = require('node:perf_hooks');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const bridgeAddon = require(path.join(__dirname, 'native', 's1_bridge.node'));
const { MainLoad } = require('./load.cjs');

const cfg = Object.assign(
  {
    scenario: 'playback',
    format: 'bgra',
    width: 3840,
    height: 2160,
    poolSize: 14,
    leadFrames: 10,
    extraPasses: 2,
    durationMs: 60_000,
    load: { longTask: true, autosave: true, agent: true, resize: true },
    drawPath: 'copy',
    hdrCanvas: false,
    latencyModes: ['baseline', 'gesture', 'orbit', 'walk'],
    inputHz: 120,
    // 'gpu-done' (onSubmittedWorkDone after close) or 'callback' (per-use release
    // callback, subtle.release(cb)); see preload.cjs.
    releaseMode: 'gpu-done',
    // 'per-frame': main calls sendSharedTexture for every frame (plan section 18 step 3).
    // 'persistent': main sends each pool slot to the viewer once; descriptors go engine
    // -> viewer over the MessagePort and the viewer makes a VideoFrame per frame from
    // the import it holds, so main's JavaScript is off the per-frame path.
    transport: 'per-frame',
    // 'none' or 'every-vsync': redraw the last frame on vsyncs without a new one.
    redraw: 'none',
    // Place the window in a small on-screen corner instead of off every screen (a
    // validity check of off-screen presentation timing). Off by default.
    onscreen: false,
    // Quality runs: briefly show the brightest HDR patch in a small on-screen window
    // and read the display's EDR headroom while it is visible.
    edrOnscreen: false,
    trace: true,
    out: null
  },
  JSON.parse(process.env.S1_CONFIG || '{}')
);

// Keep the window producing frames while it sits off screen and unfocused, and never
// take focus from the person using this Mac.
for (const s of ['disable-renderer-backgrounding', 'disable-background-timer-throttling', 'disable-backgrounding-occluded-windows']) {
  app.commandLine.appendSwitch(s);
}
app.commandLine.appendSwitch('disable-features', 'MacWebContentsOcclusion');

const log = msg => process.stdout.write(`[s1 main ${(performance.now() / 1000).toFixed(3)}] ${msg}\n`);
const nowUs = () => bridgeAddon.nowUsMain();
const sleep = ms => new Promise(r => setTimeout(r, ms));

const result = {
  config: cfg,
  versions: { electron: process.versions.electron, chrome: process.versions.chrome, node: process.versions.node },
  startedAt: new Date().toISOString(),
  errors: [],
  bridge: {},
  frames: [],
  imports: {}
};
const fail = msg => {
  log(`ERROR ${msg}`);
  result.errors.push(msg);
};

let win = null;
let engine = null;
let bridge = null;
const imported = new Map(); // poolIndex -> SharedTextureImported
const sendsPerImport = new Map();
let engineReady = null;
const engineWaiters = new Map();
let engineMsgId = 0;

function engineRequest(msg) {
  const id = ++engineMsgId;
  return new Promise(resolve => {
    engineWaiters.set(id, resolve);
    engine.postMessage({ ...msg, id });
  });
}

// --- Viewer commands --------------------------------------------------------------------

let cmdId = 0;
const cmdWaiters = new Map();
ipcMain.on('s1:result', (e, r) => {
  if (e.sender !== win?.webContents) return;
  const w = cmdWaiters.get(r.id);
  if (w) {
    cmdWaiters.delete(r.id);
    w(r.value);
  }
});
function viewer(cmd, args = {}, timeoutMs = 600_000) {
  const id = ++cmdId;
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => {
      cmdWaiters.delete(id);
      reject(new Error(`viewer command ${cmd} timed out`));
    }, timeoutMs);
    cmdWaiters.set(id, v => {
      clearTimeout(t);
      resolve(v);
    });
    win.webContents.send('s1:cmd', { id, cmd, args });
  });
}
ipcMain.on('s1:log', (_e, m) => log(`viewer: ${m}`));
ipcMain.on('s1:undelivered', (_e, r) => engine?.postMessage({ type: 'release', pool: r.pool, seq: r.seq }));

// --- The bridge -----------------------------------------------------------------------------

function colorSpaceFor(format) {
  // bgra: sRGB-encoded E in [0, 1]. rgbaf16: E extended to all reals (extended sRGB).
  return format === 'rgbaf16'
    ? { primaries: 'bt709', transfer: 'srgb-hdr', matrix: 'rgb', range: 'full' }
    : { primaries: 'bt709', transfer: 'srgb', matrix: 'rgb', range: 'full' };
}

function onBridgeEvent(ev) {
  const jsUs = nowUs();
  switch (ev.kind) {
    case 'accepted':
      result.bridge.accepted = { pid: ev.pid, pidversion: ev.pidversion, atUs: jsUs };
      log(`engine check-in accepted: pid ${ev.pid} pidversion ${ev.pidversion}`);
      break;
    case 'rejected':
      (result.bridge.rejections ??= []).push({ pid: ev.pid, reason: ev.reason });
      log(`bridge rejected a message: ${ev.reason}`);
      break;
    case 'surface': {
      const t0 = performance.now();
      try {
        const tex = sharedTexture.importSharedTexture({
          textureInfo: {
            pixelFormat: ev.format,
            codedSize: { width: ev.width, height: ev.height },
            colorSpace: colorSpaceFor(ev.format),
            handle: { ioSurface: ev.ioSurface }
          },
          allReferencesReleased: () => {
            bridge.releaseSurface(ev.poolIndex);
            result.imports.allReleased = (result.imports.allReleased ?? 0) + 1;
          }
        });
        imported.set(ev.poolIndex, tex);
        sendsPerImport.set(ev.poolIndex, 0);
        if (cfg.transport === 'persistent') sendPersistent(ev.poolIndex, tex);
        (result.imports.surfaces ??= []).push({ pool: ev.poolIndex, format: ev.format, width: ev.width, height: ev.height, bytesPerRow: ev.bytesPerRow, importMs: performance.now() - t0 });
      } catch (err) {
        fail(`importSharedTexture(${ev.poolIndex}): ${err.message}`);
      }
      break;
    }
    case 'frame':
      forwardFrame(ev, jsUs);
      break;
    case 'goodbye':
      log('engine said goodbye');
      break;
    case 'gone':
      result.bridge.gone = jsUs;
      log('engine port has no senders (engine exited)');
      break;
    case 'error':
      fail(`bridge: ${ev.reason}`);
      break;
  }
}

function sendPersistent(poolIndex, tex) {
  const t0 = nowUs();
  sendsPerImport.set(poolIndex, sendsPerImport.get(poolIndex) + 1);
  sharedTexture.sendSharedTexture({ frame: win.webContents.mainFrame, importedSharedTexture: tex }, { persistent: true, poolIndex }).then(
    () => (result.imports.persistentSends ??= []).push({ pool: poolIndex, ms: (nowUs() - t0) / 1000 }),
    err => fail(`persistent sendSharedTexture(${poolIndex}): ${err.message}`)
  );
}

const frameLog = [];
function forwardFrame(ev, jsUs) {
  const tex = imported.get(ev.poolIndex);
  const rec = [ev.seq, ev.poolIndex, ev.gpuDoneUs, ev.recvUs, jsUs, 0, 0];
  frameLog.push(rec);
  if (cfg.transport === 'persistent') {
    rec[6] = 4; // delivered engine -> viewer directly
    return;
  }
  if (!tex || !win || win.isDestroyed()) {
    engine?.postMessage({ type: 'release', pool: ev.poolIndex, seq: ev.seq });
    rec[6] = 2;
    return;
  }
  sendsPerImport.set(ev.poolIndex, sendsPerImport.get(ev.poolIndex) + 1);
  const desc = {
    poolIndex: ev.poolIndex,
    seq: ev.seq,
    ptsUs: ev.ptsUs,
    durationUs: ev.durationUs,
    revision: ev.revision,
    inputTsUs: ev.inputTsUs,
    renderStartUs: ev.renderStartUs,
    gpuDoneUs: ev.gpuDoneUs,
    bridgeRecvUs: ev.recvUs,
    mainJsUs: jsUs,
    flags: ev.flags,
    colorSpace: ev.colorSpace,
    overlay: ev.overlay
  };
  sharedTexture.sendSharedTexture({ frame: win.webContents.mainFrame, importedSharedTexture: tex }, desc).then(
    () => {
      rec[5] = nowUs();
      rec[6] = 1;
    },
    err => {
      rec[5] = nowUs();
      rec[6] = 3;
      if (result.errors.length < 20) fail(`sendSharedTexture seq ${ev.seq}: ${err.message}`);
      engine?.postMessage({ type: 'release', pool: ev.poolIndex, seq: ev.seq });
    }
  );
}

async function startEngine() {
  const name = bridgeAddon.serviceName();
  const allowUnsigned = process.env.S1_ALLOW_UNSIGNED_DEV === '1';
  bridge = new bridgeAddon.Bridge(name, allowUnsigned);
  result.bridge.policy = bridge.policy();
  result.bridge.serviceNameLength = name.length;
  log(`bridge policy ${result.bridge.policy}`);

  const env = { ...process.env, PM_S1_BRIDGE_NAME: name };
  delete env.ELECTRON_RUN_AS_NODE;
  const t0 = nowUs();
  engine = utilityProcess.fork(path.join(__dirname, 'engine-host.cjs'), [], { serviceName: 'Powermove S1 Engine', env, stdio: 'inherit' });
  engineReady = new Promise((resolve, reject) => {
    engine.on('message', m => {
      if (m.type === 'ready') resolve(m.info);
      else if (m.type === 'error') {
        fail(`engine: ${m.message}`);
        reject(new Error(m.message));
      } else if (m.id && engineWaiters.has(m.id)) {
        engineWaiters.get(m.id)(m);
        engineWaiters.delete(m.id);
      }
    });
    engine.on('exit', code => {
      log(`engine exited with ${code}`);
      reject(new Error(`engine exited with ${code}`));
    });
  });
  await new Promise(resolve => engine.once('spawn', resolve));
  const expected = JSON.parse(bridge.expectChild(engine.pid));
  result.bridge.expected = { ...expected, spawnUs: nowUs() - t0 };
  bridge.start(onBridgeEvent, 15_000);

  const { port1, port2 } = new MessageChannelMain();
  engine.postMessage({ type: 'port' }, [port2]);
  win.webContents.postMessage('s1:port', null, [port1]);
  engine.postMessage({
    type: 'start',
    config: { width: cfg.width, height: cfg.height, format: cfg.format, poolSize: cfg.poolSize, leadFrames: cfg.leadFrames, extraPasses: cfg.extraPasses, transport: cfg.transport }
  });
  const info = await engineReady;
  result.engine = info;
  log(`engine ready: ${info.adapter}, ${info.width}x${info.height} ${info.format}, pool ${info.pool_size} (${(info.pool_bytes / 2 ** 20).toFixed(0)} MiB), check-in ${info.checkin_us} us`);
}

// --- Window -----------------------------------------------------------------------------------

async function createWindow() {
  win = new BrowserWindow({
    show: false,
    width: cfg.width / 2,
    height: cfg.height / 2,
    useContentSize: true,
    frame: false,
    resizable: true,
    enableLargerThanScreen: true,
    backgroundColor: '#000000',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      backgroundThrottling: false
    }
  });
  win.webContents.on('console-message', e => {
    if (e.level === 'error' || e.level === 'warning') log(`console ${e.level}: ${e.message}`);
  });
  win.webContents.on('render-process-gone', (_e, d) => fail(`renderer gone: ${d.reason}`));
  await win.loadFile(path.join(__dirname, 'viewer.html'));
  // Shown without activation, then moved off every screen (or into a small corner of
  // the primary display for the on-screen validity run).
  win.showInactive();
  win.setBounds(cfg.onscreen ? cornerBounds(160, 90) : offscreenBounds());
  result.window = { bounds: win.getBounds(), contentBounds: win.getContentBounds(), visible: win.isVisible(), focused: win.isFocused() };
}

function offscreenBounds() {
  return { x: -12000, y: -12000, width: cfg.width / 2, height: cfg.height / 2 };
}

/** A small rectangle in the bottom-right corner of the primary display's work area. */
function cornerBounds(width, height) {
  const wa = screen.getPrimaryDisplay().workArea;
  return { x: wa.x + wa.width - width - 8, y: wa.y + wa.height - height - 8, width, height };
}

// --- Scenarios ----------------------------------------------------------------------------------

async function withTrace(fn) {
  if (!cfg.trace) return fn();
  await contentTracing.startRecording({
    included_categories: ['disabled-by-default-devtools.timeline.frame', 'blink.user_timing'],
    excluded_categories: ['*']
  });
  try {
    return await fn();
  } finally {
    const tracePath = cfg.out ? cfg.out.replace(/\.json$/, '.trace.json') : path.join(app.getPath('temp'), 's1-trace.json');
    result.trace = await contentTracing.stopRecording(tracePath);
  }
}

async function engineSnapshot() {
  const r = await engineRequest({ type: 'stats', log: true });
  return { stats: r.stats, log: r.log };
}

async function runPlayback(load) {
  // On screen, show the calm grey grid rather than the flickering frame code.
  if (cfg.onscreen) await viewer('view-origin', { x: 2400, y: 1400 });
  await viewer('mode', { mode: 'playback' });
  await sleep(1500); // fill the jitter buffer
  if (load) load.start(cfg.load);
  const t0 = nowUs();
  const v = await viewer('playback', { durationMs: cfg.durationMs }, cfg.durationMs + 60_000);
  const t1 = nowUs();
  const loadSummary = load ? load.stop() : null;
  await viewer('mode', { mode: 'idle' });
  return { windowUs: [t0, t1], viewer: v, load: loadSummary };
}

async function runLatency(load) {
  const out = {};
  // Warm both import paths and Chromium's copy pipeline with real engine frames.
  await viewer('mode', { mode: 'playback' });
  await sleep(1500);
  if (load) load.start(cfg.load);
  for (const mode of cfg.latencyModes) {
    const kind = mode === 'walk' ? 1 : 0;
    await viewer('mode', { mode });
    await sleep(500);
    const gen = [];
    const sent = [];
    const done = new Promise(resolve => {
      const total = Math.floor((cfg.durationMs * cfg.inputHz) / 1000);
      const g = new bridgeAddon.InputGenerator(cfg.inputHz, cfg.durationMs, kind, ev => {
        const js = nowUs();
        gen.push([ev.seq, ev.tsUs, js]);
        if (win.isDestroyed()) return;
        if (kind === 1) {
          win.webContents.sendInputEvent({ type: 'keyDown', keyCode: 'W' });
          win.webContents.sendInputEvent({ type: 'keyUp', keyCode: 'W' });
        } else {
          win.webContents.sendInputEvent({ type: 'mouseMove', x: ev.x, y: ev.y });
        }
        sent.push(nowUs());
        if (ev.seq + 1 >= total) setTimeout(resolve, 400);
      });
      setTimeout(() => {
        g.stop();
        resolve();
      }, cfg.durationMs + 3000);
    });
    await done;
    const v = await viewer('latency-report', { mode });
    out[mode] = { generated: gen, sentUs: sent, viewer: v };
    log(`latency ${mode}: ${gen.length} events, viewer saw ${v.inputs.length} input dispatches, drew ${v.draws.length} frames`);
  }
  const loadSummary = load ? load.stop() : null;
  await viewer('mode', { mode: 'idle' });
  return { modes: out, load: loadSummary };
}

async function runQuality() {
  const out = {};
  out.edrBefore = safe(() => bridgeAddon.edrHeadroom());
  await viewer('mode', { mode: 'playback' });
  await sleep(1500);
  out.viewer = await viewer('quality', {});
  // capturePage while the held frame is on screen.
  await viewer('hold', { hold: true });
  await sleep(500);
  const held = await viewer('held', {});
  out.edrDuring = safe(() => bridgeAddon.edrHeadroom());
  const image = await win.webContents.capturePage();
  const size = image.getSize();
  const bitmap = image.toBitmap();
  out.capture = analyzeCapture(bitmap, image.getSize(), held);
  out.capture.size = size;
  out.capture.scaleFactors = image.getScaleFactors();
  if (cfg.edrOnscreen) {
    // The held frame is drawn in an rgba16float canvas with extended tone mapping.
    // Show the 4.0 patch (frame x 3360-3840, y 576-832) in a 64 x 40 pt window for
    // about two seconds and sample the display's EDR headroom: macOS raises the
    // current headroom above 1.0 only while EDR content is visible on that screen.
    await viewer('view-origin', { x: 3400, y: 600 });
    const before = safe(() => bridgeAddon.edrHeadroom());
    win.setBounds(cornerBounds(64, 40));
    const samples = [];
    for (let i = 0; i < 8; i++) {
      await sleep(250);
      samples.push({ t: i * 250 + 250, screens: safe(() => bridgeAddon.edrHeadroom()) });
    }
    win.setBounds(offscreenBounds());
    await sleep(750);
    const after = safe(() => bridgeAddon.edrHeadroom());
    await viewer('view-origin', { x: 0, y: 0 });
    out.edrOnscreen = { window: '64 x 40 pt, bottom-right of the primary display', patch: 4.0, before, samples, after };
  }
  await viewer('hold', { hold: false });
  await viewer('mode', { mode: 'idle' });
  return out;
}

function safe(fn) {
  try {
    return fn();
  } catch (err) {
    return { error: err.message };
  }
}

/** Reads the captured BGRA bitmap and compares it with the engine's pattern. */
function analyzeCapture(buf, size, held) {
  const { width, height } = size;
  const px = (x, y) => {
    const i = (y * width + x) * 4;
    return [buf[i + 2], buf[i + 1], buf[i]]; // BGRA -> RGB
  };
  const r = { width, height, heldSeq: held?.seq ?? null };
  if (width !== cfg.width || height !== cfg.height) {
    r.error = `capture is ${width}x${height}, expected ${cfg.width}x${cfg.height} device pixels`;
    return r;
  }
  // Frame code: 64 cells of 32 px at y = 16.
  let seq = 0;
  let rev = 0;
  for (let c = 0; c < 64; c++) {
    const bit = px(c * 32 + 16, 16)[0] > 127 ? 1 : 0;
    if (c < 32) seq |= bit << c;
    else rev |= bit << (c - 32);
  }
  r.codeSeq = seq >>> 0;
  r.codeRev = rev >>> 0;
  // Undithered ramp row (y = 400): the engine's 8-bit output is round(255 * (x + 0.5) / W).
  const undithered = [];
  let maxErr = 0;
  let mismatches = 0;
  for (let x = 0; x < width; x++) {
    const v = px(x, 400);
    const ideal = (x + 0.5) / width;
    const want = cfg.format === 'bgra' ? Math.round(ideal * 255) : Math.round(ideal * 255);
    const err = Math.abs(v[0] - want);
    maxErr = Math.max(maxErr, err);
    if (err > 0) mismatches += 1;
    if (v[0] !== v[1] || v[1] !== v[2]) r.nonNeutral = (r.nonNeutral ?? 0) + 1;
    undithered.push(v[0]);
  }
  r.undithered = { maxAbsErrCV8: maxErr, mismatchedColumns: mismatches, distinct: new Set(undithered).size, monotonic: undithered.every((v, i) => i === 0 || v >= undithered[i - 1]) };
  // Dithered ramp rows (y = 64..127): local means over 16 x 64 blocks vs ideal, in 8-bit CV.
  let worst = 0;
  for (let bx = 0; bx < width / 16; bx++) {
    let sum = 0;
    for (let y = 64; y < 128; y++) for (let x = bx * 16; x < bx * 16 + 16; x++) sum += px(x, y)[0];
    const mean = sum / (16 * 64);
    const ideal = ((bx * 16 + 8) / width) * 255;
    worst = Math.max(worst, Math.abs(mean - ideal));
  }
  r.ditheredLocalMeanMaxErrCV8 = worst;
  // Patches row (y = 700): values above 1.0 are clipped in an 8-bit capture.
  r.patches = Array.from({ length: 8 }, (_, i) => px(Math.floor((i + 0.5) * (width / 8)), 700)[0]);
  return r;
}

/** Load from everything else on the machine (other agents' builds and spikes). */
function machineLoad() {
  let busy = [];
  try {
    busy = execFileSync('ps', ['-Ao', 'pcpu=,comm='], { encoding: 'utf8' })
      .split('\n')
      .map(l => l.trim().match(/^([\d.]+)\s+(.*)$/))
      .filter(m => m && +m[1] >= 20 && !m[2].includes('s1-bridge'))
      // Build tools and other spikes by name; anything else (the person's own apps)
      // only as "other", so the committed summaries carry no private process names.
      .map(m => ({ cpu: +m[1], name: /electron|rustc|cargo|clang|ld$|ffmpeg|bun|node|\/s\d\b|blender|xcodebuild|swift/i.test(m[2]) ? path.basename(m[2]) : 'other' }));
  } catch {}
  return { loadavg: os.loadavg().map(x => +x.toFixed(2)), busyProcesses: busy };
}

async function main() {
  app.setActivationPolicy('accessory');
  result.machineLoad = { before: machineLoad() };
  await app.whenReady();
  const d = screen.getPrimaryDisplay();
  result.display = { bounds: d.bounds, scaleFactor: d.scaleFactor, hz: d.displayFrequency, colorDepth: d.colorDepth, colorSpace: d.colorSpace };
  await createWindow();
  await startEngine();
  const ready = await viewer('init', { format: cfg.format, width: cfg.width, height: cfg.height, drawPath: cfg.drawPath, hdrCanvas: cfg.hdrCanvas, poolSize: cfg.poolSize, leadFrames: cfg.leadFrames, releaseMode: cfg.releaseMode, redraw: cfg.redraw });
  result.viewerInit = ready;
  log(`viewer ready: ${JSON.stringify(ready).slice(0, 300)}`);

  const load = cfg.load ? new MainLoad({ win, ipcMain, log, nowUs }) : null;
  await withTrace(async () => {
    if (cfg.scenario === 'playback') result.run = await runPlayback(load);
    else if (cfg.scenario === 'latency') result.run = await runLatency(load);
    else if (cfg.scenario === 'quality') result.run = await runQuality();
    else throw new Error(`unknown scenario ${cfg.scenario}`);
  });

  result.machineLoad.after = machineLoad();
  result.engineEnd = await engineSnapshot();
  result.receiver = await viewer('receiver-stats', {});
  result.frames = frameLog;
  result.sendsPerImport = Object.fromEntries(sendsPerImport);

  // Teardown: release main's references; allReferencesReleased must fire for every
  // surface once the renderer has released all of its per-transfer references.
  if (cfg.transport === 'persistent') result.receiver.persistentReleased = (await viewer('release-persistent', {})).released;
  for (const tex of imported.values()) tex.release();
  await sleep(1500);
  result.imports.releasedAfterTeardown = result.imports.allReleased ?? 0;
  result.imports.heldAfterTeardown = bridge.heldSurfaces();
  engine.postMessage({ type: 'stop' });
  await sleep(300);
  bridge.stop();
}

main()
  .catch(err => fail(`fatal: ${err.stack || err.message}`))
  .finally(() => {
    result.finishedAt = new Date().toISOString();
    if (cfg.out) {
      fs.mkdirSync(path.dirname(cfg.out), { recursive: true });
      fs.writeFileSync(cfg.out, JSON.stringify(result));
      log(`wrote ${cfg.out}`);
    }
    app.exit(result.errors.some(e => e.startsWith('fatal')) ? 1 : 0);
  });
