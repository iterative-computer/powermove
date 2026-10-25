import { expect, test } from './helpers/app';

test.describe('@viewer continuous rasterization', () => {
  test('renders editable text and shapes from source at their displayed density', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      return Boolean(viewer?.ov && PM.GL?.gl);
    });

    const proof: any = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Continuous source', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const shape = PM.mkLayer('shape', {
        name: 'Editable vector', dur: 4,
        d: { shape: 'ellipse', color: '#FFFFFF', w: 80, h: 60, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 400, 'scale.y': 400 },
      }, project);
      const text = PM.mkLayer('text', {
        name: 'Editable title', dur: 4,
        d: { text: 'X', font: 'Geist', weight: 700, size: 36, color: '#FF6B1A', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 400, 'scale.y': 400 },
      }, project);
      // Layer zero is frontmost. Show the editable title above the ellipse so
      // the native path can verify both sources in the presented pixels.
      project.layers = [text, shape];
      PM.replaceProject(project);
      PM.setTime(1, { raw: true, force: true });
      const viewer = PM.Kernel.services.get('viewer');
      viewer.fit = true;
      viewer.layout();
      if (PM.EngineConnection?.status()?.route === 'engine') {
        PM.invalidate('render');
        return { native: true, source: {
          shape: { type: shape.type, shape: shape.d.shape, hasAsset: Object.hasOwn(shape.d, 'asset') },
          text: { type: text.type, text: text.d.text, hasAsset: Object.hasOwn(text.d, 'asset') },
        } };
      }

      const samples: Array<{ id: string; scale: number; backingWidth: number; logicalWidth: number }> = [];
      const raster = PM.raster;
      PM.raster = (layer: any, scale: number, time: number) => {
        const result = raster(layer, scale, time);
        if (layer?.id === shape.id || layer?.id === text.id) {
          samples.push({ id: layer.id, scale, backingWidth: result.cv.width, logicalWidth: result.w });
        }
        return result;
      };
      PM.invalidate('render');
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));

      const world = PM.worldMatrix(shape, 1);
      const sx = PM.GL.canvas.width / project.w, sy = PM.GL.canvas.height / project.h;
      const matrix = [world[0] * sx, world[1] * sy, world[2] * sx, world[3] * sy];
      const aa = matrix[0] ** 2 + matrix[1] ** 2;
      const bb = matrix[0] * matrix[2] + matrix[1] * matrix[3];
      const cc = matrix[2] ** 2 + matrix[3] ** 2;
      const largest = Math.sqrt(Math.max(0, (aa + cc + Math.sqrt(Math.max(0, (aa - cc) ** 2 + 4 * bb ** 2))) / 2));
      const expectedScale = Math.ceil(Math.max(.25, Math.min(32, largest)) * 4 - 1e-9) / 4;
      const sampleFor = (id: string) => samples.filter((sample) => sample.id === id).sort((a, b) => b.scale - a.scale)[0];
      viewer.setZoom(8);
      viewer.layout();
      PM.invalidate('render');
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const highZoomShape = sampleFor(shape.id);
      const canvasRect = PM.GL.canvas.getBoundingClientRect();
      const highZoom = {
        viewport: { ...PM.GL.previewViewport },
        canvas: {
          width: PM.GL.canvas.width, height: PM.GL.canvas.height,
          cssWidth: canvasRect.width, cssHeight: canvasRect.height,
        },
        shapeScale: highZoomShape?.scale,
      };
      PM.raster = raster;
      return {
        expectedScale,
        shapeSample: sampleFor(shape.id),
        textSample: sampleFor(text.id),
        highZoom,
        source: {
          shape: { type: shape.type, shape: shape.d.shape, hasAsset: Object.hasOwn(shape.d, 'asset') },
          text: { type: text.type, text: text.d.text, hasAsset: Object.hasOwn(text.d, 'asset') },
        },
      };
    });

    if (proof.native) {
      const colors = () => page.evaluate(() => {
        const canvas = (window as any).PM.Kernel.services.get('viewer').enginePresentation.canvas as HTMLCanvasElement;
        const copy = document.createElement('canvas');
        copy.width = canvas.width; copy.height = canvas.height;
        const context = copy.getContext('2d', { willReadFrequently: true })!;
        context.drawImage(canvas, 0, 0);
        const pixels = context.getImageData(0, 0, copy.width, copy.height).data;
        let orange = 0, white = 0;
        for (let i = 0; i < pixels.length; i += 4) {
          if (pixels[i]! > 180 && pixels[i + 1]! > 80 && pixels[i + 1]! < 160 && pixels[i + 2]! < 90) orange++;
          if (pixels[i]! > 230 && pixels[i + 1]! > 230 && pixels[i + 2]! > 230) white++;
        }
        return { orange, white };
      });
      await expect.poll(() => page.evaluate(() => {
        const PM = (window as any).PM, presentation = PM.Kernel.services.get('viewer').enginePresentation;
        const frame = presentation?.latest;
        return !!frame && !frame.standIn && frame.revision === PM.EngineConnection.status().revision
          && frame.pts === Math.round(PM.time * 705_600_000);
      }), { timeout: 30_000 }).toBe(true);
      await expect.poll(async () => (await colors()).orange, { timeout: 30_000 }).toBeGreaterThan(100);
      await page.evaluate(() => {
        const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
        viewer.setZoom(8); viewer.pan = [0, 0]; viewer.layout(); PM.invalidate('render');
      });
      await expect.poll(() => page.evaluate(() => {
        const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
        const presentation = viewer.enginePresentation, frame = presentation?.latest;
        const state = frame && presentation.stateFor(frame.viewSeq);
        if (!presentation?.presenting || !frame || frame.standIn || !state?.canvas
          || frame.revision !== PM.EngineConnection.status().revision
          || frame.pts !== Math.round(PM.time * 705_600_000) || state.zoom !== 8) return null;
        const canvas = presentation.canvas, rect = canvas.getBoundingClientRect();
        return { route: PM.EngineConnection.status().route, width: canvas.width, height: canvas.height,
          render: [state.canvas.width, state.canvas.height], crop: [state.canvas.crop.width, state.canvas.crop.height],
          css: [rect.width, rect.height], visible: getComputedStyle(canvas).visibility === 'visible' };
      }), { timeout: 30_000 }).toMatchObject({ route: 'engine', visible: true });
      const native = await page.evaluate(() => {
        const presentation = (window as any).PM.Kernel.services.get('viewer').enginePresentation;
        const frame = presentation.latest, state = presentation.stateFor(frame.viewSeq), canvas = presentation.canvas;
        const rect = canvas.getBoundingClientRect();
        return { width: canvas.width, height: canvas.height, render: [state.canvas.width, state.canvas.height],
          crop: [state.canvas.crop.width, state.canvas.crop.height], css: [rect.width, rect.height],
          rect: { x: Math.ceil(rect.x), y: Math.ceil(rect.y), width: Math.floor(rect.width), height: Math.floor(rect.height) } };
      });
      expect(native.crop[0]).toBeLessThan(640);
      expect(native.crop[1]).toBeLessThan(360);
      // The cropped composition may have a pan buffer larger than the
      // displayed stage. Its own crop, not the stage canvas, sets density.
      expect(native.render[0] / native.crop[0]).toBeGreaterThanOrEqual(1);
      expect(native.render[1] / native.crop[1]).toBeGreaterThanOrEqual(1);
      expect(native.width / native.css[0]).toBeGreaterThanOrEqual(1);
      expect(native.height / native.css[1]).toBeGreaterThanOrEqual(1);
      const { orange, white } = await colors();
      console.log(`native editable source pixels ${JSON.stringify({ orange, white, canvas: [native.width, native.height], render: native.render, crop: native.crop })}`);
      expect(orange).toBeGreaterThan(100);
      expect(white).toBeGreaterThan(100);
      expect(proof.source).toEqual({
        shape: { type: 'shape', shape: 'ellipse', hasAsset: false },
        text: { type: 'text', text: 'X', hasAsset: false },
      });
      expect(session.diagnostics.pageErrors).toEqual([]);
      return;
    }

    expect(proof.expectedScale).toBeGreaterThan(1);
    for (const sample of [proof.shapeSample, proof.textSample]) {
      expect(sample?.scale).toBeGreaterThanOrEqual(proof.expectedScale);
      expect(sample?.backingWidth).toBeGreaterThan(sample!.logicalWidth);
    }
    expect(proof.highZoom.viewport.width).toBeLessThan(640);
    expect(proof.highZoom.viewport.height).toBeLessThan(360);
    expect(proof.highZoom.canvas.width / proof.highZoom.canvas.cssWidth).toBeGreaterThanOrEqual(1);
    expect(proof.highZoom.canvas.height / proof.highZoom.canvas.cssHeight).toBeGreaterThanOrEqual(1);
    expect(proof.highZoom.shapeScale).toBe(32);
    expect(proof.source).toEqual({
      shape: { type: 'shape', shape: 'ellipse', hasAsset: false },
      text: { type: 'text', text: 'X', hasAsset: false },
    });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
