/*
 * The appendix C consumer matrix (plan appendix C rows C1, C2, C17, C21;
 * the Library card C3 is consumer-surfaces.spec.ts and the mixer C6 is
 * mixer.spec.ts): each row's check in both themes, with the current viewer
 * and with forks of the viewers users can have (helpers/viewer-fork.ts).
 * The suite runs it with "Use New Engine" on and off; every case asserts
 * the same visible result in both.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/consumer-matrix.spec.ts --workers=1
 */
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import type { Page } from 'playwright';

import { expect, test, type LaunchedApp } from './helpers/app';
import { VIEWER_VARIANTS, useViewer, type ViewerVariant } from './helpers/viewer-fork';

const THEMES = ['light', 'dark'] as const;

/** Opens the editor in `theme` with the variant's viewer. */
async function setUp(session: LaunchedApp, theme: string, variant: ViewerVariant): Promise<{ page: Page; engine: boolean }> {
  await session.openEditor();
  const { page } = session;
  await useViewer(page, session.userData, variant);
  await page.evaluate((theme) => (window as any).PM.theme.apply(theme), theme);
  // Today's root carries `data-theme="dark"` in dark and no theme in light.
  await expect.poll(() => page.evaluate(() => document.documentElement.dataset.theme ?? 'light')).toBe(theme);
  const engine = await page.evaluate(() => !!(window as any).PM.EngineRender);
  return { page, engine };
}

/** The red-on-blue project the composition rows use: Main nests "Blue Comp" over a red plate. */
async function thumbsProject(page: Page): Promise<void> {
  await page.waitForFunction(() => { const PM = (window as any).PM; return Boolean(PM?.Comps && PM.Kernel.services.get('viewer')); });
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Thumbs', compName: 'Main', w: 320, h: 180, fps: 30, dur: 4, bg: '#000000' });
    const plate = PM.mkLayer('solid', { name: 'Red', from: 0, dur: 4, d: { w: 320, h: 180, color: '#E02020' }, p: { 'position.x': 160, 'position.y': 90 } }, project);
    const card = PM.mkLayer('solid', { name: 'Blue', from: 0, dur: 4, d: { w: 320, h: 180, color: '#2040E0' }, p: { 'position.x': 160, 'position.y': 90 } }, project);
    project.layers = [card, plate];
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    PM.Comps.precompose([card.id], { name: 'Blue Comp', adjustDuration: true });
    PM.WS?.activate?.('design', true);
  });
}

/** The centre pixel of a composition card's thumbnail, read in the page that shows it. */
function thumbCentre(page: Page, name: string) {
  return page.locator('.asset-card.is-comp', { hasText: name }).locator('img.comp-thumb')
    .evaluate(async (img: HTMLImageElement) => {
      await img.decode();
      const canvas = img.ownerDocument.createElement('canvas');
      canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
      const context = canvas.getContext('2d')!;
      context.drawImage(img, 0, 0);
      return Array.from(context.getImageData(canvas.width >> 1, canvas.height >> 1, 1, 1).data.slice(0, 3));
    });
}

/** Recolours the closed composition's only layer, as an undoable edit. */
async function recolourBlueComp(page: Page, color: string): Promise<void> {
  await page.evaluate((color) => {
    const PM = (window as any).PM;
    const id = PM.Comps.list().find((comp: any) => comp.name === 'Blue Comp').id;
    PM.hist.do('Recolor', () => { PM.Comps.get(id).layers[0].d.color = color; });
    PM.invalidate('all');
    PM.bus.emit('layers');
  }, color);
}

const isBlue = (rgb: number[]) => rgb[2]! > 180 && rgb[0]! < 80;
const isGreen = (rgb: number[]) => rgb[1]! > 150 && rgb[0]! < 80 && rgb[2]! < 120;

for (const theme of THEMES) for (const variant of VIEWER_VARIANTS) {
  test.describe(`consumer matrix in ${theme}, ${variant.name}`, () => {
    test('C1 composition thumbnails preview every composition and follow edits', async ({ session }) => {
      const { page } = await setUp(session, theme, variant);
      await thumbsProject(page);
      await expect(page.locator('.asset-card.is-comp img.comp-thumb')).toHaveCount(2, { timeout: 10_000 });
      // Main nests Blue Comp on top of the red plate; the closed comp renders on its own.
      expect(isBlue(await thumbCentre(page, 'Main'))).toBe(true);
      expect(isBlue(await thumbCentre(page, 'Blue Comp'))).toBe(true);
      await recolourBlueComp(page, '#20C040');
      await expect.poll(async () => isGreen(await thumbCentre(page, 'Blue Comp')), { timeout: 10_000 }).toBe(true);
      // Undo brings the blue thumbnail back.
      await page.evaluate(() => (window as any).PM.hist.undo());
      await expect.poll(async () => isBlue(await thumbCentre(page, 'Blue Comp')), { timeout: 10_000 }).toBe(true);
      expect(session.diagnostics.pageErrors).toEqual([]);
    });

    test('C2 the project thumbnail on save is the composition on screen, and follows the next save', async ({ session }) => {
      test.setTimeout(60_000);
      const { page } = await setUp(session, theme, variant);
      const id = await page.evaluate(() => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Saved thumbnail', w: 320, h: 180, fps: 30, dur: 2, bg: '#000000' });
        const solid = PM.mkLayer('solid', { name: 'Plate', from: 0, dur: 2, d: { w: 320, h: 180, color: '#2040E0' }, p: { 'position.x': 160, 'position.y': 90 } }, project);
        project.layers = [solid];
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
        PM.ProjectsScreen.hide();
        return PM.proj.id;
      });
      await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('viewer')?.stage));
      const savedCentre = () => page.evaluate(async (id) => {
        const meta = (window as any).PM.Projects.list().find((entry: any) => entry.id === id);
        if (!meta?.thumb) return null;
        const img = new Image(); img.src = meta.thumb; await img.decode();
        const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
        const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
        return { size: [img.naturalWidth, img.naturalHeight], rgb: Array.from(context.getImageData(canvas.width >> 1, canvas.height >> 1, 1, 1).data.slice(0, 3)) };
      }, id);
      await page.evaluate(() => (window as any).PM.autosave());
      await expect.poll(async () => { const saved = await savedCentre(); return !!saved && isBlue(saved.rgb); }, { timeout: 15_000 }).toBe(true);
      expect((await savedCentre())!.size).toEqual([320, 180]);
      // An edit and the next save replace it (after the five-second capture limit).
      await page.evaluate(() => {
        const PM = (window as any).PM;
        const layer = PM.proj.layers[0];
        PM.hist.do('Recolor', () => { layer.d.color = '#20C040'; });
        PM.invalidate('all');
        PM.bus.emit('layers');
        PM.autosave();
      });
      await expect.poll(async () => { const saved = await savedCentre(); return !!saved && isGreen(saved.rgb); }, { timeout: 20_000 }).toBe(true);
      expect(session.diagnostics.pageErrors).toEqual([]);
    });

    test('C17 status bar, performance panel and frame events report the engine on screen', async ({ session }) => {
      const { page, engine } = await setUp(session, theme, variant);
      await page.evaluate(() => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Status', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
        const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 80, h: 40 } }, project);
        box.p['position.x'].v = 200; box.p['position.y'].v = 120;
        project.layers = [box];
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
        PM.ProjectsScreen.hide();
        const ws = PM.WS.current;
        if (!PM.Layout.findPanel(ws, 'perf')) {
          const dock = ws.layout.docks.find((d: any) => d.id !== 'center') ?? ws.layout.docks[0];
          PM.WS.mutate((w: any) => PM.Layout.addPanel(w, 'perf', dock.id));
        }
      });
      await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('viewer')?.stage));
      const status = page.locator('#status');
      await expect(status).toContainText(engine ? 'Native engine' : 'WebGL2');
      await expect(status).toContainText('1 layers');
      await expect(status).toContainText('640×360');

      // The frame events an extension reads: one `frame:rendered` per frame on
      // screen with the exact `render.gl.previewViewport` object, then `overlay`.
      await page.evaluate(() => {
        const PM = (window as any).PM;
        const api = PM.Kernel.api('consumer-matrix-probe');
        const w = window as any;
        w.__frames = []; w.__order = [];
        api.events.on('frame:rendered', (frame: any) => {
          w.__frames.push({ same: frame.viewport === api.render.gl.previewViewport, version: frame.version, quality: frame.quality });
          w.__order.push('frame');
        });
        api.events.on('overlay', () => w.__order.push('overlay'));
        const layer = PM.proj.layers[0];
        PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'position.x', value: 260 }, { label: 'Move', origin: 'inspector' });
      });
      await expect.poll(() => page.evaluate(() => (window as any).__frames.some((f: any) => f.version === (window as any).PM.animVersion())), { timeout: 20_000 }).toBe(true);
      const seen = await page.evaluate(() => ({ frames: (window as any).__frames, order: (window as any).__order }));
      expect(seen.frames.every((f: any) => f.same)).toBe(true);
      expect(seen.frames.every((f: any) => f.quality === 1)).toBe(true);
      // Versions only move forward, and `overlay` follows each frame:rendered,
      // as today's frame loop does.
      const versions = seen.frames.map((f: any) => f.version);
      expect(versions).toEqual([...versions].sort((a: number, b: number) => a - b));
      seen.order.forEach((event: string, index: number) => { if (event === 'frame') expect(seen.order[index + 1]).toBe('overlay'); });

      // The performance panel's counts are the engine's with it on (no stale
      // GL counts), and its audio rows are exactly the engine's health.
      await expect(async () => {
        const shown = await page.evaluate(() => {
          const PM = (window as any).PM;
          const rows = [...document.querySelectorAll('#panel-perf [data-svelte-panel="perf"] .row.split')]
            .map(row => [row.querySelector('.k')?.textContent ?? '', row.querySelector('.vwrap')?.textContent ?? '']);
          const health = PM.EngineStats ? PM.EngineStats.audio?.() ?? null : null;
          const audio = health ? [
            ...(health.error ? [['Audio output', 'Unavailable']] : []),
            ['Underruns', String(health.underruns)],
            ['Overloads', String(health.deviceOverloads)],
            ['Audio waits', String(health.starvedRenders)],
            ...health.problems.map((problem: string) => ['Audio issue', problem])
          ] : [];
          return {
            core: rows.slice(0, 8).map(([label]) => label),
            layers: rows.find(([label]) => label === 'Layers')?.[1] ?? null,
            audio: rows.slice(8),
            expected: { core: ['FPS', 'Frame ms', PM.EngineStats ? 'Layers drawn' : 'GL draws', 'FX passes', 'Programs', 'Raster cache', 'Layers', 'Keyframes'], audio }
          };
        });
        expect({ core: shown.core, audio: shown.audio }).toEqual(shown.expected);
        expect(shown.layers).toBe('1');
      }).toPass({ timeout: 10_000 });

      // Playback reports a measured preview rate in the status bar.
      await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
      await expect(status).toContainText(/Preview \d+(\.\d+)? fps/, { timeout: 10_000 });
      await page.evaluate(() => (window as any).PM.pause());
      if (engine) expect(await page.evaluate(() => (window as any).PM.EngineStats.perf()?.draws ?? 0)).toBeGreaterThan(0);
      expect(session.diagnostics.pageErrors).toEqual([]);
    });

    test('C21 popped-out panels show engine pixels: composition thumbnails and a custom canvas panel', async ({ session }) => {
      test.setTimeout(60_000);
      const { page } = await setUp(session, theme, variant);
      const { app } = session;
      await thumbsProject(page);
      await expect(page.locator('.asset-card.is-comp img.comp-thumb')).toHaveCount(2, { timeout: 10_000 });

      // Media: the composition cards keep drawing in their own window.
      expect(await page.evaluate(() => (window as any).PM.Popout.open('assets'))).toBe(true);
      await expect.poll(() => app.windows().length).toBe(2);
      const media = app.windows().find(window => window !== page)!;
      await expect(media.locator('#panel-assets')).toBeVisible();
      await expect.poll(() => media.evaluate(() => document.documentElement.dataset.theme ?? 'light')).toBe(theme);
      expect(isBlue(await thumbCentre(media, 'Blue Comp'))).toBe(true);
      await recolourBlueComp(page, '#20C040');
      await expect.poll(async () => isGreen(await thumbCentre(media, 'Blue Comp')), { timeout: 10_000 }).toBe(true);
      await media.evaluate(() => window.resizeTo(520, 420));
      await expect.poll(async () => isGreen(await thumbCentre(media, 'Blue Comp')), { timeout: 10_000 }).toBe(true);
      await media.getByRole('button', { name: /Return .* to the Powermove layout/ }).click();
      await expect.poll(() => app.windows().length).toBe(1);
      await expect.poll(async () => isGreen(await thumbCentre(page, 'Blue Comp')), { timeout: 10_000 }).toBe(true);

      // A user's own canvas panel (apiVersion 1) draws `render.renderFrameTo`
      // into its canvas; popped out, it keeps drawing the current frame.
      const dir = path.join(session.userData, 'extensions', 'consumer-canvas-panel');
      await mkdir(dir, { recursive: true });
      await writeFile(path.join(dir, 'manifest.json'), JSON.stringify({ id: 'consumer-canvas-panel', name: 'Canvas probe', version: '1.0.0', apiVersion: 1, description: 'e2e fixture', author: 'user' }));
      await writeFile(path.join(dir, 'index.ts'), `export default function activate(api) {
  api.panels.register({ id: 'consumer-canvas', title: 'Canvas probe', build(body) {
    const canvas = body.ownerDocument.createElement('canvas');
    canvas.width = 160; canvas.height = 90; canvas.dataset.probe = 'canvas';
    body.appendChild(canvas);
    const draw = () => {
      const frame = api.render.renderFrameTo(api.transport.time(), 160, 90);
      canvas.getContext('2d').drawImage(frame, 0, 0);
      canvas.dataset.drawn = String(Number(canvas.dataset.drawn || 0) + 1);
    };
    draw();
    const subscription = api.events.on('project:changed', draw);
    return () => subscription.dispose();
  } });
}
`);
      await page.waitForFunction(() => Boolean((window as any).PM.PANELS['consumer-canvas']), null, { timeout: 30_000 });
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.Comps.open(PM.Comps.list().find((comp: any) => comp.name === 'Blue Comp').id);
        PM.WS.mutate((ws: any) => PM.Layout.addPanel(ws, 'consumer-canvas', ws.layout.docks.find((d: any) => d.id !== 'center')?.id ?? 'right'));
      });
      const canvasCentre = (target: Page) => target.locator('#panel-consumer-canvas canvas[data-probe="canvas"]').evaluate((canvas: HTMLCanvasElement) =>
        Array.from(canvas.getContext('2d')!.getImageData(80, 45, 1, 1).data.slice(0, 3)));
      await expect.poll(async () => isGreen(await canvasCentre(page)), { timeout: 10_000 }).toBe(true);
      expect(await page.evaluate(() => (window as any).PM.Popout.open('consumer-canvas'))).toBe(true);
      await expect.poll(() => app.windows().length).toBe(2);
      const custom = app.windows().find(window => window !== page)!;
      await expect(custom.locator('#panel-consumer-canvas canvas[data-probe="canvas"]')).toBeAttached();
      // The pixels it drew came along, and an edit draws the new frame there.
      expect(isGreen(await canvasCentre(custom))).toBe(true);
      await page.evaluate(() => {
        const PM = (window as any).PM;
        const layer = PM.proj.layers[0];
        PM.hist.do('Recolor', () => { layer.d.color = '#2040E0'; });
        PM.invalidate('all');
        PM.bus.emit('layers');
      });
      await expect.poll(async () => isBlue(await canvasCentre(custom)), { timeout: 10_000 }).toBe(true);
      await app.evaluate(({ BrowserWindow }) => {
        BrowserWindow.getAllWindows().find(window => window.webContents.getURL() === 'about:blank')!.close();
      });
      await expect.poll(() => app.windows().length).toBe(1);
      await expect(page.locator('#panel-consumer-canvas canvas[data-probe="canvas"]')).toBeVisible();
      expect(isBlue(await canvasCentre(page))).toBe(true);
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  });
}

test.describe('C2 project thumbnails reject a capture the editor moved past', () => {
  test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });
  test('a capture is dropped after a time change, an edit, or a newer save, and the retry saves the current frame', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    expect(await page.evaluate(() => !!(window as any).PM.EngineRender)).toBe(true);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Stale thumbnail', w: 320, h: 180, fps: 30, dur: 2, bg: '#000000' });
      const solid = PM.mkLayer('solid', { name: 'Plate', from: 0, dur: 2, d: { w: 320, h: 180, color: '#2040E0' }, p: { 'position.x': 160, 'position.y': 90 } }, project);
      project.layers = [solid];
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
      PM.ProjectsScreen.hide();
      /* Hold each project capture until the test releases it, and record
         every metadata write with a thumbnail. */
      const w = window as any;
      const thumbnails = PM.EngineRender.thumbnails;
      const request = thumbnails.request;
      w.__held = [];
      thumbnails.request = function (req: any, ...args: any[]) {
        const answer = request.call(this, req, ...args);
        if (!String(req.key).startsWith('project-thumbnail:')) return answer;
        let release!: () => void;
        const gate = new Promise<void>(resolve => { release = resolve; });
        w.__held.push({ release, time: req.time });
        return gate.then(() => answer);
      };
      const upsert = PM.Projects.upsertMeta;
      w.__thumbs = [];
      PM.Projects.upsertMeta = (meta: any) => { if (meta?.thumb) w.__thumbs.push(meta.thumb); return upsert(meta); };
    });
    await page.waitForFunction(() => Boolean((window as any).PM.Kernel.services.get('viewer')?.stage));
    const held = () => page.evaluate(() => (window as any).__held.length);
    const thumbs = () => page.evaluate(() => (window as any).__thumbs.length);
    const release = (index: number) => page.evaluate((index) => (window as any).__held[index].release(), index);
    const thumbCentre = (index: number) => page.evaluate(async (index) => {
      const img = new Image(); img.src = (window as any).__thumbs[index]; await img.decode();
      const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
      const context = canvas.getContext('2d')!; context.drawImage(img, 0, 0);
      return Array.from(context.getImageData(canvas.width >> 1, canvas.height >> 1, 1, 1).data.slice(0, 3));
    }, index);

    // 1. The playhead moves while the capture runs: dropped, then retried.
    await page.evaluate(() => (window as any).PM.autosave());
    await expect.poll(held, { timeout: 10_000 }).toBe(1);
    await page.evaluate(() => (window as any).PM.setTime(1, { force: true }));
    await release(0);
    await page.waitForTimeout(500);
    expect(await thumbs()).toBe(0);
    await expect.poll(held, { timeout: 10_000 }).toBe(2);
    expect(await page.evaluate(() => (window as any).__held[1].time)).toBe(1);

    // 2. An edit lands while the retry runs: dropped, then retried.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers[0];
      PM.hist.do('Recolor', () => { layer.d.color = '#20C040'; });
      PM.invalidate('all');
      PM.bus.emit('layers');
    });
    await release(1);
    await page.waitForTimeout(500);
    expect(await thumbs()).toBe(0);
    await expect.poll(held, { timeout: 10_000 }).toBe(3);

    // 3. A newer save starts while that one runs: dropped, then retried.
    await page.evaluate(() => (window as any).PM.autosave());
    await page.waitForTimeout(700);
    await release(2);
    await page.waitForTimeout(500);
    expect(await thumbs()).toBe(0);
    await expect.poll(held, { timeout: 10_000 }).toBe(4);

    // The retry that nothing moved past saves the frame on screen now.
    await release(3);
    await expect.poll(thumbs, { timeout: 10_000 }).toBe(1);
    expect(isGreen(await thumbCentre(0))).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
