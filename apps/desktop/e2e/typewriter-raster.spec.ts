import { cp, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

test('character Typewriter loads, renders partial text and cursor, and disposes cleanly', async ({ session }, testInfo) => {
  await cp(path.join(__dirname, 'fixtures/typewriter-effect'), path.join(session.userData, 'extensions/typewriter-effect'), { recursive: true });
  await session.relaunch();
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => (window as any).PM.Kernel.loader.activeIds().includes('typewriter-effect'));
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Typewriter checks', w: 400, h: 300, dur: 3, fps: 30 }));
    const layer = PM.mkLayer('text', { d: { text: 'Hello', font: 'Arial', size: 64, color: '#ffffff' }, p: { 'position.x': 100, 'position.y': 120 } });
    PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.touch();
    const before = PM.raster(layer, 1, 0).cv.toDataURL();
    layer.fx = [PM.mkEffect('typewriter')];
    const fx = layer.fx[0];
    const contact = document.createElement('canvas'); contact.width = 800; contact.height = 600;
    const contactContext = contact.getContext('2d')!;
    let index = 0;
    const surface = (progress: number, cursor = false) => {
      fx.p.progress.v = progress; fx.p.cursor.v = cursor; PM.touch();
      const raster = PM.raster(layer, 1, 0);
      const pixels = raster.cv.getContext('2d').getImageData(0, 0, raster.cv.width, raster.cv.height).data;
      let ink = 0;
      for (let i = 3; i < pixels.length; i += 4) if (pixels[i]) ink++;
      const frame = PM.GL.renderToPixels(0, 400, 300, { mblur: false });
      contactContext.drawImage(PM.renderFrameTo(0, 400, 300, { mblur: false }), (index % 2) * 400, Math.floor(index / 2) * 300);
      index++;
      return { ink, frame: Array.from(frame) };
    };
    const empty = surface(0), partial = surface(40), full = surface(100), cursor = surface(0, true);
    await PM.Kernel.loader.deactivate('typewriter-effect');
    layer.fx = []; PM.touch();
    return {
      ink: [empty.ink, partial.ink, full.ink, cursor.ink],
      framesDiffer: JSON.stringify(empty.frame) !== JSON.stringify(partial.frame)
        && JSON.stringify(partial.frame) !== JSON.stringify(full.frame)
        && JSON.stringify(empty.frame) !== JSON.stringify(cursor.frame),
      restored: PM.raster(layer, 1, 0).cv.toDataURL() === before,
      contact: contact.toDataURL()
    };
  });
  expect(result.ink[0]).toBe(0);
  expect(result.ink[1]).toBeGreaterThan(0);
  expect(result.ink[2]).toBeGreaterThan(result.ink[1]!);
  expect(result.ink[3]).toBeGreaterThan(0);
  expect(result.framesDiffer).toBe(true);
  expect(result.restored).toBe(true);
  const contactPath = testInfo.outputPath('typewriter-frames.png');
  await writeFile(contactPath, Buffer.from(result.contact.split(',')[1]!, 'base64'));
  await testInfo.attach('typewriter-frames', { path: contactPath, contentType: 'image/png' });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
