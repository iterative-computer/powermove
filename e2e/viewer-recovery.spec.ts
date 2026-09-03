import { expect, test } from './helpers/app';

async function waitForViewer(page: any): Promise<void> {
  await page.waitForFunction(() => Boolean(
    (window as any).PM?.Viewer?.ov && (window as any).PM?.GL?.gl
      && document.querySelector('#composition-recovery'),
  ));
}

test.describe('@viewer composition recovery', () => {
  test('offers a one-click return only after explicit panning fully loses the composition', async ({ session }) => {
    const { page } = session;
    await waitForViewer(page);
    const recovery = page.locator('#composition-recovery');
    await expect(recovery).toBeHidden();

    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.Viewer.fit = false;
      PM.Viewer.zoom = PM.Viewer.shown || 1;
      PM.Viewer.pan = [PM.Viewer.stage.getBoundingClientRect().width * 2, 0];
      PM.Viewer.layout();
    });

    await expect(recovery).toBeVisible();
    await expect(recovery).toContainText('Composition is out of view');
    await recovery.click();
    await expect(recovery).toBeHidden();
    expect(await page.evaluate(() => ({
      fit: (window as any).PM.Viewer.fit,
      pan: [...(window as any).PM.Viewer.pan],
    }))).toEqual({ fit: true, pan: [0, 0] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('keeps the same composition point under the cursor through small-to-oversized zoom', async ({ session }) => {
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
      return {
        x: (x - PM.Viewer.inner.getBoundingClientRect().left) / PM.Viewer.shown,
        y: (y - PM.Viewer.inner.getBoundingClientRect().top) / PM.Viewer.shown,
        shown: PM.Viewer.shown,
        frameWidth: PM.Viewer.inner.getBoundingClientRect().width,
        stageWidth: PM.Viewer.stage.getBoundingClientRect().width,
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

  test('zooms with a mouse wheel, pans with trackpad scroll, pinches to zoom, and middle-drags', async ({ session }) => {
    const { page } = session;
    await waitForViewer(page);
    await page.evaluate(() => (window as any).PM.Viewer.returnToComposition());
    const stage = await page.locator('#stage').boundingBox();
    const frame = await page.locator('#stage-inner').boundingBox();
    if (!stage || !frame) throw new Error('viewer geometry is unavailable');
    const pointer = { x: frame.x + frame.width / 2, y: frame.y + frame.height / 2 };
    const fittedZoom = await page.evaluate(() => (window as any).PM.Viewer.shown);

    await page.mouse.move(pointer.x, pointer.y);
    await page.mouse.wheel(0, -120);
    expect(await page.evaluate(() => (window as any).PM.Viewer.shown)).toBeGreaterThan(fittedZoom);

    await page.evaluate(() => (window as any).PM.Viewer.returnToComposition());
    await page.evaluate(({ x, y }) => {
      const stageElement = document.querySelector('#stage')!;
      stageElement.dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, clientX: x, clientY: y,
        deltaMode: WheelEvent.DOM_DELTA_PIXEL, deltaX: 6.5, deltaY: 10.25,
      }));
    }, pointer);
    expect(await page.evaluate(() => ({
      fit: (window as any).PM.Viewer.fit,
      zoom: (window as any).PM.Viewer.shown,
      pan: [...(window as any).PM.Viewer.pan],
    }))).toEqual({ fit: false, zoom: fittedZoom, pan: [-6.5, -10.25] });

    const beforePinch = await page.evaluate(() => (window as any).PM.Viewer.shown);
    await page.evaluate(({ x, y }) => {
      document.querySelector('#stage')!.dispatchEvent(new WheelEvent('wheel', {
        bubbles: true, cancelable: true, clientX: x, clientY: y,
        ctrlKey: true, deltaMode: WheelEvent.DOM_DELTA_PIXEL, deltaY: -24.5,
      }));
    }, pointer);
    expect(await page.evaluate(() => (window as any).PM.Viewer.shown)).toBeGreaterThan(beforePinch);

    await page.evaluate(() => (window as any).PM.Viewer.returnToComposition());
    await page.mouse.move(stage.x + stage.width / 2, stage.y + stage.height / 2);
    await page.mouse.down({ button: 'middle' });
    await page.mouse.move(stage.x + stage.width / 2 + 40, stage.y + stage.height / 2 + 20, { steps: 5 });
    await page.mouse.up({ button: 'middle' });
    expect(await page.evaluate(() => ({
      zoom: (window as any).PM.Viewer.shown,
      pan: [...(window as any).PM.Viewer.pan],
    }))).toEqual({ zoom: fittedZoom, pan: [40, 20] });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('double-clicks selected text through a full-frame top layer and selects all its source', async ({ session }) => {
    const { page } = session;
    await waitForViewer(page);
    const setup = await page.evaluate(async () => {
      const PM = (window as any).PM;
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
      PM.setTool('select');
      PM.selectLayers(text.id);
      PM.Viewer.fit = true;
      PM.Viewer.pan = [0, 0];
      PM.Viewer.layout();
      await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
      const bounds = PM.Viewer.worldBounds(text, 1);
      return { textId: text.id, text: text.d.text, point: { x: bounds.cx, y: bounds.cy } };
    });
    const frame = await page.locator('#stage-inner').boundingBox();
    if (!frame) throw new Error('composition frame is unavailable');
    const shown = await page.evaluate(() => (window as any).PM.Viewer.shown);

    await page.mouse.dblclick(frame.x + setup.point.x * shown, frame.y + setup.point.y * shown);
    expect(await page.evaluate(() => ({
      selected: [...(window as any).PM.sel.layers],
      inspectorTextareas: [...document.querySelectorAll('textarea[data-inspector-text-layer]')].map((node: any) => node.dataset.inspectorTextLayer),
    }))).toMatchObject({ selected: [setup.textId], inspectorTextareas: [setup.textId] });
    await expect(page.locator(`textarea[data-inspector-text-layer="${setup.textId}"]`)).toBeFocused();
    expect(await page.locator(`textarea[data-inspector-text-layer="${setup.textId}"]`).evaluate((textarea: HTMLTextAreaElement) => ({
      value: textarea.value,
      start: textarea.selectionStart,
      end: textarea.selectionEnd,
    }))).toEqual({ value: setup.text, start: 0, end: setup.text.length });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

test('keeps saved built-in workspace geometry across a hidden app relaunch', async ({ session }) => {
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
  await session.page.waitForFunction(() => Boolean((window as any).PM?.WS?.current));
  expect(await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const left = PM.WS.current.layout.docks.find((dock: any) => dock.id === 'left');
    return { id: PM.WS.current.id, size: left?.size };
  })).toEqual({ id: 'design', size: 444 });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
