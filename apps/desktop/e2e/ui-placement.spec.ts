import { expect, test } from './helpers/app';

test.describe('@ui-placement early panel loading', () => {
  test('a broad panel target preserves its controls and never reopens the app', async ({ session }) => {
    await session.openEditor();
    const { page, app } = session;
    const pid = app.process().pid;
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.SpatialAssistant.open(); PM.AgentUI.setAccess('project');
      PM.CodexBridge.request = (_prompt: string, _schema: any, _images: any, options: any) => new Promise(resolve => {
        (window as any).__ghostRun = { options, resolve };
      });
      (window as any).__ghostPanel = document.getElementById('panel-timeline');
      PM.AgentUI.submit('Make the timeline controls clearer');
    });
    await page.waitForFunction(() => Boolean((window as any).__ghostRun));
    const before = await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]));
    await page.evaluate(() => {
      const text = 'POWERMOVE_UI_TARGET {"kind":"panel","id":"timeline","label":"Timeline controls"}';
      const run = (window as any).__ghostRun;
      run.options.onProgress(text);
      for (const fragment of text) run.options.onTrace({ kind: 'answer', text: fragment });
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.uiPlacement?.id)).toBe('timeline');
    await expect(page.locator('[data-ui-placement-ghost], [data-agent-editing]')).toHaveCount(0);
    await expect(page.locator('#panel-timeline .tl-transport button')).toBeVisible();
    expect(await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]))).toBe(before);
    expect(await page.evaluate(() => (window as any).PM.AgentUI.state.trace.some((step: any) => step.kind === 'text' && step.text.trim()))).toBe(false);
    await page.evaluate(() => (window as any).__ghostRun.resolve({ text: JSON.stringify({ summary: 'Done', commands: [], artifacts: [], externalActions: [], notes: [] }) }));
    await expect.poll(() => page.evaluate(() => (window as any).PM.AgentUI.state.phase)).not.toBe('running');
    expect(app.process().pid).toBe(pid);
    expect(app.windows()).toHaveLength(1);
    expect(await page.evaluate(() => (window as any).__ghostPanel === document.getElementById('panel-timeline'))).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('places a new-panel skeleton at its dock insertion point and clears it on Stop', async ({ session }, testInfo) => {
    await session.openEditor();
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

  test('targets one panel area, then reserves a local new section without persisting layout', async ({ session }, testInfo) => {
    await session.openEditor();
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.SpatialAssistant.open();
      PM.AgentUI.setAccess('project');
      PM.CodexBridge.request = (_prompt: string, _schema: any, _images: any, options: any) => new Promise((_resolve, reject) => {
        (window as any).__scopedGhost = { options, reject };
      });
      PM.AgentUI.submit('Update the timeline transport and add controls beneath it');
    });
    await page.waitForFunction(() => Boolean((window as any).__scopedGhost));
    const baseline = await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]));

    await page.evaluate(() => (window as any).__scopedGhost.options.onTrace({
      kind: 'answer',
      text: 'POWERMOVE_UI_TARGET {"kind":"panel","id":"timeline","selector":".tl-transport","label":"Transport controls"}'
    }));
    const target = page.locator('#panel-timeline .tl-transport');
    await expect(target).toHaveAttribute('data-agent-editing', 'Transport controls');
    await expect(target.locator('button')).toBeVisible();
    await expect(page.locator('[data-ui-placement-ghost]')).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath('scoped-transport-loading.png') });
    const ghost = page.locator('[data-ui-placement-ghost="timeline"]');

    const canvasTop = await page.locator('#tl-canvas-wrap').evaluate(element => element.getBoundingClientRect().top);

    await page.evaluate(() => (window as any).__scopedGhost.options.onTrace({
      kind: 'answer',
      text: 'POWERMOVE_UI_TARGET {"kind":"panel","id":"timeline","selector":"#tl-canvas-wrap","insert":"before","label":"Playback options"}'
    }));
    await expect(ghost).toContainText('Building Playback options');
    await expect(target).not.toHaveAttribute('data-agent-editing');
    expect(await ghost.evaluate(element => ({
      precedesCanvas: element.nextElementSibling?.matches('#tl-canvas-wrap'),
      staysInPanel: element.closest('.panel')?.id,
      isSection: element.classList.contains('section'),
      pointerEvents: getComputedStyle(element).pointerEvents,
      reservesSpace: element.getBoundingClientRect().height > 0
    }))).toEqual({ precedesCanvas: true, staysInPanel: 'panel-timeline', isSection: true, pointerEvents: 'none', reservesSpace: true });
    // Wait through the 160ms slot-opening animation, then verify the reserved
    // box and the downstream layout displacement agree exactly.
    await expect.poll(async () => {
      const height = await ghost.evaluate(element => Math.round(element.getBoundingClientRect().height));
      const nextTop = await page.locator('#tl-canvas-wrap').evaluate(element => Math.round(element.getBoundingClientRect().top));
      return { height, shift: nextTop - Math.round(canvasTop) };
    }).toEqual({ height: 112, shift: 112 });
    expect(await page.evaluate(() => JSON.stringify([(window as any).PM.proj, (window as any).PM.WS.current]))).toBe(baseline);
    await page.screenshot({ path: testInfo.outputPath('reserved-timeline-section.png') });

    await page.evaluate(() => (window as any).__scopedGhost.reject(new Error('Generation failed')));
    await expect(ghost).toHaveCount(0);
    expect(await page.locator('#tl-canvas-wrap').evaluate(element => element.getBoundingClientRect().top)).toBe(canvasTop);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
