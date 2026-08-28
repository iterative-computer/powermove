import { expect, test } from './helpers/app';

test.describe('@viewer selection-preserving direct manipulation', () => {
  test('drags the timeline-selected layer beneath a full-frame top layer', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(async () => {
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

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
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

  test('moves a multi-selection from the empty gap inside its common box as one undoable gesture', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Common-box move', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const left = PM.mkLayer('shape', {
        name: 'Left', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 180, 'position.y': 180 },
      }, project);
      const right = PM.mkLayer('shape', {
        name: 'Right', dur: 4,
        d: { shape: 'rect', color: '#FF6B1A', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 460, 'position.y': 180 },
      }, project);
      project.layers = [right, left];
      PM.replaceProject(project);
      PM.snap = false;
      PM.tool = 'select';
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers([left.id, right.id]);
      PM.Viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const selection = PM.Viewer.resolveSelectionGeometry();
      return {
        leftId: left.id, rightId: right.id,
        leftX: left.p['position.x'].v, rightX: right.p['position.x'].v,
        point: { x: selection.pivotWorld.x, y: selection.pivotWorld.y },
        picked: PM.GL.pick(selection.pivotWorld.x, selection.pivotWorld.y, 1)?.id ?? null,
        mode: selection.mode,
      };
    });
    expect(setup.mode).toBe('common');
    expect(setup.picked).toBeNull();

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
    const shown = await page.evaluate(() => (window as any).PM.Viewer.shown);
    const start = { x: box.x + setup.point.x * shown, y: box.y + setup.point.y * shown };
    await page.mouse.move(start.x, start.y);
    await expect(frame).toHaveCSS('cursor', 'move');
    await page.mouse.down();
    await page.mouse.move(start.x + 48, start.y + 24, { steps: 6 });
    await page.mouse.up();

    const moved = await page.evaluate(({ leftId, rightId }) => {
      const PM = (window as any).PM;
      return {
        selected: [...PM.sel.layers],
        left: [PM.ev(PM.L(leftId), 'position.x', 1), PM.ev(PM.L(leftId), 'position.y', 1)],
        right: [PM.ev(PM.L(rightId), 'position.x', 1), PM.ev(PM.L(rightId), 'position.y', 1)],
      };
    }, { leftId: setup.leftId, rightId: setup.rightId });
    expect(moved.selected).toEqual([setup.leftId, setup.rightId]);
    expect(moved.left).toEqual([setup.leftX + 48 / shown, 180 + 24 / shown]);
    expect(moved.right).toEqual([setup.rightX + 48 / shown, 180 + 24 / shown]);

    const undone = await page.evaluate(({ leftId, rightId }) => {
      const PM = (window as any).PM;
      const ok = PM.hist.undo();
      return {
        ok,
        left: [PM.ev(PM.L(leftId), 'position.x', 1), PM.ev(PM.L(leftId), 'position.y', 1)],
        right: [PM.ev(PM.L(rightId), 'position.x', 1), PM.ev(PM.L(rightId), 'position.y', 1)],
      };
    }, { leftId: setup.leftId, rightId: setup.rightId });
    expect(undone).toEqual({ ok: true, left: [setup.leftX, 180], right: [setup.rightX, 180] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('keeps locked members in common chrome without transforming the unlocked subset', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Atomic locked selection', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const left = PM.mkLayer('shape', {
        name: 'Unlocked', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 180, 'position.y': 180 },
      }, project);
      const right = PM.mkLayer('shape', {
        name: 'Locked', dur: 4,
        d: { shape: 'rect', color: '#FF6B1A', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 460, 'position.y': 180 },
      }, project);
      right.lock = true;
      project.layers = [right, left];
      PM.replaceProject(project);
      PM.snap = false;
      PM.tool = 'select';
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers([left.id, right.id]);
      PM.Viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const selection = PM.Viewer.resolveSelectionGeometry();
      return {
        leftId: left.id, rightId: right.id,
        positions: [left.p['position.x'].v, right.p['position.x'].v],
        point: { x: PM.Viewer.worldBounds(left, 1).cx, y: PM.Viewer.worldBounds(left, 1).cy },
        layerIds: selection.layers.map((layer: any) => layer.id),
        transformable: selection.transformable,
        roots: selection.roots.length,
        bounds: selection.bounds,
      };
    });
    expect(setup.layerIds).toEqual([setup.leftId, setup.rightId]);
    expect(setup.transformable).toBe(false);
    expect(setup.roots).toBe(0);
    expect(setup.bounds.x0).toBeLessThan(180);
    expect(setup.bounds.x1).toBeGreaterThan(460);

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
    const shown = await page.evaluate(() => (window as any).PM.Viewer.shown);
    const start = { x: box.x + setup.point.x * shown, y: box.y + setup.point.y * shown };
    await page.mouse.move(start.x, start.y);
    await expect(frame).toHaveCSS('cursor', 'default');
    await page.mouse.down();
    await page.mouse.move(start.x + 48, start.y, { steps: 6 });
    await page.mouse.up();

    const after = await page.evaluate(({ leftId, rightId }) => {
      const PM = (window as any).PM;
      return {
        positions: [PM.ev(PM.L(leftId), 'position.x', 1), PM.ev(PM.L(rightId), 'position.x', 1)],
        selected: [...PM.sel.layers],
      };
    }, { leftId: setup.leftId, rightId: setup.rightId });
    expect(after.positions).toEqual(setup.positions);
    expect(after.selected).toEqual([setup.leftId, setup.rightId]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
