import { expect, test } from './helpers/app';
import type { Page } from '@playwright/test';
import path from 'node:path';

async function stripPoint(page: Page, id: string, time: number) {
  return page.evaluate(({ id, time }) => {
    const PM = (window as any).PM;
    const timeline = PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    const row = timeline.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === id);
    const item = timeline.trackView.layout().byId.get(id);
    return {
      x: box.x + timeline.gut + (time - timeline.scrollT) * timeline.pps,
      y: box.y + (timeline.mode === 'tracks'
        ? timeline.trackView.laneY(item.area, item.lane) + timeline.trackView.rowHeight() * .75
        : timeline.ruler + row * timeline.row - timeline.scrollY + timeline.row * .75),
      pps: timeline.pps,
    };
  }, { id, time });
}

async function scrollToStart(page: Page) {
  const point = await page.evaluate(() => {
    const timeline = (window as any).PM.Kernel.services.get('timeline');
    const box = timeline.cv.getBoundingClientRect();
    return { x: box.x + timeline.gut + 30, y: box.y + timeline.ruler + 10 };
  });
  await page.mouse.move(point.x, point.y);
  await page.mouse.wheel(-10000, 0);
}

for (const mode of ['layers', 'tracks']) test(`${mode}: strips move before zero and retain five seconds of earlier timeline space`, async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  const ids = await page.evaluate((mode) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Negative timeline QA', dur: 10 });
    const solid = PM.mkLayer('solid', { name: 'Solid', from: 1, dur: 2 }, project);
    const video = PM.mkLayer('video', { name: 'Video', from: 2, dur: 3, d: { trim: 1, speed: 2 } }, project);
    project.layers = [solid, video];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    PM.selectLayers([solid.id, video.id]);
    PM.setKey(solid, 'opacity', 2, 50);
    const timeline = PM.Kernel.services.get('timeline');
    timeline.setMode(mode);
    timeline.pps = 40;
    timeline.scrollT = 0;
    timeline.trackSnap = false;
    const fillText = CanvasRenderingContext2D.prototype.fillText;
    (window as any).__negativeRulerLabels = [];
    CanvasRenderingContext2D.prototype.fillText = function(text, x, y, maxWidth?) {
      if (this.canvas.id === 'tl-canvas' && y === 13 && this.textAlign === 'center')
        (window as any).__negativeRulerLabels.push(String(text));
      if (maxWidth === undefined) fillText.call(this, text, x, y);
      else fillText.call(this, text, x, y, maxWidth);
    };
    PM.hist.clear(); PM.touch(); PM.invalidate();
    return { project: project.id, solid: solid.id, video: video.id };
  }, mode);
  const scroll = () => page.evaluate(() => (window as any).PM.Kernel.services.get('timeline').scrollT);
  await scrollToStart(page);
  await expect.poll(scroll).toBe(-5);
  await expect.poll(() => page.evaluate(() => (window as any).__negativeRulerLabels)).toContain('-0:04');

  const timing = () => page.evaluate((ids) => {
    const PM = (window as any).PM;
    return {
      from: [PM.L(ids.solid).from, PM.L(ids.video).from],
      dur: [PM.L(ids.solid).dur, PM.L(ids.video).dur],
      key: PM.L(ids.solid).p.opacity.kf[0].t,
      trim: PM.L(ids.video).d.trim,
      work: PM.proj.work,
    };
  }, ids);
  const drag = async (time: number, delta: number) => {
    const point = await stripPoint(page, ids.solid, time);
    await page.mouse.move(point.x, point.y);
    await page.mouse.down();
    await page.mouse.move(point.x + delta * point.pps, point.y, { steps: 8 });
    await page.mouse.up();
  };
  await drag(2, -4);
  await expect.poll(timing).toEqual({ from: [-3, -2], dur: [2, 3], key: 1, trim: 1, work: [0, 10] });
  await scrollToStart(page);
  await expect.poll(scroll).toBe(-8);

  await drag(-2, -6);
  await expect.poll(timing).toEqual({ from: [-9, -8], dur: [2, 3], key: 1, trim: 1, work: [0, 10] });
  await scrollToStart(page);
  await expect.poll(scroll).toBe(-14);
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(timing).toEqual({ from: [-3, -2], dur: [2, 3], key: 1, trim: 1, work: [0, 10] });
  await page.evaluate(() => (window as any).PM.hist.redo());
  await expect.poll(timing).toEqual({ from: [-9, -8], dur: [2, 3], key: 1, trim: 1, work: [0, 10] });

  // Trimming a moved media strip still follows source time before zero.
  await page.evaluate((id) => (window as any).PM.selectLayers(id), ids.video);
  const edge = await stripPoint(page, ids.video, -8);
  await page.mouse.move(edge.x, edge.y);
  await page.mouse.down();
  await page.mouse.move(edge.x + .5 * edge.pps, edge.y, { steps: 4 });
  await page.mouse.up();
  await expect.poll(timing).toEqual({ from: [-9, -7.5], dur: [2, 2.5], key: 1, trim: 2, work: [0, 10] });
  await page.evaluate(() => (window as any).PM.hist.undo());
  await expect.poll(timing).toEqual({ from: [-9, -8], dur: [2, 3], key: 1, trim: 1, work: [0, 10] });

  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, path.join(session.userData, 'Negative timeline.pmv'));
  expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
  await session.relaunch();
  await session.openEditor();
  expect(await session.page.evaluate(async (id) => (window as any).PM.openProjectHere(id), ids.project)).toBe(true);
  await expect.poll(() => session.page.evaluate((ids) => {
    const PM = (window as any).PM;
    return {
      from: [PM.L(ids.solid)?.from, PM.L(ids.video)?.from],
      key: PM.L(ids.solid)?.p.opacity.kf[0]?.t,
      trim: PM.L(ids.video)?.d.trim,
    };
  }, ids)).toEqual({ from: [-9, -8], key: 1, trim: 1 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
