import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';

test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a copied project');

test('profile cold layer entrances and scrubbing across a copied composition', async ({ session }, info) => {
  test.setTimeout(120_000);
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  await page.setViewportSize({ width: 1440, height: 1000 });
  const data = JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8'));
  await page.evaluate(data => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    PM.ProjectsScreen.hide(); PM.pause(); PM.setTime(0);
    PM.perf.auto = false; PM.quality = 1;
  }, data);
  await page.waitForTimeout(1500);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const frames: any[] = [], scrubs: any[] = [];
    let phase = 'playback', last = 0, uploads = 0, rendering = false, warmRasters = 0;
    const upload = PM.GL.gl.texImage2D.bind(PM.GL.gl), raster = PM.raster;
    PM.GL.gl.texImage2D = (...args: any[]) => { uploads++; return upload(...args); };
    PM.raster = (...args: any[]) => { if (!rendering) warmRasters++; return raster(...args); };
    const original = PM.GL.render;
    PM.GL.render = (...args: any[]) => {
      const start = performance.now();
      const beforeUploads = uploads;
      rendering = true;
      const value = original(...args);
      rendering = false;
      frames.push({ phase, time: args[0], ms: performance.now() - start, gap: last ? start - last : 0, uploads: uploads - beforeUploads, warmRasters, textureBytes: PM.GL.memoryStats().textures.bytes, ...PM.GL.stats });
      last = start; return value;
    };
    try {
      PM.play(); await new Promise(resolve => setTimeout(resolve, 8000)); PM.pause();
      phase = 'scrub';
      for (const time of [0, .1, 0, 4, 0, 8, 0, ...Array.from({ length: 45 }, (_, i) => i * PM.proj.dur / 45)]) {
        const start = performance.now(); PM.setTime(time);
        const setMs = performance.now() - start;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        scrubs.push({ time, setMs, elapsed: performance.now() - start });
        await new Promise(resolve => setTimeout(resolve, 25));
      }
    } finally { PM.pause(); PM.GL.render = original; PM.raster = raster; PM.GL.gl.texImage2D = upload; }
    return { frames, scrubs, memory: PM.Memory.stats(), layers: PM.proj.layers.length };
  });
  writeFileSync(info.outputPath('profile.cpuprofile'), JSON.stringify((await cdp.send('Profiler.stop')).profile));
  writeFileSync(info.outputPath('metrics.json'), JSON.stringify(result));
  for (const phase of ['playback', 'scrub']) {
    const frames = result.frames.filter(f => f.phase === phase), times = frames.map(f => f.ms).sort((a,b) => a-b);
    console.log('COMPOSITION', JSON.stringify({ phase, count: times.length, p95: times[Math.floor(times.length*.95)], worst: [...frames].sort((a,b) => b.ms-a.ms).slice(0,8) }));
  }
  await page.screenshot({ path: info.outputPath('editor.png') });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
