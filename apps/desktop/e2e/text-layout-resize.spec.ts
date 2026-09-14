import { expect, test } from './helpers/app';

for (const paragraph of [false, true]) {
  test(`reflows ${paragraph ? 'paragraph' : 'point'} text under a transformed parent and restores layout with undo`, async ({ session }, testInfo) => {
    await session.openEditor();
    const { page } = session;
    const id = await page.evaluate(async paragraph => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Text layout resize', w: 1000, h: 700, dur: 4 });
      const parent = PM.mkLayer('null', {
        p: { 'position.x': 500, 'position.y': 250, 'scale.x': 120, 'scale.y': 80, rotation: 12 },
      }, project);
      const text = PM.mkLayer('text', {
        d: { text: 'Make text wrap inside its box', font: 'Geist', size: 32, weight: 500, tracking: 0, leading: 1.2, align: 'center', color: '#FFFFFF' },
        p: { 'position.x': 0, 'position.y': 0, 'scale.x': 110, 'scale.y': 90, rotation: -7 },
      }, project);
      text.parent = parent.id;
      text.d.boxWidth = PM.P(paragraph ? 520 : 0);
      text.d.boxHeight = PM.P(0);
      if (paragraph) PM.setKeyOn(text.d.boxWidth, 0, 520, 'linear', 30);
      PM.setKeyOn(text.p['scale.x'], 0, 110, 'linear', 30);
      PM.setKeyOn(text.p['scale.y'], 0, 90, 'linear', 30);
      project.layers = [text, parent];
      PM.replaceProject(project); PM.setTime(1, { raw: true, force: true }); PM.selectLayers(text.id);
      PM.Kernel.services.get('tool').setTool('select');
      const viewer = PM.Kernel.services.get('viewer'); viewer.fit = true; viewer.layout();
      await document.fonts.ready;
      PM.hist.clear(); PM.invalidate();
      return text.id;
    }, paragraph);
    const snapshot = () => page.evaluate(id => {
      const PM = (window as any).PM, layer = PM.L(id), viewer = PM.Kernel.services.get('viewer');
      const bounds = PM.GL.bounds(layer, 1), matrix = PM.worldMatrix(layer, 1);
      const rect = viewer.inner.getBoundingClientRect();
      const point = (x: number, y: number) => ({
        x: rect.x + (matrix[0] * x + matrix[2] * y + matrix[4]) * viewer.shown,
        y: rect.y + (matrix[1] * x + matrix[3] * y + matrix[5]) * viewer.shown,
      });
      return {
        content: PM.resolveContent(layer, 1), bounds,
        scale: [PM.ev(layer, 'scale.x', 1), PM.ev(layer, 'scale.y', 1)],
        scaleKeys: [layer.p['scale.x'].kf, layer.p['scale.y'].kf],
        widthKeys: layer.d.boxWidth.kf.length,
        lines: PM.textLayout(layer, 1).lines.length,
        topLeft: point(bounds.x0, bounds.y0), center: point((bounds.x0 + bounds.x1) / 2, (bounds.y0 + bounds.y1) / 2),
        east: point(bounds.x1, (bounds.y0 + bounds.y1) / 2),
        southeast: point(bounds.x1, bounds.y1),
        shrink: { x: matrix[0] * bounds.w * -.55 * viewer.shown, y: matrix[1] * bounds.w * -.55 * viewer.shown },
        grow: { x: (matrix[0] * bounds.w + matrix[2] * bounds.h) * .2 * viewer.shown, y: (matrix[1] * bounds.w + matrix[3] * bounds.h) * .2 * viewer.shown },
      };
    }, id);
    const start = await snapshot();
    await page.mouse.move(start.east.x, start.east.y); await page.mouse.down();
    await page.mouse.move(start.east.x + start.shrink.x, start.east.y + start.shrink.y, { steps: 8 });
    await page.mouse.up();
    const wrapped = await snapshot();
    expect(wrapped.content.size).toBe(start.content.size);
    expect(wrapped.content.text).toBe(start.content.text);
    expect(wrapped.scale).toEqual(start.scale);
    expect(wrapped.scaleKeys).toEqual(start.scaleKeys);
    expect(wrapped.content.boxWidth).toBeGreaterThan(0);
    expect(wrapped.content.boxHeight).toBe(0);
    expect(wrapped.bounds.w).toBeCloseTo(start.bounds.w * .45, 0);
    expect(wrapped.lines).toBeGreaterThan(start.lines);
    expect(wrapped.bounds.h).toBeGreaterThan(start.bounds.h);
    expect(wrapped.topLeft.x).toBeCloseTo(start.topLeft.x, 1);
    expect(wrapped.topLeft.y).toBeCloseTo(start.topLeft.y, 1);
    expect(wrapped.widthKeys).toBe(paragraph ? 2 : 0);
    await page.screenshot({ path: testInfo.outputPath('wrapped-text.png') });

    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await snapshot()).toEqual(start);
    await page.evaluate(() => (window as any).PM.hist.redo());
    expect(await snapshot()).toEqual(wrapped);

    // Shift constrains the box; Option keeps its center in place.
    await page.mouse.move(wrapped.southeast.x, wrapped.southeast.y); await page.mouse.down();
    await page.keyboard.down('Shift'); await page.keyboard.down('Alt');
    await page.mouse.move(wrapped.southeast.x + wrapped.grow.x, wrapped.southeast.y + wrapped.grow.y, { steps: 8 });
    const corner = await snapshot();
    expect(corner.content.size).toBe(start.content.size);
    expect(corner.scale).toEqual(start.scale);
    expect(corner.bounds.w / corner.bounds.h).toBeCloseTo(wrapped.bounds.w / wrapped.bounds.h, 2);
    expect(corner.center.x).toBeCloseTo(wrapped.center.x, 1);
    expect(corner.center.y).toBeCloseTo(wrapped.center.y, 1);
    expect(corner.content.boxHeight).toBeGreaterThan(0);
    await page.evaluate(() => window.dispatchEvent(new PointerEvent('pointercancel')));
    await page.keyboard.up('Alt'); await page.keyboard.up('Shift'); await page.mouse.up();
    expect(await snapshot()).toEqual(wrapped);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
