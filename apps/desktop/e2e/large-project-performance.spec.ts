import { test, expect } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', {
      detail: PM.mkProject({ name: 'Large project regression', w: 640, h: 360, fps: 30, dur: 2 }),
    }));
    PM.ProjectsScreen.hide();
    PM.agentFrameCapture = true;
  });
  await session.page.waitForFunction(() => { const PM = (window as any).PM, timeline = PM.Kernel.services.get('timeline'); return Boolean(PM?.GL?.gl && timeline?.cv); });
});

test.afterEach(async ({ session }) => {
  await session.page.evaluate(async () => { await (window as any).PM.flushProject(); });
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

test('670-layer previews reuse uploaded sources after CPU eviction and recover after GPU eviction', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj.layers = Array.from({ length: 670 }, (_, i) => PM.mkLayer('shape', {
      name: `Shape ${i}`, p: { 'position.x': 25 + i % 25 * 24, 'position.y': 15 + Math.floor(i / 25) * 12 },
      d: { shape: 'rect', w: 70, h: 35, radius: i % 7, stroke: 0, color: `#${(0x200000 + i * 991).toString(16)}` },
    }));
    PM.ProjectIndex.invalidate(); PM.touch();
    const render = () => PM.GL.renderToPixels(0, 640, 360, { mblur: false });
    const before = render(); render();
    const create = document.createElement.bind(document);
    let canvases = 0, uploads = 0;
    document.createElement = ((name: string, ...rest: any[]) => {
      if (name === 'canvas') canvases++;
      return (create as any)(name, ...rest);
    }) as any;
    const upload = PM.GL.gl.texImage2D.bind(PM.GL.gl);
    PM.GL.gl.texImage2D = (...args: any[]) => { uploads++; return upload(...args); };
    PM.Memory.setBudget('raster', 1024 * 1024);
    const after = render();
    const warm = { canvases, uploads, rasterBytes: PM.rasterStats().bytes, textures: PM.GL.memoryStats().textures };
    PM.GL.dropTextures('r:');
    const restored = render();
    const rebuilt = { canvases, uploads };
    const same = (pixels: Uint8Array) => pixels.every((v, i) => v === before[i]);
    const layer = PM.proj.layers[0];
    PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.color', value: '#ff0000', mode: 'static', preserveHandEdits: false });
    const edited = render();
    PM.hist.undo();
    const undone = render();
    return { warm, rebuilt, after: same(after), restored: same(restored), changed: !same(edited), undone: same(undone) };
  });
  expect(result.warm.canvases).toBe(0);
  expect(result.warm.uploads).toBe(0);
  expect(result.warm.rasterBytes).toBeLessThanOrEqual(1024 * 1024);
  expect(result.warm.textures.bytes).toBeLessThanOrEqual(result.warm.textures.maxBytes);
  expect(result.rebuilt.canvases).toBeGreaterThan(0);
  expect(result.rebuilt.uploads).toBeGreaterThan(0);
  expect(result).toMatchObject({ after: true, restored: true, changed: true, undone: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('cached text, shape animation and scaled exports match freshly rasterized pixels', async ({ session }) => {
  const results = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const shape = PM.mkLayer('shape', { p: { 'position.x': 220, 'position.y': 160 }, d: { shape: 'rect', w: 120, h: 90, radius: 12, color: '#ee7722' } });
    const text = PM.mkLayer('text', { p: { 'position.x': 320, 'position.y': 200 }, d: { text: 'Editable type', size: 28, color: '#ffffff' } });
    PM.proj.layers = [text, shape]; PM.ProjectIndex.invalidate(); PM.touch();
    for (const [layer, channel, start, end] of [[shape, 'c.color', '#ee7722', '#2277ee'], [shape, 'c.w', 120, 220], [text, 'c.text', 'Editable type', 'Updated type'], [text, 'scale.x', 100, 160]] as any[]) {
      PM.Edit.apply({ type: 'set_property', target: layer.id, path: channel, value: start, mode: 'keyframe', time: 0, preserveHandEdits: false });
      PM.Edit.apply({ type: 'set_property', target: layer.id, path: channel, value: end, mode: 'keyframe', time: 1, preserveHandEdits: false });
    }
    const result = [];
    for (const width of [320, 640, 1280]) for (const time of [0, .5, 1, .5, 0]) {
      const render = () => PM.GL.renderToPixels(time, width, width * 9 / 16, { mblur: false });
      const cached = render();
      PM.rasterClear();
      const fresh = render();
      result.push({ width, time, same: fresh.every((v: number, i: number) => v === cached[i]), sum: fresh.reduce((a: number, b: number) => a + b, 0) });
    }
    return result;
  });
  expect(results.every(result => result.same)).toBe(true);
  for (const width of [320, 640, 1280]) {
    const atSize = results.filter(result => result.width === width);
    expect(atSize[0]!.sum).not.toBe(atSize[2]!.sum);
    expect(atSize[0]!.sum).toBe(atSize[4]!.sum);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('thousands of tiny uploaded sources remain bounded even below the GPU byte budget', async ({ session }) => {
  const stats = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.proj.layers = Array.from({ length: 4200 }, (_, index) => PM.mkLayer('shape', {
      d: { w: 1, h: 1, color: `#${(0x100000 + index).toString(16)}`, stroke: 0 },
    }));
    PM.ProjectIndex.invalidate(); PM.touch();
    PM.GL.renderToPixels(0, 64, 36, { mblur: false });
    return PM.GL.memoryStats().textures;
  });
  expect(stats.entries).toBe(4096);
  expect(stats.bytes).toBeLessThan(stats.maxBytes);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('matte lookup follows edits, inverted alpha and nested composition passes', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const matte = PM.mkLayer('shape', { p: { 'position.x': 320, 'position.y': 180 }, d: { shape: 'rect', w: 100, h: 100, color: '#ffffff', stroke: 0 } });
    const source = PM.mkLayer('solid', { p: { 'position.x': 0, 'position.y': 0 }, d: { w: 640, h: 360, color: '#ff0000' } });
    PM.proj.layers = [matte, source]; PM.ProjectIndex.invalidate(); PM.touch();
    const read = () => {
      const pixels = PM.GL.renderToPixels(0, 640, 360, { transparent: true, mblur: false });
      return { center: Array.from(pixels.slice((180 * 640 + 320) * 4, (180 * 640 + 320) * 4 + 4)), corner: Array.from(pixels.slice(0, 4)) };
    };
    PM.Edit.apply({ type: 'set_layer', target: source.id, patch: { matteSource: matte.id } });
    const alpha = read();
    PM.Edit.apply({ type: 'set_property', target: source.id, path: 'l.matteMode', value: 'alpha-inverted', preserveHandEdits: false });
    const inverted = read();
    PM.Edit.apply({ type: 'set_layer', target: source.id, patch: { matteSource: null } });
    const detached = read();
    PM.hist.undo();
    const restored = read();
    const sub = PM.mkProject({ name: 'Nested matte', w: 640, h: 360, fps: 30, dur: 2 });
    sub.layers = PM.proj.layers;
    PM.proj.comps[sub.id] = sub;
    PM.proj.layers = [PM.mkLayer('precomp', { d: { comp: sub.id, w: 640, h: 360 } })];
    PM.ProjectIndex.invalidate(); PM.touch();
    const nested = read();
    return { alpha, inverted, detached, restored, nested };
  });
  expect(result.alpha.center).toEqual([255, 0, 0, 255]);
  expect(result.alpha.corner[3]).toBe(0);
  expect(result.inverted.center[3]).toBe(0);
  expect(result.inverted.corner).toEqual([255, 0, 0, 255]);
  expect(result.detached.center).toEqual([255, 255, 255, 255]);
  expect(result.restored).toEqual(result.inverted);
  expect(result.nested).toEqual(result.inverted);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
