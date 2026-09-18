import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

test('overlapping copies hold distinct frames without flickering while paused', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const original = PM.proj.layers.find((l: any) => l.type === 'video');
    const copy = structuredClone(original);
    copy.id = PM.uid('l');
    PM.proj.w = 320; PM.proj.h = 90;
    for (const [index, layer] of [original, copy].entries()) {
      layer.from = 0; layer.dur = 2;
      layer.d.trim = PM.P(index); layer.d.speed = PM.P(1);
      layer.d.w = PM.P(160); layer.d.h = PM.P(90);
      layer.p['position.x'].v = 80 + index * 160;
      layer.p['position.y'].v = 45;
    }
    PM.proj.layers = [original, copy];
    PM.ProjectIndex.invalidate(); PM.touch();
    PM.setTime(.25, { force: true });
    PM.Kernel.services.get('viewer').layout();
    PM.bus.emit('layers'); PM.invalidate();
  });
  // Read the actual interactive render repeatedly, not the offline renderer
  // that already snapshots each instance before moving a shared decoder.
  const samples = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const samples: number[][] = [];
    for (let i = 0; i < 40; i++) {
      await new Promise(resolve => setTimeout(resolve, 40));
      PM.GL.render(PM.time, { mblur: false });
      const gl = PM.GL.gl, canvas = PM.GL.canvas;
      const colors: number[] = [];
      for (const x of [.25, .75]) {
        const pixel = new Uint8Array(4);
        gl.readPixels(Math.floor(canvas.width * x), Math.floor(canvas.height / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
        colors.push(...pixel.slice(0, 3));
      }
      samples.push(colors);
    }
    return samples.slice(15);
  });
  for (const [red, green, , copyRed, copyGreen] of samples) {
    expect(red).toBeGreaterThan(220); expect(green).toBeLessThan(30);
    expect(copyRed).toBeLessThan(30); expect(copyGreen).toBeGreaterThan(220);
  }
  await page.evaluate(() => (window as any).PM.play());
  await page.waitForFunction(() => (window as any).PM.time > .4);
  const playing = await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.GL.render(PM.time, { mblur: false });
    const gl = PM.GL.gl, canvas = PM.GL.canvas;
    const result = [];
    for (const x of [.25, .75]) {
      const pixel = new Uint8Array(4);
      gl.readPixels(Math.floor(canvas.width * x), Math.floor(canvas.height / 2), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
      result.push([...pixel]);
    }
    PM.pause();
    return result;
  });
  expect(playing[0]![0]).toBeGreaterThan(220);
  expect(playing[1]![1]).toBeGreaterThan(220);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('double-click opens the file preview and leaves the timeline intact', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await importFixture(page, 'h264-aac.mp4');
  const card = page.locator('.asset-card').filter({ hasText: 'h264-aac.mp4' });
  await expect(card).toBeVisible();
  const before = await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers));
  await card.dblclick();
  const preview = page.locator('#source-preview');
  await expect(preview).toHaveAttribute('data-open', 'true');
  await expect(preview.locator('video')).toBeVisible();
  await expect(preview).toContainText('h264-aac.mp4');
  expect(await page.evaluate(() => JSON.stringify((window as any).PM.proj.layers))).toBe(before);
  await preview.getByRole('button', { name: 'Play source', exact: true }).click();
  await expect.poll(() => preview.locator('video').evaluate((video: HTMLVideoElement) => video.currentTime)).toBeGreaterThan(0);
  await preview.getByRole('button', { name: 'Close source preview', exact: true }).click();
  await expect(preview).toHaveAttribute('data-open', 'false');
  await card.getByRole('button', { name: 'Add h264-aac.mp4 to timeline', exact: true }).click();
  expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(JSON.parse(before).length + 1);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
