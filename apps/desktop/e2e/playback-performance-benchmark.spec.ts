import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a copied project');
test('measure sustained playback and paused FPS at high zoom', async ({ session }, testInfo) => {
  test.setTimeout(120000);
  const { page } = session;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1728, height: 1117, deviceScaleFactor: 2, mobile: false });
  const data = JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8'));
  await page.evaluate(data => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    PM.ProjectsScreen.hide();
  }, data);
  await page.waitForFunction(() => { const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'); return Boolean(PM?.GL?.gl && viewer?.stage); });
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const results = await page.evaluate(async () => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'), original = PM.GL.render, results = [];
    let frames: any[] = [], last = 0;
    PM.GL.render = function (...args: any[]) {
      const now = performance.now(); const value = original(...args);
      frames.push({ time: args[0], interval: last ? now - last : 0, ms: performance.now() - now }); last = now; return value;
    };
    try {
      for (const view of [{ zoom: 1, center: [960, 540] }, { zoom: 8, center: [960, 540] }, { zoom: 8, center: [200, 120] }]) {
        PM.pause(); PM.setTime(0); PM.perf.auto = false; PM.quality = 1;
        viewer.fit = false; viewer.zoom = view.zoom; viewer.pan = [(PM.proj.w / 2 - view.center[0]!) * view.zoom, (PM.proj.h / 2 - view.center[1]!) * view.zoom]; viewer.layout();
        await new Promise(resolve => setTimeout(resolve, 1200));
        const pausedFps = PM.perf.fps;
        frames = []; last = 0;
        const start = performance.now(); PM.play();
        await new Promise(resolve => setTimeout(resolve, 4000));
        const duration = performance.now() - start, playingFps = PM.perf.fps; PM.pause();
        results.push({ ...view, duration, frames, actualFps: frames.length * 1000 / duration, pausedFps, playingFps, quality: PM.quality, memory: PM.Memory.stats() });
      }
    } finally { PM.pause(); PM.GL.render = original; }
    return results;
  });
  const profile = await cdp.send('Profiler.stop');
  writeFileSync(testInfo.outputPath('playback.cpuprofile'), JSON.stringify(profile.profile));
  writeFileSync(testInfo.outputPath('playback.json'), JSON.stringify(results));
  for (const r of results) {
    const intervals = r.frames.slice(1).map((f: any) => f.interval).sort((a: number,b: number) => a-b);
    console.log('PLAYBACK', JSON.stringify({ zoom:r.zoom, center:r.center, actualFps:r.actualFps, playingFps:r.playingFps, pausedFps:r.pausedFps, frame95:intervals[Math.floor(intervals.length*.95)], quality:r.quality }));
  }
  expect(results.every(result => result.pausedFps === 0)).toBe(true);
  await expect(page.locator('#status')).toContainText('Playback paused');
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});
