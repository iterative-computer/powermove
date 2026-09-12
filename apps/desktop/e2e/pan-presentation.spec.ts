import { test, expect } from './helpers/app';

test('high-zoom pans move the presented picture before another render is available', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Pan presentation', w: 1920, h: 1080, fps: 30, dur: 3 });
    p.layers = [PM.mkLayer('shape', { d: { shape: 'rect', w: 20, h: 20, color: '#dd5522' }, p: { 'position.x': 960, 'position.y': 540 } })];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p })); PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  });
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.Viewer?.stage));
  const result = await page.evaluate(() => {
    const PM = (window as any).PM, V = PM.Viewer;
    V.fit = false; V.zoom = 8; V.pan = [0, 0]; V.layout(); PM.GL.render(0, { mblur: false });
    const rect = V.el.getBoundingClientRect(), viewport = PM.GL.previewViewport;
    const image = V.el.toDataURL();
    V.pan = [32, -24]; V.layout();
    const moved = V.el.getBoundingClientRect();
    const small = { dx: moved.left - rect.left, dy: moved.top - rect.top, sameViewport: PM.GL.previewViewport === viewport, samePixels: V.el.toDataURL() === image };
    V.pan = [160, -24]; V.layout();
    const pending = V.el.getBoundingClientRect();
    const refresh = { dx: pending.left - rect.left, dy: pending.top - rect.top, newViewport: PM.GL.previewViewport !== viewport, samePixels: V.el.toDataURL() === image };
    const orangeCenter = () => {
      const gl = PM.GL.gl, w = V.el.width, h = V.el.height, pixels = new Uint8Array(w * h * 4);
      gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      let sumX = 0, sumY = 0, count = 0;
      for (let i = 0; i < pixels.length; i += 4) if (pixels[i]! > 200 && pixels[i+1]! > 60 && pixels[i+1]! < 110 && pixels[i+2]! < 60) {
        sumX += (i / 4) % w + .5; sumY += h - Math.floor(i / 4 / w) - .5; count++;
      }
      const box = V.el.getBoundingClientRect();
      return { x: box.left + sumX / count * box.width / w, y: box.top + sumY / count * box.height / h, count };
    };
    const beforeRefresh = orangeCenter();
    PM.GL.render(0, { mblur: false });
    const afterRefresh = orangeCenter();
    const oldWidth = V.el.getBoundingClientRect().width;
    V.zoom = 6; V.layout();
    const zoomWidth = V.el.getBoundingClientRect().width, pendingZoom = orangeCenter();
    PM.GL.render(0, { mblur: false });
    const settledZoom = orangeCenter();
    return { small, refresh, drift: { x: afterRefresh.x - beforeRefresh.x, y: afterRefresh.y - beforeRefresh.y }, visible: afterRefresh.count > 0,
      zoomRatio: zoomWidth / oldWidth, zoomDrift: { x: settledZoom.x - pendingZoom.x, y: settledZoom.y - pendingZoom.y } };

  });
  expect(result.small).toEqual({ dx: 32, dy: -24, sameViewport: true, samePixels: true });
  expect(result.refresh).toEqual({ dx: 160, dy: -24, newViewport: true, samePixels: true });
  expect(result.visible).toBe(true);
  expect(Math.abs(result.drift.x)).toBeLessThan(1);
  expect(Math.abs(result.drift.y)).toBeLessThan(1);
  expect(result.zoomRatio).toBeCloseTo(.75, 5);
  expect(Math.abs(result.zoomDrift.x)).toBeLessThan(1);
  expect(Math.abs(result.zoomDrift.y)).toBeLessThan(1);
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

test('paused zoom refines after settling and content or time changes bypass deferral', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Zoom refinement', w: 1920, h: 1080, fps: 30, dur: 3 });
    p.layers = [PM.mkLayer('shape', { d: { shape: 'rect', w: 20, h: 20, color: '#dd5522' }, p: { 'position.x': 960, 'position.y': 540 } })];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p })); PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  });
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.Viewer?.stage));
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM, V = PM.Viewer;
    V.fit = false; V.zoom = 8; V.pan = [0, 0]; V.layout(); PM.GL.render(0, { mblur: false });
    const original = PM.GL.render; let renders = 0;
    PM.GL.render = (...args: any[]) => { renders++; return original(...args); };
    const pinch = () => {
      const r = V.stage.getBoundingClientRect();
      V.stage.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, ctrlKey: true, deltaY: 5, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }));
    };
    PM.agentFrameCapture = false;
    try {
      pinch();
      const deferred = V.deferNavigationRender(performance.now());
      await new Promise(requestAnimationFrame);
      const duringGesture = renders;
      await new Promise(resolve => setTimeout(resolve, 140));
      const refined = renders > 0 && !V.deferNavigationRender(performance.now());
      const image = V.el.toDataURL();
      PM.rasterClear(); PM.GL.render(0, { mblur: false });
      const exact = image === V.el.toDataURL();
      pinch();
      const edit = PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'c.color', value: '#2255dd', mode: 'static', preserveHandEdits: false });
      const editImmediate = edit.ok && !V.deferNavigationRender(performance.now());
      PM.GL.render(PM.time, { mblur: false });
      pinch(); PM.setTime(.5);
      const scrubImmediate = !V.deferNavigationRender(performance.now());
      return { deferred, duringGesture, refined, exact, editImmediate, scrubImmediate, quality: PM.quality };
    } finally { PM.GL.render = original; PM.agentFrameCapture = true; }
  });
  expect(result).toEqual({ deferred: true, duringGesture: 0, refined: true, exact: true, editImmediate: true, scrubImmediate: true, quality: 1 });
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({ app }) => app.exit(0)).catch(() => undefined);
});

test('fractional pans reuse covered pixels while crop changes and independent redraws still render', async ({ session }) => {
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.mkProject({ name: 'Pan reuse', w: 1920, h: 1080, fps: 30, dur: 3 });
    p.layers = [PM.mkLayer('shape', { d: { shape: 'rect', w: 20, h: 20, color: '#dd5522' }, p: { 'position.x': 960, 'position.y': 540 } })];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: p })); PM.ProjectsScreen.hide(); PM.agentFrameCapture = true;
  });
  await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl && (window as any).PM?.Viewer?.stage));
  const result = await page.evaluate(async () => {
    const PM = (window as any).PM, V = PM.Viewer;
    V.fit = false; V.zoom = 4.371588852276498; V.pan = [0, 0]; V.layout();
    PM.agentFrameCapture = false;
    await new Promise(resolve => setTimeout(resolve, 100));
    const original = PM.GL.render; let renders = 0;
    PM.GL.render = (...args: any[]) => { renders++; return original(...args); };
    const pan = (deltaX: number) => {
      const r = V.stage.getBoundingClientRect();
      V.stage.dispatchEvent(new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaX, clientX: r.left+r.width/2, clientY:r.top+r.height/2 }));
    };
    const frame = async () => { await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); };
    try {
      const pixels = V.el.toDataURL(), left = V.el.getBoundingClientRect().left;
      for (let i = 0; i < 12; i++) { pan(3.25); await frame(); }
      const reused = renders === 0 && pixels === V.el.toDataURL();
      const moved = V.el.getBoundingClientRect().left - left;
      // A redraw that is already pending must survive a subsequent covered pan.
      PM.invalidate('render'); pan(1); await frame();
      const externalRedraw = renders > 0;
      const previous = renders;
      pan(180); await frame();
      const newRegion = renders > previous;
      const image = V.el.toDataURL();
      PM.rasterClear(); PM.GL.render(0, { mblur: false });
      const exact = image === V.el.toDataURL();
      const beforeEdit = renders;
      PM.Edit.apply({ type: 'set_property', target: PM.proj.layers[0].id, path: 'c.color', value: '#2255dd', mode: 'static', preserveHandEdits: false });
      pan(1); await frame();
      const edit = renders > beforeEdit;
      const beforeTime = renders; PM.setTime(.5); pan(1); await frame();
      const scrub = renders > beforeTime;
      return { reused, moved, externalRedraw, newRegion, exact, edit, scrub };
    } finally { PM.GL.render = original; PM.agentFrameCapture = true; }
  });
  expect(result).toEqual({ reused: true, moved: -39, externalRedraw: true, newRegion: true, exact: true, edit: true, scrub: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
  await page.evaluate(async () => await (window as any).PM.flushProject());
  await session.app.evaluate(({app})=>app.exit(0)).catch(()=>undefined);
});
