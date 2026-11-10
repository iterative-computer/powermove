import { expect, test } from './helpers/app';

test('retired 3D inspector controls and subscriptions are collectible while the editor stays open', async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('studio-memory-regression');
    const added = api.scene3d.edit({ operation: 'add_object', object: { id: 'memory-sphere', source: { primitive: 'sphere' } } });
    if (!added.ok) throw new Error(added.message);
    PM.selectLayers([]);
    (window as any).__studioMemoryProof = { controls: [], callbacks: [], handles: [] };
    // Keep the returned handles, as callers can. Disposing must release the
    // callback even while both the API and its disposable handle remain live.
    for (let i = 0; i < 100; i++) {
      const callback = () => undefined;
      const subscription = api.on('time', callback);
      (window as any).__studioMemoryProof.callbacks.push(new WeakRef(callback));
      subscription.dispose();
      (window as any).__studioMemoryProof.handles.push(subscription);
    }
  });
  try {
    for (let i = 0; i < 12; i++) {
      await page.evaluate(() => (window as any).PM.selectLayers(['memory-sphere']));
      await page.locator('[data-inspector-layer="memory-sphere"]').waitFor();
      await page.evaluate(() => (window as any).__studioMemoryProof.controls.push(
        new WeakRef(document.querySelector('[data-inspector-layer="memory-sphere"]'))
      ));
      await page.evaluate(() => (window as any).PM.selectLayers([]));
      await page.locator('[data-inspector-layer="memory-sphere"]').waitFor({ state: 'detached' });
    }
    await expect.poll(async () => {
      await page.requestGC();
      return page.evaluate(() => {
        const proof = (window as any).__studioMemoryProof;
        return {
          controls: proof.controls.filter((ref: WeakRef<object>) => ref.deref()).length,
          callbacks: proof.callbacks.filter((ref: WeakRef<object>) => ref.deref()).length
        };
      });
    }, { timeout: 10_000 }).toEqual({ controls: 0, callbacks: 0 });
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await page.evaluate(() => { delete (window as any).__studioMemoryProof; });
  }
});
