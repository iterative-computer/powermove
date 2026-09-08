import { expect, test } from './helpers/app';

test.describe('@viewer continuous rasterization', () => {
  test('renders editable text and shapes from source at their displayed density', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl));

    const proof = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Continuous source', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
      const shape = PM.mkLayer('shape', {
        name: 'Editable vector', dur: 4,
        d: { shape: 'ellipse', color: '#FFFFFF', w: 80, h: 60, radius: 0, stroke: 0, strokeColor: '#000000', points: 5 },
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 400, 'scale.y': 400 },
      }, project);
      const text = PM.mkLayer('text', {
        name: 'Editable title', dur: 4,
        d: { text: 'Vector source', font: 'Geist', weight: 700, size: 36, color: '#FF6B1A', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180, 'scale.x': 400, 'scale.y': 400 },
      }, project);
      project.layers = [shape, text];
      PM.replaceProject(project);
      PM.setTime(1, { raw: true, force: true });
      PM.Viewer.fit = true;
      PM.Viewer.layout();

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
      PM.Viewer.setZoom(8);
      PM.Viewer.layout();
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
      text: { type: 'text', text: 'Vector source', hasAsset: false },
    });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
