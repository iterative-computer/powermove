import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';

async function save(name: string, data: string) {
  const directory = process.env.POWERMOVE_ENGINE_SHOTS;
  if (!directory) return;
  await mkdir(directory, { recursive: true });
  await writeFile(path.join(directory, name), Buffer.from(data.split(',')[1]!, 'base64'));
}

test.beforeEach(async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
});

test('cropped precomp keeps parented shapes and text in place, with unchanged timing', async ({ session }) => {
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await document.fonts.ready;
    const project = PM.mkProject({ w: 480, h: 320, dur: 6, bg: '#191919' });
    const parent = PM.mkLayer('null', { p: { 'position.x': 160, 'position.y': 130, rotation: 27, 'scale.x': 120 } }, project);
    const shape = PM.mkLayer('shape', { from: 1, dur: 4, d: { shape: 'rect', w: 130, h: 70, radius: 24, smoothing: 60, color: '#e8e2cf', stroke: 4, strokeColor: '#a5a2ff' },
      p: { 'position.x': 0, 'position.y': 0 } }, project); shape.parent = parent.id;
    const text = PM.mkLayer('text', { from: 2, dur: 3, d: { text: 'Powermove', size: 26, color: '#a5a2ff' },
      p: { 'position.x': 290, 'position.y': 210, rotation: -12 } }, project);
    project.layers = [shape, text, parent]; PM.replaceProject(project); PM.time = 2.5;
    const render = () => PM.GL.renderToPixels(2.5, 480, 320, { transparent: true, mblur: false, exporting: true });
    const png = () => {
      const pixels = PM.GL.renderToPixels(2.5, 480, 320, { opaque: true, topDownOpaque: true, mblur: false, exporting: true });
      const canvas = document.createElement('canvas'); canvas.width = 480; canvas.height = 320;
      canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), 480, 320), 0, 0);
      return canvas.toDataURL();
    };
    const before = render(), beforePNG = png();
    const id = PM.Comps.precompose([shape.id, text.id], { sizeToSelection: true, adjustDuration: true });
    const after = render(), afterPNG = png();
    let max = 0, changed = 0;
    for (let i = 0; i < before.length; i++) { const diff = Math.abs(before[i] - after[i]); max = Math.max(max, diff); if (diff > 2) changed++; }
    const comp = PM.proj.comps[id], use = PM.proj.layers.find((l: any) => l.type === 'precomp');
    return { max, changed, w: comp.w, h: comp.h, dur: comp.dur, from: use.from, shapeFrom: shape.from, textFrom: text.from, beforePNG, afterPNG };
  });
  await save('03-before-precomp.png', result.beforePNG); await save('04-after-precomp.png', result.afterPNG);
  expect(result.w).toBeLessThan(480); expect(result.h).toBeLessThan(320);
  expect(result.dur).toBe(4); expect(result.from).toBe(1); expect(result.shapeFrom).toBe(0); expect(result.textFrom).toBe(1);
  expect(result.max).toBeLessThanOrEqual(2); expect(result.changed).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('pill smoothing renders the same through preview and export, with a working stroke', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ w: 600, h: 460, dur: 3, bg: '#191919' });
    project.layers = [0, 50, 100].map((smoothing, i) => PM.mkLayer('shape', { d: { shape: 'rect', w: 500, h: 100, radius: 50, smoothing, stroke: 6, strokeColor: '#a5a2ff', color: '#e8e2cf' },
      p: { 'position.x': 300, 'position.y': 75 + i * 145 } }, project));
    PM.replaceProject(project); PM.time = 1;
    const render = (exporting: boolean) => PM.GL.renderToPixels(1, 600, 460, { opaque: true, topDownOpaque: true, mblur: false, exporting });
    const preview = render(false), exported = render(true);
    let max = 0; for (let i = 0; i < preview.length; i++) max = Math.max(max, Math.abs(preview[i] - exported[i]));
    const canvas = document.createElement('canvas'); canvas.width = 600; canvas.height = 460;
    canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(exported), 600, 460), 0, 0);
    const colorAt = (x: number, y: number) => Array.from(exported.slice((y * 600 + x) * 4, (y * 600 + x) * 4 + 3));
    return { max, png: canvas.toDataURL(), colors: [0, 1, 2].map(i => colorAt(60, 40 + i * 145)),
      stroke: colorAt(300, 25), fill: colorAt(300, 75) };
  });
  await save('05-after-pill-render-preview-export.png', result.png);
  expect(result.max).toBe(0); expect(result.colors[0]).not.toEqual(result.colors[2]);
  expect(result.stroke).not.toEqual(result.fill);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('drawn shape command stores dimensions and keeps the original drawn box', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, project = PM.mkProject({ w: 480, h: 320, bg: '#191919' });
    const properties = { 'position.x': 230, 'position.y': 150, 'scale.x': 310, 'scale.y': 90 };
    project.layers = [PM.mkLayer('shape', { d: { shape: 'rect', w: 100, h: 100, radius: 0 }, p: properties }, project)];
    PM.replaceProject(project);
    const capture = () => {
      const pixels = PM.GL.renderToPixels(0, 480, 320, { opaque: true, topDownOpaque: true, exporting: true, mblur: false });
      const canvas = document.createElement('canvas'); canvas.width = 480; canvas.height = 320;
      canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), 480, 320), 0, 0);
      return canvas.toDataURL();
    };
    const before = capture();
    PM.replaceProject(PM.mkProject({ w: 480, h: 320, bg: '#191919' }));
    const applied = PM.Edit.apply({ type: 'add_layer', layerType: 'shape', content: { shape: 'rect', w: 100, h: 100, radius: 0 }, properties }, { origin: 'canvas' });
    const layer = PM.proj.layers[0];
    return { before, after: capture(), ok: applied.ok, width: layer.d.w, height: layer.d.h, sx: layer.p['scale.x'].v, sy: layer.p['scale.y'].v };
  });
  await save('06-before-drawn-rectangle.png', result.before); await save('07-after-drawn-rectangle.png', result.after);
  expect(result).toMatchObject({ ok: true, width: 310, height: 90, sx: 100, sy: 100 });
  expect(result.after).toBe(result.before);
});

test('public resize helper holds the opposite edge and supports centered proportional resizing', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const resize = (globalThis as any).__powermove_runtime.powermove.resizeParametricShape;
    const project = PM.mkProject({ w: 480, h: 320, bg: '#191919' });
    const layer = PM.mkLayer('shape', { d: { shape: 'rect', w: 140, h: 80, radius: 15 },
      p: { 'position.x': 155, 'position.y': 135, 'anchor.x': -70, rotation: 24 } }, project);
    project.layers = [layer]; PM.replaceProject(project); PM.time = 0;
    const png = () => {
      const pixels = PM.GL.renderToPixels(0, 480, 320, { opaque: true, topDownOpaque: true, exporting: true, mblur: false });
      const canvas = document.createElement('canvas'); canvas.width = 480; canvas.height = 320;
      canvas.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), 480, 320), 0, 0);
      return canvas.toDataURL();
    };
    const before = png();
    const normal = resize(layer, 'se', { x: 70, y: 20 }, {}, { time: 0 });
    const first = PM.Edit.apply(normal.commands, { origin: 'canvas' });
    const oppositeFixed = png();
    const centered = resize(layer, 'se', { x: -25, y: -15 }, { shift: true, alt: true }, { time: 0 });
    const second = PM.Edit.apply(centered.commands, { origin: 'canvas' });
    return { before, oppositeFixed, centered: png(), ok: first.ok && second.ok,
      proportion: centered.width / centered.height, previousProportion: normal.width / normal.height };
  });
  await save('08-before-parametric-resize.png', result.before);
  await save('09-after-opposite-fixed-resize.png', result.oppositeFixed);
  await save('10-after-centered-proportional-resize.png', result.centered);
  expect(result.ok).toBe(true); expect(result.proportion).toBeCloseTo(result.previousProportion, 8);
  expect(result.before).not.toBe(result.oppositeFixed); expect(result.oppositeFixed).not.toBe(result.centered);
});
