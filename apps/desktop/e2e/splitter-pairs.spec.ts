import { test, expect } from './helpers/app';
test('timeline/shader splitter transfers height and revives a collapsed shader', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
  await page.waitForTimeout(1000);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.WS.mutate((w: any) => { const c = w.layout.docks.find((d: any) => d.id === 'center'); if (!c.panels.some((p: any) => p.id === 'shader')) c.panels.push({ id: 'shader', size: 320 }); });
  });
  await page.waitForSelector('#panel-shader'); await page.waitForTimeout(400);
  const h = (id: string) => page.evaluate((id) => document.getElementById(`panel-${id}`)!.getBoundingClientRect().height, id);
  const splitters = page.locator('#dock-center .splitter.h');
  await expect(splitters).toHaveCount(2);
  const sp = splitters.nth(1); // between timeline and shader
  const box = (await sp.boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  const v0 = await h('viewer'), t0 = await h('timeline'), s0 = await h('shader');
  console.log('before', { v0, t0, s0 });
  await page.mouse.move(x, y); await page.mouse.down();
  await page.mouse.move(x, y - 60, { steps: 6 }); await page.mouse.move(x, y - 120, { steps: 6 });
  await page.mouse.up(); await page.waitForTimeout(200);
  const v1 = await h('viewer'), t1 = await h('timeline'), s1 = await h('shader');
  console.log('after drag up', { v1, t1, s1 });
  expect(Math.abs(v1 - v0)).toBeLessThan(2);
  expect(s1).toBeGreaterThan(s0 + 100);
  expect(t1).toBeLessThan(t0 - 100);

  // collapse shader via header double-click, then drag the splitter up: it must come back
  await page.locator('#panel-shader > header').dblclick(); await page.waitForTimeout(200);
  const sc = await h('shader'); console.log('collapsed', sc);
  expect(sc).toBeLessThan(70);
  const box2 = (await sp.boundingBox())!;
  const x2 = box2.x + box2.width / 2, y2 = box2.y + box2.height / 2;
  await page.mouse.move(x2, y2); await page.mouse.down();
  await page.mouse.move(x2, y2 - 80, { steps: 8 }); await page.mouse.up(); await page.waitForTimeout(200);
  const s2 = await h('shader'), v2 = await h('viewer');
  console.log('after revive', { s2, v2 });
  expect(s2).toBeGreaterThan(120);
  expect(Math.abs(v2 - v0)).toBeLessThan(2);
  await page.screenshot({ path: 'e2e/test-results/splitter-pairs.png' });
});
