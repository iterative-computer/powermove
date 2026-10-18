import { expect, test } from './helpers/app';
import { exportStillPixels, importFixture } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

for (const file of ['animated.gif', 'animated-apng.png', 'animated.webp']) {
  test(`the native viewer and exporter draw every persisted ${file} frame without routing`, async ({ session }) => {
    test.setTimeout(120_000);
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ w: 160, h: 90, fps: 30, dur: 2, name: 'Native media' }));
      PM.ProjectsScreen.hide();
    });
    await importFixture(page, file);
    await page.waitForFunction(name => (window as any).PM.proj.layers.some((l: any) => l.name === name), file);
    await expect.poll(() => page.evaluate(() => {
      const s = (window as any).PM.EngineConnection.status();
      return { route: s.route, reason: s.routeReason };
    }), { timeout: 45_000 }).toEqual({ route: 'engine', reason: null });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().frames), { timeout: 45_000 }).toBeGreaterThan(0);
    for (const index of [0, 1, 2, 0, 2, 1]) {
      const [opaque, transparent] = await exportStillPixels(session, index / 10, 160, 90, [[40, 45], [120, 45]]);
      expect(opaque![index]).toBeGreaterThan(200);
      expect(opaque![(index + 1) % 3]).toBeLessThan(60);
      expect(opaque![3]).toBe(255);
      expect(transparent![3]).toBe(0);
      expect(await page.evaluate(() => (window as any).PM.EngineConnection.status().route)).toBe('engine');
    }
  });
}
