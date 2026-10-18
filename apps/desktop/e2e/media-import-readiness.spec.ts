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
      const readPixels = async (file: string) => page.evaluate(async bytes => {
        const image = await createImageBitmap(new Blob([Uint8Array.from(bytes)], { type: 'image/png' }));
        const canvas = new OffscreenCanvas(64, 64), g = canvas.getContext('2d')!;
        g.drawImage(image, 0, 0); image.close();
        return [16, 48].map(x => [...g.getImageData(x, 32, 1, 1).data]);
      }, [...await readFile(file)]);
      expect(await readPixels(output)).toEqual([[255, 0, 0, 255], [0, 255, 0, 255]]);
      if (flag === '1') {
        await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting)).toBe(true);
        const report = await app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
        expect(report.presenter.layerFailures, JSON.stringify(session.diagnostics.console)).toBe(0);
      }
      // Reopen, then remove the live binding to make the debounce window deterministic.
      // Export, rather than the earlier import, must establish this binding.
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.hydrateProject(JSON.parse(JSON.stringify(PM.proj))));
      });
      if (flag === '1') await app.evaluate(async () => {
        const store = (globalThis as any).__powermoveMediaStore;
        for (const owner of store.bindings.keys()) await store.release(owner);
      });
      expect(await page.evaluate(() => (window as any).PM.Export.run({
        format: 'still', w: 64, h: 64, mblur: false, audio: false
      }))).toEqual({ cancelled: false });
      expect(await app.evaluate(() => (globalThis as any).__readinessOfflineMessages)).toEqual([]);
      expect(await readPixels(output)).toEqual([[255, 0, 0, 255], [0, 255, 0, 255]]);
      if (flag === '1') {
        // A queued snapshot owns its media even once its window has switched
        // projects and released all live media bindings.
        const queuedOutput = path.join(session.userData, 'queued.png');
        await app.evaluate(({ dialog }, file) => {
          dialog.showSaveDialog = async () => ({ canceled: false, filePath: file });
        }, queuedOutput);
        expect(await page.evaluate(() => (window as any).PM.Export.addToQueue({
          format: 'still', fps: 30, scale: 1, range: 'all', quality: 'high', mblur: false, audio: false
        }))).toMatchObject({ status: 'added' });
        await page.evaluate(() => { const PM = (window as any).PM; PM.replaceProject(PM.mkProject({ name: 'Other project' })); });
        await app.evaluate(async () => {
          const store = (globalThis as any).__powermoveMediaStore;
          for (const owner of store.bindings.keys()) await store.release(owner);
          await (globalThis as any).__powermoveNativeEngine.renderQueue.start();
        });
        await expect.poll(() => app.evaluate(() =>
          (globalThis as any).__powermoveNativeEngine.renderQueue.state().jobs[0]?.state), { timeout: 30_000 }).toBe('done');
        expect(await readPixels(queuedOutput)).toEqual([[255, 0, 0, 255], [0, 255, 0, 255]]);
      }
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  });
}
