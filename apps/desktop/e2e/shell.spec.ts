import type { Page } from '@playwright/test';

import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

/** Registers named projects without opening them, the way a library does. */
async function makeProjects(page: Page, names: string[]): Promise<string[]> {
  return page.evaluate((list) => {
    const PM = (window as any).PM;
    const ids = list.map((name) => {
      const project = PM.mkProject({ name, dur: 4, w: 640, h: 360, fps: 24, bg: '#09090A' });
      PM.Projects.put(project);
      return project.id as string;
    });
    PM.bus.emit('projects:open');
    return ids;
  }, names);
}

const tabOrder = (page: Page) => page.locator('#tabs .project-doc').evaluateAll(
  (tabs) => tabs.map((tab) => (tab as HTMLElement).dataset.tabId)
);

test.describe('@shell Svelte shell', () => {
  test('boots the Svelte chrome while the dock panels stay live', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const titlebar = page.locator('#titlebar[data-svelte-shell="true"]');
    await expect(titlebar).toHaveCount(1);
    await expect(titlebar.locator('#tabs [role="tablist"]')).toHaveCount(1);
    await expect(titlebar.locator('#toolbar-strip[data-svelte-toolbar]')).toHaveCount(1);
    await expect(page.locator('#body .panel')).not.toHaveCount(0);

    const activeId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const tab = titlebar.locator(`.project-doc[data-project-id="${activeId}"]`);
    await expect(tab).toHaveCount(1);
    await expect(tab).toHaveAttribute('aria-selected', 'true');

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('opens projects from the library as tabs and switches between them', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId] = await makeProjects(page, ['Second composition']);

    await page.locator('#tabs .project-home').click();
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await page.locator('#projects-screen .ps-card').filter({ hasText: 'Second composition' }).first().click();

    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);
    await expect(page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);
    // The first document stays open beside it, and the new one is added after it.
    expect(await tabOrder(page)).toEqual([firstId, secondId]);
    await expect(page.locator(`#tabs .project-doc[data-project-id="${secondId}"]`)).toHaveClass(/\bon\b/);

    await page.locator(`#tabs .project-doc[data-project-id="${firstId}"]`).click();
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, firstId);
    expect(await tabOrder(page)).toEqual([firstId, secondId]);

    // ⌃Tab walks the strip; the menu command is what the accelerator sends.
    await page.evaluate(() => (window as any).PM.cmd('nextTab'));
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('closing a tab shows its neighbour, and closing the last shows Projects', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId, thirdId] = await makeProjects(page, ['Second', 'Third']);
    await page.evaluate(async (ids) => {
      const PM = (window as any).PM;
      for (const id of ids) await PM.Tabs.activate(id);
      await PM.Tabs.activate(ids[0]);
    }, [secondId, thirdId]);
    expect(await tabOrder(page)).toEqual([firstId, secondId, thirdId]);

    // Closing the middle tab shows the one that slides into its place.
    await page.locator(`#tabs .project-doc[data-project-id="${secondId}"] .project-doc-close`).click();
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, thirdId);
    expect(await tabOrder(page)).toEqual([firstId, thirdId]);

    // Middle-click closes an inactive tab without changing what is on screen.
    await page.locator(`#tabs .project-doc[data-project-id="${firstId}"]`).click({ button: 'middle' });
    await expect.poll(() => tabOrder(page)).toEqual([thirdId]);
    expect(await page.evaluate(() => (window as any).PM.proj.id)).toBe(thirdId);

    // ⌘W on the last tab leaves Projects over an empty window.
    await page.evaluate(() => (window as any).PM.cmd('closeTab'));
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await expect.poll(() => tabOrder(page)).toEqual([]);
    expect(await page.evaluate(() => (window as any).PM.isHomeProject())).toBe(true);

    // Closed projects are still in the library.
    expect(await page.evaluate((ids) => ids.every((id) => !!(window as any).PM.Projects.get(id)), [firstId, secondId, thirdId])).toBe(true);
    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('tabs reorder by dragging and come back in that order after a relaunch', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId, thirdId] = await makeProjects(page, ['Second', 'Third']);
    await page.evaluate(async (ids) => {
      const PM = (window as any).PM;
      for (const id of ids) await PM.Tabs.activate(id);
    }, [secondId, thirdId]);

    const dragged = page.locator(`#tabs .project-doc[data-project-id="${thirdId}"]`);
    const target = page.locator(`#tabs .project-doc[data-project-id="${firstId}"]`);
    const from = (await dragged.boundingBox())!;
    const to = (await target.boundingBox())!;
    await page.mouse.move(from.x + 20, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + 10, from.y + from.height / 2, { steps: 2 });
    await page.mouse.move(to.x + 4, to.y + to.height / 2, { steps: 12 });
    await page.mouse.up();
    await expect.poll(() => tabOrder(page)).toEqual([thirdId, firstId, secondId]);
    // A drag is not a click: the tab on screen is still the one that was.
    expect(await page.evaluate(() => (window as any).PM.proj.id)).toBe(thirdId);

    await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      await PM.Tabs.activate(id);
      await PM.flushProject();
    }, secondId);
    await session.relaunch();
    const { page: next } = session;
    await next.waitForFunction(() => Boolean((window as any).PM?.Tabs));
    await expect.poll(() => tabOrder(next)).toEqual([thirdId, firstId, secondId]);
    expect(await next.evaluate(() => (window as any).PM.proj.id)).toBe(secondId);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('a tab moves to a window of its own, and one project is never a tab in two windows', async ({ session }) => {
    const { page, app, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId] = await makeProjects(page, ['Travelling tab']);
    await page.evaluate((id) => (window as any).PM.Tabs.activate(id), secondId);

    const opened = app.waitForEvent('window');
    expect(await page.evaluate((id) => (window as any).PM.Tabs.moveToNewWindow(id), secondId)).toBe(true);
    const other = await opened;
    await other.waitForFunction((id) => (window as any).PM?.proj?.id === id, secondId);
    await expect.poll(() => tabOrder(other)).toEqual([secondId]);
    // The window it left shows the tab beside it and no longer holds it.
    await expect.poll(() => tabOrder(page)).toEqual([firstId]);
    expect(await page.evaluate(() => (window as any).PM.proj.id)).toBe(firstId);

    // Asking for it here raises the window that has it instead of loading a copy.
    expect(await page.evaluate((id) => (window as any).PM.openProjectHere(id), secondId)).toBe(false);
    expect(await tabOrder(page)).toEqual([firstId]);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('a tab pulled out of the strip and let go becomes a window of its own', async ({ session }) => {
    const { page, app, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId] = await makeProjects(page, ['Torn off']);
    await page.evaluate((id) => (window as any).PM.Tabs.activate(id), secondId);

    const tab = page.locator(`#tabs .project-doc[data-project-id="${secondId}"]`);
    const box = (await tab.boundingBox())!;
    const opened = app.waitForEvent('window');
    await page.mouse.move(box.x + 30, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + 40, box.y + 60, { steps: 4 });
    await page.mouse.move(box.x + 120, box.y + 320, { steps: 8 });
    // Out of the strip the tab leaves a ghost under the pointer.
    await expect(page.locator('#tabs .tab-ghost')).toHaveCount(1);
    await expect(tab).toHaveClass(/\btorn\b/);
    await page.mouse.up();

    const other = await opened;
    await other.waitForFunction((id) => (window as any).PM?.proj?.id === id, secondId);
    await expect.poll(() => tabOrder(other)).toEqual([secondId]);
    await expect.poll(() => tabOrder(page)).toEqual([firstId]);
    await expect(page.locator('#tabs .tab-ghost')).toHaveCount(0);
    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('a tab dropped on another window\'s strip joins it where it lands', async ({ session }) => {
    const { page, app, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId, thirdId] = await makeProjects(page, ['Stays', 'Travels']);
    await page.evaluate(async (ids) => {
      const PM = (window as any).PM;
      for (const id of ids) await PM.Tabs.activate(id);
    }, [secondId, thirdId]);

    // Hand the first tab to a window of its own, then drop the last tab onto
    // that window's strip, past its only tab.
    const opened = app.waitForEvent('window');
    await page.evaluate((id) => (window as any).PM.Tabs.moveToNewWindow(id), firstId);
    const other = await opened;
    await other.waitForFunction((id) => (window as any).PM?.proj?.id === id, firstId);
    // Read fresh each time: showing a window can move it clear of the menu bar.
    const stripOf = () => app.evaluate(({ BrowserWindow }) => {
      // The window just opened has the highest id.
      const windows = BrowserWindow.getAllWindows().sort((a, b) => b.id - a.id);
      return windows[0]!.getContentBounds();
    });
    let bounds = await stripOf();
    const point = { x: bounds.x + bounds.width - 300, y: bounds.y + 20 };
    expect(await page.evaluate(({ id, point }) => (window as any).PM.Tabs.detach(id, point), { id: thirdId, point })).toBe('window');

    await expect.poll(() => tabOrder(other)).toEqual([firstId, thirdId]);
    await other.waitForFunction((id) => (window as any).PM.proj.id === id, thirdId);
    await expect.poll(() => tabOrder(page)).toEqual([secondId]);

    // A lone tab has nowhere to go but another window; let go elsewhere, it stays.
    expect(await page.evaluate((id) => (window as any).PM.Tabs.detach(id, { x: -9999, y: -9999 }), secondId)).toBeNull();
    expect(await tabOrder(page)).toEqual([secondId]);
    // Dropped on a strip, it moves and its emptied window closes.
    bounds = await stripOf();
    const closed = page.waitForEvent('close');
    // The window closes under its own call, so the call is not awaited.
    void page.evaluate(({ id, point }) => (window as any).PM.Tabs.detach(id, point), { id: secondId, point: { x: bounds.x + 100, y: bounds.y + 20 } }).catch(() => undefined);
    await closed;
    await expect.poll(() => tabOrder(other)).toEqual([secondId, firstId, thirdId]);
    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('the tools stay on the right whatever the tabs, which scroll rather than crowd them', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.setViewportSize({ width: 1440, height: 860 });
    const geometry = () => page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
      const strip = document.querySelector('.project-tab-scroll')!;
      return {
        tabsRight: box('#tabs').right, tools: box('#toolbar-strip'),
        settings: box('#tb-right [aria-label="Open settings"]').left,
        overflowing: strip.scrollWidth > strip.clientWidth,
      };
    });
    const few = await geometry();

    const ids = await makeProjects(page, Array.from({ length: 10 }, (_, index) => `Tab ${index + 2}`));
    await page.evaluate(async (list) => {
      const PM = (window as any).PM;
      for (const id of list) await PM.Tabs.activate(id);
    }, ids);
    const many = await geometry();
    // The tools do not move for the tabs; the tabs scroll short of them.
    expect(Math.round(many.tools.left)).toBe(Math.round(few.tools.left));
    expect(many.tools.right).toBeLessThanOrEqual(many.settings);
    expect(many.tabsRight).toBeLessThanOrEqual(many.tools.left);
    expect(many.overflowing).toBe(true);
  });

  test('switching back to a tab resumes it from memory instead of reloading it', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    await importFixture(page, 'h264-aac.mp4');
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return [...PM.assets.map.values()].some((asset: any) => asset.name === 'h264-aac.mp4') && PM.proj.layers.length === 1;
    });
    // Remember the live objects: the document, its decoded media, its undo stack.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      (window as any).__live = { project: PM.proj, media: [...PM.assets.map.values()][0], undo: PM.hist.list() };
    });
    const [secondId, ...more] = await makeProjects(page, ['Second', 'Third', 'Fourth', 'Fifth', 'Sixth']);

    await page.evaluate((id) => (window as any).PM.Tabs.activate(id), secondId);
    // The other tab's media is not this tab's.
    expect(await page.evaluate(() => (window as any).PM.assets.map.size)).toBe(0);

    const reads = await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      const get = PM.Projects.get;
      let count = 0;
      PM.Projects.get = (...args: any[]) => { if (args[0] === id) count++; return get(...args); };
      try { await PM.Tabs.activate(id); } finally { PM.Projects.get = get; }
      return count;
    }, firstId);
    expect(reads).toBe(0);
    const resumed = await page.evaluate(() => {
      const PM = (window as any).PM;
      const live = (window as any).__live;
      return {
        sameProject: PM.proj === live.project,
        sameMedia: [...PM.assets.map.values()][0] === live.media,
        undo: JSON.stringify(PM.hist.list()) === JSON.stringify(live.undo) && PM.hist.canUndo(),
        loading: PM.assets.loading.size,
      };
    });
    expect(resumed).toEqual({ sameProject: true, sameMedia: true, undo: true, loading: 0 });
    // Undo still reaches the edit made before switching away.
    await page.evaluate(() => (window as any).PM.hist.undo());
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
    await page.evaluate(() => (window as any).PM.hist.redo());

    // Past the live limit the oldest tab is written out, and comes back from storage intact.
    await page.evaluate(async (ids) => {
      const PM = (window as any).PM;
      for (const id of ids) await PM.Tabs.activate(id);
    }, more);
    await page.evaluate((id) => (window as any).PM.Tabs.activate(id), firstId);
    await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((asset: any) => asset.name === 'h264-aac.mp4'));
    expect(await page.evaluate(() => (window as any).PM.proj === (window as any).__live.project)).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('renames a tab\'s project in place from the titlebar', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();

    const tab = page.locator('#tabs .project-doc');
    await tab.dblclick();
    const input = tab.locator('.project-doc-input');
    await expect(input).toHaveCount(1);
    await input.fill('Renamed from the titlebar');
    await input.press('Enter');

    await expect(tab.locator('.project-doc-label')).toHaveText('Renamed from the titlebar');
    await page.waitForFunction(() => (window as any).PM.proj.name === 'Renamed from the titlebar');

    expect(diagnostics.pageErrors).toEqual([]);
  });
});
