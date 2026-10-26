import { copyFile, mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from './helpers/app';

test.use({ desktopLaunchOptions: {
  normalWindow: process.env.PM_NAVIGATION_VISIBLE === '1',
  userData: process.env.PM_NAVIGATION_PROFILE,
} });

test('timeline motion retains subpixel updates independently of composition frame rate', async ({ session }) => {
  const durationMs = Math.max(1000, Number(process.env.PM_NAVIGATION_DURATION_MS) || 15000);
  test.setTimeout(Math.max(90_000, durationMs + 60_000));
  if (process.env.PM_NAVIGATION_PROFILE) {
    await session.page.waitForFunction(() => {
      const PM = (window as any).PM;
      return PM?.GL?.gl && PM.Kernel.services.get('timeline')?.ctx && !PM.ProjectsScreen.isOpen;
    });
  } else await session.openEditor();
  if (!process.env.PM_NAVIGATION_PROFILE && process.env.PM_NAVIGATION_PROJECT) {
    const directory = await mkdtemp(join(tmpdir(), 'powermove-navigation-'));
    const copy = join(directory, 'Project.pmv');
    await copyFile(process.env.PM_NAVIGATION_PROJECT, copy);
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
    }, copy);
    await session.page.evaluate(() => (window as any).PM.openProject());
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      if (PM.Comps) {
        const largest = PM.Comps.list().sort((a: any, b: any) => PM.Comps.get(b.id).layers.length - PM.Comps.get(a.id).layers.length)[0];
        if (largest) PM.Comps.open(largest.id);
      } else {
        const project = PM.proj;
        const largest: any = Object.values(project.comps || {}).sort((a: any, b: any) => b.layers.length - a.layers.length)[0];
        if (largest?.layers.length > project.layers.length) PM.replaceProject({ ...project, ...largest,
          id: project.id, name: project.name, assets: project.assets, comps: project.comps, work: [0, largest.dur] });
      }
    });
  }
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Kernel.loader.builtinsReady;
    await PM.Kernel.loader.whenIdle();
    await PM.projectAssetsReady;
    await document.fonts.ready;
    const deadline = performance.now() + 30000;
    let project = PM.proj, stableSince = performance.now();
    while (performance.now() < deadline) {
      await PM.Kernel.loader.whenIdle();
      if (project !== PM.proj || PM.assets.loading?.size || PM.Preview?.preparing || PM.Export?.busy) {
        project = PM.proj; stableSince = performance.now();
      }
      if (performance.now() - stableSince >= 2000) return;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error('Project/media did not remain ready for two seconds before the cadence sample.');
  });
  const native = await session.app.evaluate(({ BrowserWindow, screen }) => {
    const window = BrowserWindow.getAllWindows()[0]!;
    const display = screen.getDisplayMatching(window.getBounds());
    return { visible: window.isVisible(), focused: window.isFocused(), focusable: window.isFocusable(),
      backgroundThrottling: window.webContents.getBackgroundThrottling(),
      displayFrequency: display.displayFrequency, scaleFactor: display.scaleFactor,
      bounds: window.getBounds(), devTools: window.webContents.isDevToolsOpened() };
  });
  const result = await session.page.evaluate(async durationMs => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.pause();
    const original = { time: PM.time, auto: PM.perf.auto, quality: PM.quality, pps: timeline.pps, scrollT: timeline.scrollT };
    PM.setTime(1); PM.perf.auto = false;
    timeline.pps = 30; timeline.scrollT = 0;
    const positions: number[] = [], gaps: number[] = [];
    const context = timeline.ctx, move = context.moveTo, line = context.lineTo;
    const render = PM.GL.render;
    const play = PM.play, pause = PM.pause, setTime = PM.setTime;
    const renderTimes: number[] = [];
    let presented = 0, cacheHits = 0, playingRafs = 0;
    const transitions: Array<Record<string, unknown>> = [];
    let sampling = false, sampleOrigin = 0, projectChanges = 0;
    const projectAtStart = PM.proj;
    const callers = () => (new Error().stack || '').split('\n').slice(3, 7).map(line => {
      const name = /^\s*at\s+([^\s(]+)/.exec(line)?.[1] || '';
      return /^[\w.$<>]+$/.test(name) ? name : 'anonymous';
    });
    const record = (event: string, detail: Record<string, unknown> = {}) => {
      if (sampling && transitions.length < 100) transitions.push({ event,
        elapsedMs: Math.round(performance.now() - sampleOrigin), time: PM.time, playing: PM.playing,
        loadingAssets: PM.assets.loading?.size ?? 0, sameProject: PM.proj === projectAtStart, ...detail });
    };
    PM.play = function (...args: any[]) { const result = play.apply(this, args); record('play', { callers: callers() }); return result; };
    PM.pause = function (...args: any[]) { record('pause', { callers: callers() }); return pause.apply(this, args); };
    PM.setTime = function (...args: any[]) { record('setTime', { targetTime: Number(args[0]), callers: callers() }); return setTime.apply(this, args); };
    const onProject = () => { if (sampling) projectChanges++; record('project'); };
    const onTransport = () => record('transport');
    PM.bus.on('project', onProject); PM.bus.on('transport', onTransport);
    let stem: number | null = null;
    context.moveTo = function (x: number, y: number) {
      stem = y === timeline.ruler - (timeline.header || 0) - 2 ? x : null; // the playhead line starts under its head
      return move.call(this, x, y);
    };
    context.lineTo = function (x: number, y: number) {
      if (stem === x && y === timeline.hgt) positions.push(x);
      stem = null;
      return line.call(this, x, y);
    };
    try {
      // Deterministic 120 Hz clock isolates spatial quantization from the
      // monitor running this test. The real engine drives normal playback below.
      PM.playing = true;
      for (let frame = 0; frame < 120; frame++) {
        PM.time = 1 + frame / 120; PM.bus.emit('time', PM.time); PM.bus.emit('draw:timeline');
      }
      PM.playing = false;
      const distinct = new Set(positions.map(x => x.toFixed(4))).size;
      // Restore real editor settings before measuring the actual playback path.
      PM.perf.auto = original.auto; PM.quality = original.quality;
      timeline.pps = original.pps; timeline.scrollT = original.scrollT;
      PM.setTime(original.time); PM.bus.emit('draw:timeline');
      const settings = { projectFps: PM.proj.fps, previewFps: PM.previewFps ?? null,
        layers: PM.proj.layers.length, auto: PM.perf.auto, quality: PM.quality,
        previewResolution: PM.previewResolution ?? 'auto', pps: timeline.pps,
        scrollT: timeline.scrollT, devicePixelRatio, visibility: document.visibilityState,
        focused: document.hasFocus(), renderWidth: PM.GL.canvas.width, renderHeight: PM.GL.canvas.height,
        previewMemoryBytes: PM.Memory.budget('preview'), selectedLayers: PM.sel?.layers?.length ?? 0,
        cachedPreview: !!PM.Preview?.active, exporting: !!PM.Export?.busy, agentFrameCapture: !!PM.agentFrameCapture };
      positions.length = 0;
      PM.GL.render = function (...args: any[]) {
        const start = performance.now();
        const result = render.apply(this, args);
        renderTimes.push(performance.now() - start);
        if (result !== false) presented++;
        if (PM.GL.stats.previewHit) cacheHits++;
        return result;
      };
      if (PM.proj !== projectAtStart || PM.assets.loading?.size) throw new Error('Project/media changed before playback could start.');
      sampling = true; sampleOrigin = performance.now(); PM.play();
      if (!PM.playing) throw new Error(`Playback did not start: loadingAssets=${PM.assets.loading?.size ?? 0}`);
      await new Promise<void>(resolve => {
        let start = 0, previous = 0;
        let lastPlaying = true, lastProject = PM.proj, lastTime = PM.time;
        const sample = (now: number) => {
          if (!start) start = now;
          if (previous) gaps.push(now - previous);
          previous = now;
          if (PM.playing) playingRafs++;
          if (PM.playing !== lastPlaying || PM.proj !== lastProject || PM.time < lastTime - .05) record('sample-transition');
          lastPlaying = PM.playing; lastProject = PM.proj; lastTime = PM.time;
          if (now - start < durationMs) requestAnimationFrame(sample); else resolve();
        };
        requestAnimationFrame(sample);
      });
      const playback = { playing: PM.playing, time: PM.time, fps: PM.perf.fps, cpuMs: PM.perf.ms,
        quality: PM.quality, compiling: !!PM.GL.compiling, loadingAssets: PM.assets.loading?.size ?? 0 };
      sampling = false;
      PM.pause();
      const sorted = [...gaps].sort((a, b) => a - b);
      renderTimes.sort((a, b) => a - b);
      let changedHeads = 0;
      for (let i = 1; i < positions.length; i++) if (Math.abs(positions[i]! - positions[i - 1]!) > 1e-5) changedHeads++;
      return { distinct, paints: positions.length, uniqueHeads: new Set(positions.map(x => x.toFixed(4))).size,
        changedHeads, settings, playback, playingRafs, transitions, projectChanges,
        rendering: { attempts: renderTimes.length, presented, cacheHits,
          medianMs: renderTimes[Math.floor(renderTimes.length / 2)] ?? 0,
          p95Ms: renderTimes[Math.floor(renderTimes.length * .95)] ?? 0, maxMs: renderTimes.at(-1) ?? 0 },
        raf: gaps.length, durationMs: gaps.reduce((total, gap) => total + gap, 0),
        medianGap: sorted[Math.floor(sorted.length / 2)], p95Gap: sorted[Math.floor(sorted.length * .95)] };
    } finally {
      sampling = false;
      PM.play = play; PM.pause = pause; PM.setTime = setTime;
      PM.bus.off('project', onProject); PM.bus.off('transport', onTransport);
      PM.pause(); PM.perf.auto = original.auto; PM.quality = original.quality;
      timeline.pps = original.pps; timeline.scrollT = original.scrollT; PM.setTime(original.time);
      PM.GL.render = render;
      context.moveTo = move; context.lineTo = line;
    }
  }, durationMs);
  console.log('TIMELINE_CADENCE', JSON.stringify({ native, ...result }));
  if (process.env.PM_NAVIGATION_VISIBLE === '1') {
    expect(native.visible).toBe(true);
    expect(native.focusable).toBe(true);
    expect(native.backgroundThrottling).toBe(true);
  }
  expect(result.distinct).toBeGreaterThanOrEqual(115);
  // Follow the machine's delivered refresh rate rather than assuming 120Hz.
  // A frame-quantized head fails this on displays faster than the project FPS.
  expect(result.raf).toBeGreaterThan(durationMs / 1000 * 10);
  expect(result.playback.playing).toBe(true);
  expect(result.playingRafs).toBeGreaterThanOrEqual(result.raf * .99);
  expect(result.projectChanges).toBe(0);
  expect(result.changedHeads).toBeGreaterThanOrEqual(result.raf * .9);
  // Opt-in hardware benchmark: reject a slow editor whose head correctly
  // follows an equally slow renderer. Ordinary CI only checks clock fidelity.
  const minimumRefreshRatio = Number(process.env.PM_NAVIGATION_MIN_REFRESH_RATIO) || 0;
  if (minimumRefreshRatio > 0 && native.displayFrequency > 0) {
    expect(result.raf / (result.durationMs / 1000)).toBeGreaterThanOrEqual(native.displayFrequency * minimumRefreshRatio);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('scrub indicator follows subframe pointer motion while selected frames stay accurate', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.pause(); PM.setTime(1);
    timeline.pps = 30; timeline.scrollT = 0;
    PM.bus.emit('draw:timeline');
    const rect = timeline.cv.getBoundingClientRect();
    const context = timeline.ctx, move = context.moveTo, line = context.lineTo;
    const positions: number[] = [];
    let stem: number | null = null;
    context.moveTo = function (x: number, y: number) {
      stem = y === timeline.ruler - (timeline.header || 0) - 2 ? x : null; // the playhead line starts under its head
      return move.call(this, x, y);
    };
    context.lineTo = function (x: number, y: number) {
      if (stem === x && y === timeline.hgt) positions.push(x);
      stem = null;
      return line.call(this, x, y);
    };
    let aligned = true;
    const event = (type: string, frame: number) => {
      const value = new PointerEvent(type, {
        bubbles: true, pointerId: 71, button: 0, buttons: type === 'pointerup' ? 0 : 1,
        clientX: rect.left + timeline.gut + 30 + frame / 4, clientY: rect.top + 18,
      });
      // Programmatic events do not pass through Chromium's native hit test.
      Object.defineProperties(value, { offsetX: { value: timeline.gut + 30 + frame / 4 }, offsetY: { value: 18 } });
      return value;
    };
    try {
      timeline.cv.dispatchEvent(event('pointerdown', 0));
      positions.length = 0;
      for (let frame = 0; frame < 120; frame++) {
        window.dispatchEvent(event('pointermove', frame));
        PM.bus.emit('draw:timeline');
        aligned &&= Math.abs(PM.time * PM.proj.fps - Math.round(PM.time * PM.proj.fps)) < 1e-7;
      }
      const distinct = new Set(positions.map(x => x.toFixed(4))).size;
      window.dispatchEvent(event('pointerup', 119));
      PM.bus.emit('draw:timeline');
      return { distinct, aligned, time: PM.time };
    } finally { window.dispatchEvent(event('pointerup', 119)); context.moveTo = move; context.lineTo = line; }
  });
  console.log('TIMELINE_SCRUB', JSON.stringify(result));
  expect(result.distinct).toBeGreaterThanOrEqual(115);
  expect(result.aligned).toBe(true);
  expect(result.time).toBeCloseTo(2, 5);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
