import { expect, test } from './helpers/app';

test.describe('@viewer selection-preserving direct manipulation', () => {
  test('drags the timeline-selected layer beneath a full-frame top layer', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Selection ownership', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const text = PM.mkLayer('text', {
        name: 'Selected text', dur: 4,
        d: { text: 'MOVE ME', font: 'Geist', weight: 700, size: 72, color: '#FFFFFF', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180 },
      }, project);
      const cover = PM.mkLayer('solid', {
        name: 'Full-frame cover', dur: 4, d: { color: '#FF6B1A', w: 640, h: 360 },
        p: { opacity: 1 },
      }, project);
      project.layers = [cover, text];
      PM.replaceProject(project);
      PM.snap = false;
      PM.tool = 'select';
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers(text.id);
      PM.Viewer.layout();
      const textBounds = PM.Viewer.worldBounds(text, 1);
      const point = { x: textBounds.cx, y: textBounds.cy };
      return {
        textId: text.id, coverId: cover.id,
        pickedId: PM.GL.pick(point.x, point.y, 1)?.id,
        textX: text.p['position.x'].v, coverX: cover.p['position.x'].v,
        point, containsCenter: PM.Viewer.layerContainsPoint(text, point.x, point.y, 1),
      };
    });
    expect(setup.pickedId).toBe(setup.coverId);
    expect(setup.containsCenter).toBe(true);

    const overlay = page.locator('#overlay');
    const box = await overlay.boundingBox();
    if (!box) throw new Error('viewer overlay is unavailable');
    const shownAtStart = await page.evaluate(() => (window as any).PM.Viewer.shown);
    const start = { x: box.x + setup.point.x * shownAtStart, y: box.y + setup.point.y * shownAtStart };
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(start.x + 48, start.y, { steps: 5 });
    await page.mouse.up();

    const moved = await page.evaluate(({ textId, coverId }) => {
      const PM = (window as any).PM;
      return {
        selected: [...PM.sel.layers],
        textX: PM.L(textId).p['position.x'].v,
        coverX: PM.L(coverId).p['position.x'].v,
      };
    }, { textId: setup.textId, coverId: setup.coverId });
    expect(moved.selected).toEqual([setup.textId]);
    expect(moved.textX).toBeGreaterThan(setup.textX);
    expect(moved.coverX).toBe(setup.coverX);

    // A click without drag still explicitly chooses the visually top layer.
    const shown = await page.evaluate(() => (window as any).PM.Viewer.shown);
    await page.mouse.click(start.x + (moved.textX - setup.textX) * shown, start.y);
    expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual([setup.coverId]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
