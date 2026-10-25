import { expect, test, launchApp } from './helpers/app';

test('native drawing prepares while the viewer is absent and is claimed after reload', async () => {
  const active = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await active.openEditor();
    const { page } = active;
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting)).toBe(true);
    const proof = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const warmup = PM.Kernel.services.get('engine.viewer')?.drawingWarmup;
      if (!warmup) throw new Error('Native presentation warmup is unavailable');
      const warm = warmup.warm, take = warmup.take;
      let prepared = 0, claimed = 0, claimedCanvas: HTMLCanvasElement | null = null;
      warmup.warm = function (...args: any[]) { prepared++; return warm.apply(warmup, args); };
      warmup.take = function (...args: any[]) {
        const drawing = take.apply(warmup, args);
        if (drawing) { claimed++; claimedCanvas = drawing.canvas; }
        return drawing;
      };
      try {
        await PM.Kernel.loader.reload('viewer');
        await PM.Kernel.loader.whenIdle();
        PM.invalidate('render');
        const viewer = PM.Kernel.services.get('viewer');
        return { prepared, claimed, sameCanvas: viewer?.enginePresentation?.canvas === claimedCanvas };
      } finally { warmup.warm = warm; warmup.take = take; }
    });
    expect(proof).toEqual({ prepared: 1, claimed: 1, sameCanvas: true });
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM, presentation = PM.Kernel.services.get('viewer')?.enginePresentation;
      return presentation?.presenting && !!presentation.latest && !presentation.latest.standIn;
    }), { timeout: 30_000 }).toBe(true);
    expect(active.diagnostics.pageErrors).toEqual([]);
  } finally { await active.close(); }
});

test('2D playback leaves the unused 3D gizmo idle and selection wakes it again', async ({ session }) => {
  await session.openEditor();
  const { page } = session;
  await page.waitForSelector('[data-scene-gizmo]', { state: 'attached' });
  const allocations = await page.evaluate(() => {
    const PM = (window as any).PM, api = PM.Kernel.api('unallocated-gizmo-proof');
    const stage = document.createElement('div'); document.body.append(stage);
    const getContext = HTMLCanvasElement.prototype.getContext;
    const requests: HTMLCanvasElement[] = [];
    HTMLCanvasElement.prototype.getContext = function (kind: any, options?: any) {
      if (kind === 'webgl' || kind === 'webgl2' || kind === 'experimental-webgl') requests.push(this);
      return getContext.call(this, kind, options);
    } as typeof getContext;
    let controller: ReturnType<typeof api.scene3d.createGizmo> | null = null;
    try {
      controller = api.scene3d.createGizmo(stage);
      const canvas = stage.querySelector<HTMLCanvasElement>('[data-scene-gizmo]')!;
      const plain = PM.mkLayer('solid'); PM.proj.layers.push(plain); PM.ProjectIndex.invalidate(); PM.touch();
      const rect = { x: 0, y: 0, width: 320, height: 180 };
      controller.update(rect, []); controller.update(rect, [plain.id]); controller.update(rect, []);
      const result = { contexts: requests.filter(source => source === canvas).length,
        hidden: canvas.hidden, hover: controller.hover(0, 0), active: controller.active(), covers: controller.covers([plain.id]) };
      controller.dispose(); controller = null;
      return { ...result, remainingCanvases: stage.querySelectorAll('canvas').length };
    } finally { controller?.dispose(); stage.remove(); HTMLCanvasElement.prototype.getContext = getContext; }
  });
  expect(allocations).toEqual({ contexts: 0, hidden: true, hover: false, active: false, covers: false, remainingCanvases: 0 });

  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.Kernel.api('idle-gizmo-proof').scene3d.edit({ operation: 'add_object', object: { id: 'model', source: { primitive: 'box' } } });
    PM.selectLayers(['model']); PM.invalidate();
  });
  await expect(page.locator('[data-scene-gizmo]')).toBeVisible();
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  const idle = await page.evaluate(async () => {
    const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer');
    const canvas = document.querySelector<HTMLCanvasElement>('[data-scene-gizmo]')!;
    const stage = canvas.parentElement!, gl = canvas.getContext('webgl2')!;
    const append = stage.append, clear = gl.clear;
    let themeReads = 0, clears = 0;
    stage.append = function (...nodes) {
      themeReads += nodes.filter(node => node instanceof HTMLSpanElement && node.style.visibility === 'hidden').length;
      append.apply(this, nodes);
    };
    gl.clear = function (mask) { clears++; clear.call(this, mask); };
    try {
      PM.selectLayers([]);
      await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      themeReads = clears = 0;
      for (let i = 0; i < 12; i++) {
        viewer.sceneGizmo.update({ x: 0, y: 0, width: 320, height: 180 }, []);
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      }
      return { themeReads, clears, hidden: canvas.hidden };
    } finally { stage.append = append; gl.clear = clear; }
  });
  expect(idle).toEqual({ themeReads: 0, clears: 0, hidden: true });
  await expect(page.locator('[data-scene-gizmo]')).toBeHidden();
  await page.evaluate(() => { const PM = (window as any).PM; PM.selectLayers(['model']); PM.invalidate(); });
  await expect(page.locator('[data-scene-gizmo]')).toBeVisible();
  await expect(page.locator('#composition-3d-controls')).toBeVisible();
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a deferred 3D gizmo restores its GPU context and can be disposed and recreated', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, api = PM.Kernel.api('gizmo-recovery-proof');
    api.scene3d.edit({ operation: 'add_object', object: { id: 'recovery-model', source: { primitive: 'box' } } });
    const stage = document.createElement('div'); stage.style.cssText = 'position:fixed;left:-10000px;width:320px;height:180px';
    document.body.append(stage);
    const getContext = HTMLCanvasElement.prototype.getContext, requests: HTMLCanvasElement[] = [];
    HTMLCanvasElement.prototype.getContext = function (kind: any, options?: any) {
      if (kind === 'webgl' || kind === 'webgl2' || kind === 'experimental-webgl') requests.push(this);
      return getContext.call(this, kind, options);
    } as typeof getContext;
    const rect = { x: 0, y: 0, width: 320, height: 180 };
    let controller: ReturnType<typeof api.scene3d.createGizmo> | null = null;
    const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const event = (canvas: HTMLCanvasElement, name: string) => new Promise<void>((resolve, reject) => {
      const done = (value: Event) => { value.preventDefault(); clearTimeout(timer); resolve(); };
      const timer = setTimeout(() => { canvas.removeEventListener(name, done); reject(new Error(`Gizmo did not emit ${name}`)); }, 4000);
      canvas.addEventListener(name, done, { once: true });
    });
    try {
      controller = api.scene3d.createGizmo(stage);
      const canvas = stage.querySelector<HTMLCanvasElement>('[data-scene-gizmo]')!;
      const before = requests.filter(source => source === canvas).length;
      controller.update(rect, ['recovery-model']);
      const first = requests.filter(source => source === canvas).length;
      await nextFrame();
      const gl = getContext.call(canvas, 'webgl2') as WebGL2RenderingContext;
      const loss = gl.getExtension('WEBGL_lose_context')!;
      const lost = event(canvas, 'webglcontextlost'); loss.loseContext(); await lost;
      await new Promise(resolve => setTimeout(resolve, 100));
      const restored = event(canvas, 'webglcontextrestored'); loss.restoreContext(); await restored;
      controller.update(rect, ['recovery-model']); await nextFrame();
      const recovery = { sameCanvas: stage.querySelector('[data-scene-gizmo]') === canvas, hidden: canvas.hidden,
        lost: gl.isContextLost(), error: gl.getError(), active: controller.active() };
      controller.dispose(); controller = null;
      const afterDispose = stage.querySelectorAll('canvas').length;
      controller = api.scene3d.createGizmo(stage);
      const nextCanvas = stage.querySelector<HTMLCanvasElement>('[data-scene-gizmo]')!;
      const nextBefore = requests.filter(source => source === nextCanvas).length;
      controller.update(rect, ['recovery-model']); await nextFrame();
      return { before, first, recovery, afterDispose, nextBefore,
        nextContexts: requests.filter(source => source === nextCanvas).length, newCanvas: nextCanvas !== canvas,
        remainingCanvases: stage.querySelectorAll('canvas').length, shown: !nextCanvas.hidden };
    } finally { controller?.dispose(); stage.remove(); HTMLCanvasElement.prototype.getContext = getContext; }
  });
  expect(result).toEqual({ before: 0, first: 1,
    recovery: { sameCanvas: true, hidden: false, lost: false, error: 0, active: false },
    afterDispose: 0, nextBefore: 0, nextContexts: 1, newCanvas: true, remainingCanvases: 1, shown: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
