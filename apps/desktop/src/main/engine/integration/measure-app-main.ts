/*
 * The real-viewer measurement (amendment 18): the S1 harness's method on
 * the real app, so the gate's "today" is the WebGL viewer drawing a corpus
 * project, not the stand-in canvas, and the engine path is measured inside
 * the same app with the same project, panels, and load.
 *
 * scripts/frame-bridge-measure.mjs starts the real app (apps/desktop, built
 * out/) as a hidden background test launch with an isolated profile, and
 * loads this module first (`electron -r app-main.js <apps/desktop>`). It
 * changes nothing in the app; once the editor window exists it:
 *
 * 1. mutes the window, shows it without activation, and moves it off every
 *    screen (the run stops if the window would be visible);
 * 2. opens a copy of the corpus project through the app's own Open
 *    Project path (the dialog answers with the copy);
 * 3. injects measure-app-page.ts, which records compositor draws (today)
 *    or wraps the kernel's `engine.viewer` so the real viewer presents the
 *    interactive engine's frames over the kernel's own engine connection
 *    (engine, tracer);
 * 4. applies S1's load (the main-thread autosave is left out with the
 *    `no-autosave` profile, as the app runs since autosave serialization
 *    moved to a worker), traces Chromium's PipelineReporter, and plays or
 *    sends synthetic pointer input, exactly as measure-main.ts does.
 *
 * PM_MEASURE_APP (JSON) configures one run; the raw result goes to `out`.
 */
import { copyFileSync, readFileSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { app, BrowserWindow, contentTracing, dialog, ipcMain, screen } from 'electron';

type AppConfig = {
  scenario: 'playback' | 'latency';
  /** `tracer`: the app's own engine connection draws the document (Use New Engine). */
  mode: 'today' | 'engine' | 'tracer';
  /** A synthetic project of slice features instead of the corpus project. */
  synthetic?: { layers: number; fps?: number; fpsRational?: [number, number] | null; images?: number; seed?: number; scene3d?: boolean };
  /** Playback only: an inspector edit every `everyMs` while recording (tracer). */
  edits?: { everyMs: number };
  load: boolean;
  loadProfile?: 'gate' | 'no-autosave';
  durationMs: number;
  /** The corpus project (copied into the profile; the original is never opened). */
  project: string;
  /** Content time playback starts from, seconds. */
  startS?: number;
  inputModes?: ('drag' | 'pan' | 'orbit')[];
  /** `false`: engine-path gestures keep `pointermove` (before step 2). */
  rawGestures?: boolean;
  /** `false`: the viewer draws engine frames on the page's main thread, not in its presentation worker. */
  offMainThread?: boolean;
  /** The C′ prototype presents playback (POWERMOVE_BRIDGE_REMOTE_LAYER=1);
   *  main hosts its remote layer over the viewer's stage. */
  remoteLayer?: boolean;
  /** Where the window sits: off every screen (default) or invisible on screen. */
  placement?: 'offscreen' | 'behind';
  pageScript: string;
  out: string;
  trace: string;
};

type HarnessAddon = {
  nowUs(): number;
  allowOffscreenWindow(viewHandle: Buffer): string;
  placeBehindDesktop(viewHandle: Buffer): number;
  hostRemoteLayer(viewHandle: Buffer, contextId: number, x: number, y: number, width: number, height: number): number;
  removeRemoteLayer(id: number): boolean;
  InputGenerator: new (hz: number, durationMs: number, kind: number, callback: (input: { seq: number; tsUs: number; x: number; y: number }) => void) => { stop(): void };
};

const raw = process.env['PM_MEASURE_APP'];
const cfg: AppConfig | null = process.type === 'browser' && raw ? (JSON.parse(raw) as AppConfig) : null;
const logs: string[] = [];
const log = (message: string): void => {
  logs.push(`${(performance.now() / 1000).toFixed(3)} ${message}`);
};
const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

async function waitFor<T>(label: string, poll: () => Promise<T | null | undefined | false> | T | null | undefined | false, timeoutMs = 60_000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await poll();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`);
    await sleep(50);
  }
}

/**
 * `behind` placement (C′ runs and the WebGL runs they are compared with):
 * Core Animation neither shows nor reports, and so never frees, the
 * drawables of a layer whose window is off every screen or at opacity 0
 * (measured: every C′ frame unpresented both ways), so the remote layer
 * needs a window on a display. This one is on the primary display one
 * level below the desktop picture, which covers it: ignoring the mouse,
 * never activated, on every Space, out of Mission Control's cycle, at the
 * same 1920 × 1080 points as the off-screen runs.
 */
function placeBehind(win: BrowserWindow, harness: HarnessAddon, width: number, height: number): number {
  const area = screen.getPrimaryDisplay().workArea;
  harness.allowOffscreenWindow(win.getNativeWindowHandle());
  const level = harness.placeBehindDesktop(win.getNativeWindowHandle());
  win.setIgnoreMouseEvents(true);
  win.setBounds({ x: area.x, y: area.y, width, height });
  return level;
}

function offscreen(width: number, height: number): Electron.Rectangle {
  const right = Math.max(...screen.getAllDisplays().map((d) => d.bounds.x + d.bounds.width));
  return { x: right + 4000, y: -6000, width, height };
}

function visibleOnAnyDisplay(bounds: Electron.Rectangle): boolean {
  return screen.getAllDisplays().some((d) => {
    const b = d.bounds;
    return bounds.x < b.x + b.width && bounds.x + bounds.width > b.x && bounds.y < b.y + b.height && bounds.y + bounds.height > b.y;
  });
}

// The S1 load generator (CommonJS, shared with the spike).
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { MainLoad } = require('../../../../../../engine/spikes/s1-bridge/app/load.cjs') as {
  MainLoad: new (options: { win: BrowserWindow; ipcMain: Electron.IpcMain; log: (m: string) => void; nowUs: () => number }) => {
    start(options: Record<string, unknown>): void;
    stop(): Record<string, unknown>;
  };
};

/**
 * The interactive engine's per-frame trace since the last call (render
 * start, GPU done, descriptor sent; host µs), emptied: it rides on the
 * engine's status report with POWERMOVE_BRIDGE_TRACE=1.
 */
async function takeEngineTrace(): Promise<unknown[] | null> {
  const engine = (globalThis as { __powermoveNativeEngine?: { engine?: { report(): Promise<unknown> } | null } }).__powermoveNativeEngine?.engine;
  if (!engine) return null;
  const report = (await engine.report().catch(() => null)) as { presentTrace?: unknown[] } | null;
  return report?.presentTrace ?? null;
}

async function run(config: AppConfig): Promise<void> {
  const harness = require(process.env['POWERMOVE_PM_BRIDGE']!) as HarnessAddon; // eslint-disable-line @typescript-eslint/no-require-imports
  const display = screen.getPrimaryDisplay();
  const hz = display.displayFrequency || 60;
  const result: Record<string, unknown> = {
    config,
    versions: { electron: process.versions.electron, chrome: process.versions.chrome },
    display: { hz, scaleFactor: display.scaleFactor, colorDepth: display.colorDepth },
    machineLoad: { before: os.loadavg() },
    startedAt: new Date().toISOString()
  };
  let verdict = 'MEASURE DONE';
  try {
    const win = await waitFor('the editor window', () => BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:')) ?? null);
    const contents = win.webContents;
    contents.setAudioMuted(true);
    contents.on('console-message', (event) => {
      if (event.level === 'error' || event.level === 'warning' || event.message.includes('[bridge-page]') || event.message.includes('[measure]') || event.message.includes('engine')) log(`page ${event.level}: ${event.message.slice(0, 300)}`);
    });
    const call = <T>(expression: string): Promise<T> => contents.executeJavaScript(expression) as Promise<T>;
    /** Runs an expression for its effect (its value may not be cloneable). */
    const run = async (label: string, expression: string): Promise<void> => {
      log(label);
      await contents.executeJavaScript(`(async () => { await (${expression}); return true; })()`);
    };
    await waitFor('the page', () => call<boolean>('!!(window.PM && window.PM.Kernel && window.PM.ProjectsScreen && window.PM.mkProject)'));
    await run('builtins', 'window.PM.Kernel.loader.builtinsReady');

    // Off every screen (or behind the desktop picture), never activated;
    // refuse to run if it would show anywhere. AppKit pulls titled windows onto a screen when they are
    // shown, so the harness addon lets this one stay off.
    if (config.placement === 'behind') {
      placeBehind(win, harness, 1920, 1080);
      win.showInactive();
      // Showing must not lift it: set again and check.
      const level = harness.placeBehindDesktop(win.getNativeWindowHandle());
      await sleep(300);
      result['window'] = { placement: 'behind', level, bounds: win.getBounds(), content: win.getContentBounds() };
    } else {
      result['windowClass'] = harness.allowOffscreenWindow(win.getNativeWindowHandle());
      win.setBounds(offscreen(1920, 1080));
      if (visibleOnAnyDisplay(win.getBounds())) throw new Error(`the measurement window would be on screen at ${JSON.stringify(win.getBounds())}`);
      win.showInactive();
      await sleep(300);
      const bounds = win.getBounds();
      result['window'] = { placement: 'offscreen', bounds, content: win.getContentBounds() };
      if (visibleOnAnyDisplay(bounds)) {
        win.hide();
        throw new Error(`the measurement window moved on screen at ${JSON.stringify(bounds)}`);
      }
    }

    // Open the corpus project the way a user does (Open Project), from a copy.
    const copy = config.synthetic ? '' : path.join(app.getPath('userData'), path.basename(config.project));
    if (!config.synthetic) copyFileSync(config.project, copy);
    dialog.showOpenDialog = (async () => ({ canceled: false, filePaths: [copy] })) as typeof dialog.showOpenDialog;
    dialog.showMessageBox = (async () => ({ response: 2, checkboxChecked: false })) as typeof dialog.showMessageBox;
    await run('editor', `(() => { const PM = window.PM; if (PM.ProjectsScreen.isOpen) { window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Measurement' }) })); PM.ProjectsScreen.hide(); } })()`);
    await waitFor('the editor', () => call<boolean>('!window.PM.ProjectsScreen.isOpen'));
    const before = await call<string>('window.PM.proj.id');
    // Toasts (an open that failed says so in one) reach the log.
    await run('watch toasts', `(() => { const PM = window.PM; const toast = PM.toast; PM.toast = (...args) => { console.warn('[measure] toast: ' + String(args[0]).slice(0, 200)); return toast.apply(PM, args); }; })()`);
    // `PM.openProject()` resolves once the open finished or failed (a
    // failure shows a toast, logged above). Main closes a project reader
    // 120 s after it opened it (`save.ts`), so an open of the 138 MB corpus
    // project that takes longer on a loaded machine fails at the end with
    // "Unknown project read token": try once more (the media store already
    // holds most of it), then give up.
    const opened = (): Promise<boolean> => call<boolean>(`window.PM.proj.id !== ${JSON.stringify(before)} && window.PM.proj.layers.length > 0`);
    let pageInjected = false;
    if (config.synthetic) {
      // The synthetic project is built in the page (synthetic-project.ts).
      await contents.executeJavaScript(`${readFileSync(config.pageScript, 'utf8')}\n;true`);
      pageInjected = true;
      result['synthetic'] = await call(`window.__measureApp.synthetic(${JSON.stringify(config.synthetic)})`);
      log(`synthetic project: ${JSON.stringify(result['synthetic'])}`);
    }
    for (let attempt = 1; !config.synthetic; attempt++) {
      const openStart = performance.now();
      await run('open project', 'window.PM.openProject()');
      log(`open ${attempt} returned after ${((performance.now() - openStart) / 1000).toFixed(1)} s (load ${os.loadavg()[0]!.toFixed(1)})`);
      if (await opened()) break;
      if (attempt >= 2) throw new Error('the corpus project did not open (see the toasts in the log)');
    }
    await run('project assets', 'window.PM.projectAssetsReady');
    // The whole project plays (its saved work area would loop after 30 s).
    result['project'] = await call(`(() => { const p = window.PM.proj; p.work = [0, p.dur]; return { name: p.name, w: p.w, h: p.h, fps: p.fps, dur: p.dur, layers: p.layers.length }; })()`);
    await sleep(2000);

    // The C′ prototype: host the engine's remote layer over the viewer's
    // stage when the engine announces it (on its first pool).
    const remoteLayers: Record<string, unknown>[] = [];
    result['remoteLayers'] = remoteLayers;
    const layerHosts = new Map<number, number>();
    /** The latest announcement per viewer port: an older one still placing its host is dropped. */
    const latest = new Map<number, number>();
    let announcements = 0;
    type Installed = { frameBridge?: { listen(listener: (event: { kind: string; clientPort?: number | null; contextId?: number | null; width?: number | null; height?: number | null; scale?: number | null }) => void): () => void } | null };
    const bridgeHost = (globalThis as { __powermoveNativeEngine?: Installed }).__powermoveNativeEngine?.frameBridge ?? null;
    if (config.remoteLayer && !bridgeHost) throw new Error('the C′ prototype needs the frame bridge');
    const stopListening = config.remoteLayer
      ? bridgeHost!.listen((event) => {
          if (event.kind !== 'remoteLayer' || win.isDestroyed()) return;
          const port = event.clientPort ?? -1;
          const token = ++announcements;
          latest.set(port, token);
          if (!event.contextId || !event.width || !event.height || !event.scale) {
            const old = layerHosts.get(port);
            if (old !== undefined) harness.removeRemoteLayer(old);
            layerHosts.delete(port);
            return;
          }
          const { contextId, width, height, scale } = event;
          // A new pool keeps its context (a live resize): the old host stays
          // until the new one, at the stage's new place, is in.
          void call<{ left: number; top: number } | null>('window.__measureApp.stageRect()').then((rect) => {
            if (win.isDestroyed() || latest.get(port) !== token) return;
            try {
              const id = harness.hostRemoteLayer(win.getNativeWindowHandle(), contextId, rect?.left ?? 0, rect?.top ?? 0, width / scale, height / scale);
              const old = layerHosts.get(port);
              layerHosts.set(port, id);
              if (old !== undefined) harness.removeRemoteLayer(old);
              remoteLayers.push({ contextId, width, height, scale, at: rect, hostedUs: harness.nowUs() });
              log(`hosted remote layer ${contextId} (${width}x${height} at ${scale}x)`);
            } catch (error) {
              log(`could not host remote layer ${contextId}: ${error instanceof Error ? error.message : String(error)}`);
            }
          });
        })
      : null;

    if (!pageInjected) await contents.executeJavaScript(`${readFileSync(config.pageScript, 'utf8')}\n;true`);
    await run('init', `window.__measureApp.init(${JSON.stringify({ mode: config.mode, displayHz: hz, rawGestures: config.rawGestures !== false, offMainThread: config.offMainThread !== false })})`);
    await run('seek', `window.PM.setTime(${config.startS ?? 0})`);
    await waitFor('the viewer to present', () => call<boolean>('window.__measureApp.ready()'), 60_000);
    log('presenting');

    // Host clock (µs) → performance.now() (ms): the tightest of 30 pings.
    let best = { rttUs: Infinity, offsetUs: 0 };
    for (let i = 0; i < 30; i++) {
      const t0 = harness.nowUs();
      const pageMs = await call<number>('window.__measureApp.now()');
      const t1 = harness.nowUs();
      if (t1 - t0 < best.rttUs) best = { rttUs: t1 - t0, offsetUs: (t0 + t1) / 2 - pageMs * 1000 };
    }
    result['clock'] = best;

    // viz's Graphics.Pipeline steps time a worker canvas's frames, which
    // have no PipelineReporter of their own (same presentation feedback).
    await contentTracing.startRecording({ included_categories: ['disabled-by-default-devtools.timeline.frame', 'blink.user_timing', 'graphics.pipeline', 'viz'], excluded_categories: ['*'] });
    await sleep(200);
    await call(`window.__measureApp.mark('pm-origin')`);

    let load: InstanceType<typeof MainLoad> | null = null;
    let agentTimer: ReturnType<typeof setInterval> | null = null;
    const agent = { requests: 0, replies: 0, replyBytes: 0, mainMs: [] as number[] };
    if (config.load) {
      load = new MainLoad({ win, ipcMain, log, nowUs: () => harness.nowUs() });
      load.start({ longTask: true, autosave: config.loadProfile !== 'no-autosave', agent: false, resize: true });
      const payload = { tool: 'get_layers', args: { detail: 'full', ids: Array.from({ length: 2000 }, (_, i) => `layer-${i}`) }, blob: 'x'.repeat(40_000) };
      agentTimer = setInterval(() => {
        if (win.isDestroyed()) return;
        const id = agent.requests++;
        void call<{ id: number }>(`window.__measureApp.agent(${JSON.stringify({ id, ...payload })})`).then((reply) => {
          const t0 = performance.now();
          const text = JSON.stringify({ jsonrpc: '2.0', id: reply.id, result: reply });
          agent.replies++;
          agent.replyBytes += text.length;
          agent.mainMs.push(performance.now() - t0);
        });
      }, 50);
      await sleep(2500);
    }

    if (config.scenario === 'playback') {
      if (!(await call<boolean>(`window.__measureApp.play(${config.startS ?? 0})`))) throw new Error('could not start playback');
      await sleep(1500);
      await run('reset', 'window.__measureApp.reset()');
      await takeEngineTrace();
      // Diagnostics only (PM_MEASURE_CPU_PROFILE=<file>): a CPU profile of
      // the editor's renderer over the recording window. Attaching the
      // debugger costs time, so a profiled run is not a measurement.
      const profilePath = process.env['PM_MEASURE_CPU_PROFILE'];
      if (profilePath) {
        contents.debugger.attach('1.3');
        await contents.debugger.sendCommand('Profiler.enable');
        await contents.debugger.sendCommand('Profiler.setSamplingInterval', { interval: 500 });
        await contents.debugger.sendCommand('Profiler.start');
      }
      const startMs = await call<number>(`window.__measureApp.mark('pm-record-start')`);
      // Inspector edits while it plays (the "inspector edit during playback" row).
      const editTimer = config.edits ? setInterval(() => {
        if (!win.isDestroyed()) void call('window.__measureApp.edit()').catch(() => undefined);
      }, config.edits.everyMs) : null;
      await sleep(config.durationMs);
      if (editTimer) clearInterval(editTimer);
      const endMs = await call<number>(`window.__measureApp.mark('pm-record-end')`);
      result['recordWindow'] = { startMs, endMs };
      if (profilePath) {
        const { profile } = (await contents.debugger.sendCommand('Profiler.stop')) as { profile: unknown };
        writeFileSync(profilePath, JSON.stringify(profile));
        contents.debugger.detach();
        result['cpuProfile'] = profilePath;
      }
      result['page'] = await call('window.__measureApp.collect()');
      result['engineFrames'] = await takeEngineTrace();
      await run('pause', 'window.__measureApp.pause()');
    } else {
      const runs: Record<string, unknown> = {};
      for (const inputMode of config.inputModes ?? []) {
        const press = await call<{ x: number; y: number; button: 'left' | 'middle' } | { error: string }>(`window.__measureApp.prepare(${JSON.stringify(inputMode)})`);
        if ('error' in press) throw new Error(press.error);
        const modifiers: Electron.MouseInputEvent['modifiers'] = [press.button === 'left' ? 'leftbuttondown' : 'middlebuttondown'];
        contents.sendInputEvent({ type: 'mouseMove', x: press.x, y: press.y });
        await sleep(100);
        contents.sendInputEvent({ type: 'mouseDown', x: press.x, y: press.y, button: press.button, clickCount: 1 });
        await sleep(400);
        await run('input mode', `window.__measureApp.setInputMode(${JSON.stringify(inputMode)})`);
        await run('reset', 'window.__measureApp.reset()');
        await takeEngineTrace();
        // Diagnostics only, as in the playback scenario (PM_MEASURE_CPU_PROFILE).
        const profilePath = process.env['PM_MEASURE_CPU_PROFILE'];
        if (profilePath) {
          contents.debugger.attach('1.3');
          await contents.debugger.sendCommand('Profiler.enable');
          await contents.debugger.sendCommand('Profiler.setSamplingInterval', { interval: 500 });
          await contents.debugger.sendCommand('Profiler.start');
        }
        const generated: [number, number][] = [];
        const done = new Promise<void>((resolve) => setTimeout(resolve, config.durationMs + 500));
        let last = { x: press.x, y: press.y };
        const generator = new harness.InputGenerator(120, config.durationMs, 0, (input) => {
          generated.push([input.seq, input.tsUs]);
          // The page decodes the sequence: x = x0 − 200 + seq mod 400, y = y0 + ⌊seq / 400⌋ mod 6.
          last = { x: press.x - 200 + (input.seq % 400), y: press.y + (Math.floor(input.seq / 400) % 6) };
          if (!win.isDestroyed()) contents.sendInputEvent({ type: 'mouseMove', x: last.x, y: last.y, button: press.button, modifiers });
        });
        await done;
        generator.stop();
        if (profilePath) {
          const { profile } = (await contents.debugger.sendCommand('Profiler.stop')) as { profile: unknown };
          const target = (config.inputModes?.length ?? 0) > 1 ? profilePath.replace(/(\.cpuprofile)?$/, `-${inputMode}.cpuprofile`) : profilePath;
          writeFileSync(target, JSON.stringify(profile));
          contents.debugger.detach();
          result['cpuProfile'] = target;
        }
        await sleep(300);
        await run('input mode off', 'window.__measureApp.setInputMode(null)');
        contents.sendInputEvent({ type: 'mouseUp', x: last.x, y: last.y, button: press.button, clickCount: 1 });
        runs[inputMode] = { generated, page: await call('window.__measureApp.collect()'), engineFrames: await takeEngineTrace() };
        log(`input mode ${inputMode}: ${generated.length} events`);
        await sleep(500);
      }
      result['latency'] = runs;
    }

    if (agentTimer) clearInterval(agentTimer);
    if (load) {
      result['load'] = { ...load.stop(), agent: { requests: agent.requests, replies: agent.replies, replyBytes: agent.replyBytes, mainMsP50: agent.mainMs.sort((a, b) => a - b)[agent.mainMs.length >> 1] ?? null } };
    }
    stopListening?.();
    for (const id of layerHosts.values()) harness.removeRemoteLayer(id);
    await contentTracing.stopRecording(config.trace);
    const installed = (globalThis as { __powermoveNativeEngine?: { engine?: { status(): unknown } | null; frameBridge?: { launch?: { status(): unknown } | null } | null } }).__powermoveNativeEngine;
    result['engineStatus'] = installed?.engine?.status() ?? null;
    result['bridge'] = installed?.frameBridge?.launch?.status() ?? null;
    result['machineLoad'] = { ...(result['machineLoad'] as object), after: os.loadavg() };
  } catch (error) {
    verdict = `MEASURE FAIL ${error instanceof Error ? (error.stack ?? error.message) : String(error)}`;
    result['error'] = verdict;
  } finally {
    result['log'] = logs.slice(-200);
    writeFileSync(config.out, JSON.stringify(result));
    process.stdout.write(`${verdict}\n`);
    // The profile is disposable: skip the editor's close decision.
    app.exit(verdict === 'MEASURE DONE' ? 0 : 1);
  }
}

if (cfg) {
  for (const s of ['disable-renderer-backgrounding', 'disable-background-timer-throttling', 'disable-backgrounding-occluded-windows']) app.commandLine.appendSwitch(s);
  app.commandLine.appendSwitch('disable-features', 'MacWebContentsOcclusion');
  process.on('uncaughtException', (error) => log(`uncaught: ${error.stack ?? error.message}`));
  void app.whenReady().then(() => run(cfg));
}
