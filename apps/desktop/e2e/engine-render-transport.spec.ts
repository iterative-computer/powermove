/*
 * The Phase 3c render transport with "Use New Engine" on
 * (docs/engine/rfcs/16-render-transport.md): the apiVersion 1 to 3
 * synchronous render API (`render.snapshot`, `render.renderFrameTo`) draws
 * through the engine, pinned to the revision just edited; the async
 * siblings and the UI's idle snapshots; a synchronous snapshot from inside
 * a `raster` wrapper and from inside an animation-frame callback, which
 * must not deadlock; and frame events on presentation (`frame:rendered`
 * with the exact `render.gl.previewViewport` object, then `overlay`) with
 * engine stats for the status bar. With text (the engine's render fonts):
 * the viewer, snapshots, layer rasters (today's surface shape), and the
 * exporter's captures; and a sync snapshot during a staged resync, served
 * by main's session for the window with the document.
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
  // A frame on screen (not just drawn): later frame events are the test's own.
  await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.enginePresentation?.presenting === true), { timeout: 20_000 }).toBe(true);
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

  test('a project with text: the viewer, snapshots, rasters, and the app’s captures all draw it with the engine', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Text', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
      project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
      const title = PM.mkLayer('text', { name: 'Title', from: 0, dur: 4, d: { text: 'Engine text', font: 'Helvetica', size: 64, weight: 700, color: '#FFFFFF' } }, project);
      title.id = 'title';
      title.p['position.x'].v = 320; title.p['position.y'].v = 180;
      project.layers = [title];
      PM.replaceProject(project);
      PM.Projects?.put?.(PM.proj);
    });
    // Text no longer routes the viewer away: the engine draws it.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const before = { ...PM.EngineRender.stats };
      const url = api.render.snapshot(0.5, 320);
      const canvas = api.render.renderFrameTo(0.5, 320, 180, { mblur: false });
      const later = await api.render.snapshotAsync(0.5, 320);
      // The exporter's own captures (agent look and observe, Library thumbnails).
      const look = PM.Export.snapshot(0.5, 320);
      const observe = await PM.Export.snapshotAsync(0.5, 320);
      const layer = PM.proj.layers[0];
      const raster = api.render.raster(layer, 1, 0.5);
      const rasterAsync = await api.render.rasterAsync(layer, 1, 0.5);
      // A wrapper's modified copy (not the document's object) is drawn as passed.
      const copy = api.render.raster({ ...layer, d: { ...layer.d, text: 'Eng' } }, 1, 0.5);
      const ink = (c: HTMLCanvasElement) => {
        const d = c.getContext('2d')!.getImageData(0, 0, c.width, c.height).data;
        let n = 0;
        for (let i = 0; i < d.length; i += 4) if (d[i]! > 200 && d[i + 1]! > 200 && d[i + 2]! > 200) n++;
        return n;
      };
      return {
        url, later, look, observe,
        frameInk: ink(canvas),
        raster: raster ? { keys: Object.keys(raster).sort(), width: raster.cv.width, height: raster.cv.height, anchorX: raster.anchorX, selection: !!raster.selection, revision: raster.revision, blank: raster.blank } : null,
        rasterAsyncKeys: rasterAsync ? Object.keys(rasterAsync).sort() : null,
        copyWidth: copy ? copy.cv.width : null,
        captures: !!PM.EngineCaptures,
        sync: PM.EngineRender.stats.sync - before.sync,
        failed: PM.EngineRender.stats.failed - before.failed
      };
    });
    for (const url of [result.url, result.later, result.look, result.observe]) expect(url.startsWith('data:image/jpeg')).toBe(true);
    expect(result.frameInk).toBeGreaterThan(200);
    expect(result.captures).toBe(true);
    // render.snapshot, renderFrameTo, PM.Export.snapshot, and the raster.
    expect(result.sync).toBeGreaterThanOrEqual(4);
    expect(result.failed).toBe(0);
    expect(result.raster).not.toBeNull();
    expect(result.raster!.width).toBeGreaterThan(100);
    expect(Number.isFinite(result.raster!.anchorX)).toBe(true);
    expect(result.raster!.selection).toBe(true);
    expect(result.raster!.blank).toBe(false);
    expect(typeof result.raster!.revision).toBe('number');
    expect(result.rasterAsyncKeys).toEqual(result.raster!.keys);
    expect(result.copyWidth).toBeGreaterThan(0);
    expect(result.copyWidth).toBeLessThan(result.raster!.width);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a sync snapshot while a staged resync is in flight returns pixels at once (main serves it with the document)', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      // A document over the inline resync limit: the viewer session stages
      // it, so its resync waits for main while this thread is blocked.
      const project = PM.mkProject({ name: 'Large', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
      project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
      const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 80, h: 40 } }, project);
      box.p['position.x'].v = 200; box.p['position.y'].v = 120;
      project.layers = [box];
      project.params = { notes: 'x'.repeat(13 * 1024 * 1024) };
      PM.replaceProject(project);
      const pending = PM.EngineConnection.session()?.resyncPending ?? null;
      const before = PM.EngineRender.stats.documentSync;
      const started = performance.now();
      const url = api.render.snapshot(0.5, 320);
      return { url, ms: performance.now() - started, pending, documentSync: PM.EngineRender.stats.documentSync - before };
    });
    console.log(`sync snapshot during a staged resync: ${result.ms.toFixed(1)} ms`);
    expect(result.url.startsWith('data:image/jpeg')).toBe(true);
    expect(result.pending).toBe(true);
    expect(result.documentSync).toBe(1);
    // Not the 5 s sync wait: served without the blocked renderer's resync.
    expect(result.ms).toBeLessThan(4000);
    const pixel = await dataUrlPixel(page, result.url, 120, 70);
    expect(near(pixel, [0xff, 0x33, 0x00], 24)).toBe(true);
    // The viewer's own session catches up afterwards.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.session()?.resyncPending), { timeout: 30_000 }).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a sync snapshot of a program only the renderer draws runs the fallback on the calling thread (real WebGL)', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      const project = PM.mkProject({ name: 'Fallback', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
      project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
      // A samplerCube uniform: the native pipeline cannot translate it, the
      // renderer's GLSL fallback runner draws it (decision 19).
      const code = 'uniform samplerCube uEnv;\nvoid main(){ fragColor = vec4(texture(uEnv, vec3(uv, 1.0)).rgb + vec3(uv, 0.0), 1.0); }';
      const sky = PM.mkLayer('shader', { name: 'Sky', from: 0, dur: 4, d: { code, uniforms: {}, w: 320, h: 180 } }, project);
      // Its top-left at (160, 90): it spans 160..480 × 90..270.
      sky.p['anchor.x'].v = 0; sky.p['anchor.y'].v = 0;
      sky.p['position.x'].v = 160; sky.p['position.y'].v = 90;
      project.layers = [sky];
      PM.replaceProject(project);
      PM.Projects?.put?.(PM.proj);
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    const result = await page.evaluate(() => {
      const PM = (window as any).PM;
      const api = PM.Kernel.api('transport-probe');
      const before = { ...PM.EngineRender.stats };
      // A size the viewer never drew: the program is asked for again.
      const url = api.render.snapshot(0.5, 300);
      return { url, rounds: PM.EngineRender.stats.rounds - before.rounds, failed: PM.EngineRender.stats.failed - before.failed };
    });
    expect(result.failed).toBe(0);
    expect(result.rounds).toBeGreaterThanOrEqual(1);
    expect(result.url.startsWith('data:image/jpeg')).toBe(true);
    // The program's pixels where the layer is (what the runner drew on this
    // thread), the background elsewhere.
    const left = await dataUrlPixel(page, result.url, 90, 84);
    const right = await dataUrlPixel(page, result.url, 205, 84);
    const background = await dataUrlPixel(page, result.url, 10, 10);
    console.log(`fallback snapshot pixels: left ${left.slice(0, 3)} right ${right.slice(0, 3)} background ${background.slice(0, 3)}`);
    expect(near(background, [0x20, 0x30, 0x40], 12)).toBe(true);
    // vec4(uv, 0, 1) over an empty cube map: red is uv.x (0.1 and 0.87 at
    // these points), green uv.y (0.5).
    expect(near(left, [26, 128, 0], 8)).toBe(true);
    expect(near(right, [222, 128, 0], 8)).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('an interactive engine crash keeps the last frame with a visible state, never the current engine, and recovers', async ({ session }) => {
    const { page } = session;
    await openEngineViewer(session);
    const before = await page.evaluate(() => (window as any).PM.EngineConnection.status().connections);
    // The legacy canvas never shows with the engine on.
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').el.style.visibility)).toBe('hidden');
    // Watch every animation frame while the engine restarts: the state the
    // viewer shows, whether the last frame stays, whether the current
    // engine's canvas ever shows, and who drives the transport.
    await page.evaluate(() => {
      const w = window as any;
      const PM = w.PM;
      w.__restart = { statuses: [] as string[], presentingDropped: false, legacyShown: false, legacyDrove: false };
      const watch = () => {
        const r = w.__restart;
        const text = document.querySelector('[data-engine-status]')?.textContent;
        if (text && r.statuses.at(-1) !== text) r.statuses.push(text);
        if (document.getElementById('engine-present')?.style.visibility !== 'visible') r.presentingDropped = true;
        if (PM.Kernel.services.get('viewer').el.style.visibility !== 'hidden') r.legacyShown = true;
        if (!PM.EngineConnection.transport.active()) r.legacyDrove = true;
        if (!r.stop) requestAnimationFrame(watch);
      };
      requestAnimationFrame(watch);
    });
    const pid = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
    expect(pid).toBeGreaterThan(0);
    process.kill(pid, 'SIGKILL');
    // The engine is back: a new session presents, the state clears, and
    // the synchronous path works again.
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().connections), { timeout: 45_000 }).toBeGreaterThan(before);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await expect.poll(() => page.evaluate(() => document.querySelector('[data-engine-status]') === null), { timeout: 20_000 }).toBe(true);
    const restart = await page.evaluate(() => {
      const r = (window as any).__restart;
      r.stop = true;
      return r;
    });
    // The last frame stayed, the state said so, and the current engine
    // never drew or drove anything.
    expect(restart.statuses).toContain('Reconnecting to the engine…');
    expect(restart.presentingDropped).toBe(false);
    expect(restart.legacyShown).toBe(false);
    expect(restart.legacyDrove).toBe(false);
    const url = await page.evaluate(() => (window as any).PM.Kernel.api('transport-probe').render.snapshot(0.5, 160));
    expect(url.startsWith('data:image/jpeg')).toBe(true);
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
    // Frames of the earlier revision may still come first (the view
    // settling); the edit's frame carries its revision's animVersion.
    await expect.poll(() => page.evaluate(() => (window as any).__frames.some((f: any) => f.version === (window as any).PM.animVersion())), { timeout: 20_000 }).toBe(true);
    const seen = await page.evaluate(() => ({ frames: (window as any).__frames, order: (window as any).__order, version: (window as any).PM.animVersion() }));
    expect(seen.frames.every((f: any) => f.same)).toBe(true);
    // Versions only move forward, and the newest is the edit's.
    const versions = seen.frames.map((f: any) => f.version);
    expect(versions).toEqual([...versions].sort((a: number, b: number) => a - b));
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
