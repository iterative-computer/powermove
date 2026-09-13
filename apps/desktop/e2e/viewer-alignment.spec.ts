import { expect, test } from './helpers/app';

async function compositionPoint(page: any, x: number, y: number) {
  const box = await page.locator('#stage-inner').boundingBox();
  if (!box) throw new Error('viewer frame is unavailable');
  const shown = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });
  return { x: box.x + x * shown, y: box.y + y * shown };
}

test.describe('@viewer alignment snapping', () => {
  test('renders the composition center as full-axis guides with a fixed bullseye', async ({ session }) => {
  await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Kernel?.services?.get('viewer')?.octx));
    const rendering = await page.evaluate(() => {
      const PM = (window as any).PM, V = PM.Kernel.services.get('viewer');
      PM.replaceProject(PM.mkProject({ name: 'Center guide', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' }));
      V.fit = true; V.layout();
      const moving = V.snapCandidatesFromPoints(V.boxSnapPoints({ x0: 270, x1: 370, y0: 140, y1: 220 }));
      const composition = V.snapCandidatesFromPoints(V.boxSnapPoints(
        { x0: 0, x1: 640, y0: 0, y1: 360 }, 'composition',
      ));
      V.snapLines = V.snapBox(moving, composition, 1).lines;

      const segments: Array<[number, number, number, number]> = [], arcs: Array<[number, number, number]> = [];
      const ctx = V.octx, moveTo = ctx.moveTo, lineTo = ctx.lineTo, arc = ctx.arc;
      let start: [number, number] | null = null;
      ctx.moveTo = function(x: number, y: number) { start = [x, y]; return moveTo.call(this, x, y); };
      ctx.lineTo = function(x: number, y: number) {
        if (start) segments.push([start[0], start[1], x, y]);
        start = [x, y];
        return lineTo.call(this, x, y);
      };
      ctx.arc = function(x: number, y: number, radius: number, ...rest: any[]) {
        arcs.push([x, y, radius * V.shown]);
        return arc.call(this, x, y, radius, ...rest);
      };
      try { PM.bus.emit('overlay'); } finally {
        ctx.moveTo = moveTo; ctx.lineTo = lineTo; ctx.arc = arc; V.snapLines = null;
      }
      return { segments, arcs };
    });

    expect(rendering.segments).toContainEqual([320, 0, 320, 360]);
    expect(rendering.segments).toContainEqual([0, 180, 640, 180]);
    expect(rendering.arcs).toEqual(expect.arrayContaining([
      [320, 180, 6],
      [320, 180, 1.5],
    ]));
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('aligns both axes during an ordinary layer drag and allows Command to bypass snapping', async ({ session }) => {
  await session.openEditor();
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && (window as any).PM?.GL?.gl); });
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const tool = PM.Kernel.services.get('tool');
      const project = PM.mkProject({ name: 'Reliable alignment', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const moving = PM.mkLayer('shape', {
        name: 'Moving', dur: 4,
        d: { shape: 'rect', color: '#FFFFFF', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 140, 'position.y': 100 },
      }, project);
      const target = PM.mkLayer('shape', {
        name: 'Target', dur: 4,
        d: { shape: 'rect', color: '#FF6B1A', w: 100, h: 80, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 420, 'position.y': 180 },
      }, project);
      project.layers = [target, moving];
      PM.replaceProject(project);
      tool.setTool('select');
      PM.setTime(1, { raw: true, force: true });
      PM.selectLayers(moving.id);
      viewer.fit = true;
      viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      return { id: moving.id };
    });

    const start = await compositionPoint(page, 140, 100);
    const rawEnd = await compositionPoint(page, 316, 176);
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(rawEnd.x, rawEnd.y, { steps: 5 });
    await page.mouse.up();

    const snapped = await page.evaluate((id: string) => {
      const PM = (window as any).PM, layer = PM.L(id);
      return [PM.ev(layer, 'position.x', 1), PM.ev(layer, 'position.y', 1)];
    }, setup.id);
    expect(snapped[0]).toBeCloseTo(320, 4);
    expect(snapped[1]).toBeCloseTo(180, 4);

    expect(await page.evaluate(() => (window as any).PM.hist.undo())).toBe(true);
    await page.keyboard.down('Meta');
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(rawEnd.x, rawEnd.y, { steps: 5 });
    await page.mouse.up();
    await page.keyboard.up('Meta');

    const unsnapped = await page.evaluate((id: string) => {
      const PM = (window as any).PM, layer = PM.L(id);
      return [PM.ev(layer, 'position.x', 1), PM.ev(layer, 'position.y', 1)];
    }, setup.id);
    expect(unsnapped[0]).toBeCloseTo(316, 4);
    expect(unsnapped[1]).toBeCloseTo(176, 4);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
