import { copyFile, mkdtemp, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect, test } from './helpers/app';

// Private project/media stay outside the repository. This opt-in measurement
// exercises the normal transport and graph, then cold and repeated scrubs.
test('measure heavy project playback and graph scrubbing', async ({ session }) => {
  test.skip(!process.env.PM_HEAVY_PROJECT, 'Set PM_HEAVY_PROJECT to a private .pmv copy');
  test.setTimeout(360_000);
  // The session fixture deletes its disposable profile after Electron exits,
  // including this large archive copy and any automatically saved changes.
  const directory = await mkdtemp(join(session.userData, 'heavy-profile-'));
  const copy = join(directory, 'Project.pmv');
  await copyFile(process.env.PM_HEAVY_PROJECT!, copy);
  const { page } = session;
  await session.openEditor();
  await page.setViewportSize({ width: 1728, height: 1117 });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1728, height: 1117, deviceScaleFactor: 2, mobile: false });
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
  }, copy);
  await page.evaluate(() => (window as any).PM.openProject());
  await page.waitForFunction(() => !(window as any).PM.assets.loading?.size);
  if (process.env.PM_HEAVY_SNAPSHOT) {
    const snapshot = JSON.parse(readFileSync(process.env.PM_HEAVY_SNAPSHOT, 'utf8'));
    await page.evaluate(data => window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data })), snapshot);
    await page.waitForFunction(() => !(window as any).PM.assets.loading?.size);
  }
  await page.evaluate(() => {
    const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline');
    PM.pause(); PM.perf.auto = false; PM.quality = 1; PM.loop = false;
    // Measure the entire copied composition, including sections beyond the
    // user's current work area. The original document is never written.
    PM.proj.work = [0, PM.proj.dur];
    const candidates = PM.proj.layers.filter((layer: any) => layer.p?.['position.y']?.kf.length > 1);
    const layers = candidates.slice(0, 2);
    PM.sel.keys = layers.flatMap((layer: any) => layer.p['position.y'].kf.map((key: any) => key.i));
    PM.sel.chan = 'position.y';
    timeline.graph = true; timeline.graphCombined = true;
    timeline.graphFocus = layers[0] ? { layerId: layers[0].id, trackKey: 'position.y' } : null;
    timeline.graphShown = layers.map((layer: any) => layer.id + ':position.y');
    timeline.scrollT = timeline.scrollY = 0; timeline.pps = 50;
    PM.setTime(0); PM.bus.emit('sel'); PM.invalidate();
  });
  await page.waitForTimeout(1200);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await page.evaluate(async (seconds) => {
    const PM = (window as any).PM;
    const render = PM.GL.render, emit = PM.bus.emit;
    const frames: any[] = [], timeline: any[] = [], scrubs: any[] = [], liveScrubs: any[] = [], longTasks: any[] = [];
    let phase = 'playback', playbackStart = 0, playbackEnd = 0, playbackStarted = false;
    let seekPresented: { time: number; picture: boolean; timeline: boolean; finish: () => void } | null = null;
    PM.GL.render = (...args: any[]) => {
      const at = performance.now(), presented = render(...args);
      frames.push({ ...PM.GL.stats, phase, playing: PM.playing, time: args[0], at, ms: performance.now() - at, presented: presented !== false });
      if (presented !== false && seekPresented && Math.abs(args[0] - seekPresented.time) < 1e-6) {
        seekPresented.picture = true;
        if (seekPresented.timeline) seekPresented.finish();
      }
      return presented;
    };
    PM.bus.emit = function (event: string, ...args: any[]) {
      const at = performance.now(), value = emit.call(this, event, ...args);
      if (event === 'transport') {
        if (PM.playing) playbackStarted = true;
        else if (playbackStarted && phase === 'playback') { playbackEnd = performance.now(); phase = 'playbackIdle'; }
      }
      if (event === 'draw:timeline') {
        timeline.push({ phase, playing: PM.playing, time: PM.time, ms: performance.now() - at });
        if (seekPresented && Math.abs(PM.time - seekPresented.time) < 1e-6) {
          seekPresented.timeline = true;
          if (seekPresented.picture) seekPresented.finish();
        }
      }
      return value;
    };
    const observer = new PerformanceObserver(list => {
      for (const entry of list.getEntries()) longTasks.push({ phase, at: entry.startTime, ms: entry.duration });
    });
    observer.observe({ type: 'longtask' });
    let elapsed = 0;
    try {
      playbackStart = performance.now(); PM.play();
      await new Promise(resolve => setTimeout(resolve, seconds * 1000));
      PM.pause(); elapsed = (playbackEnd || performance.now()) - playbackStart;
      phase = 'liveScrub';
      const times = [0, 3 + 19 / 30, 4, 7, 12, 20, 30, 45, 60, 70];
      for (let repeat = 0; repeat < 2; repeat++) for (const time of times) {
        const at = performance.now();
        const firstFrame = frames.length, firstPaint = timeline.length;
        await new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => { seekPresented = null; reject(new Error('Live scrub did not present requested frame')); }, 10_000);
          seekPresented = { time, picture: false, timeline: false, finish: () => { clearTimeout(timer); seekPresented = null; resolve(); } };
          PM.setTime(time, { force: true });
        });
        liveScrubs.push({ time, repeat, totalMs: performance.now() - at,
          renderMs: frames.slice(firstFrame).reduce((sum, frame) => sum + frame.ms, 0),
          timelineMs: timeline.slice(firstPaint).reduce((sum, frame) => sum + frame.ms, 0) });
        await new Promise(resolve => setTimeout(resolve, 40));
      }
      phase = 'scrub';
      for (let repeat = 0; repeat < 2; repeat++) for (const time of times) {
        const at = performance.now(); PM.setTime(time);
        const setMs = performance.now() - at;
        await PM.prepareFrame(time);
        const preparedMs = performance.now() - at;
        PM.GL.render(time, { mblur: true, mbSamples: 12, shutter: PM.proj.shutter || .5 });
        PM.bus.emit('time', time); PM.bus.emit('draw:timeline');
        scrubs.push({ time, repeat, setMs, preparedMs, totalMs: performance.now() - at });
        await new Promise(resolve => setTimeout(resolve, 40));
      }
    } finally { PM.pause(); PM.GL.render = render; PM.bus.emit = emit; observer.disconnect(); }
    const costs = (values: number[]) => {
      values.sort((a, b) => a - b);
      return { count: values.length, median: values[Math.floor(values.length / 2)], p95: values[Math.floor(values.length * .95)], max: values.at(-1) };
    };
    const playback = frames.filter(frame => frame.playing && frame.presented);
    return { layers: PM.proj.layers.length, size: [PM.GL.canvas.width, PM.GL.canvas.height], elapsed, frames, timeline, scrubs, liveScrubs, longTasks,
      memory: PM.Memory.stats(), audio: { voices: PM.Audio.activeVoices?.(), playing: PM.playing },
      mediaErrors: Array.from(PM.assets.errors?.entries?.() ?? []),
      summary: { render: costs(playback.map(frame => frame.ms)), timeline: costs(timeline.filter(frame => frame.playing).map(frame => frame.ms)),
        scrub: costs(scrubs.map(frame => frame.totalMs)), liveScrub: costs(liveScrubs.map(frame => frame.totalMs)),
        liveScrubWork: costs(liveScrubs.map(frame => frame.renderMs + frame.timelineMs)),
        playbackWorkMs: frames.filter(frame => frame.playing).reduce((sum, frame) => sum + frame.ms, 0)
          + timeline.filter(frame => frame.playing).reduce((sum, frame) => sum + frame.ms, 0),
        fps: playback.length * 1000 / elapsed,
        gaps: costs(playback.slice(1).map((frame, index) => frame.at - playback[index].at)) } };
  }, Number(process.env.PM_HEAVY_SECONDS) || 15);
  const output = process.env.PM_HEAVY_OUTPUT || join(tmpdir(), `powermove-heavy-profile-${Date.now()}.json`);
  await writeFile(output, JSON.stringify(result));
  await writeFile(output + '.cpuprofile', JSON.stringify((await cdp.send('Profiler.stop')).profile));
  console.log('HEAVY_PROJECT', JSON.stringify({ layers: result.layers, size: result.size, ...result.summary, memory: result.memory }));
  expect(result.summary.render.count).toBeGreaterThan(20);
  if (process.env.PM_HEAVY_REQUIRE_MEDIA === '1') expect(result.mediaErrors).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
