import { expect, test } from './helpers/app';

/* Each scrub step invalidates the preview cache. Its canvas-sized targets must
   return to the working pool: allocating a fresh FBO per frame ran
   checkFramebufferStatus, which blocked the main thread until the GPU drained,
   so heavy projects scrubbed at a few frames per second and the drag lagged
   behind the pointer. */
test('scrubbing a value reuses framebuffers instead of allocating one per frame', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    // Hidden test windows cannot acquire pointer lock; the drag keeps working
    // on screen deltas, which exercises the same edit and render path.
    document.body.requestPointerLock = (() => Promise.reject(new Error('unavailable'))) as any;
    const PM = (window as any).PM;
    for (let i = 0; i < 3; i++) {
      PM.Edit.apply({ type: 'add_layer', layerType: 'solid', name: 'Layer ' + i, content: { w: 400, h: 400 } });
      PM.Edit.apply({ type: 'add_effect', target: PM.sel.layers[0], effect: 'blur', parameters: { amount: 12 } });
    }
    const w = window as any;
    w.__scrub = { allocations: 0, presents: 0 };
    const gl = PM.GL.gl, check = gl.checkFramebufferStatus.bind(gl);
    gl.checkFramebufferStatus = (target: number) => { w.__scrub.allocations++; return check(target); };
    PM.bus.on('preview:presented', () => { w.__scrub.presents++; });
  });
  const field = page.getByRole('spinbutton', { name: 'Scale X', exact: true }).first();
  await field.scrollIntoViewIfNeeded();
  const box = (await field.boundingBox())!;
  const x = box.x + box.width / 2, y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  // Let the first scrub frame settle the pool, then count steady-state frames.
  await page.mouse.move(x + 6, y);
  await page.evaluate(() => new Promise(requestAnimationFrame));
  await page.evaluate(() => new Promise(requestAnimationFrame));
  const start = await page.evaluate(() => ({ ...(window as any).__scrub }));
  for (let step = 1; step <= 20; step++) {
    await page.mouse.move(x + 6 + step * 4, y);
    await page.evaluate(() => new Promise(requestAnimationFrame));
  }
  const end = await page.evaluate(() => ({ ...(window as any).__scrub }));
  await page.mouse.up();
  expect(end.presents - start.presents).toBeGreaterThanOrEqual(10);
  expect(end.allocations - start.allocations).toBe(0);
  expect(Number(await field.getAttribute('aria-valuenow'))).toBeGreaterThan(100);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
