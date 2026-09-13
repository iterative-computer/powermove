import { expect, test } from './helpers/app';

test('selected layers show their transformed anchors at a constant screen size', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.octx); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const tool = PM.Kernel.services.get('tool');
    PM.replaceProject(PM.mkProject({ name: 'Selected anchors', w: 640, h: 360, fps: 30, dur: 4, bg: '#222222' }));
    const parent = PM.mkLayer('null', { name: 'Parent', dur: 4, p: {
      'position.x': 250, 'position.y': 150, rotation: 20, 'scale.x': 120, 'scale.y': 80,
    } }, PM.proj);
    const child = PM.mkLayer('shape', { name: 'Offset anchor', dur: 4, parent: parent.id,
      d: { shape: 'rect', color: '#6484AD', w: 140, h: 100, radius: 0, stroke: 0 },
      p: { 'position.x': 40, 'position.y': 25, 'anchor.x': 30, 'anchor.y': -15, rotation: 35 },
    }, PM.proj);
    const other = PM.mkLayer('shape', { name: 'Second anchor', dur: 4,
      d: { shape: 'ellipse', color: '#A48464', w: 90, h: 90, stroke: 0 },
      p: { 'position.x': 440, 'position.y': 240 },
    }, PM.proj);
    child.parent = parent.id;
    PM.proj.layers = [child, other, parent];
    PM.ProjectIndex.invalidate(); PM.invalidate();
    PM.setTime(1, { raw: true, force: true }); tool.setTool('select');
    PM.selectLayers([child.id, other.id]); viewer.showControls = true;
  });
  for (const zoom of [.5, 1.5]) {
    const result = await page.evaluate((zoom) => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      viewer.fit = false; viewer.zoom = zoom; viewer.pan = [17, -11]; viewer.layout();
      const expected = PM.selLayers().map((layer: any) => {
        const m = PM.worldMatrix(layer, PM.time), x = PM.ev(layer, 'anchor.x', PM.time), y = PM.ev(layer, 'anchor.y', PM.time);
        return { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5] };
      });
      const arcs: { x: number; y: number; radius: number }[] = [];
      const ctx = viewer.octx, original = ctx.arc;
      ctx.arc = function(x: number, y: number, radius: number, ...rest: any[]) {
        arcs.push({ x, y, radius: radius * viewer.shown });
        return original.call(this, x, y, radius, ...rest);
      };
      try { PM.bus.emit('overlay'); } finally { ctx.arc = original; }
      return { expected, arcs };
    }, zoom);
    for (const pivot of result.expected) {
      const marker = result.arcs.find((arc) => Math.abs(arc.x - pivot.x) < .001 && Math.abs(arc.y - pivot.y) < .001);
      expect(marker, `anchor at ${pivot.x}, ${pivot.y} at ${zoom} zoom`).toBeDefined();
      expect(marker!.radius).toBeCloseTo(5);
    }
  }
  await page.locator('#stage').screenshot({ path: '/private/tmp/powermove-selected-anchors.png' });
  const deselectedArcs = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const ctx = viewer.octx, original = ctx.arc;
    const ids = PM.selLayers().map((layer: any) => layer.id);
    PM.selectLayers([]);
    let count = 0;
    ctx.arc = function(...args: any[]) { count++; return original.apply(this, args); };
    try { PM.bus.emit('overlay'); } finally { ctx.arc = original; PM.selectLayers(ids); }
    return count;
  });
  expect(deselectedArcs).toBe(0);
  const hiddenArcs = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const ctx = viewer.octx, original = ctx.arc;
    let count = 0;
    ctx.arc = function(...args: any[]) { count++; return original.apply(this, args); };
    try { viewer.showControls = false; PM.bus.emit('overlay'); } finally { ctx.arc = original; }
    return count;
  });
  expect(hiddenArcs).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('clicking away commits canvas text and returns to Selection while explicit tool choices win', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.octx); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const tool = PM.Kernel.services.get('tool');
    PM.replaceProject(PM.mkProject({ name: 'Text click away', w: 640, h: 360, fps: 30, dur: 4 }));
    PM.setTime(1, { raw: true, force: true });
    viewer.fit = true; viewer.layout(); tool.setTool('text');
  });
  const frame = await page.locator('#stage-inner').boundingBox();
  if (!frame) throw new Error('Composition unavailable');
  await page.mouse.click(frame.x + frame.width * .3, frame.y + frame.height * .4);
  const editor = page.getByRole('textbox', { name: 'Edit text on canvas' });
  await editor.fill('Keep this text');
  expect(await page.evaluate(() => { const PM = (window as any).PM; const tool = PM.Kernel.services.get('tool'); return tool.tool; })).toBe('text');
  await page.mouse.click(frame.x + frame.width * .1, frame.y + frame.height * .1);
  await expect(editor).toHaveCount(0);
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const tool = PM.Kernel.services.get('tool');
    return { tool: tool.tool, count: PM.proj.layers.length, text: (PM.proj.layers[0].d.text?.v ?? PM.proj.layers[0].d.text) };
  });
  expect(result).toEqual({ tool: 'select', count: 1, text: 'Keep this text' });
  const undone = await page.evaluate(() => {
    const PM = (window as any).PM;
    // Click-away also records the now-undoable layer deselection.
    PM.hist.undo();
    PM.hist.undo();
    return { count: PM.proj.layers.length, text: (PM.proj.layers[0].d.text?.v ?? PM.proj.layers[0].d.text) };
  });
  expect(undone.count).toBe(1);
  expect(undone.text).not.toBe('Keep this text');
  await page.evaluate(() => { const PM = (window as any).PM; const tool = PM.Kernel.services.get('tool'); PM.hist.redo(); tool.setTool('text'); });
  await page.mouse.click(frame.x + frame.width * .3, frame.y + frame.height * .4);
  await expect(editor).toBeVisible();
  await page.getByRole('button', { name: 'Hand Tool (H)', exact: true }).click();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => { const PM = (window as any).PM; const tool = PM.Kernel.services.get('tool'); return tool.tool; })).toBe('hand');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
