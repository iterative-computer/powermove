import type { Page } from 'playwright';
import { expect, test } from './helpers/app';

async function waitForViewer(page: Page): Promise<void> {
  // A fresh profile starts on Projects, whose overlay intercepts wheel/drag
  // input even though the viewer canvas has already initialized behind it.
  await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Viewer recovery', w: 1920, h: 1080, fps: 30, dur: 4 }) }));
    PM.ProjectsScreen.hide();
  });
  await page.waitForFunction(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return Boolean(
    viewer?.ov && (window as any).PM?.GL?.gl
      && document.querySelector('#composition-recovery'),
  ); });
}

async function sampleViewerPixels(page: Page): Promise<{ visible: boolean; color: number[] | null }> {
  return page.evaluate(() => {
    const PM = (window as any).PM;
    if (PM.EngineRender) {
      const viewer = PM.Kernel.services.get('viewer');
      const presentation = viewer?.enginePresentation;
      const latest = presentation?.latest;
      const frame = viewer?.engineFrame?.();
      const revision = PM.Kernel.services.get('engine.viewer')?.revision?.();
      const canvas = document.querySelector<HTMLCanvasElement>('#stage #engine-present');
      const gl = document.querySelector<HTMLCanvasElement>('#gl');
      const visible = presentation?.presenting === true && latest != null && frame != null
        && latest.seq === frame.seq && latest.revision === frame.revision
        && latest.viewSeq === frame.viewSeq && frame.standIn === false
        && revision != null && frame.revision === revision
        && frame.pts === Math.max(0, Math.round(PM.time * 705_600_000))
        && canvas != null && canvas === presentation.canvas && canvas.width > 0 && canvas.height > 0
        && canvas.getBoundingClientRect().width > 0 && canvas.getBoundingClientRect().height > 0
        && getComputedStyle(canvas).visibility === 'visible'
        && gl != null && getComputedStyle(gl).visibility === 'hidden';
      if (!visible || !canvas) return { visible: false, color: null };
      const copy = document.createElement('canvas');
      copy.width = canvas.width; copy.height = canvas.height;
      const ctx = copy.getContext('2d')!;
      ctx.drawImage(canvas, 0, 0);
      return { visible: true, color: Array.from(ctx.getImageData(Math.floor(copy.width / 2), Math.floor(copy.height / 2), 1, 1).data).slice(0, 3) };
    }
    const canvas = document.querySelector('#gl');
    const gl = PM.GL.gl as WebGL2RenderingContext;
    const pixel = new Uint8Array(4);
    gl.readPixels(gl.drawingBufferWidth / 2, gl.drawingBufferHeight / 2, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    return { visible: canvas === gl.canvas, color: [...pixel].slice(0, 3) };
  });
}

test.describe('@viewer composition recovery', () => {
  test('keeps rendered pixels on the visible canvas across extension reloads', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Reload preview', w: 640, h: 360, dur: 4, bg: '#D92D20' }));
      PM.Kernel.services.get('viewer').returnToComposition();
    });
    const sample = () => sampleViewerPixels(page);
    await expect.poll(sample).toEqual({ visible: true, color: [217, 45, 32] });
    for (let reload = 0; reload < 3; reload++) {
      await page.evaluate(async () => {
        const PM = (window as any).PM;
        await PM.Kernel.loader.reload('viewer');
        await PM.Kernel.loader.whenIdle();
        PM.invalidate();
      });
      await expect.poll(sample).toEqual({ visible: true, color: [217, 45, 32] });
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('offers a one-click return only after explicit panning fully loses the composition', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    const recovery = page.locator('#composition-recovery');
    await expect(recovery).toBeHidden();

    await page.evaluate(() => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      viewer.fit = false;
      viewer.zoom = viewer.shown || 1;
      viewer.pan = [viewer.stage.getBoundingClientRect().width * 2, 0];
      viewer.layout();
    });

    await expect(recovery).toBeVisible();
    await expect(recovery).toContainText('Composition is out of view');
    await recovery.click();
    await expect(recovery).toBeHidden();
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return ({
      fit: viewer.fit,
      pan: [...viewer.pan],
    }); })).toEqual({ fit: true, pan: [0, 0] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('keeps the same composition point under the cursor through small-to-oversized zoom', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    const initialFrame = await page.locator('#stage-inner').boundingBox();
    if (!initialFrame) throw new Error('composition frame is unavailable');
    const pointer = {
      /* Playwright mouse coordinates are integral CSS pixels; measure the
         invariant against the exact coordinates delivered to WheelEvent. */
      x: Math.round(initialFrame.x + initialFrame.width * .72),
      y: Math.round(initialFrame.y + initialFrame.height * .31),
    };
    const compositionAtPointer = () => page.evaluate(({ x, y }) => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      return {
        x: (x - viewer.inner.getBoundingClientRect().left) / viewer.shown,
        y: (y - viewer.inner.getBoundingClientRect().top) / viewer.shown,
        shown: viewer.shown,
        frameWidth: viewer.inner.getBoundingClientRect().width,
        stageWidth: viewer.stage.getBoundingClientRect().width,
      };
    }, pointer);
    const initial = await compositionAtPointer();

    await page.mouse.move(pointer.x, pointer.y);
    let crossedOversized = false;
    const screenDrift: Array<{ x: number; y: number }> = [];
    for (let index = 0; index < 7; index += 1) {
      await page.mouse.wheel(0, -240);
      const current = await compositionAtPointer();
      crossedOversized ||= current.frameWidth > current.stageWidth;
      screenDrift.push({
        x: Math.abs(current.x - initial.x) * current.shown,
        y: Math.abs(current.y - initial.y) * current.shown,
      });
    }
    expect(crossedOversized).toBe(true);

    for (let index = 0; index < 7; index += 1) {
      await page.mouse.wheel(0, 240);
      const current = await compositionAtPointer();
      screenDrift.push({
        x: Math.abs(current.x - initial.x) * current.shown,
        y: Math.abs(current.y - initial.y) * current.shown,
      });
    }
    expect(Math.max(...screenDrift.map((point) => point.x)), JSON.stringify(screenDrift)).toBeLessThan(1);
    expect(Math.max(...screenDrift.map((point) => point.y)), JSON.stringify(screenDrift)).toBeLessThan(1);
    await expect(page.locator('#composition-recovery')).toBeHidden();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('keeps the last composition frame visible through rapid zoom changes', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const project = PM.mkProject({
        name: 'Zoom presentation', w: 640, h: 360, fps: 30, dur: 4, bg: '#D92D20',
      });
      PM.replaceProject(project);
      viewer.returnToComposition();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    });
    await expect.poll(() => sampleViewerPixels(page)).toEqual({ visible: true, color: [217, 45, 32] });

    const pixels = await page.evaluate(() => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const sample = () => {
        const frameWidth = viewer.inner.getBoundingClientRect().width;
        const stageWidth = viewer.stage.getBoundingClientRect().width;
        if (PM.EngineRender) {
          const presentation = viewer.enginePresentation;
          const latest = presentation?.latest;
          const frame = viewer.engineFrame?.();
          const canvas = document.querySelector<HTMLCanvasElement>('#stage #engine-present');
          const gl = document.querySelector<HTMLCanvasElement>('#gl');
          const rect = canvas?.getBoundingClientRect();
          const css = rect ? { x: rect.x, y: rect.y, width: rect.width, height: rect.height } : null;
          const tag = latest ? { seq: latest.seq, revision: latest.revision, viewSeq: latest.viewSeq,
            pts: latest.pts, standIn: latest.standIn } : null;
          const presenting = presentation?.presenting === true;
          const visible = presenting && latest != null && frame != null
            && latest.seq === frame.seq && latest.revision === frame.revision
            && latest.viewSeq === frame.viewSeq && frame.standIn === false
            && frame.revision === PM.Kernel.services.get('engine.viewer')?.revision?.()
            && frame.pts === Math.max(0, Math.round(PM.time * 705_600_000))
            && canvas != null && canvas === presentation.canvas && canvas.width > 0 && canvas.height > 0
            && getComputedStyle(canvas).visibility === 'visible'
            && gl != null && getComputedStyle(gl).visibility === 'hidden';
          if (!visible || !canvas) return { native: true, visible: false, presenting, latest: tag,
            color: null, frameWidth, stageWidth, zoom: viewer.shown,
            width: canvas?.width ?? 0, height: canvas?.height ?? 0, css };
          const copy = document.createElement('canvas');
          copy.width = canvas.width; copy.height = canvas.height;
          const ctx = copy.getContext('2d')!;
          ctx.drawImage(canvas, 0, 0);
          return { native: true, visible: true, presenting, latest: tag,
            color: Array.from(ctx.getImageData(Math.floor(copy.width / 2), Math.floor(copy.height / 2), 1, 1).data).slice(0, 3),
            frameWidth, stageWidth, zoom: viewer.shown, width: canvas.width, height: canvas.height, css };
        }
        const gl = PM.GL.gl as WebGL2RenderingContext;
        const rgba = new Uint8Array(4);
        gl.readPixels(
          Math.floor(PM.GL.canvas.width / 2), Math.floor(PM.GL.canvas.height / 2),
          1, 1, gl.RGBA, gl.UNSIGNED_BYTE, rgba,
        );
        const rect = PM.GL.canvas.getBoundingClientRect();
        return { native: false, visible: true, presenting: false, latest: null,
          color: Array.from(rgba).slice(0, 3), frameWidth, stageWidth, zoom: viewer.shown,
          width: PM.GL.canvas.width, height: PM.GL.canvas.height,
          css: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } };
      };
      const before = sample();
      const duringResize = [];
      /* Stress more than one input event per animation frame and cross the
         full-frame/cropped-viewport boundary in both directions. */
      for (let index = 0; index < 8; index++) {
        viewer.setZoom(viewer.shown * 1.35);
        duringResize.push(sample());
      }
      for (let index = 0; index < 8; index++) {
        viewer.setZoom(viewer.shown / 1.35);
        duringResize.push(sample());
      }
      return { before, duringResize };
    });

    expect(pixels.before.visible).toBe(true);
    expect(pixels.before.color).toEqual([217, 45, 32]);
    expect(pixels.before.frameWidth).toBeLessThanOrEqual(pixels.before.stageWidth);
    expect(pixels.duringResize.some((state) => state.frameWidth > state.stageWidth)).toBe(true);
    expect(pixels.duringResize.at(-1)!.frameWidth).toBeLessThanOrEqual(pixels.duringResize.at(-1)!.stageWidth);
    for (const state of pixels.duringResize) {
      expect(state.visible, JSON.stringify(state)).toBe(true);
      expect(state.color, JSON.stringify(state)).toEqual([217, 45, 32]);
      expect(state.width).toBeGreaterThan(0);
      expect(state.height).toBeGreaterThan(0);
      expect(state.css?.width).toBeGreaterThan(0);
      expect(state.css?.height).toBeGreaterThan(0);
      if (state.native) {
        expect(state.presenting).toBe(true);
        expect(state.latest?.standIn).toBe(false);
      }
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('zooms with a mouse wheel, holds still for a plain trackpad scroll, pans with Shift, pinches to zoom, and middle-drags', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.returnToComposition(); });
    const stage = await page.locator('#stage').boundingBox();
    const frame = await page.locator('#stage-inner').boundingBox();
    if (!stage || !frame) throw new Error('viewer geometry is unavailable');
    const pointer = { x: frame.x + frame.width / 2, y: frame.y + frame.height / 2 };
    const fittedZoom = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });

    await page.mouse.move(pointer.x, pointer.y);
    await page.mouse.wheel(0, -120);
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; })).toBeGreaterThan(fittedZoom);

    await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.returnToComposition(); });
    const scroll = (shiftKey: boolean) => page.evaluate(({ x, y, shiftKey }) => {
      const stageElement = document.querySelector('#stage')!;
      stageElement.dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, clientX: x, clientY: y, shiftKey,
        deltaMode: WheelEvent.DOM_DELTA_PIXEL, deltaX: 6.5, deltaY: 10.25,
      }));
    }, { ...pointer, shiftKey });
    await scroll(false);
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return ({
      fit: viewer.fit,
      pan: [...viewer.pan],
    }); })).toEqual({ fit: true, pan: [0, 0] });
    await scroll(true);
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return ({
      fit: viewer.fit,
      zoom: viewer.shown,
      pan: [...viewer.pan],
    }); })).toEqual({ fit: false, zoom: fittedZoom, pan: [-6.5, -10.25] });

    const beforePinch = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });
    await page.evaluate(({ x, y }) => {
      document.querySelector('#stage')!.dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, clientX: x, clientY: y,
        ctrlKey: true, deltaMode: WheelEvent.DOM_DELTA_PIXEL, deltaY: -24.5,
      }));
    }, pointer);
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; })).toBeGreaterThan(beforePinch);

    await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.returnToComposition(); });
    await page.mouse.move(stage.x + stage.width / 2, stage.y + stage.height / 2);
    await page.mouse.down({ button: 'middle' });
    await page.mouse.move(stage.x + stage.width / 2 + 40, stage.y + stage.height / 2 + 20, { steps: 5 });
    await page.mouse.up({ button: 'middle' });
    expect(await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return ({
      zoom: viewer.shown,
      pan: [...viewer.pan],
    }); })).toEqual({ zoom: fittedZoom, pan: [40, 20] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('double-clicks selected text through a full-frame top layer and selects all its source', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await waitForViewer(page);
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const tool = PM.Kernel.services.get('tool');
      const project = PM.mkProject({ name: 'Text edit ownership', w: 640, h: 360, fps: 30, dur: 4, bg: '#202020' });
      const text = PM.mkLayer('text', {
        name: 'Selected text', dur: 4,
        d: { text: 'inner shadow and drop shadow', font: 'Geist', weight: 700, size: 54, color: '#FFFFFF', align: 'center' },
        p: { 'position.x': 320, 'position.y': 180 },
      }, project);
      const cover = PM.mkLayer('solid', {
        name: 'Full-frame top layer', dur: 4, d: { color: '#101010', w: 640, h: 360 },
        p: { opacity: 1 },
      }, project);
      project.layers = [cover, text];
      PM.replaceProject(project);
      PM.setTime(1, { raw: true, force: true });
      tool.setTool('select');
      PM.selectLayers(text.id);
      viewer.fit = true;
      viewer.pan = [0, 0];
      viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const bounds = viewer.worldBounds(text, 1);
      return { textId: text.id, text: text.d.text, point: { x: bounds.cx, y: bounds.cy } };
    });
    const frame = await page.locator('#stage-inner').boundingBox();
    if (!frame) throw new Error('composition frame is unavailable');
    const shown = await page.evaluate(() => { const PM = (window as any).PM; const viewer = PM.Kernel.services.get('viewer'); return viewer.shown; });

    await page.mouse.dblclick(frame.x + setup.point.x * shown, frame.y + setup.point.y * shown);
    const editor = page.getByRole('textbox', { name: 'Edit text on canvas' });
    await expect(editor).toBeFocused();
    await expect(editor).toHaveValue(setup.text);
    expect(await page.evaluate(() => { const PM = (window as any).PM; return PM.Kernel.services.get('viewer').textSelection; }))
      .toEqual({ layer: setup.textId, start: 0, end: setup.text.length });
    expect(await page.evaluate(() => [...(window as any).PM.sel.layers])).toEqual([setup.textId]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

test('keeps saved built-in workspace geometry across a hidden app relaunch', async ({ session }) => {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.WS?.current));
  const before = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const workspace = PM.WS.get('design');
    PM.WS.activate('design', true);
    const left = workspace.layout.docks.find((dock: any) => dock.id === 'left');
    left.size = 444;
    left.flex = false;
    PM.WS.save();
    return { id: PM.WS.current.id, size: left.size };
  });
  expect(before).toEqual({ id: 'design', size: 444 });

  await session.relaunch(); // Restarts only the isolated hidden test app and keeps its temporary profile.
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.WS?.current));
  expect(await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const left = PM.WS.current.layout.docks.find((dock: any) => dock.id === 'left');
    return { id: PM.WS.current.id, size: left?.size };
  })).toEqual({ id: 'design', size: 444 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
