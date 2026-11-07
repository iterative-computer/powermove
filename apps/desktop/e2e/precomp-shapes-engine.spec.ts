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

test('pill ends stay circular through preview and export, with a working stroke', async ({ session }) => {
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const canvas = (w: number, h: number) => {
      const cv = document.createElement('canvas'); cv.width = w; cv.height = h; return cv;
    };
    const render = (smoothing: number, radius = 50, w = 500, h = 100, exporting = true) => {
      const project = PM.mkProject({ w: 600, h: 180, dur: 3, bg: '#191919' });
      project.layers = [PM.mkLayer('shape', { d: { shape: 'rect', w, h, radius, smoothing,
        stroke: 6, strokeColor: '#a5a2ff', color: '#e8e2cf' },
        p: { 'position.x': 300, 'position.y': 90 } }, project)];
      PM.replaceProject(project);
      return PM.GL.renderToPixels(1, 600, 180, { opaque: true, topDownOpaque: true, mblur: false, exporting }) as Uint8Array;
    };
    const image = (pixels: Uint8Array) => {
      const cv = canvas(600, 180);
      cv.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(pixels), 600, 180), 0, 0);
      return cv;
    };
    const difference = (a: Uint8Array, b: Uint8Array) => {
      let max = 0; for (let i = 0; i < a.length; i++) max = Math.max(max, Math.abs(a[i]! - b[i]!)); return max;
    };
    const rows = [0, 60, 100].map(smoothing => render(smoothing));
    const images = rows.map(image);
    const overview = canvas(600, 600), context = overview.getContext('2d')!;
    context.fillStyle = '#191919'; context.fillRect(0, 0, 600, 600);
    images.forEach((cv, i) => {
      context.drawImage(cv, 0, i * 200 + 20);
      context.fillStyle = '#ffffff'; context.font = '16px sans-serif'; context.fillText(`Smoothing ${[0, 60, 100][i]}%`, 24, i * 200 + 22);
    });
    const closeup = canvas(1080, 670), ctx = closeup.getContext('2d')!;
    ctx.fillStyle = '#191919'; ctx.fillRect(0, 0, 1080, 670);
    images.forEach((cv, i) => {
      ctx.drawImage(cv, 490, 30, 70, 120, i * 360, 50, 350, 600);
      ctx.save(); ctx.translate(i * 360, 50); ctx.scale(5, 5); ctx.translate(-490, -30);
      // Analytic, true semicircle at the center of the rendered stroke.
      ctx.beginPath(); ctx.arc(500, 90, 50, -Math.PI / 2, Math.PI / 2);
      ctx.strokeStyle = '#42f5b0'; ctx.lineWidth = .35; ctx.setLineDash([1.2, 1.2]); ctx.stroke(); ctx.restore();
      ctx.fillStyle = '#ffffff'; ctx.font = '16px sans-serif'; ctx.fillText(`${[0, 60, 100][i]}% — green: true semicircle`, i * 360 + 8, 24);
    });
    const nearPlain = render(0, 49), nearSmooth = render(60, 49);
    const near = canvas(600, 400), nc = near.getContext('2d')!;
    nc.fillStyle = '#191919'; nc.fillRect(0, 0, 600, 400);
    [nearPlain, nearSmooth].forEach((pixels, i) => {
      nc.drawImage(image(pixels), 0, i * 200 + 20);
      nc.fillStyle = '#ffffff'; nc.font = '16px sans-serif'; nc.fillText(`Radius 49 / max 50 — smoothing ${i ? 60 : 0}%`, 24, i * 200 + 22);
    });
    const nearCloseup = canvas(1080, 690), ncc = nearCloseup.getContext('2d')!;
    ncc.fillStyle = '#191919'; ncc.fillRect(0, 0, 1080, 690);
    const heatmap = image(nearPlain), heatContext = heatmap.getContext('2d')!;
    const heat = heatContext.getImageData(0, 0, 600, 180);
    for (let i = 0; i < nearPlain.length; i += 4) {
      const delta = Math.max(...[0, 1, 2].map(c => Math.abs(nearPlain[i+c]! - nearSmooth[i+c]!)));
      heat.data[i] = Math.min(255, delta*12); heat.data[i+1] = Math.min(255, delta*6); heat.data[i+2] = 25; heat.data[i+3] = 255;
    }
    heatContext.putImageData(heat, 0, 0);
    [image(nearPlain), image(nearSmooth), heatmap].forEach((cv, i) => {
      ncc.drawImage(cv, 490, 30, 70, 120, i*360, 60, 350, 600);
      ncc.fillStyle = '#ffffff'; ncc.font = '16px sans-serif';
      ncc.fillText(['Radius 49: smoothing 0%', 'Radius 49: smoothing 60%', 'Difference amplified 12×'][i]!, i*360 + 8, 25);
    });
    const colorAt = (x: number, y: number) => Array.from(rows[1]!.slice((y * 600 + x) * 4, (y * 600 + x) * 4 + 3));
    return { overview: overview.toDataURL(), closeup: closeup.toDataURL(), near: near.toDataURL(), nearCloseup: nearCloseup.toDataURL(),
      pills: images.map(cv => cv.toDataURL()),
      roundDifference: Math.max(difference(rows[0]!, rows[1]!), difference(rows[0]!, rows[2]!)),
      previewDifference: difference(rows[1]!, render(60, 50, 500, 100, false)),
      verticalDifference: difference(render(0, 50, 100, 160), render(100, 50, 100, 160)),
      nearDifference: difference(nearPlain, nearSmooth), stroke: colorAt(300, 40), fill: colorAt(300, 90) };
  });
  const prefix = process.env.POWERMOVE_CORNER_SHOT_PREFIX || '02-after';
  await save(`${prefix}-pills-0-60-100.png`, result.overview);
  await save(`${prefix}-semicircle-overlay.png`, result.closeup);
  await save(`${prefix}-near-limit-radius49.png`, result.near);
  await save(`${prefix}-near-limit-closeup.png`, result.nearCloseup);
  for (const [i, smoothing] of [0, 60, 100].entries()) await save(`${prefix}-pill-${smoothing}.png`, result.pills[i]!);
  expect(result.roundDifference).toBeLessThanOrEqual(2);
  expect(result.verticalDifference).toBeLessThanOrEqual(2);
  expect(result.previewDifference).toBe(0);
  expect(result.nearDifference).toBeGreaterThan(2);
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
