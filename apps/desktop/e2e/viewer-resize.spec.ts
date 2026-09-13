import { expect, test } from './helpers/app';

test.describe('@viewer direct resize', () => {
  test('resizes the selected layer in local space with a fixed opposite edge and correct cursors', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && (window as any).PM?.GL?.gl); });
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const tool = PM.Kernel.services.get('tool');
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
      tool.tool = 'select';
      PM.setTime(1, { raw: true, force: true });
      PM.setKey(text, 'scale.y', 0, 100, 'linear');
      PM.selectLayers(text.id);
      viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));

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
        eastCursor: viewer.resizeCursorForHandle(matrix, 'e'),
        scaleX: text.p['scale.x'].v, scaleY: text.p['scale.y'].v,
        scaleYKeys: text.p['scale.y'].kf.length,
        coverScaleX: cover.p['scale.x'].v,
        parentScaleX: parent.p['scale.x'].v,
      };
    });

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
    const shown = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });
    const east = { x: box.x + setup.east.x * shown, y: box.y + setup.east.y * shown };
    await page.mouse.move(east.x, east.y);
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', setup.eastCursor);

    await page.mouse.down();
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
    await page.mouse.move(east.x + setup.axis.x * 60, east.y + setup.axis.y * 60, { steps: 8 });
    await page.mouse.up();
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));

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
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const text = PM.L(textId);
      text.p.rotation.v = 90;
      PM.touch(); PM.invalidate();
      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0 + bounds.w * .5, y = bounds.y0;
      return {
        x: matrix[0] * x + matrix[2] * y + matrix[4],
        y: matrix[1] * x + matrix[3] * y + matrix[5],
        cursor: viewer.resizeCursorForHandle(matrix, 'n'),
      };
    }, setup.textId);
    await page.mouse.move(box.x + rotatedNorth.x * shown, box.y + rotatedNorth.y * shown);
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', rotatedNorth.cursor);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('maps text handle resizing to typography Size without touching Scale X/Y', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && (window as any).PM?.GL?.gl); });
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const tool = PM.Kernel.services.get('tool');
      const project = PM.mkProject({ name: 'Proportional text resize', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const text = PM.mkLayer('text', {
        name: 'Selected text', dur: 4,
        d: { text: 'MAKE THE MOVE.', font: 'SF Pro Display', weight: 700, size: 64, tracking: -2, leading: 1, color: '#FFFFFF', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180 },
      }, project);
      project.layers = [text];
      PM.replaceProject(project);
      tool.tool = 'select';
      PM.setKey(text, 'scale.x', 0, 100, 'linear');
      PM.setKey(text, 'scale.y', 0, 100, 'linear');
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers(text.id);
      viewer.layout();
      await document.fonts.ready;
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
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const text = PM.L(textId);
      const bounds = PM.GL.bounds(text, 1), matrix = PM.worldMatrix(text, 1);
      const x = bounds.x0 + bounds.w * .5, y = bounds.y1;
      const point = { x: matrix[0] * x + matrix[2] * y + matrix[4], y: matrix[1] * x + matrix[3] * y + matrix[5] };
      const rect = viewer.inner.getBoundingClientRect();
      return { x: rect.left + point.x * viewer.shown, y: rect.top + point.y * viewer.shown };
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

  test('resizes and rotates a multi-selection around the common pivot with one-step undo', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && (window as any).PM?.GL?.gl); });
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const tool = PM.Kernel.services.get('tool');
      const project = PM.mkProject({ name: 'Common transform', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const parent = PM.mkLayer('null', {
        name: 'Skewed parent', dur: 4,
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 160, 'scale.y': 60, rotation: 10, skew: 25 },
      }, project);
      const shape = PM.mkLayer('shape', {
        name: 'Shape', dur: 4,
        d: { shape: 'rect', color: '#FF6B1A', w: 120, h: 80, radius: 8, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': -90, 'position.y': 0 },
      }, project);
      shape.parent = parent.id;
      const text = PM.mkLayer('text', {
        name: 'Text', dur: 4,
        d: { text: 'TOGETHER', font: 'Geist', weight: 700, size: 54, tracking: -1, leading: 1, color: '#FFFFFF', align: 'center' },
        p: { 'position.x': 450, 'position.y': 180 },
      }, project);
      const textChild = PM.mkLayer('shape', {
        name: 'Text child', dur: 4,
        d: { shape: 'rect', color: '#4C8DFF', w: 44, h: 44, radius: 6, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 0, 'position.y': 90 },
      }, project);
      textChild.parent = text.id;
      project.layers = [textChild, text, shape, parent];
      PM.replaceProject(project);
      tool.tool = 'select';
      PM.setKey(shape, 'scale.x', 0, 100, 'linear');
      PM.setKey(shape, 'scale.y', 0, 100, 'linear');
      PM.setKey(text, 'scale.x', 0, 100, 'linear');
      PM.setKey(text, 'scale.y', 0, 100, 'linear');
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers([shape.id, text.id, textChild.id]);
      viewer.layout();
      await document.fonts.ready;
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const selection = viewer.resolveSelectionGeometry();
      const axis = (layer: any) => {
        const matrix = PM.worldMatrix(layer, 1);
        return {
          x: { x: matrix[0], y: matrix[1] },
          y: { x: matrix[2], y: matrix[3] },
        };
      };
      return {
        shapeId: shape.id, textId: text.id, textChildId: textChild.id,
        shapeScale: [PM.ev(shape, 'scale.x', 1), PM.ev(shape, 'scale.y', 1)],
        shapeTransform: {
          rotation: PM.ev(shape, 'rotation', 1),
          scaleX: PM.ev(shape, 'scale.x', 1),
          scaleY: PM.ev(shape, 'scale.y', 1),
          skew: PM.ev(shape, 'skew', 1),
        },
        shapeKeys: [shape.p['scale.x'].kf.length, shape.p['scale.y'].kf.length],
        textSize: text.d.size,
        textScale: [PM.ev(text, 'scale.x', 1), PM.ev(text, 'scale.y', 1)],
        textScaleKeys: [text.p['scale.x'].kf.length, text.p['scale.y'].kf.length],
        textChildScale: [PM.ev(textChild, 'scale.x', 1), PM.ev(textChild, 'scale.y', 1)],
        axes: { shape: axis(shape), text: axis(text) },
        positions: {
          shape: [PM.ev(shape, 'position.x', 1), PM.ev(shape, 'position.y', 1)],
          text: [PM.ev(text, 'position.x', 1), PM.ev(text, 'position.y', 1)],
        },
        bounds: selection.bounds,
        southeast: selection.handles.se,
        mode: selection.mode,
        rootIds: selection.roots.map((layer: any) => layer.id),
      };
    });
    expect(setup.mode).toBe('common');
    expect(setup.rootIds).toEqual([setup.shapeId, setup.textId]);

    const frame = page.locator('#stage-inner');
    const box = await frame.boundingBox();
    if (!box) throw new Error('viewer frame is unavailable');
    const shown = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });
    const southeast = {
      x: box.x + setup.southeast.x * shown,
      y: box.y + setup.southeast.y * shown,
    };
    await page.mouse.move(southeast.x, southeast.y);
    await expect(frame).toHaveCSS('cursor', 'nwse-resize');
    await page.mouse.down();
    await page.mouse.move(southeast.x + 72, southeast.y + 48, { steps: 8 });
    await page.mouse.up();

    const resized = await page.evaluate(({ shapeId, textId, textChildId }) => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const shape = PM.L(shapeId), text = PM.L(textId), textChild = PM.L(textChildId);
      const selection = viewer.resolveSelectionGeometry();
      return {
        bounds: selection.bounds,
        shapeScale: [PM.ev(shape, 'scale.x', 1), PM.ev(shape, 'scale.y', 1)],
        shapeKeys: [shape.p['scale.x'].kf.length, shape.p['scale.y'].kf.length],
        textSize: text.d.size,
        textScale: [PM.ev(text, 'scale.x', 1), PM.ev(text, 'scale.y', 1)],
        textScaleKeys: [text.p['scale.x'].kf.length, text.p['scale.y'].kf.length],
        textChildScale: [PM.ev(textChild, 'scale.x', 1), PM.ev(textChild, 'scale.y', 1)],
      };
    }, { shapeId: setup.shapeId, textId: setup.textId, textChildId: setup.textChildId });
    expect(resized.bounds.x0).toBeCloseTo(setup.bounds.x0, 1);
    expect(resized.bounds.y0).toBeCloseTo(setup.bounds.y0, 1);
    expect(resized.shapeScale[0]).toBeGreaterThan(setup.shapeScale[0]);
    expect(resized.shapeScale[1]).toBeGreaterThan(setup.shapeScale[1]);
    expect(resized.shapeKeys[0]).toBeGreaterThan(setup.shapeKeys[0]);
    expect(resized.shapeKeys[1]).toBeGreaterThan(setup.shapeKeys[1]);
    expect(resized.textSize).toBe(setup.textSize);
    expect(resized.textScale[0]).toBeGreaterThan(setup.textScale[0]);
    expect(resized.textScale[1]).toBeGreaterThan(setup.textScale[1]);
    expect(resized.textScaleKeys[0]).toBeGreaterThan(setup.textScaleKeys[0]);
    expect(resized.textScaleKeys[1]).toBeGreaterThan(setup.textScaleKeys[1]);
    expect(resized.textChildScale).toEqual(setup.textChildScale);

    const resizeUndone = await page.evaluate(({ shapeId, textId, textChildId }) => {
      const PM = (window as any).PM;
      const ok = PM.hist.undo(), shape = PM.L(shapeId), text = PM.L(textId), textChild = PM.L(textChildId);
      return {
        ok,
        shapeScale: [PM.ev(shape, 'scale.x', 1), PM.ev(shape, 'scale.y', 1)],
        textSize: text.d.size,
        textScale: [PM.ev(text, 'scale.x', 1), PM.ev(text, 'scale.y', 1)],
        textChildScale: [PM.ev(textChild, 'scale.x', 1), PM.ev(textChild, 'scale.y', 1)],
        positions: {
          shape: [PM.ev(shape, 'position.x', 1), PM.ev(shape, 'position.y', 1)],
          text: [PM.ev(text, 'position.x', 1), PM.ev(text, 'position.y', 1)],
        },
      };
    }, { shapeId: setup.shapeId, textId: setup.textId, textChildId: setup.textChildId });
    expect(resizeUndone).toEqual({
      ok: true, shapeScale: setup.shapeScale, textSize: setup.textSize,
      textScale: setup.textScale, textChildScale: setup.textChildScale, positions: setup.positions,
    });

    const rotation = await page.evaluate(() => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const selection = viewer.resolveSelectionGeometry();
      return {
        center: selection.pivotWorld,
        north: selection.handles.n,
      };
    });
    const center = { x: box.x + rotation.center.x * shown, y: box.y + rotation.center.y * shown };
    const rotateStart = { x: box.x + rotation.north.x * shown, y: box.y + rotation.north.y * shown - 22 };
    const radius = Math.hypot(rotateStart.x - center.x, rotateStart.y - center.y);
    const rotateEnd = {
      x: center.x + radius * Math.SQRT1_2,
      y: center.y - radius * Math.SQRT1_2,
    };
    await page.mouse.move(rotateStart.x, rotateStart.y);
    await page.mouse.down();
    await page.keyboard.down('Meta');
    await page.mouse.move(rotateEnd.x, rotateEnd.y, { steps: 8 });
    await page.keyboard.up('Meta');
    await page.mouse.up();

    const rotated = await page.evaluate(({ shapeId, textId }) => {
      const PM = (window as any).PM;
      const shape = PM.L(shapeId), text = PM.L(textId);
      const axis = (layer: any) => {
        const matrix = PM.worldMatrix(layer, 1);
        return {
          x: { x: matrix[0], y: matrix[1] },
          y: { x: matrix[2], y: matrix[3] },
        };
      };
      return {
        rotations: [PM.ev(shape, 'rotation', 1), PM.ev(text, 'rotation', 1)],
        axes: { shape: axis(shape), text: axis(text) },
        positions: {
          shape: [PM.ev(shape, 'position.x', 1), PM.ev(shape, 'position.y', 1)],
          text: [PM.ev(text, 'position.x', 1), PM.ev(text, 'position.y', 1)],
        },
      };
    }, { shapeId: setup.shapeId, textId: setup.textId });
    const rotatedVector = (vector: { x: number; y: number }) => ({
      x: (vector.x - vector.y) * Math.SQRT1_2,
      y: (vector.x + vector.y) * Math.SQRT1_2,
    });
    for (const layer of ['shape', 'text'] as const) {
      for (const basis of ['x', 'y'] as const) {
        const expected = rotatedVector(setup.axes[layer][basis]);
        expect(rotated.axes[layer][basis].x).toBeCloseTo(expected.x, 3);
        expect(rotated.axes[layer][basis].y).toBeCloseTo(expected.y, 3);
      }
    }
    expect(rotated.rotations[0]).not.toBe(45);
    expect(rotated.rotations[1]).toBe(45);
    expect(rotated.positions.shape).not.toEqual(setup.positions.shape);
    expect(rotated.positions.text).not.toEqual(setup.positions.text);

    const rotationUndone = await page.evaluate(({ shapeId, textId }) => {
      const PM = (window as any).PM;
      const ok = PM.hist.undo(), shape = PM.L(shapeId), text = PM.L(textId);
      return {
        ok,
        rotations: [PM.ev(shape, 'rotation', 1), PM.ev(text, 'rotation', 1)],
        shapeTransform: {
          rotation: PM.ev(shape, 'rotation', 1),
          scaleX: PM.ev(shape, 'scale.x', 1),
          scaleY: PM.ev(shape, 'scale.y', 1),
          skew: PM.ev(shape, 'skew', 1),
        },
        positions: {
          shape: [PM.ev(shape, 'position.x', 1), PM.ev(shape, 'position.y', 1)],
          text: [PM.ev(text, 'position.x', 1), PM.ev(text, 'position.y', 1)],
        },
      };
    }, { shapeId: setup.shapeId, textId: setup.textId });
    expect(rotationUndone).toEqual({
      ok: true, rotations: [0, 0], shapeTransform: setup.shapeTransform, positions: setup.positions,
    });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
