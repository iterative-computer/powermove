import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('a newly activated video clip waits for a decoded frame before uploading', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((layer: any) => layer.type === 'video'));
  const initial = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause(); PM.perf.auto = false; PM.quality = 1;
    const original = PM.proj.layers.find((layer: any) => layer.type === 'video');
    original.from = 0; original.dur = 1;
    original.d.trim = PM.P(0);
    original.d.w = PM.P(160); original.d.h = PM.P(90);
    original.p['position.x'].v = 80; original.p['position.y'].v = 45;
    const copy = structuredClone(original);
    copy.id = PM.uid('l'); copy.from = 1; copy.d.trim = PM.P(1);
    PM.proj.w = 160; PM.proj.h = 90; PM.proj.layers = [original, copy];
    PM.ProjectIndex.invalidate(); PM.touch();
    PM.GL.previewViewport = null; PM.GL.resize(160, 90);
    PM.time = 0; PM.GL.render(0, { mblur: false });
    const gl = PM.GL.gl, upload = gl.texImage2D.bind(gl);
    const probe = { unavailableUploads: 0, videoUploads: 0, errors: [] as number[] };
    (window as any).videoUploadProbe = probe;
    gl.texImage2D = (...args: any[]) => {
      const source = args[5];
      if (source instanceof HTMLVideoElement) {
        probe.videoUploads++;
        if (source.readyState < source.HAVE_CURRENT_DATA || !source.videoWidth || !source.videoHeight) probe.unavailableUploads++;
      }
      return upload(...args);
    };
    // The second clip allocates its own decoder in this synchronous render.
    // No media events have run yet, so it cannot have a decoded frame.
    PM.time = 1; PM.GL.render(1, { mblur: false });
    const error = gl.getError();
    if (error !== gl.NO_ERROR) probe.errors.push(error);
    return { ...probe };
  });
  expect(initial.unavailableUploads).toBe(0);
  expect(initial.errors).toEqual([]);
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM;
    PM.GL.render(1, { mblur: false });
    const gl = PM.GL.gl, pixel = new Uint8Array(4);
    gl.readPixels(Math.floor(gl.drawingBufferWidth / 2), Math.floor(gl.drawingBufferHeight / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return pixel[1]! > 220 && pixel[0]! < 30;
  })).toBe(true);
  const probe = await page.evaluate(() => (window as any).videoUploadProbe);
  expect(probe.videoUploads).toBeGreaterThan(0);
  expect(probe.unavailableUploads).toBe(0);
  expect(session.diagnostics.console.filter(record => /texImage2D.*no video/.test(record.text))).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
