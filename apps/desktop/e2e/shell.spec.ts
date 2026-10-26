import type { Page } from '@playwright/test';

import { chooseNativeMenu, expect, test } from './helpers/app';
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

/** The window's open projects, in order. The titlebar lists them in its title menu. */
const tabOrder = (page: Page) => page.evaluate(() => [...(window as any).PM.Tabs.list()] as string[]);
const title = (page: Page) => page.locator('#doc-title .tb-title-btn');

test.describe('@shell Svelte shell', () => {
  test('boots the Svelte chrome while the dock panels stay live', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const titlebar = page.locator('#titlebar[data-svelte-shell="true"]');
    await expect(titlebar).toHaveCount(1);
    await expect(titlebar.locator('[role="tab"], #tabs')).toHaveCount(0);
    await expect(titlebar.locator('.tb-left #toolbar-strip[data-svelte-toolbar]')).toHaveCount(1);
    await expect(page.locator('#body .panel')).not.toHaveCount(0);

    const activeId = await page.evaluate(() => (window as any).PM.proj.id as string);
    await expect(title(page)).toHaveAttribute('data-project-id', activeId);
    await expect(title(page)).toHaveAttribute('aria-haspopup', 'menu');

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('opens projects from the library and switches between them from the title menu', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId] = await makeProjects(page, ['Second composition']);

    await page.locator('.tb-left .project-home').click();
    await expect(page.locator('#projects-screen')).toHaveClass(/\bon\b/);
    await page.locator('#projects-screen .ps-card').filter({ hasText: 'Second composition' }).first().click();

    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);
    await expect(page.locator('#projects-screen')).not.toHaveClass(/\bon\b/);
    // The first document stays open beside it, and the new one is added after it.
    expect(await tabOrder(page)).toEqual([firstId, secondId]);
    await expect(title(page)).toHaveAttribute('data-project-id', secondId);

    const firstName = await page.evaluate((id) => (window as any).PM.Projects.get(id).name, firstId);
    const labels = await chooseNativeMenu(session, firstName, () => title(page).click());
    expect(labels.slice(0, 2)).toEqual([firstName, 'Second composition']);
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, firstId);
    expect(await tabOrder(page)).toEqual([firstId, secondId]);

    // ⌃Tab walks the strip; the menu command is what the accelerator sends.
    await page.evaluate(() => (window as any).PM.cmd('nextTab'));
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, secondId);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('closing a project shows its neighbour, and closing the last shows Projects', async ({ session }) => {
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

    // Closing the middle project shows the one that slides into its place.
    await chooseNativeMenu(session, 'Close Project', () => title(page).click());
    await page.waitForFunction((id) => (window as any).PM.proj.id === id, thirdId);
    expect(await tabOrder(page)).toEqual([firstId, thirdId]);

    // Close Other Projects keeps only what is on screen.
    await chooseNativeMenu(session, 'Close Other Projects', () => title(page).click({ button: 'right' }));
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

  test('relaunch opens the active document while retaining the others in Recents', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();
    const firstId = await page.evaluate(() => (window as any).PM.proj.id as string);
    const [secondId, thirdId] = await makeProjects(page, ['Second', 'Third']);
    for (const id of [firstId, secondId, thirdId]) {
      await session.app.evaluate(({ dialog }, filePath) => {
        dialog.showSaveDialog = async () => ({ canceled: false, filePath });
      }, `${session.userData}/${id}.pmv`);
      await page.evaluate((id) => (window as any).PM.Tabs.activate(id), id);
      expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    }

    expect(await tabOrder(page)).toEqual([firstId, secondId, thirdId]);

    await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      await PM.Tabs.activate(id);
      await PM.flushProject();
    }, secondId);
    await session.relaunch();
    const { page: next } = session;
    await next.waitForFunction(() => Boolean((window as any).PM?.Tabs));
    await expect.poll(() => tabOrder(next)).toEqual([secondId]);
    expect(await next.evaluate(() => (window as any).PM.proj.id)).toBe(secondId);
    const recentIds = await next.evaluate(() => (window as any).PM.Projects.list().map((project: any) => project.id));
    expect(recentIds).toEqual(expect.arrayContaining([firstId, secondId, thirdId]));

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

  test('a project handed to another window joins the end of its open projects', async ({ session }) => {
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
    // Handed over, it joins the end of the other window's projects and its emptied window closes.
    bounds = await stripOf();
    const closed = page.waitForEvent('close');
    // The window closes under its own call, so the call is not awaited.
    void page.evaluate(({ id, point }) => (window as any).PM.Tabs.detach(id, point), { id: secondId, point: { x: bounds.x + 100, y: bounds.y + 20 } }).catch(() => undefined);
    await closed;
    await expect.poll(() => tabOrder(other)).toEqual([firstId, thirdId, secondId]);
    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('tools lead on the left and the project is titled in the middle, however many are open', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    const geometry = () => page.evaluate(() => {
      const box = (selector: string) => document.querySelector(selector)!.getBoundingClientRect();
      const title = box('#doc-title .tb-title-btn');
      return {
        home: box('.tb-left .project-home'), tools: box('#toolbar-strip'), titleCentre: title.left + title.width / 2,
        titleLeft: title.left, titleRight: title.right, settings: box('#tb-right [aria-label="Open settings"]').left,
        width: window.innerWidth,
      };
    });
    const few = await geometry();
    expect(few.home.right).toBeLessThanOrEqual(few.tools.left);
    expect(few.tools.right).toBeLessThanOrEqual(few.titleLeft);
    expect(Math.abs(few.titleCentre - few.width / 2)).toBeLessThanOrEqual(2);
    expect(few.titleRight).toBeLessThanOrEqual(few.settings);

    const ids = await makeProjects(page, Array.from({ length: 10 }, (_, index) => `Project ${index + 2}`));
    await page.evaluate(async (list) => {
      const PM = (window as any).PM;
      for (const id of list) await PM.Tabs.activate(id);
    }, ids);
    const many = await geometry();
    // Nothing moves for more open projects: they are in the title's menu.
    expect(Math.round(many.tools.left)).toBe(Math.round(few.tools.left));
    expect(Math.abs(many.titleCentre - many.width / 2)).toBeLessThanOrEqual(2);
    await expect(page.locator('#titlebar [role="tab"]')).toHaveCount(0);
  });

  test('keeps a usable window drag area with many projects and overlays', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.setViewportSize({ width: 980, height: 640 });
    const ids = await makeProjects(page, Array.from({ length: 10 }, (_, index) => `Tab ${index + 2}`));
    await page.evaluate(async (list) => {
      for (const id of list) await (window as any).PM.Tabs.activate(id);
    }, ids);

    /* The widest stretch of a drag spacer that nothing covers, either side of the title. */
    const dragArea = () => page.evaluate(() => {
      let best = { width: 0, hit: false, region: '' };
      for (const drag of document.querySelectorAll<HTMLElement>('#titlebar > .titlebar-drag:not(.titlebar-drag-traffic)')) {
        const box = drag.getBoundingClientRect();
        let run = 0, middle: Element | null = null;
        for (let x = Math.ceil(box.left); x < box.right; x++) {
          const hit = document.elementFromPoint(x, box.top + box.height / 2);
          const ok = hit === drag || !!hit?.classList.contains('titlebar-overlay-drag');
          run = ok ? run + 1 : 0;
          if (run > best.width) { best = { width: run, hit: true, region: '' }; middle = hit; }
        }
        if (middle && best.width) best.region = getComputedStyle(middle).getPropertyValue('-webkit-app-region');
      }
      return best;
    });
    expect(await dragArea()).toMatchObject({ hit: true, region: 'drag' });
    expect((await dragArea()).width).toBeGreaterThanOrEqual(72);

    await page.evaluate(() => { (window as any).__dragModal = (window as any).PM.modal({ title: 'Move this window', body: 'Still draggable' }); });
    expect(await dragArea()).toMatchObject({ hit: true, region: 'drag' });
    await page.evaluate(() => { (window as any).__dragModal.close(); });

    await page.evaluate(() => (window as any).PM.LibraryUI.open());
    await expect(page.locator('#library-screen')).toHaveClass(/\bon\b/);
    expect(await dragArea()).toMatchObject({ hit: true, region: 'drag' });
    await expect.poll(async () => {
      const library = (await page.locator('#library-screen').boundingBox())!;
      return library.y >= 0 && library.y + library.height <= 640 + .1;
    }).toBe(true);
    await page.evaluate(() => (window as any).PM.LibraryUI.close());

    await page.evaluate(() => {
      const picker = document.createElement('div');
      picker.className = 'fill-picker-layer';
      document.body.append(picker);
    });
    expect(await dragArea()).toMatchObject({ hit: true, region: 'drag' });
    await page.locator('.fill-picker-layer').evaluate(element => element.remove());
  });

  test('switching back to a project resumes it from memory instead of reloading it', async ({ session }) => {
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

    // Empty tabs do not evict the document, its Undo state or its decoded media.
    await page.evaluate(async (ids) => {
      const PM = (window as any).PM;
      for (const id of ids) await PM.Tabs.activate(id);
    }, more);
    await page.evaluate((id) => (window as any).PM.Tabs.activate(id), firstId);
    await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((asset: any) => asset.name === 'h264-aac.mp4'));
    expect(await page.evaluate(() => (window as any).PM.proj === (window as any).__live.project)).toBe(true);
    expect(await page.evaluate(() => [...(window as any).PM.assets.map.values()][0] === (window as any).__live.media)).toBe(true);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);

    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('renames the project in place from the titlebar', async ({ session }) => {
    const { page, diagnostics } = session;
    await session.openEditor();

    await chooseNativeMenu(session, 'Rename Project…', () => title(page).click());
    const input = page.locator('#doc-title .tb-title-input');
    await expect(input).toHaveCount(1);
    await input.fill('Renamed from the titlebar');
    await input.press('Enter');

    await expect(title(page).locator('.tb-title-name')).toHaveText('Renamed from the titlebar');
    await page.waitForFunction(() => (window as any).PM.proj.name === 'Renamed from the titlebar');

    expect(diagnostics.pageErrors).toEqual([]);
  });
});
