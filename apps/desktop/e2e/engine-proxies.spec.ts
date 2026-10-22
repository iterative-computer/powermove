/** Phase4a: real whole-clip generation, persistence and export isolation. */
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect, launchApp } from './helpers/app';

for (const enabled of [false, true]) {
  test(`proxy controls and original-only export with the engine ${enabled ? 'on' : 'off'}`, async () => {
    test.setTimeout(180_000);
    const root = await mkdtemp(path.join(os.tmpdir(), 'pm-proxy-e2e-'));
    let session: Awaited<ReturnType<typeof launchApp>> | null = null;
    try {
      session = await launchApp({ userData: path.join(root, 'profile'), env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      const { app, page } = session;
      await session.openEditor();
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.proj = PM.mkProject({ name: 'Proxy gate', w: 320, h: 180, fps: 24, dur: 1 });
        PM.proj.work = [0, 1]; PM.touch(); PM.bus.emit('project');
        const input = document.createElement('input'); input.type = 'file'; input.id = 'proxy-source'; document.body.appendChild(input);
      });
      await page.locator('#proxy-source').setInputFiles(path.resolve(__dirname, 'fixtures/prores422.mov'));
      const asset = await page.evaluate(async () => {
        const PM = (window as any).PM;
        await PM.importFiles([(document.querySelector('#proxy-source') as HTMLInputElement).files![0]]);
        const layer = PM.proj.layers.find((l: any) => l.type === 'video');
        layer.from = 0; layer.dur = 1; PM.touch(); PM.bus.emit('layers');
        PM.pause(); PM.setTime(.25, { raw: true, force: true });
        return String(layer.d.asset);
      });
      expect(await page.evaluate(() => !!(window as any).PM.Proxies)).toBe(enabled);
      const files = [path.join(root, 'empty.png'), path.join(root, 'proxies.png')];
      await app.evaluate(({ dialog }, files) => {
        let n = 0; dialog.showSaveDialog = (async () => ({ canceled: false, filePath: files[n++]! })) as typeof dialog.showSaveDialog;
      }, files);
      const exportFrame = () => page.evaluate(() => (window as any).PM.Export.run({ format: 'still', scale: 1, alpha: false, mblur: false, audio: false }));
      expect(await exportFrame()).toEqual({ cancelled: false });
      if (enabled) {
        await page.evaluate(async ({ asset, location }) => {
          const PM = (window as any).PM;
          PM.Proxies.setIntent({ create: 'onlyWhenAsked', size: 'half' });
          await PM.Proxies.flush();
          await PM.Proxies.setLocation({ kind: 'folder', path: location });
          await PM.Proxies.create(asset);
        }, { asset, location: path.join(root, 'generated') });
        await page.waitForFunction((asset) => ['ready', 'failed'].includes((window as any).PM.Proxies.assetState(asset).state), asset, { timeout: 90_000 });
        const ready = await page.evaluate((asset) => (window as any).PM.Proxies.assetState(asset), asset);
        expect(ready, JSON.stringify(ready)).toMatchObject({ state: 'ready' });
        const stored = await page.evaluate(() => (window as any).PM.EngineProxies.snapshot((window as any).PM.proj.id));
        expect(stored.assets[asset]).toMatchObject({ state: 'ready' });
        await page.evaluate((asset) => {
          const PM = (window as any).PM;
          PM.Proxies.setAlwaysPlayOriginal(asset, true);
          PM.setTime(.25, { raw: true, force: true });
        }, asset);
        expect(await page.evaluate((asset) => (window as any).PM.proj.assets[asset].alwaysPlayOriginal, asset)).toBe(true);
        await page.evaluate((asset) => {
          const PM = (window as any).PM;
          PM.Proxies.setAlwaysPlayOriginal(asset, false);
          PM.Kernel.services.get('viewer').setMediaPreference('proxies');
          PM.invalidate();
        }, asset);
        await page.waitForFunction(() => (window as any).PM.EngineConnection.session()?.lastFrame?.quality?.representation === 'proxy', undefined, { timeout: 30_000 });
      }
      expect(await exportFrame()).toEqual({ cancelled: false });
      expect(await readFile(files[1]!)).toEqual(await readFile(files[0]!));
      if (enabled) {
        const cached = await page.evaluate(async () => {
          const PM = (window as any).PM;
          PM.proj.work = [0, 3 / 24]; PM.touch();
          await PM.Preview.cache(); PM.pause();
          return { count: PM.Preview.count, bytes: PM.Preview.bytes };
        });
        expect(cached.count).toBe(3);
        expect(cached.bytes).toBeGreaterThan(0);
        await page.evaluate(async (asset) => {
          const PM = (window as any).PM;
          PM.Preview.clear(); PM.Kernel.services.get('viewer').setMediaPreference('automatic');
          await PM.Proxies.remove(asset);
        }, asset);
        expect(await page.evaluate((asset) => (window as any).PM.Proxies.assetState(asset), asset)).toEqual({ state: 'none' });
      }
    } finally { try { await session?.close(); } finally { await rm(root, { recursive: true, force: true }); } }
  });
}
