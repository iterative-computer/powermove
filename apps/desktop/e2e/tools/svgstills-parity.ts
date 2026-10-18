/* Composition verification of private decoder fixtures. Captures are made
   through the same file picker and exporters as users. Run under the
   Electron lock after svgstills-capture.ts has generated its manifest. */
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { test, expect, launchApp } from '../helpers/app';

test('measured still classes match the legacy composition before gates retire', async () => {
  test.setTimeout(600_000);
  const root = process.env.PM_SVGSTILLS_EVIDENCE;
  expect(root).toBeTruthy();
  const names: string[] = process.env.PM_SVGSTILLS_PARITY_CASES
    ? JSON.parse(await readFile(process.env.PM_SVGSTILLS_PARITY_CASES, 'utf8'))
    : ['bmp24.bmp', 'bmp32.bmp', 'icon.ico', 'gif-still.gif', 'jpeg-gray.jpg',
    'jpeg-progressive.jpg', 'progressive-gray.jpg', 'gray-1.png', 'gray-2.png', 'gray-4.png', 'gamma-only.png', 'tiff8-palette.tiff', 'avif-10bit.avif', 'avif-10bit-color.avif', 'palette-1.png', 'palette-2.png', 'palette-4.png', 'gamma-ramp.png', 'tiff32f-rgb.tiff', 'late-exif.png'];
  const manifest: Array<{ name: string; file: string }> = JSON.parse(await readFile(path.join(root!, 'manifest.json'), 'utf8'));
  const out = path.join(root!, 'composition'); await mkdir(out, { recursive: true });
  const frames: number[][][] = [];
  for (const native of [false, true]) {
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' } });
    const capture: number[][] = [];
    try {
      await session.openEditor();
      for (const name of names) {
        await session.page.evaluate(() => {
          const PM = (window as any).PM;
          PM.replaceProject(PM.mkProject({ w: 160, h: 120, fps: 30, dur: 1, bg: '#526e83', name: 'Decoder composition' }));
          PM.ProjectsScreen.hide();
        });
        const chooser = session.page.waitForEvent('filechooser');
        await session.page.evaluate(() => (window as any).PM.pickFiles());
        await (await chooser).setFiles(manifest.find(row => row.name === name)?.file || path.join(root!, 'sources', name));
        await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'image'));
        // Exercise filtering and transformed edges, rather than only an
        // unscaled decoder upload. The dimensions differ from every source.
        await session.page.evaluate(() => {
          const PM = (window as any).PM;
          const layer = PM.proj.layers.find((l: any) => l.type === 'image');
          PM.Edit.apply([{ type: 'set_property', target: layer.id, path: 'rotation', value: 13, time: 0, mode: 'static' }]);
        });
        if (native) await expect.poll(() => session.page.evaluate(() => {
          const s = (window as any).PM.EngineConnection.status(); return { route: s.route, reason: s.routeReason };
        })).toEqual({ route: 'engine', reason: null });
        await session.page.evaluate(() => {
          const w = window as any; w.__svgstillsJobs = [];
          w.__svgstillsOff = w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => w.__svgstillsJobs.push(s));
        });
        const file = path.join(out, `${native ? 'native' : 'legacy'}-${name}.png`);
        await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
        expect(await session.page.evaluate(async () => (window as any).PM.Export.run({ format: 'still', w: 160, h: 120, alpha: false, mblur: false }))).toEqual({ cancelled: false });
        const jobs = await session.page.evaluate(() => {
          const w = window as any; w.__svgstillsOff(); return w.__svgstillsJobs;
        });
        await writeFile(path.join(out, `${native ? 'native' : 'legacy'}-${name}.jobs.json`), JSON.stringify(jobs, null, 2));
        if (native) {
          expect(jobs.some((j: any) => j.state === 'done' && !j.routed), `native export must finish in engine: ${name}`).toBe(true);
          expect(jobs.some((j: any) => j.routed), `native export must not route: ${name}`).toBe(false);
        }
        capture.push(await session.page.evaluate(async bytes => {
          const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
          const c = new OffscreenCanvas(160, 120), ctx = c.getContext('2d')!; ctx.drawImage(image, 0, 0); image.close();
          return [...ctx.getImageData(0, 0, 160, 120).data];
        }, [...await readFile(file)]));
      }
    } finally { await session.close(); }
    frames.push(capture);
  }
  const metrics = names.map((name, i) => {
    const a = frames[0]![i]!, b = frames[1]![i]!;
    let max = 0, over1 = 0;
    for (let p = 0; p < a.length; p += 4) {
      let d = 0; for (let c = 0; c < 4; c++) d = Math.max(d, Math.abs(a[p + c]! - b[p + c]!));
      max = Math.max(max, d); if (d > 1) over1++;
    }
    return { name, max, over1, pixels: a.length / 4 };
  });
  await writeFile(path.join(out, 'metrics.json'), JSON.stringify(metrics, null, 2));
  for (const m of metrics) {
    expect(m.max, JSON.stringify(m)).toBeLessThanOrEqual(4);
    expect(m.over1 / m.pixels, JSON.stringify(m)).toBeLessThanOrEqual(0.001);
  }
});
