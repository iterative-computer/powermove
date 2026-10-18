import { unlink } from 'node:fs/promises';
import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('an offline copied image stays in the native viewer and names its failed layer', async ({ session }) => {
  test.setTimeout(90_000);
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'still-red.png');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'image'));
  const identity = await page.evaluate(() => {
    const PM = (window as any).PM;
    const layer = PM.proj.layers.find((l: any) => l.type === 'image');
    return { asset: layer.d.asset, layer: layer.id };
  });
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route)).toBe('engine');
  const storedPath = await session.app.evaluate(async (_electron, asset) => {
    const store = (globalThis as any).__powermoveMediaStore;
    const resolved = await store.resolveAsset(asset);
    if (!resolved?.path) throw new Error('copied image did not resolve');
    return resolved.path as string;
  }, identity.asset);
  await unlink(storedPath);
  await page.evaluate(asset => {
    const PM = (window as any).PM;
    // A changed record starts a fresh host resolution, as Replace File does.
    PM.proj.assets[asset].name = 'offline-image.png';
    PM.touch(); PM.bus.emit('assets'); PM.invalidate('render');
  }, identity.asset);
  await expect.poll(() => page.evaluate(() => {
    const status = (window as any).PM.EngineConnection.status();
    return { route: status.route, failed: status.failures?.some((f: string) => f.includes('offline-image.png') || f.includes((window as any).PM.proj.layers.find((l: any) => l.type === 'image')?.id)) };
  }), { timeout: 45_000 }).toEqual({ route: 'engine', failed: true });
});
