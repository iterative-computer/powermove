/*
 * The Phase 3c render transport with "Use New Engine" on
 * (docs/engine/rfcs/16-render-transport.md): the apiVersion 1 to 3
 * synchronous render API (`render.snapshot`, `render.renderFrameTo`) draws
 * through the engine, pinned to the revision just edited; the async
 * siblings and the UI's idle snapshots; a synchronous snapshot from inside
 * a `raster` wrapper and from inside an animation-frame callback, which
 * must not deadlock; and frame events on presentation (`frame:rendered`
 * with the exact `render.gl.previewViewport` object, then `overlay`) with
 * engine stats for the status bar.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-render-transport.spec.ts --workers=1
 */
import type { Page } from 'playwright';

import { expect, test, type LaunchedApp } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

/** A 640×360 composition: a #FF3300 box at (200, 120) over #203040. */
async function solidsProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Transport', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 80, h: 40 } }, project);
    box.p['position.x'].v = 200; box.p['position.y'].v = 120;
    project.layers = [box];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
  });
}

async function openEngineViewer(session: LaunchedApp): Promise<void> {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
  await solidsProject(page);
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
  await expect.poll(() => page.evaluate(() => document.getElementById('engine-present')?.style.visibility), { timeout: 20_000 }).toBe('visible');
}

/** The RGB of a data URL's pixel (decoded in the page). */
const dataUrlPixel = (page: Page, url: string, x: number, y: number): Promise<number[]> => page.evaluate(async ({ url, x, y }) => {
  const image = new Image();
  image.src = url;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.width; canvas.height = image.height;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(image, 0, 0);
  const d = ctx.getImageData(x, y, 1, 1).data;
  return [d[0]!, d[1]!, d[2]!, image.width, image.height];
}, { url, x, y });

const near = (a: number[], b: number[], tolerance: number): boolean => b.every((v, i) => Math.abs((a[i] ?? -999) - v) <= tolerance);

test.describe('Use New Engine: the render transport', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('sync and async snapshots come from the engine, pinned to the edit just made', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    // apiVersion 1-3: render.snapshot returns a JPEG data URL synchronously.
    const first = await page.evaluate(() => {
      const api = (window as any).PM.Kernel.api('transport-probe');
      const url = api.render.snapshot(0.5, 320);
      return { url, stats: { ...(window as any).PM.EngineRender.stats } };
    });
    expect(first.url.startsWith('data:image/jpeg')).toBe(true);
    expect(first.stats.sync).toBeGreaterThanOrEqual(1);
    // 320 wide at half scale: the box center (240, 140) → (120, 70).
    const pixel = await dataUrlPixel(page, first.url, 120, 70);
    expect(pixel.slice(3)).toEqual([320, 180]);
    expect(near(pixel, [0xff, 0x33, 0x00], 24)).toBe(true);
    // An edit, then at once a synchronous frame: the edit is in it
    // (read-your-writes), through renderFrameTo's canvas.
    const edited = await page.evaluate(() => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const layer = PM.proj.layers[0];
      PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 50 }, { label: 'Opacity', origin: 'inspector' });
      const revision = PM.EngineDoc.barrier.revision;
      const canvas = api.render.renderFrameTo(0.5, 640, 360, { mblur: false });
      const d = canvas.getContext('2d').getImageData(240, 140, 1, 1).data;
      return { rgb: [d[0], d[1], d[2]], revision, width: canvas.width };
    });
    expect(edited.width).toBe(640);
    // #FF3300 at 50% over #203040.
    expect(near(edited.rgb, [0x90, 0x32, 0x20], 3)).toBe(true);
    // Async siblings and an idle UI snapshot, revision-tagged.
    const asyncResult = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const url = await api.render.snapshotAsync(0.5, 640);
      const thumb = await PM.EngineRender.thumbnails.request({ key: 'probe', time: 0.5, width: 64, height: 36 }, () => PM.EngineDoc.barrier.revision);
      return { url, thumb: thumb ? { width: thumb.width, revision: thumb.revision } : null, revision: PM.EngineDoc.barrier.revision, stats: { ...PM.EngineRender.stats } };
    });
    expect(asyncResult.url.startsWith('data:image/jpeg')).toBe(true);
    expect(asyncResult.thumb).toEqual({ width: 64, revision: asyncResult.revision });
    expect(asyncResult.stats.async).toBeGreaterThanOrEqual(1);
    expect(asyncResult.stats.idle).toBeGreaterThanOrEqual(1);
    expect(asyncResult.stats.failed).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a synchronous snapshot inside a raster wrapper and inside an animation frame does not deadlock', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const base = api.services.get('raster');
      const inside: string[] = [];
      const off = api.services.register('raster', (...args: any[]) => {
        // An old extension's override calling the synchronous API from
        // inside the raster service.
        inside.push(api.render.snapshot(0.25, 160));
        return base(...args);
      });
      const text = PM.mkLayer('text', { name: 'T', from: 0, dur: 4, d: { text: 'Hi', size: 40, color: '#FFFFFF' } }, PM.proj);
      api.render.raster(text, 1, 0.25);
      off.dispose?.();
      const inFrame = await new Promise<string>((resolve) => requestAnimationFrame(() => resolve(api.render.snapshot(0.25, 160))));
      return { inside, inFrame };
    });
    expect(result.inside.length).toBeGreaterThanOrEqual(1);
    for (const url of result.inside) expect(url.startsWith('data:image/jpeg')).toBe(true);
    expect(result.inFrame.startsWith('data:image/jpeg')).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('frame events fire on presentation with the exact previewViewport object, and engine stats reach the status bar', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const w = window as any;
      w.__frames = [];
      w.__order = [];
      api.events.on('frame:rendered', (frame: any) => {
        w.__frames.push({ same: frame.viewport === api.render.gl.previewViewport, time: frame.time, version: frame.version, quality: frame.quality });
        w.__order.push('frame');
      });
      api.events.on('overlay', () => w.__order.push('overlay'));
    });
    // An edit makes a new engine frame on screen.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers[0];
      PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'position.x', value: 260 }, { label: 'Move', origin: 'inspector' });
    });
    await expect.poll(() => page.evaluate(() => (window as any).__frames.length), { timeout: 20_000 }).toBeGreaterThanOrEqual(1);
    const seen = await page.evaluate(() => ({ frames: (window as any).__frames, order: (window as any).__order, version: (window as any).PM.animVersion() }));
    expect(seen.frames.every((f: any) => f.same)).toBe(true);
    expect(seen.frames.at(-1).version).toBe(seen.version);
    expect(seen.frames.at(-1).quality).toBeGreaterThan(0);
    // `overlay` follows each frame:rendered, as today's frame loop does.
    const firstFrame = seen.order.indexOf('frame');
    expect(seen.order[firstFrame + 1]).toBe('overlay');
    // Engine stats replace GL.stats.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineStats?.perf?.()?.ms ?? 0), { timeout: 10_000 }).toBeGreaterThan(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
