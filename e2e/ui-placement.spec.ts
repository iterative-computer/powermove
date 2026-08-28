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
    const overlayState = await ghost.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const panel = document.getElementById('panel-timeline')!.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + 10, rect.top + 10);
      return {
        inset: Math.round(rect.left - panel.left),
        followsTop: Math.round(rect.top - panel.top),
        pointerEvents: getComputedStyle(element).pointerEvents,
        interceptsPointer: hit === element || element.contains(hit)
      };
    });
    expect(overlayState).toEqual({ inset: 5, followsTop: 5, pointerEvents: 'none', interceptsPointer: false });
    expect(await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]))).toBe(baseline);
    await page.screenshot({ path: testInfo.outputPath('timeline-loading.png') });

    await page.emulateMedia({ reducedMotion: 'reduce' });
    expect(await ghost.evaluate(element => getComputedStyle(element, '::after').animationName)).toBe('none');
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => PM.Layout.movePanel(workspace, 'timeline', 'left'));
    });
    await expect.poll(async () => ghost.evaluate(element => Math.round(element.getBoundingClientRect().left - document.getElementById('panel-timeline')!.getBoundingClientRect().left))).toBe(5);
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
      PM.AgentUI.submit('Add an easing panel above properties');
    });
    const ghost = page.locator('[data-ui-placement-ghost="right"]');
    await expect(ghost).toBeVisible();
    await expect(ghost).toContainText('Building Easing controls');
    expect(await ghost.evaluate(element => element.classList.contains('new-panel'))).toBe(true);
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
