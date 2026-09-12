import { expect, test } from './helpers/app';

test.describe('@dock-layout Svelte DockLayout', () => {
  test('keeps overfilled side panels usable between the titlebar and status bar', async ({ session }) => {
    await session.openEditor();
    const { page, diagnostics } = session;
    const ids = ['overflow-flex', 'overflow-fixed-a', 'overflow-fixed-b', 'overflow-fixed-c'];

    await page.evaluate((panelIds) => {
      const PM = (window as any).PM;
      for (const id of panelIds) {
        PM.registerPanel(id, {
          title: id,
          size: 260,
          build(body: HTMLElement) {
            const content = document.createElement('div');
            content.style.height = '220px';
            content.textContent = id;
            body.appendChild(content);
          }
        });
      }
      PM.WS.mutate((workspace: any) => {
        for (const dock of workspace.layout.docks) {
          dock.panels = dock.panels.filter((panel: any) => !panelIds.includes(panel.id));
        }
        const right = PM.Layout.ensureDock(workspace, 'right');
        right.hidden = false;
        right.panels = panelIds.map((id: string, index: number) => index === 0
          ? { id, flex: true }
          : { id, size: 260 });
      });
    }, ids);

    await expect(page.locator('#dock-right .panel')).toHaveCount(ids.length);
    const initial = await page.evaluate((panelIds) => {
      const dock = document.getElementById('dock-right')!;
      const leftDock = document.getElementById('dock-left')!;
      const body = document.getElementById('body')!;
      const titlebar = document.getElementById('titlebar')!;
      const status = document.getElementById('status')!;
      const first = document.getElementById(`panel-${panelIds[0]}`)!;
      const last = document.getElementById(`panel-${panelIds.at(-1)}`)!;
      return {
        overflowY: getComputedStyle(dock).overflowY,
        leftOverflowY: getComputedStyle(leftDock).overflowY,
        scrollHeight: dock.scrollHeight,
        clientHeight: dock.clientHeight,
        firstHeight: first.getBoundingClientRect().height,
        firstTop: first.getBoundingClientRect().top,
        lastBottom: last.getBoundingClientRect().bottom,
        bodyTop: body.getBoundingClientRect().top,
        bodyBottom: body.getBoundingClientRect().bottom,
        titlebarBottom: titlebar.getBoundingClientRect().bottom,
        statusTop: status.getBoundingClientRect().top
      };
    }, ids);

    expect(initial.overflowY).toBe('auto');
    expect(initial.leftOverflowY).toBe('auto');
    expect(initial.scrollHeight).toBeGreaterThan(initial.clientHeight);
    expect(initial.firstHeight).toBeGreaterThanOrEqual(240);
    expect(initial.firstTop).toBeGreaterThanOrEqual(initial.bodyTop - 1);
    expect(initial.titlebarBottom).toBeLessThanOrEqual(initial.bodyTop + 1);
    expect(initial.bodyBottom).toBeLessThanOrEqual(initial.statusTop + 1);
    expect(initial.lastBottom).toBeGreaterThan(initial.bodyBottom);

    await page.locator('#body > .splitter').last().hover();
    await page.mouse.wheel(0, 480);
    await expect.poll(() => page.locator('#dock-right').evaluate((dock) => dock.scrollTop)).toBeGreaterThan(0);

    await page.mouse.wheel(0, -480);
    await expect.poll(() => page.locator('#dock-right').evaluate((dock) => dock.scrollTop)).toBe(0);
    const edge = await page.locator('#body').boundingBox();
    expect(edge).not.toBeNull();
    await page.mouse.move(edge!.x + edge!.width - 1, edge!.y + edge!.height / 2);
    await page.mouse.wheel(0, 240);
    await expect.poll(() => page.locator('#dock-right').evaluate((dock) => dock.scrollTop)).toBeGreaterThan(0);
    await page.mouse.wheel(0, -10000);
    await expect.poll(() => page.locator('#dock-right').evaluate((dock) => dock.scrollTop)).toBe(0);
    await page.locator('#dock-right').hover();
    await page.mouse.wheel(0, 240);
    await expect.poll(() => page.locator('#dock-right').evaluate((dock) => dock.scrollTop)).toBeGreaterThan(0);

    const scrolled = await page.evaluate((lastId) => {
      const dock = document.getElementById('dock-right')!;
      dock.scrollTop = dock.scrollHeight;
      return new Promise<{ scrollTop: number; lastBottom: number; bodyBottom: number }>((resolve) => {
        requestAnimationFrame(() => resolve({
          scrollTop: dock.scrollTop,
          lastBottom: document.getElementById(`panel-${lastId}`)!.getBoundingClientRect().bottom,
          bodyBottom: document.getElementById('body')!.getBoundingClientRect().bottom
        }));
      });
    }, ids.at(-1)!);

    expect(scrolled.scrollTop).toBeGreaterThan(0);
    expect(scrolled.lastBottom).toBeLessThanOrEqual(scrolled.bodyBottom + 1);
    // Move the same overflowing stack left and exercise that divider too.
    await page.evaluate((panelIds) => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => {
        for (const id of panelIds) PM.Layout.movePanel(workspace, id, 'left');
      });
    }, ids);
    await expect(page.locator('#dock-left #panel-overflow-fixed-c')).toHaveCount(1);
    await page.locator('#body > .splitter').first().hover();
    await page.mouse.wheel(0, 240);
    await expect.poll(() => page.locator('#dock-left').evaluate((dock) => dock.scrollTop)).toBeGreaterThan(0);
    await page.mouse.wheel(0, -10000);
    await expect.poll(() => page.locator('#dock-left').evaluate((dock) => dock.scrollTop)).toBe(0);
    await page.mouse.move(edge!.x + 1, edge!.y + edge!.height / 2);
    await page.mouse.wheel(0, 240);
    await expect.poll(() => page.locator('#dock-left').evaluate((dock) => dock.scrollTop)).toBeGreaterThan(0);
    await page.mouse.wheel(0, -10000);
    await expect.poll(() => page.locator('#dock-left').evaluate((dock) => dock.scrollTop)).toBe(0);
    expect(diagnostics.pageErrors).toEqual([]);
  });

  test('keeps panel and viewer hosts alive across layout and workspace moves', async ({ session }) => {
    const { page, diagnostics } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.GL?.gl));
    await expect(page.locator('#body > .dock .panel[data-panel]')).not.toHaveCount(0);
    await expect(page.locator('#pm-panel-pool')).toHaveCount(1);

    const movedPanel = await page.evaluate(() => {
      const PM = (window as any).PM;
      if (!PM.Layout.findPanel(PM.WS.current, 'perf')) {
        PM.WS.mutate((workspace: any) => PM.Layout.addPanel(workspace, 'perf', 'right'));
      }
      const panel = document.getElementById('panel-perf')!;
      (panel as any).__dockLayoutIdentity = 'perf-live';
      const from = PM.Layout.findPanel(PM.WS.current, 'perf').dock.id;
      const to = from === 'left' ? 'right' : 'left';
      PM.WS.mutate((workspace: any) => PM.Layout.movePanel(workspace, 'perf', to));
      const moved = document.getElementById('panel-perf')!;
      return { from, to, sameNode: moved === panel };
    });
    expect(movedPanel.sameNode).toBe(true);
    await expect(page.locator(`#body #dock-${movedPanel.to} #panel-perf`)).toHaveCount(1);

    const panelState = await page.evaluate(() => {
      const PM = (window as any).PM;
      const moved = document.getElementById('panel-perf')!;
      PM.WS.mutate((workspace: any) => PM.Layout.hidePanel(workspace, 'perf'));
      const hiddenInPool = Boolean(document.querySelector('#pm-panel-pool #panel-perf'));
      PM.WS.mutate((workspace: any) => PM.Layout.restorePanel(workspace, 'perf'));
      return {
        identity: (document.getElementById('panel-perf') as any)?.__dockLayoutIdentity,
        sameNode: document.getElementById('panel-perf') === moved,
        hiddenInPool,
        restored: Boolean(document.querySelector('#body > .dock #panel-perf'))
      };
    });
    expect(panelState).toEqual({ identity: 'perf-live', sameNode: true, hiddenInPool: true, restored: true });

    const viewerState = await page.evaluate(() => {
      const PM = (window as any).PM;
      const originalWorkspace = PM.WS.current.id;
      const alternate = PM.WS.all.find((workspace: any) => workspace.id !== originalWorkspace);
      const canvas = document.getElementById('gl')!;
      (canvas as any).__dockLayoutCanvasIdentity = 'viewer-live';
      if (alternate) {
        PM.WS.activate(alternate.id, true);
        PM.WS.activate(originalWorkspace, true);
      }
      const current = document.getElementById('gl');
      return {
        sameNode: current === canvas,
        identity: (current as any)?.__dockLayoutCanvasIdentity,
        glAlive: Boolean(PM.GL.gl),
        svelteLayout: Boolean(document.getElementById('pm-panel-pool'))
      };
    });
    expect(viewerState).toEqual({ sameNode: true, identity: 'viewer-live', glAlive: true, svelteLayout: true });
    await expect(page.locator('#body .dock #gl')).toHaveCount(1);
    expect(diagnostics.pageErrors).toEqual([]);
  });
});
