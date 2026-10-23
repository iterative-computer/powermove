import path from 'node:path';
import { buildSync } from 'esbuild';
import { expect, test } from './helpers/app';

test('native surface presentation preserves F16 HDR, alpha and orientation in sRGB and P3 canvases', async ({ session }) => {
  await session.openEditor();
  const code = buildSync({ entryPoints: [path.join(__dirname, 'tools/frame-painter-probe.ts')], bundle: true, format: 'iife', globalName: 'FramePainterProbe', write: false }).outputFiles[0]!.text;
  const rows = await session.page.evaluate(async code => {
    return new Function(`${code}; return FramePainterProbe.run();`)();
  }, code);
  expect(rows).toHaveLength(14);
  for (const row of rows) expect(row.actual, JSON.stringify({ space: row.space, color: row.color })).toEqual(row.expected);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

/*
 * The same comparison on the viewer's own surface imports (the production
 * path: IOSurfaces the engine rendered, imported by main and sent through
 * Electron's sharedTexture bridge), for the display's pool and for the
 * 8-bit pool an SDR display gets. Every pixel the WebGL2 F16 painter
 * draws must equal the WebGPU external-texture painter's (after 8-bit
 * quantization for a bgra pool, whose bytes the screen shows). Viewer
 * frames are opaque wherever the composition draws and transparent black
 * around it (the placement writes alpha 1; compose.rs), so the WebGL
 * upload's premultiplication cannot change a production pixel: the
 * alpha classes are asserted too.
 */
const ENGINE_ON = { POWERMOVE_NEW_ENGINE: '1' };

async function realFrameRows(session: import('./helpers/app').LaunchedApp, sdr: boolean) {
  const { page } = session;
  if (sdr) {
    // An SDR display: `(dynamic-range: high)` does not match.
    await page.evaluate(() => {
      const original = window.matchMedia.bind(window);
      window.matchMedia = ((query: string) => /dynamic-range/.test(query)
        ? ({ matches: false, media: query, onchange: null, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, dispatchEvent: () => false })
        : original(query)) as typeof window.matchMedia;
    });
  }
  await session.openEditor();
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Painter', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
    // A translucent background and a half-opacity layer: alpha below 1
    // inside the composition before the viewer's placement.
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#20304080', position: 0 }] };
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 160, h: 90 } }, project);
    box.p['position.x'].v = 200; box.p['position.y'].v = 120; box.p.opacity.v = 50;
    project.layers = [box];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
  await page.waitForTimeout(500);
  const code = buildSync({ entryPoints: [path.join(__dirname, 'tools/real-frame-painter-probe.ts')], bundle: true, format: 'iife', globalName: 'RealFramePainterProbe', write: false }).outputFiles[0]!.text;
  return page.evaluate(async code => new Function(`${code}; return RealFramePainterProbe.run();`)(), code);
}

test.describe('Use New Engine: painters on the real surface imports', () => {
  test.use({ desktopLaunchOptions: { env: ENGINE_ON } });

  for (const sdr of [false, true]) {
    test(`WebGL2 and WebGPU painters draw the ${sdr ? 'SDR (bgra8)' : "display's"} pool identically`, async ({ session }) => {
      test.setTimeout(180_000);
      const report = await realFrameRows(session, sdr);
      console.log('REAL FRAME PAINTERS', JSON.stringify({ ...report, slots: report.slots.map((s) => ({ ...s })) }));
      if (sdr) expect(report.edr).toBe(false);
      expect(report.format).toBe(report.edr ? 'rgba16Float' : 'bgra8Unorm');
      const drawn = report.slots.filter((s) => s.colors >= 2);
      expect(drawn.length).toBeGreaterThan(0);
      for (const slot of report.slots) {
        expect(slot.differing, JSON.stringify(slot.firstDifference)).toBe(0);
        expect(slot.alpha.partial).toBe(0);
      }
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  }
});
