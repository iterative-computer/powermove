import { readFileSync, writeFileSync } from 'node:fs';
import { test, expect } from './helpers/app';

test.skip(!process.env.PM_PERF_PROJECT, 'Set PM_PERF_PROJECT to a copied project');
test('measure zooming the copied project', async ({ session }, testInfo) => {
  test.setTimeout(120000);
  await session.openEditor();
  const { page } = session;
  await page.setViewportSize({ width: 1440, height: 1000 });
  const data = JSON.parse(readFileSync(process.env.PM_PERF_PROJECT!, 'utf8'));
  await page.evaluate(data => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: data.proj || data }));
    PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  }, data);
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean((window as any).PM?.GL?.gl && viewer?.stage); });
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const metrics = [];
  for (const zoom of [0.5, 1, 2]) {
    const result = await page.evaluate(async ({ zoom, sourceClipping }) => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      viewer.fit = false; viewer.zoom = zoom; viewer.pan = [0, 0]; viewer.layout();
      const samples = [];
      for (let i = 0; i < 4; i++) {
        const start = performance.now(); PM.GL.render(0, { mblur: false, sourceClipping }); samples.push(performance.now() - start);
        await new Promise(resolve => setTimeout(resolve, 80));
      }
      PM.GL.render(0, { mblur: false, sourceClipping });
      return { zoom, samples, viewport: PM.GL.previewViewport, stats: PM.GL.stats, memory: PM.Memory.stats(), image: PM.GL.canvas.toDataURL() };
    }, { zoom, sourceClipping: process.env.PM_REFERENCE_SOURCES !== '1' });
    const { image, ...metric } = result;
    writeFileSync(testInfo.outputPath(`zoom-${zoom}.png`), Buffer.from(image.split(',')[1]!, 'base64'));
    metrics.push(metric);
    console.log('ZOOM', JSON.stringify(metric));
  }
  const gesture = await page.evaluate(async sourceClipping => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const samples = [];
    for (let i = 0; i < 12; i++) {
      const start = performance.now();
      viewer.fit = false; viewer.zoom = 1.1 + i * .1; viewer.pan = [i * 3, i * -2]; viewer.layout();
      PM.GL.render(0, { mblur: false, sourceClipping });
      samples.push(performance.now() - start);
      await new Promise(resolve => setTimeout(resolve, 80));
    }
    return { samples, memory: PM.Memory.stats() };
  }, process.env.PM_REFERENCE_SOURCES !== '1');
  console.log('GESTURE', JSON.stringify(gesture));
  writeFileSync(testInfo.outputPath('gesture.json'), JSON.stringify(gesture));
  if (process.env.PM_REFERENCE_SOURCES !== '1') {
    const highZoom = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const results = [];
      for (const zoom of [4, 8]) for (const center of [[960, 540], [200, 120]]) {
        viewer.zoom = zoom; viewer.pan = [(PM.proj.w / 2 - center[0]!) * zoom, (PM.proj.h / 2 - center[1]!) * zoom]; viewer.layout();
        const start = performance.now(); PM.GL.render(0, { mblur: false });
        results.push({ zoom, center, ms: performance.now() - start, rasterBytes: PM.rasterStats().bytes, draws: PM.GL.stats.draws });
        await new Promise(resolve => setTimeout(resolve, 80));
      }
      return results;
    });
    writeFileSync(testInfo.outputPath('high-zoom.json'), JSON.stringify(highZoom));
    console.log('HIGH_ZOOM', JSON.stringify(highZoom));
  }
  const profile = await cdp.send('Profiler.stop');
  writeFileSync(testInfo.outputPath('zoom.cpuprofile'), JSON.stringify(profile.profile));
  writeFileSync(testInfo.outputPath('metrics.json'), JSON.stringify(metrics));
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});
