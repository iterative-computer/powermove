/*
 * The Phase 2 tracer bullet in the real app (plan section 26, Phase 2;
 * docs/engine/tracer.md), behind "Use New Engine": the viewer shows
 * engine frames for a composition the engine supports, edits reach the
 * engine through pm-wasm's patch stream, overlays line up with engine
 * geometry, unsupported compositions stay with the current engine without
 * errors, playback runs on the engine's clock, a killed engine and a lost
 * GPU recover, and switching engines loses nothing.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/native-engine-tracer.spec.ts --workers=1
 */
import { rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { mkdtemp } from 'node:fs/promises';

import type { Page } from 'playwright';

import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const OFF = { POWERMOVE_NEW_ENGINE: '0' };

type Status = {
  connected: boolean; generation: number; route: string; routeReason: string | null; frames: number; revision: number;
  resyncs: number; batches: number; failures: string[]; driving: boolean; hostOffsetUs: number | null;
  clock: { state: string; mediaTime: number; epoch: number } | null;
};

const status = (page: Page): Promise<Status> => page.evaluate(() => (window as any).PM.EngineConnection.status());

/** A 640×360, 30 fps, 4 s composition with a background and two solids. */
async function tracerProject(page: Page, extra: { fx?: boolean } = {}): Promise<string> {
  return page.evaluate((extra) => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Tracer', w: 640, h: 360, fps: 30, dur: 4, bg: '#203040' });
    project.backgroundFill = { type: 'solid', angle: 0, stops: [{ id: 's1', color: '#203040', position: 0 }] };
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 4, d: { color: '#FF3300', w: 80, h: 40 } }, project);
    box.id = 'box';
    box.p['anchor.x'].v = 0; box.p['anchor.y'].v = 0;
    box.p['position.x'].v = 100; box.p['position.y'].v = 60;
    const bar = PM.mkLayer('solid', { name: 'Bar', from: 0, dur: 4, d: { color: '#33CC66', w: 200, h: 20 } }, project);
    bar.id = 'bar';
    bar.p['anchor.x'].v = 0; bar.p['anchor.y'].v = 0;
    bar.p['position.x'].v = 300; bar.p['position.y'].v = 250;
    if (extra.fx) box.fx.push(PM.mkEffect('invert'));
    project.layers = [box, bar];
    PM.replaceProject(project);
    PM.Projects?.put?.(PM.proj);
    return PM.proj.id as string;
  }, extra);
}

async function waitRoute(page: Page, route: 'engine' | 'legacy', timeout = 45_000): Promise<void> {
  await expect.poll(async () => (await status(page)).route, { timeout }).toBe(route);
}

async function enginePresenting(page: Page): Promise<boolean> {
  return page.evaluate(() => {
    const canvas = document.getElementById('engine-present');
    return !!canvas && canvas.style.visibility === 'visible';
  });
}

/** The engine frame on screen (viewer `engineFrame()`): tags and overlay geometry in stage CSS pixels. */
async function engineFrame(page: Page): Promise<any> {
  return page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.engineFrame?.() ?? null);
}

/** The mirror's revision (what the engine's frames must reach). */
const mirrorRevision = (page: Page): Promise<number> => page.evaluate(() => {
  const doc = (window as any).PM.EngineDoc;
  doc.barrier.flush();
  return doc.barrier.revision as number;
});

/** Where the viewer puts the composition and a layer, from its own layout (stage CSS px). */
async function viewerGeometry(page: Page, layerId: string): Promise<{ x: number; y: number; width: number; height: number; layer: { x0: number; y0: number; x1: number; y1: number } }> {
  return page.evaluate((layerId) => {
    const PM = (window as any).PM;
    const V = PM.Kernel.services.get('viewer');
    const stage = V.stage.getBoundingClientRect();
    const p = PM.proj;
    const z = V.shown;
    const x = (stage.width - p.w * z) / 2 + (V.fit ? 0 : V.pan[0]);
    const y = (stage.height - p.h * z) / 2 + (V.fit ? 0 : V.pan[1]);
    const layer = p.layers.find((l: any) => l.id === layerId);
    // pm-wasm's bounds (layer space) through the layer's world matrix.
    const b = PM.GL.bounds(layer, PM.time);
    const m = PM.worldMatrix(layer, PM.time);
    const corners = [[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]].map(([u, v]) => [m[0] * u + m[2] * v + m[4], m[1] * u + m[3] * v + m[5]]);
    const xs = corners.map((c) => x + c[0]! * z);
    const ys = corners.map((c) => y + c[1]! * z);
    return { x, y, width: p.w * z, height: p.h * z, layer: { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) } };
  }, layerId);
}

function quadBox(quad: [number, number][]): { x0: number; y0: number; x1: number; y1: number } {
  const xs = quad.map((p) => p[0]);
  const ys = quad.map((p) => p[1]);
  return { x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys) };
}

/** A pixel of the window (capturePage of the hidden window, never the screen). */
async function windowPixel(session: LaunchedApp, cssX: number, cssY: number): Promise<number[]> {
  const origin = await session.page.evaluate(() => {
    const rect = (window as any).PM.Kernel.services.get('viewer').stage.getBoundingClientRect();
    return { x: rect.left, y: rect.top };
  });
  return session.app.evaluate(async ({ BrowserWindow }, at) => {
    const window = BrowserWindow.getAllWindows().find((w) => !w.isDestroyed() && w.webContents.getURL().startsWith('app:')) ?? BrowserWindow.getAllWindows()[0]!;
    const image = await window.webContents.capturePage({ x: Math.round(at.x), y: Math.round(at.y), width: 1, height: 1 });
    const bitmap = image.toBitmap();
    // BGRA.
    return [bitmap[2]!, bitmap[1]!, bitmap[0]!, bitmap[3]!];
  }, { x: origin.x + cssX, y: origin.y + cssY });
}

async function openTracer(session: LaunchedApp, extra: { fx?: boolean } = {}): Promise<string> {
  await session.openEditor();
  const { page } = session;
  await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
  const id = await tracerProject(page, extra);
  return id;
}

test.describe('Use New Engine: the tracer bullet', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('the viewer shows engine frames of a supported composition, with overlays on the engine geometry', async ({ session }) => {
    const { page } = session;
    await openTracer(session);
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    const revision = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 20_000 }).toBeGreaterThanOrEqual(revision);
    const frame = await engineFrame(page);
    const view = await viewerGeometry(page, 'box');
    // The composition sits where the viewer lays it out (snapped to device pixels).
    expect(Math.abs(frame.overlay.viewport.x - view.x)).toBeLessThanOrEqual(0.5);
    expect(Math.abs(frame.overlay.viewport.y - view.y)).toBeLessThanOrEqual(0.5);
    expect(Math.abs(frame.overlay.viewport.width - view.width)).toBeLessThanOrEqual(1);
    expect(Math.abs(frame.overlay.viewport.height - view.height)).toBeLessThanOrEqual(1);
    // The engine's quad for the layer is where pm-wasm's bounds put it.
    const quad = quadBox(frame.overlay.layers.find((l: any) => l.layer === 'box').bounds);
    for (const key of ['x0', 'y0', 'x1', 'y1'] as const) expect(Math.abs(quad[key] - view.layer[key])).toBeLessThanOrEqual(0.75);
    // The pixels: the layer's color inside it, the composition background,
    // and the viewer's stage (not black) around the composition.
    const inside = await windowPixel(session, (view.layer.x0 + view.layer.x1) / 2, (view.layer.y0 + view.layer.y1) / 2);
    expect(Math.abs(inside[0]! - 0xff)).toBeLessThanOrEqual(3);
    expect(Math.abs(inside[1]! - 0x33)).toBeLessThanOrEqual(3);
    expect(inside[2]!).toBeLessThanOrEqual(3);
    const background = await windowPixel(session, view.x + 5, view.y + view.height - 5);
    expect(background.slice(0, 3).map((v, i) => Math.abs(v - [0x20, 0x30, 0x40][i]!)).every((d) => d <= 3)).toBe(true);
    const stageBackground = await page.evaluate(() => getComputedStyle((window as any).PM.Kernel.services.get('viewer').stage.closest('.body') ?? document.body).backgroundColor);
    if (view.x > 4) {
      const around = await windowPixel(session, view.x / 2, view.y + view.height / 2);
      expect(around.slice(0, 3), `the stage shows around the composition (${stageBackground})`).not.toEqual([0, 0, 0]);
    }
    // The legacy canvas rests underneath.
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').el.style.visibility)).toBe('hidden');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('inspector edits and canvas drags reach the engine through the patch stream', async ({ session }) => {
    const { page } = session;
    await openTracer(session);
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    const before = quadBox((await engineFrame(page)).overlay.layers.find((l: any) => l.layer === 'box').bounds);
    const shown = await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').shown as number);
    // An inspector edit (the Edit command the inspector applies).
    const latencyMs = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const viewer = PM.Kernel.services.get('viewer');
      const start = performance.now();
      PM.Edit.apply({ type: 'set_property', target: 'box', path: 'position.x', value: 160 }, { label: 'Position', origin: 'inspector' });
      PM.EngineDoc.barrier.flush();
      const wanted = PM.EngineDoc.barrier.revision;
      return new Promise<number>((resolve) => {
        const off = viewer.enginePresentation.onPresented((frame: any) => {
          if (frame.revision >= wanted) {
            off();
            resolve(performance.now() - start);
          }
        });
      });
    });
    const moved = quadBox((await engineFrame(page)).overlay.layers.find((l: any) => l.layer === 'box').bounds);
    expect(moved.x0 - before.x0).toBeCloseTo(60 * shown, 1);
    expect(latencyMs).toBeLessThan(1000);
    // A canvas drag: the select tool moves the layer; the frame follows.
    await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').engineFrame());
    const view = await viewerGeometry(page, 'bar');
    const stage = await page.evaluate(() => {
      const r = (window as any).PM.Kernel.services.get('viewer').stage.getBoundingClientRect();
      return { x: r.left, y: r.top };
    });
    const startX = stage.x + (view.layer.x0 + view.layer.x1) / 2;
    const startY = stage.y + (view.layer.y0 + view.layer.y1) / 2;
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    for (let i = 1; i <= 10; i++) await page.mouse.move(startX + i * 4, startY + i * 2);
    await page.mouse.up();
    const dragged = await page.evaluate(() => {
      const bar = (window as any).PM.proj.layers.find((l: any) => l.id === 'bar');
      return { x: bar.p['position.x'].v, y: bar.p['position.y'].v };
    });
    expect(dragged.x).not.toBe(300);
    const wanted = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 10_000 }).toBeGreaterThanOrEqual(wanted);
    const after = await viewerGeometry(page, 'bar');
    const engineBar = quadBox((await engineFrame(page)).overlay.layers.find((l: any) => l.layer === 'bar').bounds);
    for (const key of ['x0', 'y0', 'x1', 'y1'] as const) expect(Math.abs(engineBar[key] - after.layer[key])).toBeLessThanOrEqual(0.75);
    // Undo reaches the engine too.
    await page.evaluate(() => (window as any).PM.hist.undo());
    const undone = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 10_000 }).toBeGreaterThanOrEqual(undone);
    const restored = quadBox((await engineFrame(page)).overlay.layers.find((l: any) => l.layer === 'bar').bounds);
    expect(Math.abs(restored.x0 - view.layer.x0)).toBeLessThanOrEqual(0.75);
    expect((await status(page)).failures.filter((f) => !f.startsWith('deviceLost'))).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a composition with unsupported features stays with the current engine, without errors', async ({ session }) => {
    const { page } = session;
    await openTracer(session, { fx: true });
    await waitRoute(page, 'legacy');
    expect((await status(page)).routeReason).toMatch(/effects/);
    expect(await enginePresenting(page)).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').el.style.visibility)).toBe('');
    // The legacy compositor draws it.
    await expect.poll(() => page.evaluate(() => Boolean((window as any).PM.GL.gl))).toBe(true);
    // Removing the effect brings engine frames back; adding one routes again.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.hist.do('Remove effect', () => { PM.proj.layers.find((l: any) => l.id === 'box').fx = []; });
      PM.touch();
    });
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.hist.do('Add effect', () => { PM.proj.layers.find((l: any) => l.id === 'bar').fx.push(PM.mkEffect('invert')); });
      PM.touch();
    });
    await waitRoute(page, 'legacy');
    await expect.poll(() => enginePresenting(page)).toBe(false);
    const errors = session.diagnostics.console.filter((c) => c.type === 'error');
    expect(errors.map((e) => e.text)).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('playback runs on the engine clock: play, pause, seek, loop, frame steps', async ({ session }) => {
    const { page } = session;
    await openTracer(session);
    await waitRoute(page, 'engine');
    await expect.poll(async () => (await status(page)).driving, { timeout: 20_000 }).toBe(true);
    await page.evaluate(() => { const PM = (window as any).PM; PM.loop = false; PM.setTime(0.5); PM.play(); });
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 10_000 }).toBe('playing');
    const a = await page.evaluate(() => ({ t: (window as any).PM.time as number, now: performance.now() }));
    await page.waitForTimeout(1000);
    const b = await page.evaluate(() => ({ t: (window as any).PM.time as number, now: performance.now() }));
    const rate = (b.t - a.t) / ((b.now - a.now) / 1000);
    expect(rate).toBeGreaterThan(0.95);
    expect(rate).toBeLessThan(1.05);
    // Timed engine frames are on screen while playing.
    const frame = await engineFrame(page);
    expect(frame.pts / 705_600_000).toBeGreaterThan(0.5);
    await page.evaluate(() => (window as any).PM.pause());
    await expect.poll(async () => (await status(page)).clock?.state).toBe('stopped');
    const paused = await page.evaluate(() => (window as any).PM.time as number);
    expect(Math.abs(paused * 30 - Math.round(paused * 30))).toBeLessThan(1e-6);
    // A step and a seek show their frames.
    await page.evaluate(() => (window as any).PM.step(3));
    const stepped = paused + 3 / 30;
    await expect.poll(async () => Math.abs(((await engineFrame(page))?.pts ?? 0) / 705_600_000 - stepped), { timeout: 5000 }).toBeLessThan(1e-6);
    await page.evaluate(() => (window as any).PM.setTime(3.9));
    // Without the loop a run stops at the end.
    await page.evaluate(() => (window as any).PM.play());
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing), { timeout: 10_000 }).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.time)).toBeGreaterThanOrEqual(3.9);
    // With the loop on the work area, it wraps.
    await page.evaluate(() => { const PM = (window as any).PM; PM.proj.work = [1, 1.5]; PM.loop = true; PM.setTime(1.2); PM.play(); });
    const times: number[] = [];
    for (let i = 0; i < 20; i++) {
      times.push(await page.evaluate(() => (window as any).PM.time as number));
      await page.waitForTimeout(100);
    }
    await page.evaluate(() => (window as any).PM.pause());
    expect(Math.max(...times)).toBeLessThan(1.5 + 1e-6);
    expect(times.some((t, i) => i > 0 && t < times[i - 1]!)).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a killed interactive engine and a lost GPU recover and resync', async ({ session }) => {
    const { page, app } = session;
    await openTracer(session);
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    const before = await status(page);
    const pid = await app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
    process.kill(pid, 'SIGKILL');
    // While it restarts the current engine draws; then a new launch resyncs.
    await expect.poll(async () => (await status(page)).generation, { timeout: 20_000 }).toBeGreaterThan(before.generation);
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    const after = await status(page);
    expect(after.resyncs).toBeGreaterThanOrEqual(1);
    expect(after.revision).toBe(await mirrorRevision(page));
    // Edits after the restart reach the new engine.
    await page.evaluate(() => (window as any).PM.Edit.apply({ type: 'set_property', target: 'box', path: 'position.y', value: 90 }, { label: 'Position' }));
    const wanted = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 10_000 }).toBeGreaterThanOrEqual(wanted);

    // A GPU device loss: the presenter rebuilds its device and pool and draws again.
    const generation = await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').enginePresentation.stats()?.generation ?? 0);
    expect(await app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.simulateGpuLoss())).toBe(true);
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').enginePresentation.stats()?.generation ?? 0), { timeout: 20_000 }).toBeGreaterThan(generation);
    await page.evaluate(() => (window as any).PM.Edit.apply({ type: 'set_property', target: 'box', path: 'position.y', value: 120 }, { label: 'Position' }));
    const lost = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 20_000 }).toBeGreaterThanOrEqual(lost);
    expect(await enginePresenting(page)).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});

/** The project as stored, with keys sorted (the engine-switch comparison). */
async function storedProject(page: Page, id: string): Promise<string> {
  return page.evaluate((id) => {
    const PM = (window as any).PM;
    const project = PM.proj?.id === id ? PM.proj : PM.Projects.get(id);
    const sort = (value: any): any => {
      if (Array.isArray(value)) return value.map(sort);
      if (value && typeof value === 'object') {
        const out: any = {};
        for (const key of Object.keys(value).sort()) {
          // Timestamps of opening and saving are not content.
          if (['opened', 'updated', 'modified', 'savedAt'].includes(key)) continue;
          if (value[key] !== undefined) out[key] = sort(value[key]);
        }
        return out;
      }
      return value;
    };
    return JSON.stringify(sort(JSON.parse(JSON.stringify(project))));
  }, id);
}

async function reopen(page: Page, id: string): Promise<void> {
  await page.waitForFunction(() => Boolean((window as any).PM?.openProjectHere));
  if (await page.evaluate(() => (window as any).PM.proj?.id) !== id) {
    const opened = await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      const ok = await PM.openProjectHere(id);
      if (ok) PM.ProjectsScreen?.hide?.();
      return ok;
    }, id);
    expect(opened).toBe(true);
  }
}

test('switching engines on, off, and on again loses nothing', async () => {
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-tracer-switch-'));
  try {
    let session = await launchApp({ userData, env: ON });
    let id = '';
    try {
      id = await openTracer(session);
      const { page } = session;
      await waitRoute(page, 'engine');
      // Keyframes and markers: content both engines must keep.
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.hist.do('Animate', () => {
          const box = PM.proj.layers.find((l: any) => l.id === 'box');
          PM.setKeyOn(box.p.opacity, 0, 100, 'linear', 30);
          PM.setKeyOn(box.p.opacity, 2, 25, 'linear', 30);
          PM.proj.markers = [{ id: 'm1', t: 1, label: 'Beat' }];
        });
        PM.touch();
        PM.Projects.put(PM.proj);
      });
    } finally { await session.close(); }

    // The project as the new engine opens it is the baseline (opening fills defaults).
    let baseline = '';
    session = await launchApp({ userData, env: ON });
    try {
      await reopen(session.page, id);
      baseline = await storedProject(session.page, id);
      await waitRoute(session.page, 'engine');
    } finally { await session.close(); }

    session = await launchApp({ userData, env: OFF });
    try {
      const { page } = session;
      await reopen(page, id);
      expect(await page.evaluate(() => Boolean((window as any).PM.EngineConnection))).toBe(false);
      expect(await storedProject(page, id)).toBe(baseline);
      await page.evaluate(() => (window as any).PM.Projects.put((window as any).PM.proj));
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { await session.close(); }

    session = await launchApp({ userData, env: ON });
    try {
      const { page } = session;
      await reopen(page, id);
      expect(await storedProject(page, id)).toBe(baseline);
      await waitRoute(page, 'engine');
      await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { await session.close(); }
  } finally {
    await rm(userData, { recursive: true, force: true });
  }
});

test.describe('Settings', () => {
  test.use({ desktopLaunchOptions: { env: OFF } });

  test('Use New Engine is a switch in General that applies at the next launch', async ({ session }) => {
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
    const toggle = page.getByRole('button', { name: 'Use New Engine' });
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByText('Restart Powermove to switch engines.')).toBeVisible();
    expect(await page.evaluate(() => (window as any).PM.store.get('useNewEngine'))).toBe(true);
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
  });
});
