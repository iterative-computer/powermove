/** G-C4: a paused native viewer at Full/100% shows the equal-size export.
 * The box is checked in composition and snapped device coordinates. This
 * display gate uses exact 0/1 primary-color geometry: the SDR BGRA surface
 * intentionally dithers fractional colors, while the snapshot API does not.
 * The renderer's fractional-color working-frame identity is checked in Rust. */
import { expect, test, launchApp } from './helpers/app';

test('paused Full/100% native viewer equals the export at its device box', async () => {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' }, args: ['--force-color-profile=srgb'] });
  try {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Viewer export agreement', w: 320, h: 180, fps: 30, dur: 2, bg: '#000000' });
      project.colorDepth = 16;
      project.layers = [
        ['red top left', '#FF0000', 8, 8, 32, 24],
        ['green top right', '#00FF00', 280, 8, 32, 24],
        ['blue bottom left', '#0000FF', 8, 148, 32, 24],
        ['white bottom right', '#FFFFFF', 280, 148, 32, 24],
        ['white centre bar', '#FFFFFF', 158, 30, 4, 120],
      ].map(([name, color, x, y, w, h]) => {
        const layer = PM.mkLayer('solid', { name, dur: 2, d: { color, w, h }, p: { opacity: 100 } }, project);
        // mkLayer's solid defaults deliberately reset position to the origin.
        layer.p['position.x'].v = x;
        layer.p['position.y'].v = y;
        return layer;
      });
      PM.replaceProject(project);
      PM.selectLayers([]);
      PM.setTime(0.5, { force: true });
      PM.pause();
      const viewer = PM.Kernel.services.get('viewer');
      viewer.fit = false;
      viewer.setZoom(1);
      viewer.pan = [0, 0];
      viewer.layout();
      PM.invalidate('render');
    });
    await page.locator('select[aria-label="Preview resolution"]').selectOption('1');
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
      const presentation = viewer?.enginePresentation, frame = presentation?.latest;
      const state = frame && presentation.stateFor(frame.viewSeq);
      return PM.EngineConnection?.status()?.route === 'engine' && presentation?.presenting
        && !!frame && !frame.standIn && frame.viewSeq === state?.viewSeq
        && frame.pts === Math.round(PM.time * 705_600_000) && state?.resolution === 'full';
    }), { timeout: 45_000 }).toBe(true);

    const proof = await page.evaluate(() => {
      const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
      const presentation = viewer.enginePresentation, frame = presentation.latest;
      const state = presentation.stateFor(frame.viewSeq), box = state.canvas;
      const canvas = presentation.canvas as HTMLCanvasElement;
      const stage = viewer.stage.getBoundingClientRect();
      const dprX = canvas.width / stage.width, dprY = canvas.height / stage.height;
      const left = Math.round(box.css.x * dprX), top = Math.round(box.css.y * dprY);
      const right = Math.round((box.css.x + box.css.width) * dprX);
      const bottom = Math.round((box.css.y + box.css.height) * dprY);
      const width = right - left, height = bottom - top;
      const copy = document.createElement('canvas');
      copy.width = canvas.width; copy.height = canvas.height;
      const context = copy.getContext('2d', { colorSpace: 'srgb', willReadFrequently: true })!;
      context.drawImage(canvas, 0, 0);
      const shown = context.getImageData(left, top, width, height).data;
      // Full renders at the snapped device box, not at the composition's
      // 320x180 logical dimensions. Compare every device pixel directly.
      const exported = PM.EngineRender.snapshotSync({
        time: PM.time, width, height,
        pixels: 'opaque', motionBlurSamples: 0, background: true,
      });
      let different = 0, maximum = 0;
      const differentByChannel = [0, 0, 0, 0];
      let firstDifference: { x: number; y: number; channel: number; preview: number; export: number } | null = null;
      const sample = (x: number, y: number) => ({
        preview: Array.from(shown.slice((y * width + x) * 4, (y * width + x) * 4 + 4)),
        export: Array.from(exported.data.slice((y * width + x) * 4, (y * width + x) * 4 + 4)),
      });
      const samples = {
        red: sample(48, 40), green: sample(592, 40),
        blue: sample(48, 320), white: sample(592, 320),
        centre: sample(320, 180), black: sample(200, 90),
      };
      const edges = {
        redX: [15, 16, 79, 80].map(x => sample(x, 40).preview),
        redY: [15, 16, 63, 64].map(y => sample(48, y).preview),
        greenX: [559, 560, 623, 624].map(x => sample(x, 40).preview),
        blueY: [295, 296, 343, 344].map(y => sample(48, y).preview),
        centreX: [315, 316, 323, 324].map(x => sample(x, 180).preview),
      };
      if (exported.width === width && exported.height === height) {
        for (let y = 0; y < height; y++) {
          for (let x = 0; x < width; x++) {
            for (let channel = 0; channel < 4; channel++) {
              const preview = shown[(y * width + x) * 4 + channel]!;
              const reference = exported.data[(y * width + x) * 4 + channel]!;
              const error = Math.abs(preview - reference);
              if (error) {
                different++;
                differentByChannel[channel]!++;
                firstDifference ??= { x, y, channel, preview, export: reference };
              }
              maximum = Math.max(maximum, error);
            }
          }
        }
      }
      return {
        route: PM.EngineConnection.status().route,
        zoom: state.zoom, resolution: state.resolution,
        crop: box.crop,
        css: box.css,
        stage: { width: stage.width, height: stage.height },
        devicePixelRatio: state.devicePixelRatio,
        canvas: [canvas.width, canvas.height],
        stateCanvas: [box.width, box.height],
        snapped: { left, top, right, bottom, width, height },
        export: [exported.width, exported.height],
        different, maximum, differentByChannel, firstDifference,
        samples, edges,
      };
    });

    console.log(`G-C4 ${JSON.stringify(proof)}`);

    expect(proof.route).toBe('engine');
    expect(proof.zoom).toBe(1);
    expect(proof.resolution).toBe('full');
    expect(proof.crop).toEqual({ x: 0, y: 0, width: 320, height: 180 });
    expect(proof.css).toEqual({
      x: (proof.stage.width - 320) / 2, y: (proof.stage.height - 180) / 2,
      width: 320, height: 180,
    });
    expect(proof.canvas).toEqual([
      Math.round(proof.stage.width * proof.devicePixelRatio),
      Math.round(proof.stage.height * proof.devicePixelRatio),
    ]);
    expect(proof.stateCanvas).toEqual([320, 180]);
    expect(proof.snapped.width / proof.stateCanvas[0]).toBe(proof.devicePixelRatio);
    expect(proof.snapped.height / proof.stateCanvas[1]).toBe(proof.devicePixelRatio);
    expect(proof.export).toEqual([proof.snapped.width, proof.snapped.height]);
    expect(proof.samples).toEqual({
      red: { preview: [255, 0, 0, 255], export: [255, 0, 0, 255] },
      green: { preview: [0, 255, 0, 255], export: [0, 255, 0, 255] },
      blue: { preview: [0, 0, 255, 255], export: [0, 0, 255, 255] },
      white: { preview: [255, 255, 255, 255], export: [255, 255, 255, 255] },
      centre: { preview: [255, 255, 255, 255], export: [255, 255, 255, 255] },
      black: { preview: [0, 0, 0, 255], export: [0, 0, 0, 255] },
    });
    const black = [0, 0, 0, 255], red = [255, 0, 0, 255];
    const green = [0, 255, 0, 255], blue = [0, 0, 255, 255], white = [255, 255, 255, 255];
    expect(proof.edges).toEqual({
      redX: [black, red, red, black], redY: [black, red, red, black],
      greenX: [black, green, green, black], blueY: [black, blue, blue, black],
      centreX: [black, white, white, black],
    });
    expect(proof.different, `preview/export maximum channel error ${proof.maximum}`).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session.close(); }
});
