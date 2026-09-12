import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';

test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a copied project');
test('profile sustained 800% pan at Retina density', async ({ session }, testInfo) => {
  test.setTimeout(120000);
  const { page } = session;
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1728, height: 1117, deviceScaleFactor: 2, mobile: false });
  const data = JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8'));
  await page.evaluate(data => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  }, data);
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.Viewer?.stage));
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM, V = PM.Viewer, GL = PM.GL;
    const results = [];
    let typeCosts: any = {}, raster = PM.raster;
    PM.raster = function (layer: any, ...args: any[]) {
      const start = performance.now(), result = raster(layer, ...args);
      const cost = typeCosts[layer.type] ||= { calls: 0, ms: 0, bytes: 0 };
      cost.calls++; cost.ms += performance.now() - start;
      if (result.cv) cost.bytes += result.cv.width * result.cv.height * 4;
      return result;
    };
    try {
      for (const center of [[960, 540], [200, 120]]) {
        const samples = []; typeCosts = {};
        for (let i = 0; i < 60; i++) {
          await new Promise(requestAnimationFrame);
          const start = performance.now();
          V.fit = false; V.zoom = 8;
          V.pan = [(PM.proj.w / 2 - center[0]!) * 8 + i * 7, (PM.proj.h / 2 - center[1]!) * 8 - i * 3];
          V.layout();
          const layout = performance.now();
          GL.render(0, { mblur: false });
          const submitted = performance.now();
          GL.gl.finish();
          const completed = performance.now();
          await new Promise(requestAnimationFrame);
          samples.push({ layout: layout - start, submit: submitted - layout, gpu: completed - submitted, total: completed - start, frame: performance.now() - start, width: GL.canvas.width, height: GL.canvas.height, draws: GL.stats.draws });
        }
        results.push({ center, samples, typeCosts, raster: PM.rasterStats(), memory: PM.Memory.stats() });
      }
    } finally { PM.raster = raster; }
    return { dpr: devicePixelRatio, results };
  });
  // Exercise the actual wheel handler and normal scheduled renderer as well
  // as the controlled submissions above. No direct GL.render calls here.
  const wheel = await page.evaluate(async () => {
    const PM = (window as any).PM, V = PM.Viewer, original = PM.GL.render;
    V.zoom = 8; V.pan = [0, 0]; V.layout();
    PM.agentFrameCapture = false;
    const frames: number[] = [], costs: number[] = []; let previous = performance.now();
    PM.GL.render = function (...args: any[]) {
      const start = performance.now(); const value = original(...args); costs.push(performance.now() - start); return value;
    };
    try {
      for (let i = 0; i < 60; i++) {
        await new Promise(requestAnimationFrame);
        const now = performance.now(); frames.push(now - previous); previous = now;
        const rect = V.stage.getBoundingClientRect();
        V.stage.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaX: 7, deltaY: 3, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
      }
      await new Promise(requestAnimationFrame);
      return { frames, costs, pan: [...V.pan], zoom: V.zoom };
    } finally { PM.GL.render = original; PM.agentFrameCapture = true; }
  });
  writeFileSync(testInfo.outputPath('wheel.json'), JSON.stringify(wheel));
  console.log('WHEEL', JSON.stringify(wheel));
  expect(wheel.zoom).toBe(8); expect(Math.abs(wheel.pan[0])).toBeGreaterThan(100); expect(wheel.costs.length).toBeGreaterThan(0); expect(wheel.costs.length).toBeLessThan(15);
  const pinch = await page.evaluate(async () => {
    const PM = (window as any).PM, V = PM.Viewer;
    V.zoom = 8; V.pan = [(PM.proj.w / 2 - 230) * 8, (PM.proj.h / 2 - 100) * 8]; V.layout();
    PM.agentFrameCapture = false;
    const intervals: number[] = [], resizeEvents: any[] = []; let previous = performance.now();
    const originalResize = PM.GL.resize;
    PM.GL.resize = function (w: number, h: number, ...args: any[]) {
      const old = [PM.GL.canvas.width, PM.GL.canvas.height], start = performance.now();
      const value = originalResize(w, h, ...args);
      resizeEvents.push({ old, next: [w, h], ms: performance.now() - start, frame: intervals.length }); return value;
    };
    try {
      for (let i = 0; i < 72; i++) {
        await new Promise(requestAnimationFrame);
        const now = performance.now(); intervals.push(now - previous); previous = now;
        const rect = V.stage.getBoundingClientRect();
        V.stage.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: Math.floor(i / 24) % 2 ? -5 : 5, clientX: rect.left + rect.width / 2, clientY: rect.top + rect.height / 2 }));
      }
      await new Promise(requestAnimationFrame);
      await new Promise(resolve => setTimeout(resolve, 120));
      return { intervals, zoom: V.zoom, resizeEvents };
    } finally { PM.GL.resize = originalResize; PM.agentFrameCapture = true; }
  });
  writeFileSync(testInfo.outputPath('pinch.json'), JSON.stringify(pinch));
  const sortedPinch = [...pinch.intervals].sort((a,b) => a-b);
  console.log('PINCH', JSON.stringify({ median: sortedPinch[36], p95: sortedPinch[68], max: sortedPinch[71], zoom: pinch.zoom }));
  expect(pinch.zoom).toBeLessThan(8);
  const profile = await cdp.send('Profiler.stop');
  writeFileSync(testInfo.outputPath('high-zoom.cpuprofile'), JSON.stringify(profile.profile));
  writeFileSync(testInfo.outputPath('high-zoom.json'), JSON.stringify(result));
  for (const r of result.results) {
    const percentile = (key: string, p: number) => r.samples.map((s: any) => s[key]).sort((a: number,b: number) => a-b)[Math.floor((r.samples.length-1)*p)];
    console.log('HIGH_ZOOM', JSON.stringify({ center:r.center, typeCosts:r.typeCosts, median:percentile('total',.5), p95:percentile('total',.95), frame95:percentile('frame',.95), layout95:percentile('layout',.95), submit95:percentile('submit',.95), gpu95:percentile('gpu',.95), raster:r.raster }));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});
