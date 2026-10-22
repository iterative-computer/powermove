/** Phase4a: real whole-clip generation, persistence and export isolation. */
import { appendFile, copyFile, mkdtemp, readFile, readdir, rm, unlink, writeFile } from 'node:fs/promises';
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
      const projectId = await page.evaluate(() => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Proxy gate', w: 320, h: 180, fps: 24, dur: 1 });
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
        PM.ProjectsScreen.hide();
        return project.id as string;
      });
      await page.waitForFunction((id) => (window as any).PM.proj?.id === id, projectId);
      if (enabled) await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
      await page.evaluate(() => {
        const PM = (window as any).PM;
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
      const proxyDiagnostics = async () => {
        const state = await page.evaluate(async (asset) => {
          const PM = (window as any).PM;
          const media = PM.proj.assets[asset];
          return {
            assetState: PM.Proxies.assetState(asset),
            snapshot: await PM.EngineProxies.snapshot(PM.proj.id),
            playing: PM.playing,
            eligible: PM.Proxies.eligible(asset),
            asset: media,
            projectProxyIntent: PM.proj.media?.proxies ?? null,
            mediaRecord: PM.HostMedia?.store?.recordFor?.(asset) ?? null,
            mediaRecordByKey: media?.storageKey ? PM.HostMedia?.store?.recordFor?.(media.storageKey) ?? null : null,
            lastFrameQuality: PM.EngineConnection.session()?.lastFrame?.quality ?? null,
            engineConnection: PM.EngineConnection.status(),
            engineFailures: PM.EngineDoc?.failures?.() ?? [],
            engine: await (window as any).__powermoveTestBridge?.nativeEngine?.status?.(),
            toasts: [...document.querySelectorAll('[class*="toast"]')].map((element) => element.textContent?.trim()).filter(Boolean),
          };
        }, asset).catch((diagnosticError) => ({ diagnosticError: String(diagnosticError) }));
        const generated = await readdir(path.join(root, 'generated'), { recursive: true }).catch((diagnosticError) => [`readdir failed: ${String(diagnosticError)}`]);
        const locations = await readFile(path.join(root, 'profile', 'Engine Cache', 'Proxy Locations.json'), 'utf8').catch((diagnosticError) => `read failed: ${String(diagnosticError)}`);
        const records = await Promise.all(generated.filter((file) => file.endsWith('.json')).map((file) => readFile(path.join(root, 'generated', file), 'utf8').catch((diagnosticError) => `read failed: ${String(diagnosticError)}`)));
        return JSON.stringify({ state, generated, locations, records, console: session.diagnostics.console.slice(-20), pageErrors: session.diagnostics.pageErrors });
      };
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
        try {
          await page.waitForFunction((asset) => ['ready', 'failed'].includes((window as any).PM.Proxies.assetState(asset).state), asset, { timeout: 90_000 });
        } catch (error) {
          throw new Error(`Proxy did not reach a terminal state: ${await proxyDiagnostics()}`, { cause: error });
        }
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
        try {
          await page.waitForFunction(() => (window as any).PM.EngineConnection.session()?.lastFrame?.quality?.representation === 'proxy', undefined, { timeout: 30_000 });
        } catch (error) {
          throw new Error(`Ready proxy was not shown in the viewer: ${await proxyDiagnostics()}`, { cause: error });
        }
      }
      expect(await exportFrame()).toEqual({ cancelled: false });
      expect(await readFile(files[1]!)).toEqual(await readFile(files[0]!));
      if (enabled) {
        const cached = await page.evaluate(async () => {
          const PM = (window as any).PM;
          PM.proj.work = [0, 3 / 24]; PM.touch();
          await PM.Preview.cache(); PM.pause();
          return { count: PM.Preview.count, bytes: PM.Preview.bytes,
            budget: PM.Memory?.budget?.('preview'), active: PM.Preview.active,
            toasts: [...document.querySelectorAll('[class*="toast"]')].map((node) => node.textContent?.trim()).filter(Boolean),
            lastFrame: PM.EngineConnection.session()?.lastFrame?.quality ?? null };
        });
        expect(cached.count, JSON.stringify(cached)).toBe(3);
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

test('a linked original can go offline, relink, and relaunch without losing its proxy', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-proxy-relink-'));
  let session: Awaited<ReturnType<typeof launchApp>> | null = null;
  try {
    const original = path.join(root, 'original.mov');
    const moved = path.join(root, 'renamed.mov');
    const projectFile = path.join(root, 'Linked proxy.pmv');
    await copyFile(path.resolve(__dirname, 'fixtures/prores422.mov'), original);
    // Make the linked file unique so Spotlight cannot relink to the fixture.
    await appendFile(original, Buffer.from([0]));
    const bytes = await readFile(original);
    session = await launchApp({ userData: path.join(root, 'profile'), env: { POWERMOVE_NEW_ENGINE: '1' } });
    await session.openEditor();
    let { app, page } = session;
    await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
    const importMode = page.locator('select[aria-label="When importing"]');
    await importMode.waitFor({ state: 'attached' });
    await importMode.evaluate((element: HTMLSelectElement) => {
      element.value = 'link'; element.dispatchEvent(new Event('change'));
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.HostMedia.projectStorage())).toBe('link');
    await page.evaluate(() => (window as any).PM.SettingsUI.close());
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(original);
    await page.waitForFunction(() => Object.values((window as any).PM.proj.assets).some((media: any) => media.name === 'original.mov' && (window as any).PM.assets.get(media.id)));
    const { asset } = await page.evaluate(() => {
      const PM = (window as any).PM;
      const media = Object.values(PM.proj.assets).find((entry: any) => entry.name === 'original.mov') as any;
      const layer = PM.proj.layers.find((entry: any) => entry.type === 'video');
      layer.from = 0; layer.dur = 1;
      PM.proj.work = [0, 1]; PM.touch(); PM.bus.emit('layers');
      PM.pause(); PM.setTime(.25, { raw: true, force: true });
      return { asset: media.id as string };
    });
    await page.evaluate(() => (window as any).PM.HostMedia.flushBindings());
    await page.evaluate(async (asset) => {
      const PM = (window as any).PM;
      PM.Proxies.setIntent({ create: 'onlyWhenAsked', size: 'half' });
      await PM.Proxies.flush();
      await PM.Proxies.create(asset);
    }, asset);
    try {
      await page.waitForFunction((asset) => ['ready', 'failed'].includes((window as any).PM.Proxies.assetState(asset).state), asset, { timeout: 90_000 });
    } catch (error) {
      const state = await page.evaluate(async (asset) => {
        const PM = (window as any).PM;
        return { asset: PM.Proxies.assetState(asset), snapshot: await PM.EngineProxies.snapshot(PM.proj.id),
          notices: [...document.querySelectorAll('[class*="toast"]')].map((node) => node.textContent?.trim()).filter(Boolean) };
      }, asset).catch((diagnosticError) => ({ diagnosticError: String(diagnosticError) }));
      throw new Error(`Linked proxy did not finish generation: ${JSON.stringify(state)}`, { cause: error });
    }
    expect(await page.evaluate((asset) => (window as any).PM.Proxies.assetState(asset).state, asset)).toBe('ready');
    await app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, projectFile);
    expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);

    await unlink(original);
    const offline = await page.evaluate(async (asset) => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const restored = await PM.assets.restoreProject(PM.proj);
      return { missing: restored.missing.map((item: any) => item.id), live: !!PM.assets.get(asset) };
    }, asset);
    expect(offline).toEqual({ missing: [asset], live: false });
    await expect(page.locator(`[data-asset-id="${asset}"]`)).toContainText('offline');
    expect(await page.evaluate((asset) => (window as any).PM.Proxies.assetState(asset).state, asset)).toBe('ready');
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.Kernel.services.get('viewer').setMediaPreference('proxies');
      PM.setTime(.25, { raw: true, force: true });
      PM.invalidate();
    });
    await page.waitForFunction(() => (window as any).PM.EngineConnection.session()?.lastFrame?.quality?.representation === 'proxy', undefined, { timeout: 30_000 });

    await writeFile(moved, bytes);
    const relinked = await page.evaluate(async (asset) => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const restored = await PM.assets.restoreProject(PM.proj);
      return { restored: restored.restored.map((item: any) => item.id), path: PM.proj.assets[asset].link?.path };
    }, asset);
    expect(relinked).toEqual({ restored: [asset], path: moved });
    expect(await page.evaluate((asset) => (window as any).PM.Proxies.assetState(asset).state, asset)).toBe('ready');
    expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);

    await session.relaunch();
    ({ app, page } = session);
    await session.openEditor();
    await app.evaluate(({ dialog }, filePath) => {
      dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] });
    }, projectFile);
    await page.evaluate(() => (window as any).PM.openProject());
    try {
      await page.waitForFunction(({ asset, moved }) => {
        const PM = (window as any).PM;
        return PM.proj?.assets?.[asset]?.link?.path === moved
          && !!PM.assets.get(asset) && PM.Proxies?.assetState(asset).state === 'ready';
      }, { asset, moved }, { timeout: 45_000 });
    } catch (error) {
      const state = await page.evaluate(async (asset) => {
        const PM = (window as any).PM;
        return { project: PM.proj?.id, asset: PM.proj?.assets?.[asset], live: !!PM.assets.get(asset),
          proxy: PM.Proxies?.assetState(asset), snapshot: await PM.EngineProxies.snapshot(PM.proj.id),
          toasts: [...document.querySelectorAll('[class*="toast"]')].map((node) => node.textContent?.trim()).filter(Boolean) };
      }, asset).catch((diagnosticError) => ({ diagnosticError: String(diagnosticError) }));
      throw new Error(`Linked proxy was not restored after relaunch: ${JSON.stringify(state)}`, { cause: error });
    }
    expect(await page.evaluate((asset) => (window as any).PM.proj.assets[asset].link?.path, asset)).toBe(moved);
  } finally {
    try { await session?.close(); } finally { await rm(root, { recursive: true, force: true }); }
  }
});
