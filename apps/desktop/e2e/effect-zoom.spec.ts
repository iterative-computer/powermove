import { test, expect } from './helpers/app';

test('adding a neutral effect preserves sharp text at 800% zoom', async ({ session }, testInfo) => {
  await session.openEditor();
  const result = await session.page.evaluate(() => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    PM.agentFrameCapture = true;
    const project = PM.mkProject({ name: 'Effect zoom', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
    const text = PM.mkLayer('text', { d: { text: 'a', font: 'Arial', size: 40, color: '#ffffff', align: 'center' }, p: { 'position.x': 320, 'position.y': 165 } }, project);
    project.layers = [text]; PM.replaceProject(project);
    PM.perf.auto = false; PM.quality = 1; PM.previewResolution = '1';
    viewer.fit = false; viewer.zoom = 8; viewer.pan = [0, 0]; viewer.layout();
    const capture = () => {
      viewer.layout(); PM.GL.render(0, { mblur: false, exporting: true });
      const cv = PM.GL.canvas, gl = PM.GL.gl, pixels = new Uint8Array(cv.width * cv.height * 4);
      gl.readPixels(0, 0, cv.width, cv.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      return { width: cv.width, height: cv.height, viewport: PM.GL.previewViewport, image: cv.toDataURL(), pixels };
    };
    const before = capture();
    text.fx.push(PM.mkEffect('color')); PM.touch();
    const after = capture();
    text.fx.push(PM.mkEffect('blur')); PM.touch();
    const blur = capture();
    let different = 0, max = 0;
    if (before.pixels.length === after.pixels.length) for (let i = 0; i < before.pixels.length; i++) {
      const delta = Math.abs(before.pixels[i] - after.pixels[i]); if (delta > 1) different++; max = Math.max(max, delta);
    }
    return { before: { ...before, pixels: undefined }, after: { ...after, pixels: undefined }, blur: { ...blur, pixels: undefined }, different, max };
  });
  for (const key of ['before', 'after', 'blur'] as const) await testInfo.attach(key, { body: Buffer.from(result[key].image.split(',')[1]!, 'base64'), contentType: 'image/png' });
  expect(result.after.viewport).not.toBeNull();
  expect(result.blur.viewport).not.toBeNull();
  expect(result.blur.width).toBeGreaterThan(result.after.width);
  expect([result.after.width, result.after.height]).toEqual([result.before.width, result.before.height]);
  expect(result.different).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
