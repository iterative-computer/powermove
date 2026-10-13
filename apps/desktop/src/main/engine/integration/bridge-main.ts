/*
 * The frame bridge's Electron integration test (plan section 18; section
 * 28 requires it on every Electron upgrade). Run by
 * scripts/frame-bridge-integration.mjs.
 *
 * Everything is the production path: main's `installNativeEngine` with the
 * real pm-bridge addon (per-launch Mach rendezvous, audit token and
 * signature policy, import once, send once per receiver), the interactive
 * engine in a utility process with the real pm-node addon and its bridge
 * test source, the real preload and its receiver, and the viewer's
 * `ViewerPresentation` in the page (bridge-page.ts). The window is shown
 * without activation and moved off every screen.
 *
 * It checks the Electron behaviour the bridge relies on (S1's API
 * findings): `importSharedTexture` from a Mach-received IOSurface;
 * `sendSharedTexture` to a frame with arguments; `setSharedTextureReceiver`
 * in a sandboxed, context-isolated preload; `VideoFrame`s across the
 * context bridge; repeated `getVideoFrame()` on one import seeing new
 * contents (every drawn frame's code read back); `release()` reaching main
 * so `allReferencesReleased` fires once per surface; `capturePage` reading
 * the presented frame. And the bridge's own behaviour: playback through
 * the jitter buffer, view changes tagging frames, a resize replacing the
 * pool, popout following, a reload re-sending surfaces to the new
 * receiver, an engine crash with a new rendezvous, and teardown.
 */
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { app, BrowserWindow, ipcMain, screen, type NativeImage } from 'electron';

import { installNativeEngine, type InstalledNativeEngine } from '../index';

type Step = { name: string; ms: number; detail?: unknown };
const steps: Step[] = [];
const logs: string[] = [];
const started = performance.now();
let last = started;

function step(name: string, detail?: unknown): void {
  const now = performance.now();
  steps.push({ name, ms: Math.round((now - last) * 10) / 10, ...(detail === undefined ? {} : { detail }) });
  last = now;
}

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`check failed: ${message}`);
}

async function waitFor<T>(label: string, poll: () => Promise<T | null | undefined | false> | T | null | undefined | false, timeoutMs = 20_000): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const value = await poll();
    if (value) return value;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms));

for (const s of ['disable-renderer-backgrounding', 'disable-background-timer-throttling', 'disable-backgrounding-occluded-windows']) app.commandLine.appendSwitch(s);
app.commandLine.appendSwitch('disable-features', 'MacWebContentsOcclusion');

/** Off every screen: shown without activation, never focused. */
function offscreen(width: number, height: number): Electron.Rectangle {
  const right = Math.max(...screen.getAllDisplays().map((d) => d.bounds.x + d.bounds.width));
  return { x: right + 4000, y: -6000, width, height };
}

type PageStats = {
  presented: number;
  presenting: boolean;
  presenter: { received: number; drawn: number; dropped: Record<string, number>; released: number; generation: number; sessions: number; viewSeq: number; parkTimeouts: number } | null;
  state: { size: { width: number; height: number }; devicePixelRatio: number; edr: boolean; viewSeq: number; zoom: number; pan: [number, number] } | null;
  latest: { seq: number; revision: number; viewSeq: number; timed: boolean } | null;
  overlayCss: { viewport: { x: number; y: number; width: number; height: number } } | null;
  code: { checked: number; ok: number; mismatches: string[]; distinct: number } | null;
  receiver: { launch: number; held: number; received: number; released: number; videoFrames: number; misses: number; errors: string[] };
  connections: number;
  connection: { generation: number; errors: string[] } | null;
  inPopout: boolean;
  dpr: number;
};

async function main(): Promise<void> {
  const desktop = process.env['PM_IT_DESKTOP'];
  const outDir = process.env['PM_IT_OUT'];
  check(desktop && outDir, 'PM_IT_DESKTOP and PM_IT_OUT are set');
  const home = await mkdtemp(path.join(os.tmpdir(), 'pm-bridge-it-'));
  const hz = screen.getPrimaryDisplay().displayFrequency || 60;

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
      log: (level, message) => logs.push(`${level}: ${message}`),
      windows: () => BrowserWindow.getAllWindows()
    });
    const engine = installed.engine!;
    const bridge = installed.frameBridge;
    check(bridge, `the frame bridge addon loaded (${logs.join(' | ')})`);
    await waitFor('the interactive engine', () => engine.status().interactive.state === 'ready');
    const firstLaunch = bridge.launch!;
    const firstPid = engine.supervisor('interactive').current!.pid!;
    await waitFor('the engine check-in', () => firstLaunch.status().accepted);
    const accepted = firstLaunch.status();
    check(accepted.accepted!.pid === firstPid, `main accepted the interactive engine's own pid (${accepted.accepted!.pid} vs ${firstPid})`);
    check(accepted.accepted!.pidversion > 0, 'the audit token carried a pidversion');
    check(accepted.rejections === 0, 'no message was refused');
    step('rendezvous: one verified check-in from the spawned engine', { policy: accepted.policy, pid: firstPid, pidversion: accepted.accepted!.pidversion, name: accepted.name });

    win = new BrowserWindow({
      show: false,
      width: 640,
      height: 360,
      useContentSize: true,
      frame: false,
      focusable: false,
      enableLargerThanScreen: true,
      backgroundColor: '#000000',
      webPreferences: { preload: path.join(outDir, 'preload.js'), contextIsolation: true, sandbox: true, nodeIntegration: false, backgroundThrottling: false }
    });
    win.webContents.on('console-message', (event) => logs.push(`page ${event.level}: ${event.message}`));
    win.webContents.on('did-finish-load', () => logs.push('main: the page finished loading'));
    win.webContents.on('render-process-gone', (_e, details) => logs.push(`renderer gone: ${details.reason}`));
    win.webContents.setWindowOpenHandler(() => ({
      action: 'allow',
      overrideBrowserWindowOptions: { show: false, frame: false, focusable: false, enableLargerThanScreen: true, backgroundColor: '#000000', webPreferences: { backgroundThrottling: false } }
    }));
    win.webContents.on('did-create-window', (child) => {
      logs.push('main: popout window created');
      child.showInactive();
      child.setBounds(offscreen(480, 270));
    });
    installed.attachWindow(win);
    await win.loadFile(path.join(outDir, 'page.html'), { query: { hz: String(hz) } });
    win.showInactive();
    win.setBounds(offscreen(640, 360));
    check(!win.isFocused(), 'the test window never takes focus');
    const page = win;
    const call = <T>(expression: string): Promise<T> => page.webContents.executeJavaScript(expression) as Promise<T>;
    const stats = (): Promise<PageStats> => call<PageStats>('window.__bridgeIt.stats()');
    await call('window.__bridgeIt.start()');

    // ── first frames ──
    const first = await waitFor('the first engine frame', async () => {
      const s = await stats();
      return s.presenting && s.code && s.code.checked > 0 ? s : null;
    });
    const launch = bridge.launch!.status();
    check(launch.imports === launch.surfaces && launch.sends === launch.imports, `every surface imported once and sent once (${JSON.stringify(launch)})`);
    check(first.receiver.received === launch.imports, `the preload received each import once (${first.receiver.received} of ${launch.imports})`);
    check(first.state!.size.width === Math.round(640 * first.dpr) || first.dpr > 2, `the pool is at viewer device pixels (${JSON.stringify(first.state!.size)} at DPR ${first.dpr})`);
    const retainAtFirst = bridge.launch!.retainCount(first.presenter!.generation, 0);
    step('surfaces imported once, sent once, and received once; the first frame presented', {
      retainCount: retainAtFirst,
      pool: launch.imports,
      size: first.state!.size,
      edr: first.state!.edr,
      dpr: first.dpr,
      format: first.state!.edr ? 'rgbaf16' : 'bgra'
    });

    // ── playback through the jitter buffer ──
    check(await call<boolean>(`window.__bridgeIt.transport({ command: 'play', rate: { num: 1, den: 1 } })`), 'transport play sent');
    await sleep(3000);
    check(await call<boolean>(`window.__bridgeIt.transport({ command: 'pause' })`), 'transport pause sent');
    await sleep(300);
    const played = await stats();
    const p = played.presenter!;
    check(played.code!.mismatches.length === 0, `every drawn frame's pixels carry its descriptor's code (${played.code!.mismatches.join('; ')})`);
    check(played.code!.distinct > launch.imports * 4, `one persistent import shows many frames (${played.code!.distinct} distinct codes from ${launch.imports} surfaces)`);
    check(played.receiver.videoFrames >= p.drawn, 'a VideoFrame per drawn frame');
    check(p.released >= p.drawn, `every drawn frame released (${p.released} of ${p.drawn})`);
    step('3 s of playback: every frame code read back matches its descriptor', {
      drawn: p.drawn,
      received: p.received,
      dropped: p.dropped,
      codes: played.code!.checked,
      distinct: played.code!.distinct,
      releases: p.released
    });

    // ── capturePage reads the presented frame ──
    await sleep(200);
    const settled = await stats();
    const image: NativeImage = await page.webContents.capturePage();
    const captured = decodeCapture(image, settled.state!.size.width);
    check(captured.seq === settled.latest!.seq % 2 ** 32, `capturePage shows the held frame (pixels seq ${captured.seq}, presented ${settled.latest!.seq})`);
    check(captured.rampMaxError <= 1, `the 8-bit ramp row survives capture (max error ${captured.rampMaxError} CV8)`);
    step('capturePage reads back the presented frame', { seq: captured.seq, rampMaxError: captured.rampMaxError, size: image.getSize() });

    // ── quality lines: a 10-bit ramp without banding, values above 1.0 ──
    const quality = await call<{ format: string; rows: { width: number; ramp: number[]; patches: number[] } }>('window.__bridgeIt.quality()');
    const ramp = quality.rows.ramp;
    const width = quality.rows.width;
    let monotonic = true;
    let maxErrorCv10 = 0;
    for (let x = 0; x < ramp.length; x++) {
      if (x > 0 && ramp[x]! < ramp[x - 1]! - 1e-6) monotonic = false;
      maxErrorCv10 = Math.max(maxErrorCv10, Math.abs(ramp[x]! - (x + 0.5) / width) * 1023);
    }
    const codes10 = new Set(ramp.map((v) => Math.round(v * 1023))).size;
    const PATCHES = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2, 4];
    if (quality.format === 'rgbaf16') {
      check(monotonic && codes10 >= 1020 && maxErrorCv10 <= 0.5, `the rgbaf16 ramp has no banding (${codes10} of 1024 10-bit codes, max error ${maxErrorCv10.toFixed(3)} CV10, monotonic ${monotonic})`);
      const patchError = Math.max(...quality.rows.patches.map((v, i) => Math.abs(v - PATCHES[i]!) / PATCHES[i]!));
      check(patchError < 0.002, `values above 1.0 survive import and sampling (${quality.rows.patches.map((v) => v.toFixed(4)).join(', ')})`);
    } else {
      check(quality.rows.patches.slice(3).every((v) => Math.abs(v - 1) < 0.002), `bgra clips at 1.0 (${quality.rows.patches.join(', ')})`);
    }
    step('quality lines through the Electron path', {
      format: quality.format,
      rampCodes10: codes10,
      rampMaxErrorCv10: +maxErrorCv10.toFixed(3),
      monotonic,
      patches: quality.rows.patches.map((v) => +v.toFixed(4))
    });

    // ── a view change tags frames and moves overlay geometry ──
    const zoom = 320 / 3840;
    const viewSeq = await call<number>(`window.__bridgeIt.setView({ zoom: ${zoom}, pan: [40, -20] })`);
    const viewed = await waitFor('a frame of the new view', async () => {
      const s = await stats();
      return s.latest && s.latest.viewSeq === viewSeq ? s : null;
    });
    const expectedX = (640 - 3840 * zoom) / 2 + 40;
    check(Math.abs(viewed.overlayCss!.viewport.x - expectedX) < 0.01 && Math.abs(viewed.overlayCss!.viewport.width - 320) < 0.01, `overlay geometry is at the frame's view (${JSON.stringify(viewed.overlayCss!.viewport)})`);
    step('a view change: frames carry the new view sequence and overlay geometry', { viewSeq, overlay: viewed.overlayCss!.viewport });

    // ── a resize replaces the pool; the old generation is released everywhere ──
    const beforeResize = bridge.launch!.status();
    page.setContentSize(800, 450);
    const resized = await waitFor('a frame from the resized pool', async () => {
      const s = await stats();
      return s.presenter && s.presenter.generation > first.presenter!.generation && s.code!.checked > viewed.code!.checked ? s : null;
    });
    await waitFor('the old pool released by main and the renderer', () => {
      const s = bridge.launch!.status();
      return s.allReleased >= beforeResize.imports ? s : null;
    });
    const afterResize = bridge.launch!.status();
    check(afterResize.retired >= beforeResize.imports, 'main released its references to the retired generation');
    step('a resize: a new pool generation; allReferencesReleased once per old surface', {
      size: resized.state!.size,
      generation: resized.presenter!.generation,
      allReleased: afterResize.allReleased,
      held: afterResize.held
    });

    // ── popout following ──
    check(await call<boolean>('window.__bridgeIt.popout()'), 'the stage moved to a popout');
    const before = (await stats()).code!.ok;
    const inPopout = await waitFor('frames presented in the popout', async () => {
      const s = await stats();
      // Paused: one frame per new viewer state (the popout's size).
      return s.inPopout && s.code!.ok > before && s.latest!.viewSeq === s.state!.viewSeq && s.state!.viewSeq > resized.state!.viewSeq ? s : null;
    });
    check(await call<boolean>(`window.__bridgeIt.transport({ command: 'play', rate: { num: 1, den: 1 } })`), 'play in the popout');
    await sleep(1000);
    await call(`window.__bridgeIt.transport({ command: 'pause' })`);
    const poppedPlay = await stats();
    check(poppedPlay.code!.mismatches.length === 0 && poppedPlay.code!.ok > inPopout.code!.ok + 20, `playback in the popout (${poppedPlay.code!.ok - inPopout.code!.ok} frames)`);
    await call('window.__bridgeIt.popin()');
    const back = await waitFor('frames back in the main window', async () => {
      const s = await stats();
      return !s.inPopout && s.code!.ok > poppedPlay.code!.ok && s.latest!.viewSeq === s.state!.viewSeq && s.state!.viewSeq > poppedPlay.state!.viewSeq ? s : null;
    });
    step('popout following: the loop and viewer state moved to the popout and back', {
      popoutSize: inPopout.state!.size,
      popoutFrames: poppedPlay.code!.ok - before,
      backSize: back.state!.size
    });

    // ── a reload: new ports, a new session, surfaces re-sent to the new receiver ──
    const beforeReload = bridge.launch!.status();
    const liveGeneration = (await stats()).presenter!.generation;
    // Main's reference is the only one in main while the page holds an import.
    const retainBeforeReload = bridge.launch!.retainCount(liveGeneration, 0);
    await page.webContents.reload();
    await new Promise<void>((resolve) => page.webContents.once('did-finish-load', () => resolve()));
    await call('window.__bridgeIt.start()');
    const reloaded = await waitFor('frames after the reload', async () => {
      const s = await stats();
      return s.presenting && s.code && s.code.ok > 0 ? s : null;
    });
    const afterReload = await waitFor('the old page\'s pool released or reclaimed', () => {
      const s = bridge.launch!.status();
      return s.allReleased + s.reclaimed >= beforeReload.imports ? s : null;
    });
    check(reloaded.receiver.received > 0 && afterReload.sends === afterReload.imports, 'the reloaded page received its own imports, each sent once');
    check(afterReload.held.count === reloaded.receiver.held, `main holds only the new page's pool (${afterReload.held.count} held, ${reloaded.receiver.held} in the page)`);
    step('a reload: the new page got new ports, a new pool, and its imports; the old page\'s surfaces were reclaimed', {
      received: reloaded.receiver.received,
      retainCountWhileImported: retainBeforeReload,
      allReleased: afterReload.allReleased,
      reclaimed: afterReload.reclaimed,
      held: afterReload.held
    });

    // ── an engine crash: a new launch, a new name, one new check-in ──
    const crashed = bridge.launch!;
    const oldName = crashed.status().name;
    const oldGeneration = engine.supervisor('interactive').current!.generation;
    process.kill(firstPid, 'SIGKILL');
    await waitFor('the interactive engine restarted', () => engine.supervisor('interactive').current?.generation === oldGeneration + 1 && engine.status().interactive.state === 'ready', 15_000);
    const relaunch = await waitFor('the new launch checked in', () => {
      const s = bridge.launch!.status();
      return s.accepted ? s : null;
    });
    const newPid = engine.supervisor('interactive').current!.pid!;
    check(relaunch.name !== oldName, 'a restarted engine gets a new bootstrap name');
    check(relaunch.accepted!.pid === newPid, 'main accepted the new engine');
    const recovered = await waitFor('frames from the restarted engine', async () => {
      const s = await stats();
      return s.receiver.launch === relaunch.launch && s.presenting && s.presenter!.sessions > 1 && s.code!.ok > reloaded.code!.ok ? s : null;
    }, 20_000);
    check(recovered.code!.mismatches.length === 0, `frames after the restart match (${recovered.code!.mismatches.join('; ')})`);
    // The crashed launch's surfaces: main released its imports; the page
    // released its own when the new launch's first surface arrived.
    const crashedFinal = await waitFor('the crashed launch\'s surfaces released', () => {
      const s = crashed.status();
      return s.held.count === 0 ? s : null;
    });
    step('an engine crash: new name, verified check-in, the viewer presents again; the old launch\'s surfaces released', {
      pid: newPid,
      launch: relaunch.launch,
      sessions: recovered.presenter!.sessions,
      oldLaunch: { allReleased: crashedFinal.allReleased, reclaimed: crashedFinal.reclaimed }
    });

    // ── teardown ──
    const lastLaunch = bridge.launch!;
    win.destroy();
    win = null;
    await installed.dispose();
    const final = lastLaunch.status();
    check(final.held.count === 0 && final.live === 0, `main holds no surface after teardown (${JSON.stringify(final.held)}, ${final.live} live imports)`);
    step('teardown: engines stopped, no surface held', { held: final.held, allReleased: final.allReleased, imports: final.imports });
  } catch (error) {
    logs.push(`bridge at failure: ${JSON.stringify(installed?.frameBridge?.launch?.status())}`);
    if (win && !win.isDestroyed()) {
      const last = await Promise.race([win.webContents.executeJavaScript('window.__bridgeIt?.stats()'), sleep(2000)]).catch((e: unknown) => String(e));
      logs.push(`page stats at failure: ${JSON.stringify(last)}`);
    }
    throw error;
  } finally {
    win?.destroy();
    await installed?.dispose().catch(() => {});
    await rm(home, { recursive: true, force: true });
    process.stdout.write(`${JSON.stringify({ steps, totalMs: Math.round(performance.now() - started), versions: { electron: process.versions.electron, chrome: process.versions.chrome }, log: logs.slice(-120) })}\n`);
  }
}

/** Reads the frame code and the 8-bit ramp row from a capture of a frame
 *  `frameWidth` pixels wide (the capture may be at another scale). */
function decodeCapture(image: NativeImage, frameWidth: number): { seq: number; rampMaxError: number } {
  const { width, height } = image.getSize();
  const bitmap = image.toBitmap(); // BGRA
  const scale = width / frameWidth;
  const at = (x: number, y: number): number => {
    const px = Math.min(width - 1, Math.max(0, Math.floor(x * scale)));
    const py = Math.min(height - 1, Math.max(0, Math.floor(y * scale)));
    return bitmap[(py * width + px) * 4 + 2]!; // red
  };
  const cell = Math.min(32, Math.max(1, Math.floor(frameWidth / 64)));
  let seq = 0;
  for (let c = 0; c < 32; c++) if (at((c + 0.5) * cell, 16) > 127) seq += 2 ** c;
  let rampMaxError = 0;
  for (let i = 0; i < 64; i++) {
    const x = Math.floor((i + 0.5) * (frameWidth / 64));
    const expected = Math.round(((x + 0.5) / frameWidth) * 255);
    rampMaxError = Math.max(rampMaxError, Math.abs(at(x + 0.5, 448) - expected));
  }
  return { seq, rampMaxError };
}

process.on('uncaughtException', (error) => {
  process.stdout.write(`FRAME-BRIDGE-IT FAIL uncaught ${error.stack ?? error.message}\n`);
  app.exit(1);
});

app.dock?.hide();
app.on('window-all-closed', () => {});
void app.whenReady().then(() => main()).then(
  () => {
    process.stdout.write('FRAME-BRIDGE-IT PASS\n');
    app.exit(0);
  },
  (error: unknown) => {
    process.stdout.write(`FRAME-BRIDGE-IT FAIL ${error instanceof Error ? (error.stack ?? error.message) : String(error)}\n`);
    app.exit(1);
  }
);
setTimeout(() => {
  process.stdout.write('FRAME-BRIDGE-IT FAIL timeout\n');
  app.exit(1);
}, 180_000).unref();
