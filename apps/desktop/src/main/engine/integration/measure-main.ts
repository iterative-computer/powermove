/*
 * The production frame bridge's S1 measurement (plan section 18,
 * amendment 5's gate): one scenario per Electron launch, configured by
 * PM_MEASURE (JSON), run by scripts/frame-bridge-measure.mjs, which also
 * analyzes the raw output.
 *
 * The production path end to end: main's `installNativeEngine` with the
 * pm-bridge addon (built with its `harness` feature for the synthetic
 * input generator), the interactive engine's bridge test source at 4K and
 * 60 fps, the real preload receiver, and the viewer's presentation
 * (measure-page.ts). "Today's viewer" runs on the same harness: the
 * renderer draws each frame itself at the animation frame.
 *
 * Load (S1's gate load, engine/spikes/s1-bridge/app/load.cjs): autosave of
 * a 7.1 MB document every 2 s, a 50 ms long task every second, a 1 s live
 * resize every 5 s, and agent tool traffic 20 times a second (a 64 KB
 * request into the page and a 600-layer reply serialized in main).
 *
 * Timing: presentation times come from Chromium's PipelineReporter trace
 * events (rAF to viz presentation feedback), as in S1; a user-timing mark
 * maps performance.now() onto the trace clock and a ping maps the host
 * clock (CLOCK_UPTIME_RAW µs) onto performance.now().
 */
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { app, BrowserWindow, contentTracing, ipcMain, screen } from 'electron';

import { installNativeEngine, type InstalledNativeEngine } from '../index';

type Config = {
  scenario: 'playback' | 'latency';
  mode: 'engine' | 'today';
  load: boolean;
  /** `no-autosave`: the gate load without its main-thread autosave, as the
   *  app runs since autosave serialization moved to a persistence worker
   *  (docs/engine/autosave-off-main.md). Default: S1's gate load. */
  loadProfile?: 'gate' | 'no-autosave';
  durationMs: number;
  /** `off`: the engine presents 8-bit even on an EDR display. */
  edr?: 'auto' | 'off';
  /** Engine gestures read `pointerrawupdate`. */
  rawInput?: boolean;
  /** The C′ prototype presents playback (POWERMOVE_BRIDGE_REMOTE_LAYER=1);
   *  main hosts its remote layer over the page. */
  remoteLayer?: boolean;
  /** Where the window sits: off every screen (default) or invisible on screen. */
  placement?: 'offscreen' | 'behind';
  /** Latency: input modes to run, each for `durationMs`. */
  inputModes?: ('drag' | 'orbit' | 'walk' | 'baseline')[];
  /** Content rate of the engine's test source (default 60). */
  fps?: number;
  /** Main asks the display for a multiple of the content rate while playback
   *  is recorded (section 15, through public API: `requestRefreshRate`). */
  mainRate?: boolean;
  out: string;
  trace: string;
  /** Agent snapshots during playback (plan section 25, Phase 2 row): main
   *  sends a batch of `frames` agent frames to the job engine every
   *  `turnMs`, through the production agent path (agent-jobs.ts), on a
   *  `layers`-layer 4K project that changes every turn (each batch is a
   *  new revision, so the job engine resyncs as it does after agent edits).
   *  `heavy: 'precomp'` puts the layers in a full-size 4K precomposition,
   *  which pm-render draws whole for every band of rows: a band costs about
   *  as much as the whole frame (an over-bound pass when the frame is). */
  agentSnapshots?: { turnMs: number; frames: number; width: number; layers: number; heavy?: 'precomp' };
};

/** A 3840 × 2160, 24 fps project of solid layers, one color per turn; with
 *  `heavy`, the solids sit in a 3840 × 2160 precomposition (not a group). */
function snapshotProject(layers: number, turn: number, heavy?: 'precomp'): Record<string, unknown> {
  const ch = (v: unknown) => ({ v, kf: [], expr: null });
  const transform = (x: number, y: number) => Object.fromEntries([
    ['anchor.x', 0], ['anchor.y', 0], ['position.x', x], ['position.y', y], ['scale.x', 100], ['scale.y', 100],
    ['rotation', 0], ['opacity', 85], ['skew', 0]
  ].map(([k, v]) => [k, ch(v)]));
  const list = Array.from({ length: layers }, (_, i) => ({
    id: `s${i}`, type: 'solid', name: `Solid ${i}`, from: 0, dur: 4, on: true, solo: false, blend: 'normal', mblur: false,
    parent: null, group: null, matteSource: null, threeD: false, fx: [], masks: [], transitionIn: null, transitionOut: null,
    p: transform((i * 157) % 3200, (i * 89) % 1800),
    d: { color: `#${((turn * 0x1f3d5b + i * 0x2a7f13) & 0xffffff).toString(16).padStart(6, '0')}`, w: 640 + (i % 5) * 240, h: 360 + (i % 3) * 200 }
  }));
  if (heavy === 'precomp') {
    const pre = {
      ...list[0]!, id: 'pre', type: 'precomp', name: 'Heavy precomp', p: transform(1920, 1080), d: { comp: 'heavy', w: 3840, h: 2160 }
    };
    (pre.p as Record<string, unknown>)['opacity'] = ch(100);
    return {
      id: 'Pmeasure', name: 'measure', w: 3840, h: 2160, fps: 24, fpsRational: [24, 1], dur: 4, bg: '#101010',
      compId: 'Pmeasure-root', compName: 'measure', markers: [], work: [0, 4], params: {}, revision: turn, layers: [pre], assets: {},
      comps: {
        heavy: {
          id: 'heavy', name: 'Heavy', w: 3840, h: 2160, fps: 24, dur: 4, bg: '#000000', markers: [], layers: list,
          backgroundFill: { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#000000', position: 0 }] }
        }
      }
    };
  }
  return {
    id: 'Pmeasure', name: 'measure', w: 3840, h: 2160, fps: 24, fpsRational: [24, 1], dur: 4, bg: '#101010',
    compId: 'Pmeasure-root', compName: 'measure', markers: [], work: [0, 4], params: {}, revision: turn, layers: list, assets: {}
  };
}

type HarnessAddon = {
  nowUs(): number;
  placeBehindDesktop(viewHandle: Buffer): number;
  hostRemoteLayer(viewHandle: Buffer, contextId: number, x: number, y: number, width: number, height: number): number;
  removeRemoteLayer(id: number): boolean;
  requestRefreshRate(hz: number): number[];
  InputGenerator: new (hz: number, durationMs: number, kind: number, callback: (input: { seq: number; tsUs: number; x: number; y: number }) => void) => { stop(): void };
};

// The S1 load generator (CommonJS, shared with the spike).
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { MainLoad } = require('../../../../../../engine/spikes/s1-bridge/app/load.cjs') as {
  MainLoad: new (options: { win: BrowserWindow; ipcMain: Electron.IpcMain; log: (m: string) => void; nowUs: () => number }) => {
    start(options: Record<string, unknown>): void;
    stop(): Record<string, unknown>;
  };
};

const cfg = JSON.parse(process.env['PM_MEASURE'] ?? '{}') as Config;
const logs: string[] = [];
/** The job service's latest counters (POWERMOVE_ENGINE_STATUS_LOG_MS). */
let jobStatus: { atMs: number; jobs: Record<string, unknown> } | null = null;
const log = (message: string): void => {
  const status = /\[job engine\] job service status (\{.*\})$/.exec(message);
  if (status) {
    try {
      jobStatus = { atMs: performance.now(), jobs: JSON.parse(status[1]!) as Record<string, unknown> };
    } catch {
      // Not JSON: keep it as an ordinary line.
      logs.push(`${(performance.now() / 1000).toFixed(3)} ${message}`);
    }
    return;
  }
  logs.push(`${(performance.now() / 1000).toFixed(3)} ${message}`);
};
const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

for (const s of ['disable-renderer-backgrounding', 'disable-background-timer-throttling', 'disable-backgrounding-occluded-windows']) app.commandLine.appendSwitch(s);
app.commandLine.appendSwitch('disable-features', 'MacWebContentsOcclusion');

async function waitFor<T>(label: string, poll: () => Promise<T | null | undefined | false>, timeoutMs = 30_000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await poll();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`);
    await sleep(25);
  }
}

/**
 * `behind` placement (C′ runs and the Chromium runs they are compared
 * with): Core Animation neither shows nor reports, and so never frees, the
 * drawables of a layer whose window is off every screen or at opacity 0
 * (measured: every C′ frame unpresented both ways), so the remote layer
 * needs a window on a display. This one is on the primary display one
 * level below the desktop picture, which covers it: ignoring the mouse,
 * never activated, on every Space, out of Mission Control's cycle.
 */
function placeBehind(win: BrowserWindow, harness: HarnessAddon, width: number, height: number): number {
  const area = screen.getPrimaryDisplay().workArea;
  const level = harness.placeBehindDesktop(win.getNativeWindowHandle());
  win.setIgnoreMouseEvents(true);
  // The same 1920 × 1080 points as off screen (frameless, larger than the screen allowed).
  win.setBounds({ x: area.x, y: area.y, width, height });
  return level;
}

function offscreen(width: number, height: number): Electron.Rectangle {
  const right = Math.max(...screen.getAllDisplays().map((d) => d.bounds.x + d.bounds.width));
  return { x: right + 4000, y: -6000, width, height };
}

async function main(): Promise<void> {
  const desktop = process.env['PM_IT_DESKTOP']!;
  const outDir = process.env['PM_IT_OUT']!;
  const harness = require(process.env['POWERMOVE_PM_BRIDGE']!) as HarnessAddon; // eslint-disable-line @typescript-eslint/no-require-imports
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-bridge-measure-'));
  const display = screen.getPrimaryDisplay();
  const hz = display.displayFrequency || 60;
  const result: Record<string, unknown> = {
    config: cfg,
    versions: { electron: process.versions.electron, chrome: process.versions.chrome },
    display: { hz, scaleFactor: display.scaleFactor, colorDepth: display.colorDepth },
    machineLoad: { before: os.loadavg() },
    startedAt: new Date().toISOString()
  };
  let installed: InstalledNativeEngine | null = null;
  let win: BrowserWindow | null = null;
  try {
    installed = installNativeEngine({
      enabled: true,
      ipc: ipcMain,
      isTrustedSender: () => true,
      userData: path.join(home, 'userData'),
      home,
      appPath: desktop,
      isPackaged: false,
      resourcesPath: path.join(home, 'resources'),
      mainDirectory: outDir,
      env: process.env,
      log: (level, message) => log(`${level}: ${message}`),
      windows: () => BrowserWindow.getAllWindows()
    });
    const engine = installed.engine!;
    await waitFor('the interactive engine', async () => engine.status().interactive.state === 'ready');
    if (cfg.agentSnapshots) await waitFor('the job engine', async () => engine.status().job.state === 'ready');
    const playbackSignals: Array<[number, boolean]> = [];
    engine.listen({ playback: (playing) => playbackSignals.push([performance.now(), playing]) });
    // The C′ prototype: host the engine's remote layer over the page.
    const layerHosts = new Map<number, number>();
    const remoteLayers: Record<string, unknown>[] = [];
    installed.frameBridge?.listen((event) => {
      if (event.kind !== 'remoteLayer' || !cfg.remoteLayer || !win || win.isDestroyed()) return;
      const port = event.clientPort ?? -1;
      // The new host goes in before the old one leaves: a new pool keeps
      // its context (a resize), and it must not be unhosted in between.
      const old = layerHosts.get(port);
      layerHosts.delete(port);
      if (!event.contextId || !event.width || !event.height || !event.scale) {
        if (old !== undefined) harness.removeRemoteLayer(old);
        return;
      }
      try {
        const id = harness.hostRemoteLayer(win.getNativeWindowHandle(), event.contextId, 0, 0, event.width / event.scale, event.height / event.scale);
        layerHosts.set(port, id);
        if (old !== undefined) harness.removeRemoteLayer(old);
        remoteLayers.push({ contextId: event.contextId, width: event.width, height: event.height, scale: event.scale, hostedUs: harness.nowUs() });
        log(`hosted remote layer ${event.contextId} (${event.width}x${event.height} at ${event.scale}x)`);
      } catch (error) {
        log(`could not host remote layer ${event.contextId}: ${error instanceof Error ? error.message : String(error)}`);
      }
    });
    result['remoteLayers'] = remoteLayers;

    win = new BrowserWindow({
      show: false,
      width: 1920,
      height: 1080,
      useContentSize: true,
      frame: false,
      focusable: false,
      enableLargerThanScreen: true,
      backgroundColor: '#000000',
      webPreferences: { preload: path.join(outDir, 'preload.js'), contextIsolation: true, sandbox: true, nodeIntegration: false, backgroundThrottling: false }
    });
    win.webContents.on('console-message', (event) => log(`page ${event.level}: ${event.message}`));
    installed.attachWindow(win);
    await win.loadFile(path.join(outDir, 'measure.html'), { query: { hz: String(hz), edr: cfg.edr ?? 'auto', raw: cfg.rawInput ? '1' : '0' } });
    let level: number | null = null;
    if (cfg.placement === 'behind') {
      placeBehind(win, harness, 1920, 1080);
      win.showInactive();
      // Showing must not lift it: set again and check.
      level = harness.placeBehindDesktop(win.getNativeWindowHandle());
    } else {
      win.showInactive();
      win.setBounds(offscreen(1920, 1080));
    }
    result['placement'] = { kind: cfg.placement ?? 'offscreen', bounds: win.getBounds(), level };
    const page = win;
    const call = <T>(expression: string): Promise<T> => page.webContents.executeJavaScript(expression) as Promise<T>;
    await call(`window.__measure.start(${JSON.stringify(cfg.mode)})`);
    await waitFor('the viewer to present', () => call<boolean>('window.__measure.ready()'));
    log('presenting');

    // Host clock (µs) → performance.now() (ms): the tightest of 30 pings.
    let best = { rttUs: Infinity, offsetUs: 0 };
    for (let i = 0; i < 30; i++) {
      const t0 = harness.nowUs();
      const pageMs = await call<number>('window.__measure.now()');
      const t1 = harness.nowUs();
      if (t1 - t0 < best.rttUs) best = { rttUs: t1 - t0, offsetUs: (t0 + t1) / 2 - pageMs * 1000 };
    }
    result['clock'] = best;

    await contentTracing.startRecording({ included_categories: ['disabled-by-default-devtools.timeline.frame', 'blink.user_timing'], excluded_categories: ['*'] });
    await sleep(200);
    await call(`window.__measure.mark('pm-origin')`);

    let load: InstanceType<typeof MainLoad> | null = null;
    let agentTimer: ReturnType<typeof setInterval> | null = null;
    const agent = { requests: 0, replies: 0, replyBytes: 0, mainMs: [] as number[] };
    if (cfg.load) {
      load = new MainLoad({ win, ipcMain, log, nowUs: () => harness.nowUs() });
      load.start({ longTask: true, autosave: cfg.loadProfile !== 'no-autosave', agent: false, resize: true });
      const payload = { tool: 'get_layers', args: { detail: 'full', ids: Array.from({ length: 2000 }, (_, i) => `layer-${i}`) }, blob: 'x'.repeat(40_000) };
      agentTimer = setInterval(() => {
        if (page.isDestroyed()) return;
        const id = agent.requests++;
        void call<{ id: number }>(`window.__measure.agent(${JSON.stringify({ id, ...payload })})`).then((reply) => {
          const t0 = performance.now();
          const text = JSON.stringify({ jsonrpc: '2.0', id: reply.id, result: reply });
          agent.replies++;
          agent.replyBytes += text.length;
          agent.mainMs.push(performance.now() - t0);
        });
      }, 50);
      await sleep(2500);
    }

    if (cfg.scenario === 'playback') {
      // Section 15: the largest multiple of the content rate the panel can show.
      const panelHz = screen.getPrimaryDisplay().displayFrequency || 60;
      const fps = cfg.fps ?? 60;
      const rateHz = fps >= panelHz ? panelHz : Math.floor(panelHz / fps + 1e-9) * fps;
      if (cfg.mainRate) {
        harness.requestRefreshRate(rateHz);
        log(`main asks the display for ${rateHz} Hz`);
      }
      await call(`window.__measure.transport({ command: 'play', rate: { num: 1, den: 1 } })`);
      await sleep(1500);
      await call('window.__measure.reset()');
      if (cfg.mainRate) harness.requestRefreshRate(rateHz);
      const startMs = await call<number>(`window.__measure.mark('pm-record-start')`);
      const agentRun = cfg.agentSnapshots ? agentSnapshots(installed, cfg.agentSnapshots) : null;
      await sleep(cfg.durationMs);
      const endMs = await call<number>(`window.__measure.mark('pm-record-end')`);
      // Counted at the end of the window; a batch still waiting then is
      // reported, and settles after playback stops.
      const agentStats = agentRun?.stop();
      const jobsAtEnd = jobStatus;
      if (cfg.mainRate) {
        // The refresh times main's display link saw while recording.
        const ticks = harness.requestRefreshRate(0);
        const histogram: Record<string, number> = {};
        for (let i = 1; i < ticks.length; i++) {
          const ms = ((ticks[i]! - ticks[i - 1]!) * 1000).toFixed(1);
          histogram[ms] = (histogram[ms] ?? 0) + 1;
        }
        result['mainRate'] = { requestedHz: rateHz, panelHz, callbacks: ticks.length, intervalsMs: histogram };
      }
      result['window'] = { startMs, endMs };
      result['page'] = await call('window.__measure.collect()');
      await call(`window.__measure.transport({ command: 'pause' })`);
      if (agentRun && agentStats) {
        const settled = await agentRun.settled(30_000);
        // One more status period, so the counters after the settle are in.
        await sleep(Number(process.env['POWERMOVE_ENGINE_STATUS_LOG_MS'] ?? 0) + 100);
        const jobsAfter = jobStatus as typeof jobStatus;
        result['agentSnapshots'] = {
          ...agentStats, ...settled, playbackSignals,
          ...(jobsAtEnd ? { jobsAtEnd: { ...jobsAtEnd.jobs, lagMs: +(performance.now() - jobsAtEnd.atMs).toFixed(0) } } : {}),
          ...(jobsAfter ? { jobsAfterSettle: jobsAfter.jobs } : {})
        };
      }
    } else {
      const runs: Record<string, unknown> = {};
      for (const inputMode of cfg.inputModes ?? []) {
        await call(`window.__measure.setInputMode(${JSON.stringify(inputMode)})`);
        await sleep(500);
        await call('window.__measure.reset()');
        const generated: [number, number][] = [];
        const done = new Promise<void>((resolve) => setTimeout(resolve, cfg.durationMs + 500));
        const generator = new harness.InputGenerator(120, cfg.durationMs, 0, (input) => {
          generated.push([input.seq, input.tsUs]);
          if (!page.isDestroyed()) page.webContents.sendInputEvent({ type: 'mouseMove', x: input.x, y: input.y });
        });
        await done;
        generator.stop();
        await sleep(300);
        await call('window.__measure.setInputMode(null)');
        runs[inputMode] = { generated, page: await call('window.__measure.collect()') };
        log(`input mode ${inputMode}: ${generated.length} events`);
      }
      result['latency'] = runs;
    }

    if (agentTimer) clearInterval(agentTimer);
    if (load) {
      result['load'] = { ...load.stop(), agent: { requests: agent.requests, replies: agent.replies, replyBytes: agent.replyBytes, mainMsP50: agent.mainMs.sort((a, b) => a - b)[agent.mainMs.length >> 1] ?? null } };
    }
    await contentTracing.stopRecording(cfg.trace);
    result['engineStatus'] = engine.status();
    result['bridge'] = installed.frameBridge?.launch?.status() ?? null;
    result['machineLoad'] = { ...(result['machineLoad'] as object), after: os.loadavg() };
  } finally {
    result['log'] = logs.slice(-200);
    await writeFile(cfg.out, JSON.stringify(result));
    win?.destroy();
    await installed?.dispose().catch(() => {});
    await rm(home, { recursive: true, force: true });
  }
}

/** Agent frame batches through AgentJobs until stopped: count, latency,
 *  queue time, and failures. */
function agentSnapshots(installed: InstalledNativeEngine, opts: NonNullable<Config['agentSnapshots']>) {
  const jobs = installed.agentJobs!;
  const stats = { batches: 0, frames: 0, routed: [] as string[], failures: [] as string[], batchMs: [] as number[], queuedMs: [] as number[], sizes: [] as string[] };
  let stopped = false;
  let inFlightSince: number | null = null;
  const loop = (async () => {
    for (let turn = 1; !stopped; turn++) {
      const t0 = performance.now();
      inFlightSince = t0;
      const times = Array.from({ length: opts.frames }, (_, i) => ((turn * opts.frames + i) % 96) / 24);
      try {
        const r = await jobs.renderFrames(-2, 'measure-agent-run', { project: snapshotProject(opts.layers, turn, opts.heavy), times, width: opts.width, revision: turn });
        if (r.status === 'routed') stats.routed.push(r.reason);
        else {
          stats.batches++;
          stats.frames += r.images.length;
          stats.queuedMs.push(r.queuedMs);
          if (stats.sizes.length < 1) stats.sizes.push(...r.images.map((i) => `${i.width}x${i.height}`));
        }
      } catch (error) {
        stats.failures.push(error instanceof Error ? error.message : String(error));
      }
      inFlightSince = null;
      if (!stopped) stats.batchMs.push(performance.now() - t0);
      const wait = opts.turnMs - (performance.now() - t0);
      if (wait > 0 && !stopped) await sleep(wait);
    }
  })();
  return {
    stop() {
      stopped = true;
      const pendingMs = inFlightSince === null ? null : +(performance.now() - inFlightSince).toFixed(1);
      const sorted = [...stats.batchMs].sort((a, b) => a - b);
      const q = (p: number) => sorted.length ? +sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))]!.toFixed(1) : null;
      return { ...stats, batchMs: undefined, batchMsP50: q(0.5), batchMsP95: q(0.95), batchMsMax: q(1), failures: stats.failures.slice(0, 5), failureCount: stats.failures.length, routed: stats.routed.slice(0, 3), routedCount: stats.routed.length, pendingAtEndMs: pendingMs };
    },
    /** After playback stopped: whether the batch still waiting finished. */
    async settled(timeoutMs: number) {
      const t0 = performance.now();
      const done = await Promise.race([loop.then(() => true), sleep(timeoutMs).then(() => false)]);
      return { settledAfterStopMs: done ? +(performance.now() - t0).toFixed(1) : null };
    }
  };
}

process.on('uncaughtException', (error) => {
  process.stdout.write(`MEASURE FAIL uncaught ${error.stack ?? error.message}\n`);
  app.exit(1);
});
app.dock?.hide();
app.on('window-all-closed', () => {});
void app.whenReady().then(() => main()).then(
  () => {
    process.stdout.write('MEASURE DONE\n');
    app.exit(0);
  },
  (error: unknown) => {
    process.stdout.write(`MEASURE FAIL ${error instanceof Error ? (error.stack ?? error.message) : String(error)}\n`);
    app.exit(1);
  }
);
