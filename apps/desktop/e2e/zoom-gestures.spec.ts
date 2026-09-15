import { expect, test } from './helpers/app';

for (const panel of ['viewer', 'timeline'] as const) {
  test(`${panel} zoom responds promptly, reverses cleanly, and preserves its cursor anchor`, async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const pointer = await page.evaluate(async (panel) => {
      const PM = (window as any).PM;
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const view = PM.Kernel.services.get(panel);
      if (panel === 'viewer') {
        view.fit = false; view.zoom = .5; view.pan = [0, 0]; view.layout();
      } else {
        view.pps = 100; view.scrollT = 2; PM.invalidate('timeline');
      }
      const rect = (panel === 'viewer' ? view.stage : view.cv).getBoundingClientRect();
      return {
        x: Math.round(rect.left + (panel === 'viewer' ? rect.width * .65 : view.gut + 120)),
        y: Math.round(rect.top + rect.height * .4),
      };
    }, panel);
    const state = () => page.evaluate(({ panel, pointer }) => {
      const view = (window as any).PM.Kernel.services.get(panel);
      if (panel === 'viewer') {
        const rect = view.inner.getBoundingClientRect();
        return { scale: view.shown, anchor: [(pointer.x - rect.left) / view.shown, (pointer.y - rect.top) / view.shown] };
      }
      const rect = view.cv.getBoundingClientRect();
      return { scale: view.pps, anchor: [view.scrollT + (pointer.x - rect.left - view.gut) / view.pps] };
    }, { panel, pointer });
    const gesture = async (delta: number, modifier: 'Control' | 'Meta') => {
      await page.keyboard.down(modifier);
      for (let i = 0; i < 12; i++) await page.mouse.wheel(0, delta);
      await page.keyboard.up(modifier);
      // Wheel delivery is asynchronous in Chromium; read after it is painted.
      await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
      return state();
    };
    await page.mouse.move(pointer.x, pointer.y);
    const initial = await state();
    const magnified = await gesture(-10, 'Control');
    expect(magnified.scale / initial.scale).toBeGreaterThan(1.8);
    expect(magnified.scale / initial.scale).toBeLessThan(2.5);
    const restored = await gesture(10, 'Control');
    expect(restored.scale).toBeCloseTo(initial.scale, 6);
    initial.anchor.forEach((value, index) => {
      // Browser layout rounds fractional CSS positions. Allow under one pixel
      // of screen drift, including accumulated rounding over the full gesture.
      expect(Math.abs(magnified.anchor[index]! - value) * magnified.scale).toBeLessThan(1);
      expect(Math.abs(restored.anchor[index]! - value) * restored.scale).toBeLessThan(1);
    });
    const commandZoom = await gesture(-10, 'Meta');
    expect(commandZoom.scale / restored.scale).toBeGreaterThan(1.8);
    await gesture(10, 'Meta');
    const beforePan = await state();
    await page.evaluate((panel) => {
      const view = (window as any).PM.Kernel.services.get(panel);
      (panel === 'viewer' ? view.stage : view.cv).dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, deltaX: 12.5, deltaY: 8.25,
      }));
    }, panel);
    const afterPan = await state();
    expect(afterPan.scale).toBe(beforePan.scale);
    expect(afterPan.anchor).not.toEqual(beforePan.anchor);
    await page.evaluate((panel) => {
      const view = (window as any).PM.Kernel.services.get(panel);
      (panel === 'viewer' ? view.stage : view.cv).dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, deltaY: -3, deltaMode: 1, ctrlKey: true,
      }));
    }, panel);
    const lineRatio = (await state()).scale / afterPan.scale;
    expect(lineRatio).toBeGreaterThan(1.8);
    expect(lineRatio).toBeLessThanOrEqual(2);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
