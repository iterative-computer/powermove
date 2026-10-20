import { expect, test } from './helpers/app';

test('loading placeholders never bleed through Projects', async ({ session }) => {
  await session.openEditor();
  await expect(session.page.locator('#boot-ghost')).toHaveCount(0);
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Navigation during loading' });
    PM.Projects.put(project);
    const get = PM.Projects.getAsync;
    PM.Projects.getAsync = async (id: string) => {
      await new Promise<void>(resolve => (window as any).__releaseGhostLoad = resolve);
      return get(id);
    };
    (window as any).__ghostNavigationOpen = PM.openProjectHere(project.id);
  });
  const ghost = session.page.locator('[data-project-ghost]');
  await expect(ghost).toBeVisible();
  await session.page.evaluate(() => {
    (window as any).PM.ProjectsScreen.show();
    // Also exercise the pre-JavaScript placeholder's visibility safeguard.
    document.body.insertAdjacentHTML('beforeend', '<div id="boot-ghost"></div>');
  });
  await expect(session.page.locator('#projects-screen')).toHaveClass(/\bon\b/);
  await expect(ghost).toBeHidden();
  await expect(session.page.locator('#boot-ghost')).toBeHidden();
  await session.page.screenshot({ path: '/tmp/powermove-projects-loading-fixed.png' });
  await session.page.evaluate(() => {
    document.getElementById('boot-ghost')?.remove();
    (window as any).PM.ProjectsScreen.hide();
    (window as any).__releaseGhostLoad();
  });
  expect(await session.page.evaluate(() => (window as any).__ghostNavigationOpen)).toBe(true);
  await expect(ghost).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test.describe('ordinary window loading', () => {
  test.use({ desktopLaunchOptions: { normalWindow: true } });
  test('a hidden window finishes loading keyframe-heavy projects without timer delays', async ({ session }) => {
    test.setTimeout(30_000);
    await session.openEditor();
    const id = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Background keyframes' });
      project.layers = Array.from({ length: 100 }, () => {
        const layer = PM.mkLayer('shape', { on: false });
        layer.p['position.x'].k = Array.from({ length: 500 }, (_, index) => ({ t: index / 30, v: index }));
        return layer;
      });
      PM.Projects.put(project);
      return project.id;
    });
    const window = await session.app.browserWindow(session.page);
    await window.evaluate(window => window.hide());
    try {
      const result = await session.page.evaluate(async id => {
        const PM = (window as any).PM, start = performance.now();
        const opened = await PM.openProjectHere(id);
        return { opened, ms: performance.now() - start, layers: PM.proj.layers.length, loading: !!PM.projectLoading };
      }, id);
      expect(result.opened).toBe(true);
      expect(result.layers).toBe(100);
      expect(result.loading).toBe(false);
      expect(result.ms).toBeLessThan(5000);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { await window.evaluate(window => window.show()); }
  });
});

test('opening a large stored project allows input and paint between loading steps', async ({ session }) => {
  test.setTimeout(60_000);
  await session.openEditor();
  const id = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Large loading test', w: 640, h: 360 });
    project.layers = Array.from({ length: 3000 }, (_, index) => PM.mkLayer('shape', { name: `Layer ${index}`, on: false }));
    PM.Projects.put(project);
    return project.id;
  });
  const result = await session.page.evaluate(async id => {
    const PM = (window as any).PM;
    let ticks = 0, maxGap = 0, last = performance.now();
    const states: boolean[] = [];
    const off = PM.bus.on('project:loading', () => states.push(!!PM.projectLoading));
    const timer = setInterval(() => { const now = performance.now(); maxGap = Math.max(maxGap, now - last); last = now; ticks++; }, 8);
    try {
      const opened = await PM.openProjectHere(id);
      return { opened, ticks, maxGap, states, layers: PM.proj.layers.length, loading: !!PM.projectLoading };
    } finally { clearInterval(timer); off(); }
  }, id);
  console.log('Project loading responsiveness', result);
  expect(result.opened).toBe(true);
  expect(result.layers).toBe(3000);
  expect(result.ticks).toBeGreaterThan(0);
  expect(result.maxGap).toBeLessThan(250);
  expect(result.states).toEqual([true, false]);
  expect(result.loading).toBe(false);
  await expect(session.page.locator('#body')).not.toHaveAttribute('aria-busy', 'true');
  await session.page.evaluate(async () => { await (window as any).PM.flushProject(); });
  await session.page.addInitScript(() => {
    (window as any).__syncLoadingReads = [];
    const timer = setInterval(() => {
      const PM = (window as any).PM;
      if (!PM?.Projects?.get) return;
      clearInterval(timer);
      const get = PM.Projects.get;
      PM.Projects.get = (id: string) => {
        if (PM.projectLoading) (window as any).__syncLoadingReads.push(id);
        return get(id);
      };
    }, 0);
  });
  await session.page.reload();
  await session.page.evaluate(async () => { await (window as any).PM.projectReady; });
  expect(await session.page.evaluate(() => (window as any).__syncLoadingReads)).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('closing the pending tab cancels its load and repeated clicks join the same open', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, outgoing = PM.proj.id;
    const project = PM.mkProject({ name: 'Pending close test' });
    PM.Projects.put(project);
    const get = PM.Projects.getAsync;
    let release!: () => void, reads = 0;
    PM.Projects.getAsync = async (id: string) => {
      reads++;
      await new Promise<void>(resolve => release = resolve);
      return get(id);
    };
    try {
      const first = PM.openProjectHere(project.id);
      while (!release) await new Promise(resolve => setTimeout(resolve, 0));
      const repeated = PM.openProjectHere(project.id);
      const active = PM.Tabs.active;
      const closed = await PM.Tabs.closeCurrent();
      release();
      return { reads, active: active === project.id, closed, results: await Promise.all([first, repeated]),
        outgoing: PM.proj.id === outgoing, stillOpen: PM.Tabs.list().includes(outgoing), reopened: PM.Tabs.list().includes(project.id) };
    } finally { PM.Projects.getAsync = get; }
  });
  expect(result).toEqual({ reads: 1, active: true, closed: true, results: [false, false], outgoing: true, stillOpen: true, reopened: false });
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('all open tabs retain their document, Undo state and cached artwork', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const references = new Map<string, any>();
    for (let index = 0; index < 8; index++) {
      const project = PM.mkProject({ name: `Warm tab ${index}`, w: 320, h: 180 });
      project.layers = [PM.mkLayer('shape', { d: { w: 80, h: 50, color: `#${(0xaa0000 + index).toString(16)}` } })];
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      PM.hist.do('Remember background', () => { PM.proj.bg = '#222222'; });
      references.set(project.id, { project: PM.proj, pixels: PM.GL.renderToPixels(0, 320, 180, { mblur: false }) });
    }
    const original = PM.Projects.getAsync;
    PM.Projects.getAsync = () => { throw new Error('A warm document was loaded from storage'); };
    const upload = PM.GL.gl.texImage2D;
    let uploads = 0, loadingEvents = 0, exporting = false;
    const off = PM.bus.on('project:loading', () => loadingEvents++);
    PM.GL.gl.texImage2D = function (...args: any[]) { if (exporting && args.at(-1) != null) uploads++; return upload.apply(this, args); };
    try {
      const retained = [];
      for (const [id, reference] of references) {
        await PM.Tabs.activate(id);
        exporting = true;
        const pixels = PM.GL.renderToPixels(0, 320, 180, { mblur: false });
        exporting = false;
        retained.push({ sameDocument: PM.proj === reference.project, undo: PM.hist.list().includes('Remember background'),
          pixels: pixels.every((value: number, index: number) => value === reference.pixels[index]) });
      }
      return { retained, uploads, loadingEvents };
    } finally { PM.Projects.getAsync = original; PM.GL.gl.texImage2D = upload; off(); }
  });
  expect(result.retained).toEqual(Array.from({ length: 8 }, () => ({ sameDocument: true, undo: true, pixels: true })));
  expect(result.uploads).toBe(0);
  expect(result.loadingEvents).toBe(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('a newer tab choice wins over an unfinished open and failures release the editor', async ({ session }) => {
  await session.openEditor();
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    const first = PM.mkProject({ name: 'Slow project' }), second = PM.mkProject({ name: 'Latest project' });
    PM.Projects.put(first); PM.Projects.put(second);
    const get = PM.Projects.getAsync;
    let release!: () => void;
    PM.Projects.getAsync = async (id: string) => {
      if (id === first.id) await new Promise<void>(resolve => release = resolve);
      return get(id);
    };
    try {
      const slow = PM.openProjectHere(first.id);
      while (!release) await new Promise(resolve => setTimeout(resolve, 0));
      const latest = await PM.openProjectHere(second.id);
      release();
      const superseded = await slow;
      PM.Projects.getAsync = async () => { throw new Error('Unreadable project'); };
      const failure = await PM.openProjectHere(first.id);
      return { latest, superseded, failure, active: PM.proj.id === second.id, loading: !!PM.projectLoading };
    } finally { PM.Projects.getAsync = get; }
  });
  expect(result).toEqual({ latest: true, superseded: false, failure: false, active: true, loading: false });
  await expect(session.page.locator('#body')).not.toHaveAttribute('aria-busy', 'true');
  expect(await session.page.locator('#body').evaluate(element => (element as HTMLElement).inert)).toBe(false);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

for (const theme of ['light', 'dark']) test(`loading ghost follows native panels in ${theme} appearance`, async ({ session }) => {
  await session.openEditor();
  await session.page.evaluate(theme => {
    const PM = (window as any).PM;
    PM.theme.apply(theme);
    const project = PM.mkProject({ name: 'Loading preview' });
    PM.Projects.put(project);
    const get = PM.Projects.getAsync;
    PM.Projects.getAsync = async (id: string) => {
      await new Promise(resolve => setTimeout(resolve, 800));
      return get(id);
    };
    (window as any).__ghostOpen = PM.openProjectHere(project.id);
  }, theme);
  await expect(session.page.locator('#body')).toHaveAttribute('aria-busy', 'true');
  await session.page.keyboard.press('Meta+t');
  expect(await session.page.evaluate(() => (window as any).PM.proj.layers.length)).toBe(0);
  const ghost = session.page.locator('[data-project-ghost]');
  await expect(ghost).toBeVisible();
  await expect.poll(() => ghost.evaluate(element => Number(getComputedStyle(element).opacity))).toBeGreaterThan(.9);
  const bounds = await ghost.boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(44);
  expect(bounds!.height).toBeLessThan((await session.page.viewportSize())?.height || 900);
  await session.page.screenshot({ path: `/tmp/powermove-project-ghost-${theme}.png` });
  expect(await session.page.evaluate(() => (window as any).__ghostOpen)).toBe(true);
  await expect(ghost).toHaveCount(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
