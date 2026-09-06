import { expect, test } from './helpers/app';

test.describe('@ui-placement early panel loading', () => {
  test('shows a click-through ghost before completion, tracks layout, and never reopens the app', async ({ session }, testInfo) => {
    const { page, app } = session;
    const pid = app.process().pid;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.SpatialAssistant.open();
      PM.AgentUI.setAccess('project');
      // Deterministic public model stream; no real Codex call or user-data writes.
      PM.CodexBridge.request = (prompt: string, _schema: any, _images: any, options: any) => new Promise((resolve, reject) => {
        (window as any).__ghostRun = { prompt, options, resolve, reject };
      });
      (window as any).__ghostDocument = document;
      (window as any).__ghostPanel = document.getElementById('panel-timeline');
      PM.AgentUI.submit('Make the timeline controls clearer');
    });
    await page.waitForFunction(() => Boolean((window as any).__ghostRun));
    const baseline = await page.evaluate(() => {
      const PM = (window as any).PM;
      return JSON.stringify([PM.proj, PM.WS.current]);
    });
    const ghost = page.locator('[data-ui-placement-ghost]');
    await expect(ghost).toHaveCount(0);
    await page.evaluate(() => {
      const run = (window as any).__ghostRun;
      const text = 'POWERMOVE_UI_TARGET {"kind":"panel","id":"timeline","label":"Timeline controls"}';
      run.options.onProgress(text);
      run.options.onTrace({ kind: 'answer', text });
    });
    await expect(ghost).toBeVisible();
    await expect(ghost).toContainText('Updating Timeline controls');
    const field = ghost.locator('.ghost-edge-field');
    // The Motion GPU field reports itself only once it has presented a frame.
    await expect(field).toHaveAttribute('data-ghost-renderer', 'motion-gpu', { timeout: 10_000 });
    const overlayState = await ghost.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const panel = document.getElementById('panel-timeline')!.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + 10, rect.top + 10);
      const canvas = element.querySelector('canvas')!;
      return {
        inset: Math.round(rect.left - panel.left),
        followsTop: Math.round(rect.top - panel.top),
        pointerEvents: getComputedStyle(element).pointerEvents,
        interceptsPointer: hit === element || element.contains(hit),
        canvasIsCapped: canvas.width <= Math.ceil(canvas.getBoundingClientRect().width * 1.5)
      };
    });
    expect(overlayState).toEqual({ inset: 0, followsTop: 0, pointerEvents: 'none', interceptsPointer: false, canvasIsCapped: true });
    expect(await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]))).toBe(baseline);
    // The overlay belongs to the panel, so the dock clips it during scrolling.
    expect(await ghost.evaluate(element => element.parentElement?.id)).toBe('panel-timeline');
    await page.screenshot({ path: testInfo.outputPath('timeline-loading.png') });

    await page.emulateMedia({ reducedMotion: 'reduce' });
    await expect(field).toHaveAttribute('data-ghost-renderer', 'static');
    await expect(field).toHaveAttribute('data-ghost-fallback', 'reduced-motion');
    expect(await field.evaluate(element => getComputedStyle(element).display)).toBe('none');
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => PM.Layout.movePanel(workspace, 'timeline', 'left'));
    });
    await expect.poll(async () => ghost.evaluate(element => Math.round(element.getBoundingClientRect().left - document.getElementById('panel-timeline')!.getBoundingClientRect().left))).toBe(0);
    // Scroll the panel behind the dock's top edge. Probe hit testing with
    // pointer events temporarily enabled to verify the actual paint clip.
    await page.evaluate(() => {
      const dock = document.getElementById('dock-left')!;
      const panel = document.getElementById('panel-timeline')!;
      dock.style.height = '180px';
      dock.style.maxHeight = '180px';
      dock.style.flex = '0 0 250px';
      panel.style.minHeight = '600px';
      dock.scrollTop = panel.getBoundingClientRect().top - dock.getBoundingClientRect().top + 100;
    });
    const clipping = await ghost.evaluate(element => {
      const dock = document.getElementById('dock-left')!;
      const bounds = dock.getBoundingClientRect();
      const rect = element.getBoundingClientRect();
      const overlay = element as HTMLElement;
      overlay.style.pointerEvents = 'auto';
      const inside = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top + 30);
      const outside = document.elementFromPoint(bounds.left + bounds.width / 2, bounds.top - 10);
      overlay.style.removeProperty('pointer-events');
      return {
        scrolled: rect.top < bounds.top,
        inside: inside === element || element.contains(inside),
        outside: outside === element || element.contains(outside),
        fullWidth: Math.round(rect.width) === Math.round(element.parentElement!.getBoundingClientRect().width),
      };
    });
    expect(clipping).toEqual({ scrolled: true, inside: true, outside: false, fullWidth: true });
    await page.screenshot({ path: testInfo.outputPath('scrolled-panel-loading.png') });
    await page.evaluate(() => {
      (window as any).__ghostRun.resolve({ text: JSON.stringify({ summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] }) });
    });
    await expect(ghost).toHaveCount(0);
    expect(app.process().pid).toBe(pid);
    expect(app.windows()).toHaveLength(1);
    expect(await page.evaluate(() => (window as any).__ghostDocument === document && (window as any).__ghostPanel === document.getElementById('panel-timeline'))).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('places a new-panel skeleton at its dock insertion point and clears it on Stop', async ({ session }, testInfo) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.SpatialAssistant.open();
      PM.AgentUI.setAccess('project');
      PM.CodexBridge.request = (_prompt: string, _schema: any, _images: any, options: any) => new Promise((_resolve, reject) => {
        (window as any).__ghostOptions = options;
        options.signal.addEventListener('abort', () => reject(Object.assign(new Error('Stopped'), { name: 'AbortError' })));
        options.onTrace({ kind: 'answer', text: 'POWERMOVE_UI_TARGET {"kind":"dock","id":"right","beforePanelId":"inspector","label":"Easing controls"}' });
      });
    });
    const before = await page.evaluate(() => document.getElementById('panel-inspector')!.getBoundingClientRect().top);
    await page.evaluate(() => (window as any).PM.AgentUI.submit('Add an easing panel above properties'));
    const ghost = page.locator('[data-ui-placement-ghost="right"]');
    await expect(ghost).toBeVisible();
    await expect(ghost).toContainText('Building Easing controls');
    expect(await ghost.evaluate(element => element.classList.contains('new-panel'))).toBe(true);
    // The ghost holds the slot for real: it sits in the dock's flow above the
    // announced panel, at panel width, and nothing is drawn on top of anything.
    await expect.poll(async () => ghost.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const inspector = document.getElementById('panel-inspector')!.getBoundingClientRect();
      return {
        dock: element.parentElement?.id,
        gapAbovePanel: Math.round(inspector.top - rect.bottom),
        sameWidth: Math.round(rect.width - inspector.width),
        overlapsPanel: rect.bottom > inspector.top + 0.5,
        reservedHeight: Math.round(rect.height) > 80
      };
    })).toEqual({ dock: 'dock-right', gapAbovePanel: 10, sameWidth: 0, overlapsPanel: false, reservedHeight: true });
    expect(await page.evaluate(() => document.getElementById('panel-inspector')!.getBoundingClientRect().top)).toBeGreaterThan(before);
    // Screenshot the settled field, not the moment before it has presented.
    await expect(ghost.locator('.ghost-edge-field')).toHaveAttribute('data-ghost-renderer', 'motion-gpu', { timeout: 10_000 });
    await page.screenshot({ path: testInfo.outputPath('new-panel-loading.png') });
    await page.evaluate(() => (window as any).PM.theme.apply('dark'));
    await page.screenshot({ path: testInfo.outputPath('new-panel-loading-dark.png') });
    await page.evaluate(() => (window as any).PM.AgentUI.stop());
    await expect(ghost).toHaveCount(0);
    await page.evaluate(() => (window as any).__ghostOptions.onTrace({ kind: 'answer', text: 'POWERMOVE_UI_TARGET {"kind":"panel","id":"timeline","label":"Late update"}' }));
    await expect(page.locator('[data-ui-placement-ghost]')).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
