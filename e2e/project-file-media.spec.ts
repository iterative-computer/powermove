import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('saved project files restore imported video without the original session media', async ({ session }) => {
  await importFixture(session.page, 'h264-aac.mp4');
  await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.name === 'h264-aac.mp4'));
  await session.app.evaluate(({ ipcMain }) => {
    ipcMain.removeHandler('file:save');
    ipcMain.handle('file:save', (_event, request) => {
      (globalThis as any).__savedProjectFile = Buffer.from(request.data).toString('utf8');
      return { ok: true, path: '/tmp/test.pmv' };
    });
  });
  expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  const saved = await session.app.evaluate(() => (globalThis as any).__savedProjectFile as string);
  const document = JSON.parse(saved);
  expect(Object.keys(document.media || {})).toHaveLength(1);
  await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    for (const asset of Object.values(PM.proj.assets)) await PM.MediaStore.remove(asset);
  });
  await session.relaunch();
  const chooser = session.page.waitForEvent('filechooser');
  await session.page.evaluate(() => (window as any).PM.openProject());
  await (await chooser).setFiles({ name: 'restored.pmv', mimeType: 'application/json', buffer: Buffer.from(saved) });
  await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'h264-aac.mp4' && a.el?.readyState >= 2));
  const restored = await session.page.evaluate(async () => {
    const PM = (window as any).PM, asset = Object.values(PM.proj.assets)[0];
    const blob = await PM.MediaStore.get(asset);
    const layer = PM.proj.layers.find((item: any) => item.name === 'h264-aac.mp4');
    PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0 } });
    PM.setTime(1.5, { raw: true, force: true });
    const live = PM.assets.get(layer.d.asset);
    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('Restored video did not seek')), 8000);
      const poll = () => {
        if (Math.abs(live.el.currentTime - 1.5) < .05 && live.el.readyState >= 2) { clearTimeout(timeout); resolve(); }
        else setTimeout(poll, 50);
      }; poll();
    });
    const frame = PM.renderFrameTo(1.5, PM.proj.w, PM.proj.h);
    return { bytes: blob?.size, pixel: [...frame.getContext('2d').getImageData(frame.width / 2, frame.height / 2, 1, 1).data] };
  });
  expect(restored.bytes).toBeGreaterThan(0);
  expect(restored.pixel[1]).toBeGreaterThan(225);
  expect(restored.pixel[0]).toBeLessThan(30);
});
