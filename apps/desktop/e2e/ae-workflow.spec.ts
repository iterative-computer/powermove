import type { Page } from '@playwright/test';

import { expect, test } from './helpers/app';

/* A text layer in a 2007×1253 composition, opened the way the app opens projects. */
async function textProject(page: Page): Promise<string> {
  const id = await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Chrome', w: 2007, h: 1253, fps: 30, dur: 8, bg: '#09090A' });
    const text = PM.mkLayer('text', {
      name: 'Chrome', dur: 8,
      d: { text: 'Chrome', font: 'Geist', weight: 600, size: 128, color: '#F2F2F2', align: 'center' },
      p: { 'position.x': 1003, 'position.y': 626 },
    }, project);
    project.layers = [text];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    return text.id as string;
  });
  await page.waitForFunction((layerId) => !!(window as any).PM.L?.(layerId), id);
  await page.evaluate((layerId) => (window as any).PM.selectLayers(layerId), id);
  return id;
}

const settle = (page: Page) => page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

test.describe('@ae-workflow After Effects workflow', () => {
  test('Properties shows every row in a side column, and the column scrolls as one page', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await textProject(page);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.WS.mutate((workspace: any) => {
        for (const dock of workspace.layout.docks) dock.panels = dock.panels.filter((panel: any) => !['inspector', 'layer-effects', 'fxbrowser'].includes(panel.id));
        const right = PM.Layout.ensureDock(workspace, 'right');
        right.hidden = false;
        right.panels = [{ id: 'inspector', flex: true }, { id: 'layer-effects', flex: true }];
      });
    });
    await settle(page);
    const geometry = await page.evaluate(() => {
      const body = document.querySelector<HTMLElement>('#panel-inspector .body')!;
      const dock = document.getElementById('dock-right')!;
      const splitter = document.querySelector<HTMLElement>('#dock-right > .splitter.h');
      return { body: [body.scrollHeight, body.clientHeight], dock: [dock.scrollHeight, dock.clientHeight], fixedDivider: !!splitter?.classList.contains('fixed') };
    });
    // No scroller inside Properties: all of its rows are laid out.
    expect(geometry.body[0]).toBeLessThanOrEqual(geometry.body[1] + 1);
    // The column holds more than fits and scrolls instead.
    expect(geometry.dock[0]).toBeGreaterThan(geometry.dock[1]);
    // A divider under a content-height panel has nothing to resize.
    expect(geometry.fixedDivider).toBe(true);
  });

  test('the Anchor Point tool drags an anchor on a corner of the box, and a press elsewhere brings it there', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const id = await textProject(page);
    const corner = await page.evaluate((layerId) => {
      const PM = (window as any).PM;
      const layer = PM.L(layerId), t = PM.time, b = PM.GL.bounds(layer, t);
      const dx = b.x0 - PM.ev(layer, 'anchor.x', t), dy = b.y0 - PM.ev(layer, 'anchor.y', t), m = PM.localMatrix(layer, t);
      PM.Edit.apply([
        { type: 'set_property', target: layerId, path: 'anchor.x', value: b.x0 },
        { type: 'set_property', target: layerId, path: 'anchor.y', value: b.y0 },
        { type: 'set_property', target: layerId, path: 'position.x', value: PM.ev(layer, 'position.x', t) + m[0] * dx + m[2] * dy },
        { type: 'set_property', target: layerId, path: 'position.y', value: PM.ev(layer, 'position.y', t) + m[1] * dx + m[3] * dy },
      ], { label: 'Anchor: Upper left', origin: 'interface' });
      PM.cmd('toolAnchor');
      const viewer = PM.Kernel.services.get('viewer'), inner = viewer.inner.getBoundingClientRect(), world = PM.worldMatrix(layer, t);
      const at = (x: number, y: number) => ({ x: inner.left + (world[0] * x + world[2] * y + world[4]) * viewer.shown, y: inner.top + (world[1] * x + world[3] * y + world[5]) * viewer.shown });
      return { anchor: at(b.x0, b.y0), inside: at(b.x1 - 8, b.y1 - 8) };
    }, id);
    const anchor = () => page.evaluate((layerId) => {
      const PM = (window as any).PM, layer = PM.L(layerId);
      return [PM.ev(layer, 'anchor.x', PM.time), PM.ev(layer, 'anchor.y', PM.time), PM.ev(layer, 'position.x', PM.time)];
    }, id);
    const world = () => page.evaluate((layerId) => (window as any).PM.worldMatrix((window as any).PM.L(layerId), (window as any).PM.time).map((v: number) => Math.round(v * 100) / 100), id);
    const start = await anchor(), pose = await world();

    // Just outside the box, on the marker: the press that used to do nothing.
    await expect(async () => {
      await page.mouse.move(corner.anchor.x - 4, corner.anchor.y - 4);
      await page.mouse.down();
      await page.mouse.move(corner.anchor.x + 40, corner.anchor.y + 20, { steps: 6 });
      await page.mouse.up();
      expect((await anchor())[0]).not.toBe(start[0]);
    }).toPass();
    // Position compensates, so the layer does not move on screen.
    (await world()).forEach((value: number, index: number) => expect(value).toBeCloseTo(pose[index]!, 1));

    const moved = await anchor();
    await page.mouse.move(corner.inside.x, corner.inside.y);
    await page.mouse.down();
    await page.mouse.up();
    expect((await anchor())[0]).toBeGreaterThan(moved[0]);
  });

  test('timeline layer switches turn on 3D and Continuously Rasterize, and Release Pre-comp keeps the picture', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const id = await textProject(page);
    await page.evaluate((layerId) => (window as any).PM.Comps.precompose([layerId], { name: 'Pre-comp 1' }), id);
    const precompId = await page.evaluate(() => (window as any).PM.proj.layers[0].id as string);
    const row = await page.evaluate(() => {
      const T = (window as any).PM.Kernel.services.get('timeline'), rect = T.cv.getBoundingClientRect();
      return { left: rect.left, y: rect.top + T.ruler - (T.scrollY || 0) + T.row / 2, gut: T.gut };
    });
    const flags = () => page.evaluate((layerId) => {
      const layer = (window as any).PM.L(layerId);
      return { threeD: !!layer.threeD, raster: !!layer.continuousRaster };
    }, precompId);
    // Left of the 110px Link Layers column: 3D, then Continuously Rasterize at the edge.
    await page.mouse.click(row.left + row.gut - 137, row.y);
    await expect.poll(flags).toEqual({ threeD: true, raster: false });
    await page.mouse.click(row.left + row.gut - 137, row.y);
    await page.mouse.click(row.left + row.gut - 119, row.y);
    await expect.poll(flags).toEqual({ threeD: false, raster: true });

    // Rasterized at its displayed size, a 300% pre-comp renders the nested comp at 3×.
    const density = await page.evaluate((layerId) => {
      const PM = (window as any).PM;
      PM.Edit.apply([{ type: 'set_property', target: layerId, path: 'scale.x', value: 300 }, { type: 'set_property', target: layerId, path: 'scale.y', value: 300 }], { label: 'Scale', origin: 'interface' });
      const grabs: number[] = [];
      const render = PM.GL.renderProject;
      PM.GL.renderProject = (project: any, time: number, width: number, height: number, options: any) => {
        if (project !== PM.proj) grabs.push(width);
        return render(project, time, width, height, options);
      };
      try { PM.GL.renderProject(PM.proj, PM.time, PM.GL.canvas.width, PM.GL.canvas.height, {}); }
      finally { PM.GL.renderProject = render; }
      return grabs[0]! / (PM.GL.canvas.width);
    }, precompId);
    expect(density).toBeGreaterThan(2.5);

    // The text's final transform is the same before and after: pre-comp × inner layer, then group × layer.
    const before = await page.evaluate(({ layerId, textId }) => {
      const PM = (window as any).PM;
      const times = (a: number[], b: number[]) => [a[0]!*b[0]!+a[2]!*b[1]!, a[1]!*b[0]!+a[3]!*b[1]!, a[0]!*b[2]!+a[2]!*b[3]!, a[1]!*b[2]!+a[3]!*b[3]!, a[0]!*b[4]!+a[2]!*b[5]!+a[4]!, a[1]!*b[4]!+a[3]!*b[5]!+a[5]!];
      const outer = PM.worldMatrix(PM.L(layerId), PM.time);
      const comp = PM.compOf(PM.L(layerId));
      PM.scope.push(comp);
      try { return times(outer, PM.worldMatrix(comp.layers.find((layer: any) => layer.id === textId), PM.time)); }
      finally { PM.scope.pop(); }
    }, { layerId: precompId, textId: id });
    await page.evaluate((layerId) => (window as any).PM.cmd('releasePrecomp', layerId), precompId);
    expect(await page.evaluate(() => (window as any).PM.proj.layers.map((layer: any) => layer.type))).toEqual(['group', 'text']);
    const after = await page.evaluate(() => {
      const PM = (window as any).PM;
      return PM.worldMatrix(PM.proj.layers[1], PM.time);
    });
    after.forEach((value: number, index: number) => expect(value).toBeCloseTo(before[index]!, 3));
  });
  test('the layer timeline opens only animated layers, heads their keyframes by section, and mutes audio from its speaker', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Teaser', w: 1920, h: 1080, fps: 30, dur: 6 });
      const text = PM.mkLayer('text', { name: 'Title', dur: 6, d: { text: 'Title' } }, project);
      const solid = PM.mkLayer('solid', { name: 'Still', dur: 6 }, project);
      const music = PM.mkLayer('audio', { name: 'Music', dur: 6, d: { asset: 'missing-audio' } }, project);
      project.layers = [text, solid, music];
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      return { text: text.id, solid: solid.id, music: music.id };
    });
    await page.waitForFunction((id) => !!(window as any).PM.L?.(id), ids.text);
    await page.evaluate(({ text }) => {
      const PM = (window as any).PM, L = PM.L(text);
      PM.hist.do('keys', () => { PM.setKeyOn(L.p.opacity, 0, 0, 'linear', 30); PM.setKeyOn(L.p.opacity, 1, 100, 'linear', 30); });
      PM.proj.layers.forEach((layer: any) => PM.UIState.setLayerCollapsed(layer, true));
      PM.bus.emit('layers'); PM.invalidate();
    }, ids);
    await settle(page);
    const point = (id: string, x: number) => page.evaluate(({ id, x }) => {
      const tl = (window as any).PM.Kernel.services.get('timeline');
      const index = tl.rows.findIndex((row: any) => row.kind === 'layer' && row.L.id === id);
      const rect = tl.cv.getBoundingClientRect();
      return { x: rect.x + x, y: rect.y + tl.ruler + index * tl.row - tl.scrollY + tl.row / 2 };
    }, { id, x });
    const rowsOf = (id: string) => page.evaluate((layerId) => (window as any).PM.Kernel.services.get('timeline').rows
      .filter((row: any) => row.L.id === layerId).map((row: any) => row.kind === 'heading' ? `# ${row.label}` : row.kind === 'prop' ? row.label : 'layer'), id);

    // A layer with nothing keyframed has no chevron to open.
    const still = await point(ids.solid, 87);
    await page.mouse.click(still.x, still.y);
    await settle(page);
    expect(await rowsOf(ids.solid)).toEqual(['layer']);

    // An animated layer opens onto its keyframed properties, under their Properties section.
    const title = await point(ids.text, 87);
    await page.mouse.click(title.x, title.y);
    await settle(page);
    expect(await rowsOf(ids.text)).toEqual(['layer', '# Transform', 'Opacity']);

    // An audio layer's speaker mutes it, as one undoable step.
    const speaker = await point(ids.music, 18);
    await page.mouse.click(speaker.x, speaker.y);
    expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.music)).toBe(false);
    await page.evaluate(() => (window as any).PM.cmd('undo'));
    expect(await page.evaluate((id) => (window as any).PM.L(id).on, ids.music)).not.toBe(false);
  });
  test('the After Effects board: tabbed Project and Effect Controls, Track Matte as an effect, composition path, resizable layer list, empty timeline', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    const ids = await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Board', w: 1920, h: 1080, fps: 30, dur: 6 });
      const title = PM.mkLayer('text', { name: 'Title', dur: 6, d: { text: 'Title' } }, project);
      const card = PM.mkLayer('shape', { name: 'Card', dur: 6 }, project);
      project.layers = [title, card];
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      return { title: title.id, card: card.id };
    });
    await page.waitForFunction((id) => !!(window as any).PM.L?.(id), ids.title);
    await settle(page);

    // Project and Effect Controls share one tabbed slot, Effect Controls shown.
    const tabs = page.locator('#dock-left .ptabs [role="tab"]');
    await expect(tabs).toHaveText(['Project', 'Effect Controls']);
    await expect(page.locator('#panel-layer-effects')).toBeVisible();
    await expect(page.locator('#panel-assets')).toBeHidden();
    await tabs.filter({ hasText: 'Project' }).click();
    await expect(page.locator('#panel-assets')).toBeVisible();
    await expect(page.locator('#panel-layer-effects')).toBeHidden();
    await page.locator('#dock-left .ptabs [role="tab"]', { hasText: 'Effect Controls' }).click();

    // Track Matte is applied from Effects & Presets and shows in Effect Controls.
    await page.evaluate((id) => (window as any).PM.selectLayers(id), ids.card);
    await page.locator('#panel-fxbrowser .fxb-search').fill('matte');
    await page.locator('#panel-fxbrowser button[aria-label="Add Track Matte"]').click();
    expect(await page.evaluate((id) => (window as any).PM.L(id).matteSource, ids.card)).toBe(ids.title);
    await expect(page.locator('#panel-layer-effects [data-effect-id="track-matte"]')).toBeVisible();
    await page.locator('#panel-layer-effects .fx-matte .fx-reset').click();
    expect(await page.evaluate((id) => (window as any).PM.L(id).matteSource ?? null, ids.card)).toBeNull();

    // Zoom and quality sit along the bottom of the viewer; nothing sits above it.
    await expect(page.locator('#viewer-foot #composition-zoom')).toHaveCount(1);
    await expect(page.locator('#viewer-foot #viewer-time')).toHaveCount(0);
    await expect(page.locator('#viewer-head')).toHaveCount(0);

    // Inside a pre-comp, Tab maps the nesting and leads back out.
    await page.evaluate((id) => {
      const PM = (window as any).PM;
      const comp = PM.Comps.precompose([id], { name: 'Nested', adjustDuration: true });
      PM.Comps.open(PM.proj.layers.find((layer: any) => layer.type === 'precomp').d.comp);
      void comp;
    }, ids.card);
    const activeName = () => page.evaluate(() => {
      const comps = (window as any).PM.Kernel.services.get('compositions');
      return comps.get(comps.active())?.name;
    });
    await expect.poll(activeName).toBe('Nested');
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur?.());
    await page.keyboard.press('Tab');
    await expect(page.locator('#viewer-flowchart')).toContainText('Used in');
    await expect(page.locator('#viewer-flowchart')).toContainText('Board');
    await page.locator('#viewer-flowchart button', { hasText: 'Board' }).click();
    await expect.poll(activeName).toBe('Board');

    // The layer list's right edge drags wider, and the width is saved.
    const edge = await page.evaluate(() => {
      const tl = (window as any).PM.Kernel.services.get('timeline');
      const rect = tl.cv.getBoundingClientRect();
      return { x: rect.x + tl.gut, y: rect.y + tl.ruler + tl.row / 2, gut: tl.gut };
    });
    await page.mouse.move(edge.x, edge.y);
    await page.mouse.down();
    await page.mouse.move(edge.x + 60, edge.y, { steps: 6 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate(() => (window as any).PM.WS.current.chrome.timeline.gutterWidth)).toBe(edge.gut + 60);

    // An empty composition offers its first layers.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Empty', w: 1920, h: 1080, fps: 30, dur: 6 });
      project.layers = [];
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    });
    const empty = page.locator('#panel-timeline .tl-empty');
    await expect(empty).toBeVisible();
    await empty.getByRole('button', { name: /Text/ }).click();
    await expect.poll(() => page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(1);
    await expect(empty).toBeHidden();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
