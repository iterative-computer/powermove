import { expect, test } from './helpers/app';
import type { Page } from 'playwright';
import { importFixture } from './helpers/media';

async function seed(page: Page, overlap = false) {
  const ids = await page.evaluate(overlap => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 320, h: 180, dur: 5 });
    const blue = PM.mkLayer('solid', { name: 'Blue clip', from: overlap ? 0 : 1, dur: overlap ? 5 : 1 }, project);
    const red = PM.mkLayer('solid', { name: 'Red clip', from: 0, dur: 5 }, project);
    blue.d.color = '#0000FF'; red.d.color = '#FF0000';
    blue.track = overlap ? 0 : 1; red.track = overlap ? 1 : 0;
    project.layers = overlap ? [red, blue] : [blue, red];
    PM.replaceProject(project); PM.selectLayers(blue.id); PM.setTime(1.5);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.pps = 100; timeline.scrollT = timeline.trackScrollY = 0; timeline.trackSnap = false;
    PM.hist.clear(); PM.invalidate();
    return { blue: blue.id, red: red.id };
  }, overlap);
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  return ids;
}

const point = (page: Page, lane: number) => page.evaluate(lane => {
  const timeline = (window as any).PM.Kernel.services.get('timeline');
  const rect = timeline.cv.getBoundingClientRect();
  return { x: rect.left + timeline.gut + 150,
    y: rect.top + timeline.trackView.laneY('video', lane) + timeline.trackView.rowHeight() / 2 };
}, lane);

// Read the actual preview; requesting another render here would hide a missing
// redraw during the gesture.
const pixel = (page: Page) => page.evaluate(() => {
  const gl = (window as any).PM.GL.gl, rgba = new Uint8Array(4);
  gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2),
    1, 1, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
  return Array.from(rgba);
});

test('moving a clip in time updates the composition before release and creates one undo step', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await seed(page);
  await expect.poll(() => pixel(page)).toEqual([0, 0, 255, 255]);
  const start = await point(page, 1);
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(start.x + 100, start.y, { steps: 5 });
  expect(await page.evaluate(id => (window as any).PM.L(id).from, ids.blue)).toBe(2);
  await expect.poll(() => pixel(page)).toEqual([255, 0, 0, 255]);
  await page.mouse.move(start.x, start.y, { steps: 5 });
  await expect.poll(() => pixel(page)).toEqual([0, 0, 255, 255]);
  await page.mouse.move(start.x + 100, start.y, { steps: 5 });
  await expect.poll(() => pixel(page)).toEqual([255, 0, 0, 255]);
  await page.mouse.up();
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
  expect(await page.evaluate(id => (window as any).PM.L(id).from, ids.blue)).toBe(1);
  await expect.poll(() => pixel(page)).toEqual([0, 0, 255, 255]);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(false);
  expect(await page.evaluate(() => (window as any).PM.hist.redo())).toBe(true);
  await expect.poll(() => pixel(page)).toEqual([255, 0, 0, 255]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('moving between tracks previews stacking and Escape restores the composition without an edit', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  const ids = await seed(page, true);
  const original = await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers));
  await expect.poll(() => pixel(page)).toEqual([255, 0, 0, 255]);
  const start = await point(page, 0), to = await point(page, 1);
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 5 });
  expect(await page.evaluate(() => (window as any).PM.proj.layers[0].id)).toBe(ids.blue);
  await expect.poll(() => pixel(page)).toEqual([0, 0, 255, 255]);
  await page.keyboard.press('Escape'); await page.mouse.up();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(original);
  await expect.poll(() => pixel(page)).toEqual([255, 0, 0, 255]);
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(false);
  expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').trackDrag)).toBeNull();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a paused video seeks to its new local time during a clip drag and restores on pointer cancellation', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'video'));
  const original = await page.evaluate(() => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((layer: any) => layer.type === 'video');
    PM.proj.layers = [layer]; PM.proj.w = 160; PM.proj.h = 90; PM.proj.dur = 5;
    layer.from = 0; layer.dur = 2;
    layer.d.w = PM.P(160); layer.d.h = PM.P(90);
    layer.d.trim = PM.P(0); layer.d.speed = PM.P(1);
    layer.p['position.x'].v = 80; layer.p['position.y'].v = 45;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    PM.ProjectIndex.invalidate(); PM.touch(); PM.bus.emit('layers');
    PM.Kernel.services.get('viewer').layout(); PM.selectLayers(layer.id); PM.setTime(1.5);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.pps = 100; timeline.scrollT = timeline.trackScrollY = 0; timeline.trackSnap = false;
    PM.hist.clear(); PM.invalidate();
    return JSON.stringify(PM.proj.layers);
  });
  await page.locator('#tl-mode [data-mode="tracks"]').click();
  const color = () => pixel(page).then(([red, green]) => red! > 220 && green! < 30 ? 'red'
    : green! > 220 && red! < 30 ? 'green' : 'other');
  await expect.poll(color).toBe('green');
  const start = await point(page, 0);
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(start.x + 100, start.y, { steps: 5 });
  await expect.poll(color).toBe('red');
  expect(await page.evaluate(() => (window as any).PM.time)).toBe(1.5);
  await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel')));
  await page.mouse.up();
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(original);
  await expect.poll(color).toBe('green');
  expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
