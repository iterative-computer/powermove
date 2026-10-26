import { expect, test } from './helpers/app';

test.describe('@viewer gradient ramp effect', () => {
  test('anchors linear and radial ramps to the layer through transforms and resizing', async ({ session }) => {
    await session.openEditor();
    const proof = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Attached gradient', w: 256, h: 256, dur: 4, bg: '#000000' });
      const layer = PM.mkLayer('solid', { d: { color: '#FFFFFF', w: 16, h: 16 } }, project);
      layer.p['position.x'].v = 80;
      layer.p['position.y'].v = 80;
      const effect = PM.mkEffect('gradient',1);
      for (const [key, value] of Object.entries({ angle: 0, startColor: '#FF0000', endColor: '#0000FF', dither: 0 })) effect.p[key].v = value;
      layer.fx.push(effect);
      project.layers = [layer];
      PM.replaceProject(project);
      const sample = () => {
        PM.touch();
        const canvas = PM.renderFrameTo(1, 256, 256) as HTMLCanvasElement;
        const ctx = canvas.getContext('2d')!;
        const bounds = PM.GL.bounds(layer, 1), m = PM.worldMatrix(layer, 1);
        return [[2.5 / 16, 8.5 / 16], [8.5 / 16, 8.5 / 16], [13.5 / 16, 8.5 / 16]].map(([u, v]) => {
          const x = bounds.x0 + u * bounds.w, y = bounds.y0 + v * bounds.h;
          return Array.from(ctx.getImageData(Math.floor(m[0] * x + m[2] * y + m[4]), Math.floor(m[1] * x + m[3] * y + m[5]), 1, 1).data);
        });
      };
      const variants = () => {
        const original = sample();
        layer.p['scale.x'].v = 300; layer.p['scale.y'].v = 500;
        const scaled = sample();
        layer.p['position.x'].v = 140; layer.p['position.y'].v = 60;
        const moved = sample();
        layer.p.rotation.v = 90;
        const rotated = sample();
        layer.p['anchor.x'].v = 8; layer.p['anchor.y'].v = 8;
        const anchored = sample();
        const parent = PM.mkLayer('null', { p: { 'position.x': 120, 'position.y': 100, 'anchor.x': 0, 'anchor.y': 0, 'scale.x': 300, 'scale.y': 300, rotation: 90 } }, project);
        project.layers.push(parent);
        layer.parent = parent.id;
        layer.p['position.x'].v = 0; layer.p['position.y'].v = 0;
        layer.p['scale.x'].v = 100; layer.p['scale.y'].v = 100; layer.p.rotation.v = 0;
        const parented = sample();
        layer.parent = null; project.layers.pop();
        layer.p['position.x'].v = 80; layer.p['position.y'].v = 80;
        layer.p['anchor.x'].v = 0; layer.p['anchor.y'].v = 0;
        layer.d.w = 48; layer.d.h = 48;
        const resized = sample();
        layer.d.w = 16; layer.d.h = 16;
        return { original, scaled, moved, rotated, anchored, parented, resized };
      };
      const linear = variants();
      effect.p.radial.v = true;
      effect.p.centerX.v = 25; effect.p.centerY.v = -25;
      const radial = variants();
      return { linear, radial };
    });
    expect(proof.linear.original[0][0]).toBeGreaterThan(200);
    expect(proof.linear.original[2][2]).toBeGreaterThan(200);
    for (const ramp of [proof.linear, proof.radial]) {
      for (const [name, variant] of Object.entries(ramp)) {
        variant.forEach((pixel, index) => pixel.forEach((channel, c) => {
          expect(Math.abs(channel - ramp.original[index][c]), `${name}: ${JSON.stringify(variant)} vs ${JSON.stringify(ramp.original)}`).toBeLessThanOrEqual(3);
        }));
      }
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
    expect(session.diagnostics.console.filter((entry) => entry.type === 'error')).toEqual([]);
  });

  test('keeps shape and text gradients attached in cropped previews and exports', async ({ session }) => {
    await session.openEditor();
    const proof = await session.page.evaluate(() => {
      const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
      PM.agentFrameCapture = true;
      const project = PM.mkProject({ name: 'Gradient zoom', w: 640, h: 360, dur: 4, bg: '#000000' });
      PM.replaceProject(project);
      PM.perf.auto = false; PM.quality = 1; PM.previewResolution = '1';
      viewer.fit = false; viewer.zoom = 8; viewer.pan = [0, 0];
      const results: Array<{ type: string; scale: number; cropped: boolean; preview: number; exported: number; maxError: number }> = [];
      for (const type of ['shape', 'text']) {
        const layer = PM.mkLayer(type, { d: type === 'shape'
          ? { w: 64, h: 64, radius: 0, color: '#FFFFFF' }
          : { text: 'H', font: 'Arial', size: 64, color: '#FFFFFF', align: 'center' } }, project);
        layer.p['position.x'].v = 320; layer.p['position.y'].v = 180;
        const effect = PM.mkEffect('gradient',1);
        for (const [key, value] of Object.entries({ angle: 0, startColor: '#FF0000', endColor: '#0000FF', dither: 0 })) effect.p[key].v = value;
        layer.fx.push(effect); project.layers = [layer];
        for (const scale of [100, 300]) {
          layer.p['scale.x'].v = scale; layer.p['scale.y'].v = scale;
          PM.touch(); viewer.layout();
          PM.GL.render(1, { mblur: false, exporting: true });
          const cv = PM.GL.canvas, gl = PM.GL.gl, viewport = PM.GL.previewViewport;
          const pixels = new Uint8Array(cv.width * cv.height * 4);
          gl.readPixels(0, 0, cv.width, cv.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
          const bounds = PM.GL.bounds(layer, 1);
          let maxError = 0;
          const check = (data: ArrayLike<number>, w: number, h: number, area: { x: number; y: number; width: number; height: number }) => {
            let count = 0;
            for (let y = 0; y < h; y += 3) for (let x = 0; x < w; x += 3) {
              const i = (y * w + x) * 4;
              if (data[i] + data[i + 2] < 250) continue; // Fully covered source pixels only.
              const localX = (area.x + (x + .5) * area.width / w - 320) / (scale / 100);
              const u = Math.max(0, Math.min(1, (localX - bounds.x0) / bounds.w));
              maxError = Math.max(maxError, Math.abs(data[i] - 255 * (1 - u)), Math.abs(data[i + 2] - 255 * u));
              count++;
            }
            return count;
          };
          // WebGL readback is bottom-up; this horizontal ramp is constant in Y.
          const preview = check(pixels, cv.width, cv.height, viewport);
          const exported = check(PM.GL.renderToPixels(1, 640, 360, { mblur: false }), 640, 360, { x: 0, y: 0, width: 640, height: 360 });
          results.push({ type, scale, cropped: !!viewport && viewport.width < 640, preview, exported, maxError });
        }
      }
      return results;
    });
    for (const result of proof) {
      expect(result.cropped).toBe(true);
      expect(result.preview, `${result.type} preview at ${result.scale}%`).toBeGreaterThan(20);
      expect(result.exported, `${result.type} export at ${result.scale}%`).toBeGreaterThan(20);
      expect(result.maxError, `${result.type} at ${result.scale}%`).toBeLessThan(6);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
    expect(session.diagnostics.console.filter((entry) => entry.type === 'error')).toEqual([]);
  });

  test('ramps linearly, radially, and only inside the layer alpha', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));

    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Gradient proof', w: 16, h: 16, fps: 30, dur: 4, bg: '#000000' });
      const fill = PM.mkLayer('solid', {
        name: 'White fill', from: 0, dur: 4, d: { color: '#FFFFFF', w: 16, h: 16 },
      }, project);
      const effect = PM.mkEffect('gradient',1);
      fill.fx.push(effect);
      project.layers = [fill];
      PM.replaceProject(project);

      const pixel = (x: number, y: number) => {
        const canvas = PM.renderFrameTo(1, 16, 16) as HTMLCanvasElement;
        return Array.from(canvas.getContext('2d')!.getImageData(x, y, 1, 1).data) as number[];
      };
      const set = (values: Record<string, unknown>) => {
        for (const [key, value] of Object.entries(values)) effect.p[key].v = value;
        PM.touch();
      };

      /* Defaults: white → black travelling top to bottom at 90°. */
      const top = pixel(8, 1), middle = pixel(8, 8), bottom = pixel(8, 14);

      set({ angle: 0, startColor: '#FF0000', endColor: '#0000FF' });
      const left = pixel(1, 8), right = pixel(14, 8);

      set({ angle: 90, startColor: '#FFFFFF', endColor: '#000000', radial: true });
      const center = pixel(8, 8), corner = pixel(1, 1);

      set({ radial: false, amount: 0 });
      const off = pixel(8, 14);

      /* A smaller solid proves the ramp respects the layer's own alpha. */
      const small = PM.mkProject({ name: 'Gradient alpha proof', w: 16, h: 16, fps: 30, dur: 4, bg: '#000000' });
      const patch = PM.mkLayer('solid', {
        name: 'Small fill', from: 0, dur: 4, d: { color: '#FFFFFF', w: 4, h: 4 },
      }, small);
      patch.p['position.x'].v = 6;
      patch.p['position.y'].v = 6;
      patch.fx.push(PM.mkEffect('gradient',1));
      small.layers = [patch];
      PM.replaceProject(small);
      const outside = pixel(1, 1), inside = pixel(7, 7);

      return { top, middle, bottom, left, right, center, corner, off, outside, inside };
    });

    const value = (rgba: number[]) => rgba[0]!;
    /* Linear ramp: bright at the start edge, dark at the end edge, monotonic. */
    expect(value(proof.top)).toBeGreaterThan(value(proof.middle));
    expect(value(proof.middle)).toBeGreaterThan(value(proof.bottom));
    expect(value(proof.top)).toBeGreaterThan(220);
    expect(value(proof.bottom)).toBeLessThan(60);

    /* Angle 0 runs the ramp left to right between the two colours. */
    expect(proof.left[0]).toBeGreaterThan(200);
    expect(proof.left[2]).toBeLessThan(60);
    expect(proof.right[2]).toBeGreaterThan(200);
    expect(proof.right[0]).toBeLessThan(60);

    /* Radial: the start colour sits at the centre, the end colour at the rim. */
    expect(value(proof.center)).toBeGreaterThan(220);
    expect(value(proof.corner)).toBeLessThan(60);

    /* Amount 0 leaves the source untouched. */
    expect(proof.off.slice(0, 3)).toEqual([255, 255, 255]);

    /* Transparent pixels keep the background; the ramp paints the solid only. */
    expect(proof.outside.slice(0, 3)).toEqual([0, 0, 0]);
    expect(value(proof.inside)).toBeGreaterThan(60);

    expect(session.diagnostics.pageErrors).toEqual([]);
    expect(session.diagnostics.console.filter((entry) => entry.type === 'error')).toEqual([]);
  });
});
