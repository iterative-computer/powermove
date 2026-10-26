import { expect, test } from './helpers/app';

test('playback and scrubbing draw only the active timeline viewport, including after reload', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('timeline')?.cv));
  // Observe actual canvas paints: checking service.pps alone misses a stale
  // instance painting its own zoom into the same canvas between active draws.
  await page.evaluate(() => {
    const original = CanvasRenderingContext2D.prototype.fillText;
    (window as any).__timelineRulerPaints = [];
    CanvasRenderingContext2D.prototype.fillText = function(text, x, y, maxWidth?) {
      // Ruler labels: the mono "15s" / "1m00s" names in the time scale's band.
      if (this.canvas.id === 'tl-canvas' && y < 30 && /^-?\d+(s|f|m\d\ds)$/.test(String(text))) {
        (window as any).__timelineRulerPaints.push(String(text));
      }
      if (maxWidth === undefined) original.call(this, text, x, y);
      else original.call(this, text, x, y, maxWidth);
    };
  });
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Kernel.loader.whenIdle();
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Playback viewport QA', dur: 87.4 }),
    }));
    PM.ProjectsScreen.hide();
    PM.proj.layers = [PM.mkLayer('text', { name: 'Text', from: 10, dur: 75 })];
    PM.touch();
    const timeline = PM.Kernel.services.get('timeline');
    timeline.pps = 6;
    timeline.scrollT = 0;
    PM.setTime(1);
    // Include the first vector paint: steady playback now restores cached
    // artwork and correctly emits no ruler fillText calls.
    (window as any).__timelineRulerPaints = [];
    PM.invalidate();
  });


  for (const reload of [false, true]) {
    if (reload) {
      await page.evaluate(async () => {
        const PM = (window as any).PM;
        (window as any).__timelineRulerPaints = [];
        await PM.Kernel.loader.reload('timeline');
        await PM.Kernel.loader.whenIdle();
      });
    }
    await page.getByRole('button', { name: 'Play / Pause (Space)' }).click();
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(2);
    await page.getByRole('button', { name: 'Play / Pause (Space)' }).click();
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing)).toBe(false);

    const ruler = await page.evaluate(() => {
      const timeline = (window as any).PM.Kernel.services.get('timeline');
      const box = timeline.cv.getBoundingClientRect();
      return { x: box.x + timeline.gut + 30, y: box.y + timeline.ruler - 6 };
    });
    await page.mouse.move(ruler.x, ruler.y);
    await page.mouse.down();
    await page.mouse.move(ruler.x + 12, ruler.y, { steps: 8 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeCloseTo(7, 1);

    const painted = await page.evaluate(() => [...new Set((window as any).__timelineRulerPaints)] as string[]);
    expect(painted).toContain('15s');
    expect(painted.filter(label => !/^(?:(?:0|15|30|45)s|\d+m(?:00|15|30|45)s)$/.test(label))).toEqual([]);
    expect(await page.evaluate(() => {
      const timeline = (window as any).PM.Kernel.services.get('timeline');
      return { pps: timeline.pps, scrollT: timeline.scrollT };
    })).toEqual({ pps: 6, scrollT: 0 });
    await page.evaluate(() => (window as any).PM.setTime(1));
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
  expect(session.diagnostics.console.filter(record => record.text.includes('[timeline draw]'))).toEqual([]);
});
