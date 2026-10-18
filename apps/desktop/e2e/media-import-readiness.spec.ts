import path from 'node:path';
import { readFile } from 'node:fs/promises';

import { expect, test } from './helpers/app';

// Generated Files exercise streaming and the bounded parallel importer.
for (const flag of ['0', '1']) {
  test.describe(`media import readiness, engine ${flag}`, () => {
    test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: flag } } });

    test('every image in a multi-import is readable immediately and exports', async ({ session }) => {
      test.setTimeout(60_000);
      await session.openEditor();
      const { page, app } = session;
      const assets = await page.evaluate(async () => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.mkProject({ name: 'Import readiness', w: 64, h: 64, fps: 30, dur: 0.1 }));
        const files: File[] = [];
        for (const [name, color] of [['a', '#ff0000'], ['b', '#00ff00']]) {
          const canvas = new OffscreenCanvas(32, 64);
          const g = canvas.getContext('2d')!;
          g.fillStyle = color!; g.fillRect(0, 0, 32, 64);
          files.push(new File([await canvas.convertToBlob()], `${name}.png`, { type: 'image/png' }));
        }
        await PM.importFiles(files, { placement: null, sequence: false });
        return Object.values<any>(PM.proj.assets).map(a => ({ id: a.id, hash: a.contentHash, name: a.name }))
          .sort((a, b) => a.name.localeCompare(b.name));
      });
      expect(assets.map(a => a.name).sort()).toEqual(['a.png', 'b.png']);
      if (flag === '1') {
        // No polling or binding flush: returning from import is the barrier.
        const hashes = await app.evaluate(async (_electron, ids) => {
          const store = (globalThis as any).__powermoveMediaStore;
          return Promise.all(ids.map(async id => {
            const resolved = await store.resolveAsset(id);
            return resolved ? Buffer.from(resolved.contentHash).toString('hex') : null;
          }));
        }, assets.map(a => a.id));
        expect(hashes).toEqual(assets.map(a => a.hash));
      }
      const output = path.join(session.userData, 'import.png');
      await app.evaluate(({ dialog }, output) => {
        (globalThis as any).__readinessOfflineMessages = [];
        dialog.showMessageBox = async (options: any) => {
          (globalThis as any).__readinessOfflineMessages.push(options.message);
          return { response: 1, checkboxChecked: false };
        };
        dialog.showSaveDialog = async () => ({ canceled: false, filePath: output });
      }, output);
      const exported = await page.evaluate(async assets => {
        const PM = (window as any).PM;
        PM.Edit.apply(assets.map((a, i) => ({ type: 'add_layer', layerType: 'image', duration: 0.1,
          content: { asset: a.id, w: 32, h: 64 }, properties: { 'position.x': 16 + i * 32, 'position.y': 32 } })), { label: 'Imported images' });
        return PM.Export.run({ format: 'still', w: 64, h: 64, mblur: false, audio: false });
      }, assets);
      expect(exported).toEqual({ cancelled: false });
      expect(await app.evaluate(() => (globalThis as any).__readinessOfflineMessages)).toEqual([]);
      const pixels = await page.evaluate(async bytes => {
        const image = await createImageBitmap(new Blob([Uint8Array.from(bytes)], { type: 'image/png' }));
        const canvas = new OffscreenCanvas(64, 64), g = canvas.getContext('2d')!;
        g.drawImage(image, 0, 0); image.close();
        return [16, 48].map(x => [...g.getImageData(x, 32, 1, 1).data]);
      }, [...await readFile(output)]);
      expect(pixels).toEqual([[255, 0, 0, 255], [0, 255, 0, 255]]);
      if (flag === '1') {
        await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting)).toBe(true);
        const report = await app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
        expect(report.presenter.layerFailures).toBe(0);
      }
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  });
}
