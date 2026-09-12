import { expect, test } from './helpers/app';

test('agent frame capture preserves the displayed canvas before the next animation frame', async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM.GL.canvas));
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause();
    const gl = PM.GL.gl as WebGL2RenderingContext;
    const canvas = PM.GL.canvas;
    const width = canvas.width, height = canvas.height;
    // A distinctive displayed frame, independent of the sample project's content.
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.clearColor(.8, .2, .4, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    const pixel = () => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const bytes = new Uint8Array(4);
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
      return [...bytes];
    };
    const before = pixel();
    const image = PM.Export.snapshot(0, 160);
    const after = pixel();
    const dimensions = [canvas.width, canvas.height];
    const originalRender = PM.GL.render;
    let failurePixel: number[];
    try {
      PM.GL.render = () => { throw new Error('capture failure'); };
      try { PM.Export.snapshot(0, 160); } catch {}
      failurePixel = pixel();
    } finally { PM.GL.render = originalRender; }
    return { before, after, failurePixel, dimensions, expectedDimensions: [width, height], image };
  });
  expect(result.image).toMatch(/^data:image\/jpeg;base64,/);
  expect(result.after).toEqual(result.before);
  expect(result.failurePixel).toEqual(result.before);
  expect(result.dimensions).toEqual(result.expectedDimensions);
});
