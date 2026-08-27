import { expect, test } from './helpers/app';

test.describe('@viewer direct resize', () => {
  test('resizes the selected layer in local space with a fixed opposite edge and correct cursors', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Direct resize', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const parent = PM.mkLayer('null', {
        name: 'Transform parent', dur: 4,
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 120, 'scale.y': 80, rotation: 15 },
      }, project);
      const text = PM.mkLayer('shape', {
        name: 'Selected shape', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 200, h: 100, radius: 8, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 0, 'position.y': 0, rotation: 30 },
      }, project);
      text.parent = parent.id;
      const cover = PM.mkLayer('solid', {
        name: 'Full-frame cover', dur: 4, d: { color: '#FF6B1A', w: 640, h: 360 },
        p: { opacity: 1 },
      }, project);
      project.layers = [cover, text, parent];
      PM.replaceProject(project);
      PM.snap = false;
      PM.tool = 'select';
      PM.setTime(1, { raw: true, force: true });
      PM.setKey(text, 'scale.y', 0, 100, 'linear');
      PM.selectLayers(text.id);
      PM.Viewer.layout();

      const bounds = PM.GL.bounds(text, 1);
      const matrix = PM.worldMatrix(text, 1);
      const axisLength = Math.hypot(matrix[0], matrix[1]);
      const point = (hx: number, hy: number) => {
        const x = bounds.x0 + bounds.w * hx, y = bounds.y0 + bounds.h * hy;
        return { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] };
      };
      return {
        textId: text.id, coverId: cover.id, parentId: parent.id,
        east: point(1, .5), west: point(0, .5),
        axis: { x: matrix[0] / axisLength, y: matrix[1] / axisLength },
        eastCursor: PM.Viewer.resizeCursorForHandle(matrix, 'e'),
        scaleX: text.p['scale.x'].v, scaleY: text.p['scale.y'].v,
        scaleYKeys: text.p['scale.y'].kf.length,
        coverScaleX: cover.p['scale.x'].v,
        parentScaleX: parent.p['scale.x'].v,
      };
    });

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
    const shown = await page.evaluate(() => (window as any).PM.Viewer.shown);
    const east = { x: box.x + setup.east.x * shown, y: box.y + setup.east.y * shown };
    await page.mouse.move(east.x, east.y);
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', setup.eastCursor);

    await page.mouse.down();
    await page.mouse.move(east.x + setup.axis.x * 60, east.y + setup.axis.y * 60, { steps: 8 });
    await page.mouse.up();

    const resized = await page.evaluate(({ textId, coverId, parentId }) => {
      const PM = (window as any).PM;
      const text = PM.L(textId), bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0, y = bounds.y0 + bounds.h * .5;
      return {
        selected: [...PM.sel.layers],
        scaleX: PM.ev(text, 'scale.x', 1), scaleY: PM.ev(text, 'scale.y', 1),
        scaleYKeys: text.p['scale.y'].kf.length,
        west: { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] },
        coverScaleX: PM.ev(PM.L(coverId), 'scale.x', 1),
        parentScaleX: PM.ev(PM.L(parentId), 'scale.x', 1),
      };
    }, { textId: setup.textId, coverId: setup.coverId, parentId: setup.parentId });
    expect(resized.selected).toEqual([setup.textId]);
    expect(resized.scaleX).toBeGreaterThan(setup.scaleX);
    expect(resized.scaleY).toBe(setup.scaleY);
    expect(resized.scaleYKeys).toBe(setup.scaleYKeys);
    expect(resized.west.x).toBeCloseTo(setup.west.x, 2);
    expect(resized.west.y).toBeCloseTo(setup.west.y, 2);
    expect(resized.coverScaleX).toBe(setup.coverScaleX);
    expect(resized.parentScaleX).toBe(setup.parentScaleX);

    const rotatedNorth = await page.evaluate((textId) => {
      const PM = (window as any).PM, text = PM.L(textId);
      text.p.rotation.v = 90;
      PM.touch(); PM.invalidate();
      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0 + bounds.w * .5, y = bounds.y0;
      return {
        x: matrix[0] * x + matrix[2] * y + matrix[4],
        y: matrix[1] * x + matrix[3] * y + matrix[5],
        cursor: PM.Viewer.resizeCursorForHandle(matrix, 'n'),
      };
    }, setup.textId);
    await page.mouse.move(box.x + rotatedNorth.x * shown, box.y + rotatedNorth.y * shown);
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', rotatedNorth.cursor);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('maps text handle resizing to typography Size without touching Scale X/Y', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Proportional text resize', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const text = PM.mkLayer('text', {
        name: 'Selected text', dur: 4,
        d: { text: 'MAKE THE MOVE.', font: 'SF Pro Display', weight: 700, size: 64, tracking: -2, leading: 1, color: '#FFFFFF', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180 },
      }, project);
      project.layers = [text];
      PM.replaceProject(project);
      PM.tool = 'select';
      PM.setKey(text, 'scale.x', 0, 100, 'linear');
      PM.setKey(text, 'scale.y', 0, 100, 'linear');
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers(text.id);
      PM.Viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));

      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const point = (hx: number, hy: number) => {
        const x = bounds.x0 + bounds.w * hx, y = bounds.y0 + bounds.h * hy;
        return { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] };
      };
      return {
        textId: text.id,
        top: point(.5, 0),
        size: text.d.size,
        scaleX: PM.ev(text, 'scale.x', 1), scaleY: PM.ev(text, 'scale.y', 1),
      };
    });

    const bottom = await page.evaluate((textId) => {
      const PM = (window as any).PM, text = PM.L(textId);
      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0 + bounds.w * .5, y = bounds.y1;
      const point = { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] };
      const rect = PM.Viewer.inner.getBoundingClientRect();
      return { x: rect.left + point.x * PM.Viewer.shown, y: rect.top + point.y * PM.Viewer.shown };
    }, setup.textId);
    await page.mouse.move(bottom.x, bottom.y);
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', 'ns-resize');
    await page.mouse.down();
    await expect(page.locator('body')).toHaveCSS('cursor', 'ns-resize');
    await page.mouse.move(bottom.x, bottom.y + 72, { steps: 8 });
    await page.mouse.up();

    const resized = await page.evaluate((textId) => {
      const PM = (window as any).PM, text = PM.L(textId);
      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0 + bounds.w * .5, y = bounds.y0;
      return {
        size: text.d.size,
        scaleX: PM.ev(text, 'scale.x', 1), scaleY: PM.ev(text, 'scale.y', 1),
        scaleXKeys: text.p['scale.x'].kf.length, scaleYKeys: text.p['scale.y'].kf.length,
        top: { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] },
      };
    }, setup.textId);
    expect(resized.size).toBeGreaterThan(setup.size);
    expect(resized.scaleX).toBe(setup.scaleX);
    expect(resized.scaleY).toBe(setup.scaleY);
    expect(resized.scaleXKeys).toBe(1);
    expect(resized.scaleYKeys).toBe(1);
    expect(resized.top.x).toBeCloseTo(setup.top.x, 2);
    expect(resized.top.y).toBeCloseTo(setup.top.y, 2);
    const inspectorSize = Number(await page.getByRole('spinbutton', { name: 'Size' }).getAttribute('aria-valuenow'));
    expect(inspectorSize).toBeCloseTo(resized.size, 4);
    const undone = await page.evaluate((textId) => {
      const PM = (window as any).PM;
      const ok = PM.hist.undo(), text = PM.L(textId);
      return { ok, size: text.d.size, scaleX: PM.ev(text, 'scale.x', 1), scaleY: PM.ev(text, 'scale.y', 1) };
    }, setup.textId);
    expect(undone).toEqual({ ok: true, size: setup.size, scaleX: setup.scaleX, scaleY: setup.scaleY });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
