import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => {
  await session.page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
});

test('library shows a panel grid, adds panels to the workspace, and edits panels with the agent', async ({ session }) => {
  const { page } = session;
  await expect(page.locator('.panel-refine')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Ask Powermove agent', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).PM.PanelRefiner)).toBeUndefined();
  const timelineStyle = await page.evaluate(() => {
    const timeline = document.getElementById('panel-timeline')!;
    const previous = timeline.getAttribute('style');
    timeline.style.setProperty('position', 'fixed', 'important');
    timeline.style.setProperty('width', '137px', 'important');
    timeline.style.setProperty('height', '691px', 'important');
    const rect = timeline.getBoundingClientRect();
    return {
      previous,
      width: rect.width,
      height: rect.height,
      runtime: {
        pps: (window as any).PM.TL.pps,
        scrollT: (window as any).PM.TL.scrollT,
        scrollY: (window as any).PM.TL.scrollY
      }
    };
  });
  expect(timelineStyle.width).toBeLessThan(200);
  expect(timelineStyle.height).toBeGreaterThan(600);
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  const library = page.getByRole('dialog', { name: 'Panel library', exact: true });
  await expect(library).toBeVisible();
  const toolbarGeometry = await library.evaluate((screen) => {
    const tabs = screen.querySelector<HTMLElement>('.library-top .segmented')!.getBoundingClientRect();
    const actions = screen.querySelector<HTMLElement>('.library-top-action')!.getBoundingClientRect();
    const actionButton = screen.querySelector<HTMLElement>('.library-top-action .btn')!.getBoundingClientRect();
    const bounds = screen.getBoundingClientRect();
    return {
      left: tabs.left - bounds.left,
      right: bounds.right - actions.right,
      tabsHeight: tabs.height,
      buttonHeight: actionButton.height
    };
  });
  expect(Math.abs(toolbarGeometry.left - toolbarGeometry.right)).toBeLessThan(1);
  expect(Math.abs(toolbarGeometry.tabsHeight - toolbarGeometry.buttonHeight)).toBeLessThan(1);
  const icons = await library.locator('.library-card .library-thumb-icon svg').evaluateAll(nodes => nodes.map(node => (node as SVGElement).dataset.icon));
  expect(new Set(icons).size).toBe(icons.length);
  expect(icons).not.toContain('missing');
  expect(icons).not.toContain('panel');
  const panelCount = await page.evaluate(() => Object.keys((window as any).PM.PANELS).filter(id => id !== 'toolbar').length);
  expect(await library.locator('.library-card').count()).toBe(panelCount);
  await expect(library.locator('[data-panel-id="toolbar"]')).toHaveCount(0);
  await expect(library.getByRole('button', { name: 'Edit Tools', exact: true })).toHaveCount(0);
  await expect(library.locator('.library-panel-meta')).toHaveCount(0);
  const previewLayout = await library.locator('.library-card[data-panel-id]').evaluateAll((cards) => cards.map((card) => {
    const preview = card.querySelector<HTMLElement>('.library-live')!;
    const frame = card.querySelector<HTMLElement>('.library-live-frame');
    const previewRect = preview.getBoundingClientRect();
    const frameRect = frame?.getBoundingClientRect();
    const style = getComputedStyle(card);
    return {
      id: (card as HTMLElement).dataset.panelId,
      panelOnly: style.borderTopWidth === '0px' && style.backgroundColor === 'rgba(0, 0, 0, 0)',
      preview: { width: previewRect.width, height: previewRect.height },
      frame: frameRect ? { width: frameRect.width, height: frameRect.height } : null,
      fitted: !frameRect || (
        Math.abs(frameRect.left - previewRect.left) < 1
        && Math.abs(frameRect.top - previewRect.top) < 1
        && Math.abs(frameRect.width - previewRect.width) < 1
        && Math.abs(frameRect.height - previewRect.height) < 1
      ),
      fallbackHidden: !frame || preview.classList.contains('has-preview')
    };
  }));
  expect(previewLayout.filter(item => !item.panelOnly)).toEqual([]);
  expect(previewLayout.filter(item => !item.fitted)).toEqual([]);
  expect(previewLayout.filter(item => !item.fallbackHidden)).toEqual([]);
  const timelinePreview = library.locator('[data-panel-id="timeline"] .library-live-frame');
  await expect(timelinePreview).toHaveCSS('width', '800px');
  await expect(timelinePreview).toHaveCSS('height', '440px');
  const timelineFit = await library.locator('[data-panel-id="timeline"] .library-clone').evaluate((clone) => {
    const canvas = clone.querySelector<HTMLElement>('[data-library-source-id="tl-canvas"]');
    const wrap = clone.querySelector<HTMLElement>('[data-library-source-id="tl-canvas-wrap"]');
    if (!canvas || !wrap) return null;
    const canvasRect = canvas.getBoundingClientRect();
    const wrapRect = wrap.getBoundingClientRect();
    return {
      position: getComputedStyle(canvas).position,
      bitmapWidth: canvas.width / window.devicePixelRatio,
      bitmapHeight: canvas.height / window.devicePixelRatio,
      cssWidth: canvas.clientWidth,
      cssHeight: canvas.clientHeight,
      previewMode: canvas.dataset.timelinePreviewMode,
      previewStart: Number(canvas.dataset.timelinePreviewStart),
      previewEnd: Number(canvas.dataset.timelinePreviewEnd),
      previewRight: Number(canvas.dataset.timelinePreviewRight),
      inset: Math.max(
        Math.abs(canvasRect.left - wrapRect.left),
        Math.abs(canvasRect.top - wrapRect.top),
        Math.abs(canvasRect.right - wrapRect.right),
        Math.abs(canvasRect.bottom - wrapRect.bottom)
      )
    };
  });
  expect(timelineFit).not.toBeNull();
  expect(timelineFit?.position).toBe('absolute');
  expect(timelineFit?.bitmapWidth).toBeCloseTo(timelineFit?.cssWidth ?? 0, 0);
  expect(timelineFit?.bitmapHeight).toBeCloseTo(timelineFit?.cssHeight ?? 0, 0);
  expect(timelineFit?.previewMode).toBe('full-duration');
  expect(timelineFit?.previewStart).toBe(0);
  expect(timelineFit?.previewEnd).toBe(await page.evaluate(() => (window as any).PM.proj.dur));
  expect(timelineFit?.previewRight).toBeLessThanOrEqual(timelineFit?.cssWidth ?? 0);
  expect((timelineFit?.cssWidth ?? 0) - (timelineFit?.previewRight ?? 0)).toBeLessThanOrEqual(17);
  expect(timelineFit?.inset).toBeLessThan(1);
  expect(await page.evaluate(() => ({
    pps: (window as any).PM.TL.pps,
    scrollT: (window as any).PM.TL.scrollT,
    scrollY: (window as any).PM.TL.scrollY
  }))).toEqual(timelineStyle.runtime);
  await page.evaluate((previous) => {
    const timeline = document.getElementById('panel-timeline')!;
    if (previous === null) timeline.removeAttribute('style');
    else timeline.setAttribute('style', previous);
  }, timelineStyle.previous);
  await expect(library.getByRole('button', { name: /^Refine / })).toHaveCount(0);
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('Notes');
  await expect(library.locator('.library-card')).toHaveCount(1);
  await library.getByRole('button', { name: 'Add Notes to workspace', exact: true }).click();
  await expect(library).toBeHidden();
  await expect(page.locator('#panel-notes')).toBeVisible();
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('Notes');
  await expect(library.locator('.library-card .library-live-frame .panel.library-clone')).toBeVisible();
  await library.getByRole('button', { name: 'Edit Notes', exact: true }).click();
  await expect(library.locator('.library-editor')).toBeVisible();
  await expect(library.locator('.library-stage-frame #panel-notes')).toBeVisible();
  await expect(library.locator('#agent-composer-library')).toBeVisible();
  await expect(library.getByRole('button', { name: 'Back to panels', exact: true }).locator('svg[data-icon="chev"]')).toBeVisible();
  await expect(library.getByRole('button', { name: 'Choose focused panels', exact: true })).toHaveCount(0);
  await expect(library.getByRole('combobox', { name: 'Model', exact: true })).toBeVisible();
  await expect(library.getByRole('combobox', { name: 'Reasoning effort', exact: true })).toBeVisible();
  const libraryOptionAlignment = await library.locator('.library-chat-foot').evaluate((footer) => {
    const options = footer.querySelector('.agent-option-bar')!.getBoundingClientRect();
    const model = footer.querySelector('.agent-modelbar')!.getBoundingClientRect();
    return Math.abs(options.left - model.left);
  });
  expect(libraryOptionAlignment).toBeLessThan(4);
  await library.getByRole('button', { name: 'Back to panels', exact: true }).click();
  await expect(library.locator('.library-grid')).toBeVisible();
  await expect(page.locator('#panel-notes')).toBeVisible();
  await library.getByRole('searchbox', { name: 'Search panels' }).fill('');
  const dragId = await page.evaluate(() => {
    const PM = (window as any).PM;
    return Object.keys(PM.PANELS).find(id => id !== 'toolbar' && !PM.Layout.findPanel(PM.WS.current, id));
  });
  expect(dragId).toBeTruthy();
  await page.evaluate(async (id) => {
    const card = document.querySelector(`.library-card[data-panel-id="${id}"]`)!;
    const transfer = new DataTransfer();
    card.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: transfer }));
    await new Promise(resolve => setTimeout(resolve, 20));
    const dock = document.querySelector('[data-dock="right"]')!;
    dock.dispatchEvent(new DragEvent('dragover', { bubbles: true, dataTransfer: transfer }));
    dock.dispatchEvent(new DragEvent('drop', { bubbles: true, dataTransfer: transfer }));
    card.dispatchEvent(new DragEvent('dragend', { bubbles: true }));
  }, dragId);
  await expect(library).toBeHidden();
  await expect(page.locator(`#dock-right #panel-${dragId}`)).toBeVisible();
  await page.getByRole('button', { name: 'Open panel library', exact: true }).click();
  const workspaceCount = await page.evaluate(() => (window as any).PM.WS.all.length);
  await library.getByRole('button', { name: /^Workspaces/ }).click();
  await expect(library.locator('.library-card')).toHaveCount(workspaceCount);
  await expect(library.locator('.library-card .workspace-map').first()).toBeVisible();
  await library.getByRole('button', { name: /^Panels/ }).click();
  await library.getByRole('button', { name: 'New panel', exact: true }).click();
  await expect(page.locator('#agent-composer-agent')).toHaveValue('Create a new panel that ');
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('drawing a box notes every touched panel and sends that focus with the prompt', async ({ session }) => {
  const { page } = session;
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const region = await page.evaluate(() => {
    const PM = (window as any).PM;
    const viewer = document.getElementById('panel-viewer')!.getBoundingClientRect();
    const timeline = document.getElementById('panel-timeline')!.getBoundingClientRect();
    PM.WindowCapture.request = async () => null;
    PM.SpatialAssistant.activate(40, 70);
    return { x: viewer.left + 40, y: viewer.bottom - 35, toX: viewer.right - 40, toY: timeline.top + 60 };
  });
  await expect(page.locator('.spatial-compose')).toBeVisible();
  const options = page.locator('.spatial-focus-host');
  await expect(options).toBeHidden();
  await expect(page.locator('.spatial-compose textarea')).toHaveAttribute('placeholder', 'Full composition');
  await expect(page.locator('.spatial-send')).toBeVisible();
  await page.locator('.spatial-compose textarea').fill('Keep this draft');
  await page.waitForTimeout(360); // The deliberate arming interval protects a shake from becoming a selection.
  await page.mouse.move(region.x, region.y); await page.mouse.down();
  await page.mouse.move(region.toX, region.toY, { steps: 12 });
  await expect(options).toBeHidden();
  await page.mouse.up();
  await expect(options).toBeVisible();
  await expect(options.getByRole('button', { name: 'Choose focused panels' })).toBeVisible();
  await expect(options.getByRole('combobox', { name: 'Model', exact: true })).toBeVisible();
  await expect(options.getByRole('combobox', { name: 'Reasoning effort', exact: true })).toBeVisible();
  await expect(page.locator('.spatial-compose textarea')).toHaveValue('Keep this draft');
  await expect(page.locator('.spatial-target')).toContainText('Composition');
  await expect(page.locator('.spatial-target')).toContainText('Timeline');
  await page.keyboard.press('Escape');
  await expect(page.locator('.spatial-compose')).toHaveCount(0);
  await page.evaluate(() => (window as any).PM.SpatialAssistant.activate(40, 70));
  await expect(page.locator('.spatial-compose')).toBeVisible();
  await expect(options).toBeHidden();
  await page.waitForTimeout(360);
  // An undersized box must not reveal the options, even with remembered panel focus.
  await page.mouse.move(region.x, region.y); await page.mouse.down();
  await page.mouse.move(region.x + 20, region.y + 20); await page.mouse.up();
  await expect(page.locator('.spatial-compose')).toBeVisible();
  await expect(options).toBeHidden();
  await page.keyboard.press('Escape');
  expect(session.diagnostics.pageErrors).toEqual([]);
});
