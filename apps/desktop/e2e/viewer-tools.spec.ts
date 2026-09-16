import { expect, test, chooseNativeMenu, inspectNativeMenu } from './helpers/app';

async function cleanProject(page: any, name: string) {
  return page.evaluate(async (projectName: string) => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const tool = PM.Kernel.services.get('tool');
    const project = PM.mkProject({ name: projectName, w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
    PM.replaceProject(project);
    PM.setTime(1, { raw: true, force: true });
    PM.selectLayers([]);
    tool.setTool('select');
    viewer.fit = true;
    viewer.pan = [0, 0];
    viewer.layout();
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  }, name);
}

async function compositionPoint(page: any, x: number, y: number) {
  const box = await page.locator('#stage-inner').boundingBox();
  if (!box) throw new Error('viewer frame is unavailable');
  const shown = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });
  return { x: box.x + x * shown, y: box.y + y * shown, shown };
}

test.describe('@viewer After Effects tool behavior', () => {
  test('keeps tools distinct from creation commands and draws an undoable shape', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); const tool = PM.Kernel.services.get('tool'); return Boolean(viewer?.ov && tool?.setTool); });
    await cleanProject(page, 'Shape tool');

    const tools = await page.locator('#toolbar button[data-tool]').evaluateAll((buttons: HTMLElement[]) =>
      buttons.map((button) => button.dataset.tool));
    expect(tools).toEqual(['select', 'hand', 'shape', 'text']);
    await page.locator('#toolbar').screenshot({ path: '/private/tmp/powermove-toolbar.png' });
    const menu = await inspectNativeMenu(session, () => page.getByRole('button', { name: 'Selection and transform tools', exact: true }).click());
    expect(menu).toContainEqual({ label: 'Rotation Tool (W)', enabled: true });
    await page.locator('#toolbar button[data-tool="shape"]').click();
    expect(await page.evaluate(() => { const PM = (window as any).PM; const tool = PM.Kernel.services.get('tool'); return [tool.tool, tool.toolShape]; })).toEqual(['shape', 'rect']);

    const start = await compositionPoint(page, 100, 100);
    const end = await compositionPoint(page, 300, 220);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 8 });
    await page.mouse.up();

    const shape = await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers[0];
      return {
        count: PM.proj.layers.length,
        type: layer?.type,
        content: layer?.d,
        position: [layer?.p['position.x'].v, layer?.p['position.y'].v],
        scale: [layer?.p['scale.x'].v, layer?.p['scale.y'].v],
        selected: [...PM.sel.layers],
      };
    });
    expect(shape.count).toBe(1);
    expect(shape.type).toBe('shape');
    expect(shape.content).toMatchObject({ shape: 'rect', w: 100, h: 100 });
    expect(shape.position).toEqual([200, 160]);
    expect(shape.scale).toEqual([200, 120]);
    expect(shape.selected).toHaveLength(1);
    expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('marquee-selects enclosed layers and Shift toggles the enclosed set', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); const tool = PM.Kernel.services.get('tool'); return Boolean(viewer?.ov && tool?.setTool); });
    await cleanProject(page, 'Selection marquee');
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const left = PM.mkLayer('shape', {
        name: 'Left', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 160, 'position.y': 180 },
      }, PM.proj);
      const right = PM.mkLayer('shape', {
        name: 'Right', dur: 4,
        d: { shape: 'rect', color: '#FF6B1A', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 480, 'position.y': 180 },
      }, PM.proj);
      PM.proj.layers = [right, left];
      PM.ProjectIndex.invalidate(); PM.invalidate(); viewer.layout();
      return { left: left.id, right: right.id };
    });

    const start = await compositionPoint(page, 60, 100);
    const end = await compositionPoint(page, 260, 260);
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 8 }); await page.mouse.up();
    expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual([ids.left]);

    await page.keyboard.down('Shift');
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 8 }); await page.mouse.up();
    await page.keyboard.up('Shift');
    expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('Rotation, Pan Behind, Type, Zoom, and temporary Hand use professional gesture semantics', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); const tool = PM.Kernel.services.get('tool'); return Boolean(viewer?.ov && tool?.setTool); });
    await cleanProject(page, 'Transform tools');
    const setup = await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.mkLayer('shape', {
        name: 'Target', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 160, h: 100, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 320, 'position.y': 180 },
      }, PM.proj);
      PM.proj.layers = [layer]; PM.ProjectIndex.invalidate(); PM.selectLayers(layer.id); PM.invalidate();
      return { id: layer.id };
    });

    // Rotation tool: drag a quarter turn around the anchor; Shift constrains to 15°.
    await page.keyboard.press('w');
    const east = await compositionPoint(page, 390, 180);
    const south = await compositionPoint(page, 320, 250);
    await page.keyboard.down('Shift');
    await page.mouse.move(east.x, east.y); await page.mouse.down();
    await page.mouse.move(south.x, south.y, { steps: 8 }); await page.mouse.up();
    await page.keyboard.up('Shift');
    expect(await page.evaluate((id: string) => (window as any).PM.L(id).p.rotation.v, setup.id)).toBe(90);
    expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);

    // Pan Behind moves Anchor and compensates Position, preserving the world matrix.
    await page.keyboard.press('y');
    const beforeAnchor = await page.evaluate((id: string) => {
      const PM = (window as any).PM; const layer = PM.L(id);
      return { matrix: [...PM.worldMatrix(layer, 1)], anchor: [layer.p['anchor.x'].v, layer.p['anchor.y'].v] };
    }, setup.id);
    const pivot = await compositionPoint(page, 320, 180);
    await page.mouse.move(pivot.x, pivot.y); await page.mouse.down();
    await page.mouse.move(pivot.x + 40, pivot.y + 20, { steps: 8 }); await page.mouse.up();
    const afterAnchor = await page.evaluate((id: string) => {
      const PM = (window as any).PM; const layer = PM.L(id);
      return { matrix: [...PM.worldMatrix(layer, 1)], anchor: [layer.p['anchor.x'].v, layer.p['anchor.y'].v] };
    }, setup.id);
    expect(afterAnchor.anchor).not.toEqual(beforeAnchor.anchor);
    expect(afterAnchor.matrix).toEqual(beforeAnchor.matrix);

    // Command+T activates Type; a click creates editable source text at that point.
    await page.keyboard.press('Meta+t');
    const typeAt = await compositionPoint(page, 120, 80);
    await page.mouse.click(typeAt.x, typeAt.y);
    const text = await page.evaluate(() => {
      const PM = (window as any).PM;
      const tool = PM.Kernel.services.get('tool');
      const layer = PM.proj.layers[0];
      return { tool: tool.tool, type: layer.type, text: layer.d.text, position: [layer.p['position.x'].v, layer.p['position.y'].v] };
    });
    expect(text).toEqual({ tool: 'text', type: 'text', text: '', position: [120, 80] });

    // Dragging with Type creates an AE-style paragraph box whose dimensions
    // are ordinary animation channels, not static source metadata.
    await page.locator('#toolbar button[data-tool="text"]').click();
    const paragraphStart = await compositionPoint(page, 430, 60);
    const paragraphEnd = await compositionPoint(page, 590, 150);
    await page.mouse.move(paragraphStart.x, paragraphStart.y); await page.mouse.down();
    await page.mouse.move(paragraphEnd.x, paragraphEnd.y, { steps: 8 }); await page.mouse.up();
    const paragraph = await page.evaluate(() => {
      const PM = (window as any).PM; const layer = PM.proj.layers[0];
      return {
        type: layer.type,
        position: [layer.p['position.x'].v, layer.p['position.y'].v],
        box: [layer.d.boxWidth.v, layer.d.boxHeight.v],
        channels: PM.allProps(layer).filter((item: any) => ['c.boxWidth', 'c.boxHeight'].includes(item.key)).map((item: any) => item.key),
      };
    });
    expect(paragraph).toEqual({
      type: 'text', position: [430, 60], box: [160, 90],
      channels: ['c.boxWidth', 'c.boxHeight'],
    });

    // Zoom follows the pointer; Space temporarily pans and restores the active tool.
    await chooseNativeMenu(session, 'Zoom Tool (Z)', () => page.getByRole('button', { name: 'Navigation tools', exact: true }).click());
    const zoomAt = await compositionPoint(page, 160, 90);
    await page.mouse.click(zoomAt.x, zoomAt.y);
    const zoomed = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); const tool = PM.Kernel.services.get('tool'); return ({ tool: tool.tool, fit: viewer.fit, zoom: viewer.zoom }); });
    expect(zoomed.tool).toBe('zoom'); expect(zoomed.fit).toBe(false); expect(zoomed.zoom).toBeGreaterThan(0);

    await chooseNativeMenu(session, 'Selection Tool (V)', () => page.getByRole('button', { name: 'Selection and transform tools', exact: true }).click());
    await page.mouse.move(zoomAt.x, zoomAt.y);
    await page.keyboard.down('Space');
    await expect(page.locator('#stage-inner')).toHaveCSS('cursor', 'grab');
    await page.mouse.down(); await page.mouse.move(zoomAt.x + 35, zoomAt.y + 15, { steps: 5 }); await page.mouse.up();
    await page.keyboard.up('Space');
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); const tool = PM.Kernel.services.get('tool'); return [tool.tool, viewer.temporaryTool]; })).toEqual(['select', null]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
