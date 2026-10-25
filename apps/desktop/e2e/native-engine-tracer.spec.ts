/*
 * The Phase 2 tracer bullet in the real app (plan section 26, Phase 2;
 * docs/engine/tracer.md), behind "Use New Engine": the viewer shows
 * engine frames for a composition the engine supports, edits reach the
 * engine through pm-wasm's patch stream, overlays line up with engine
 * geometry, a composition the engine cannot draw says why on the stage
 * while the transport stays on the engine (amendment 21), playback runs on the engine's clock, a killed engine and a lost
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
import { fixturePath } from './helpers/media';
import { buildSyntheticProject } from '../src/main/engine/integration/synthetic-project';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const OFF = { POWERMOVE_NEW_ENGINE: '0' };

type Status = {
  connected: boolean; generation: number; route: string; routeReason: string | null; frames: number; revision: number;
  resyncs: number; batches: number; failures: string[]; driving: boolean; hostOffsetUs: number | null;
  clock: { state: string; mediaTime: number; epoch: number } | null;
};

const status = (page: Page): Promise<Status> => page.evaluate(() => (window as any).PM.EngineConnection.status());

/** A 640×360, 30 fps, 4 s composition with a background and two solids. */
async function tracerProject(page: Page, extra: { fx?: boolean; pathMask?: boolean; unknownEffect?: boolean } = {}): Promise<string> {
  return page.evaluate((extra) => {
    const PM = (window as any).PM;
    // A bezier path mask (drawn by the engine since compclose).
    const pathMask = (PM: any, comp: any) => {
      const P = PM.P;
      const vertex = (i: number, x: number, y: number) => ({ id: `mv${i}`, p: { x: P(x), y: P(y), inX: P(0), inY: P(0), outX: P(0), outY: P(0) } });
      const channels: Record<string, unknown> = { x: 40, y: 20, rotation: 0, scaleX: 100, scaleY: 100, closed: true, fill: '#ffffff', fillEnabled: true, fillOpacity: 100, stroke: '#ffffff', strokeWidth: 2, strokeOpacity: 100, trimStart: 0, trimEnd: 100, trimOffset: 0, copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0 };
      const mask = PM.mkMask('rect', comp);
      mask.path = { id: 'mp1', name: 'Path', parent: null, p: Object.fromEntries(Object.entries(channels).map(([k, v]) => [k, P(v)])), vertices: [vertex(0, -30, -15), vertex(1, 30, -15), vertex(2, 0, 15)] };
      return mask;
    };
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
    if (extra.pathMask) box.masks.push(pathMask(PM, project));
    if (extra.unknownEffect) {
      // An effect no extension defines (a disabled or removed extension's):
      // the engine has no definition to draw it with.
      const fx = PM.mkEffect('invert');
      fx.type = 'e2e-unknown-effect';
      box.fx.push(fx);
    }
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

async function openTracer(session: LaunchedApp, extra: { fx?: boolean; pathMask?: boolean; unknownEffect?: boolean } = {}): Promise<string> {
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
      const before = PM.EngineDoc.barrier.revision;
      PM.Edit.apply({ type: 'set_property', target: 'box', path: 'position.x', value: 160 }, { label: 'Position', origin: 'inspector' });
      // The edit leaves the renderer at the end of this task, without any
      // query flushing the barrier (the connection flushes it).
      await Promise.resolve();
      if (PM.EngineDoc.barrier.pending || PM.EngineDoc.barrier.revision <= before) throw new Error('the edit did not reach the patch stream by itself');
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
    // While the drag's edits stream to the engine, the selection box and
    // handles are drawn at a revision the frame on screen includes: they
    // never lead the pixels.
    let lagged = 0;
    for (let i = 1; i <= 10; i++) {
      await page.mouse.move(startX + i * 4, startY + i * 2);
      const tags = await page.evaluate(() => {
        const V = (window as any).PM.Kernel.services.get('viewer');
        return { overlay: V.overlayRevision as number | null, frame: V.engineFrame()?.revision ?? null, mirror: (window as any).PM.EngineDoc.barrier.revision as number };
      });
      if (tags.overlay !== null && tags.frame !== null) expect(tags.overlay).toBeLessThanOrEqual(tags.frame);
      if (tags.frame !== null && tags.frame < tags.mirror) lagged++;
    }
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
    // Caught up: the handles are at the document's revision again.
    await expect.poll(() => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').overlayRevision), { timeout: 5000 }).toBe(wanted);
    test.info().annotations.push({ type: 'drag', description: `${lagged} of 10 moves sampled while the frame lagged the document` });
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

  test('dragging a precomp in a project of slice features streams every move to the engine', async ({ session }) => {
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
    await page.evaluate(buildSyntheticProject, { layers: 200, images: 1 });
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    const press = await page.evaluate(() => {
      const V = (window as any).PM.Kernel.services.get('viewer');
      const rect = V.stage.getBoundingClientRect();
      for (let gy = 0.2; gy <= 0.8; gy += 0.02) for (let gx = 0.2; gx <= 0.8; gx += 0.02) {
        const x = Math.round(rect.left + rect.width * gx), y = Math.round(rect.top + rect.height * gy);
        const layer = V.layerAtPoint?.(x, y);
        if (layer?.type === 'precomp') return { x, y, id: layer.id as string };
      }
      return null;
    });
    expect(press).not.toBeNull();
    const before = await page.evaluate((id) => (window as any).PM.proj.layers.find((l: any) => l.id === id).p['position.x'].v as number, press!.id);
    // A precomp drag is a plain history transaction: its moves reach the
    // barrier only through the flushes queries make (a speculative diff),
    // which must neither loop nor wait for the commit.
    const start = await mirrorRevision(page);
    await page.mouse.move(press!.x, press!.y);
    await page.mouse.down();
    for (let i = 1; i <= 30; i++) await page.mouse.move(press!.x + i * 3, press!.y + (i % 3));
    const during = await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').engineFrame()?.revision ?? 0);
    expect(during).toBeGreaterThan(start);
    await page.mouse.up();
    const after = await page.evaluate((id) => (window as any).PM.proj.layers.find((l: any) => l.id === id).p['position.x'].v as number, press!.id);
    expect(after).not.toBe(before);
    const wanted = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 10_000 }).toBeGreaterThanOrEqual(wanted);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a composition the engine cannot draw names why on the stage and keeps the transport on the engine', async ({ session }) => {
    const { page } = session;
    await openTracer(session);
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    /** The stage: the current engine's canvas, the engine's state line. */
    const stage = () => page.evaluate(() => {
      const V = (window as any).PM.Kernel.services.get('viewer');
      const line = V.stage.querySelector('[data-engine-status]') as HTMLElement | null;
      const box = line?.getBoundingClientRect();
      return { legacy: V.el.style.visibility as string, status: line?.textContent ?? null, role: line?.getAttribute('role') ?? null, visible: !!box && box.width > 0 && box.height > 0 };
    });
    /** A project color setting this engine does not know (a project saved
     *  by a newer app, say): one of the remaining `pm_render::unsupported`
     *  reasons, which the current engine ignores. */
    const unknownColor = () => page.evaluate(() => {
      const PM = (window as any).PM;
      PM.hist.do('Color', () => { PM.proj.color = { mode: 'unknown-to-this-engine' }; });
      PM.touch(); PM.bus.emit('project');
    });
    const why = /cannot draw this composition yet: .*project color/;
    await unknownColor();
    await waitRoute(page, 'legacy');
    expect((await status(page)).routeReason).toMatch(/project color/);
    // Amendment 21: nothing hands the viewer to the current engine. Its
    // canvas stays hidden and the stage says what the engine cannot draw.
    await expect.poll(async () => (await stage()).status, { timeout: 20_000 }).toMatch(why);
    expect(await stage()).toMatchObject({ legacy: 'hidden', role: 'status', visible: true });
    expect(await enginePresenting(page)).toBe(false);
    // The transport and its clock stay with the engine: Play plays.
    expect((await status(page)).driving).toBe(true);
    await page.evaluate(() => { const PM = (window as any).PM; PM.loop = false; PM.setTime(0.5); PM.play(); });
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 10_000 }).toBe('playing');
    await expect.poll(() => page.evaluate(() => (window as any).PM.time), { timeout: 10_000 }).toBeGreaterThan(0.9);
    expect(await page.evaluate(() => (window as any).PM.playing)).toBe(true);
    expect((await stage()).legacy).toBe('hidden');
    await page.evaluate(() => (window as any).PM.pause());
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 10_000 }).toBe('stopped');
    // Removing the setting brings engine frames back and clears the line;
    // setting it again says so again.
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.hist.do('Color', () => { delete PM.proj.color; });
      PM.touch(); PM.bus.emit('project');
    });
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    // Cleared once that frame is on screen (the canvas shows it a few
    // animation frames after the GPU finished it).
    await expect.poll(async () => (await stage()).status, { timeout: 5_000 }).toBeNull();
    await unknownColor();
    await waitRoute(page, 'legacy');
    await expect.poll(() => enginePresenting(page)).toBe(false);
    await expect.poll(async () => (await stage()).status).toMatch(why);
    expect((await stage()).legacy).toBe('hidden');
    const errors = session.diagnostics.console.filter((c) => c.type === 'error');
    expect(errors.map((e) => e.text)).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('switching between a composition the engine cannot draw and one it can, with the viewer attached', async ({ session }) => {
    const { page } = session;
    await openTracer(session, { unknownEffect: true });
    const first = await page.evaluate(() => (window as any).PM.proj.compId as string);
    await waitRoute(page, 'legacy');
    const why = /cannot draw this composition yet: .*e2e-unknown-effect/;
    /** The renderer's view of the stage, the session, and the engine frame on screen. */
    const view = () => page.evaluate(() => {
      const PM = (window as any).PM;
      const V = PM.Kernel.services.get('viewer');
      const line = V.stage.querySelector('[data-engine-status]') as HTMLElement | null;
      const canvas = document.getElementById('engine-present');
      const s = PM.EngineConnection.status();
      const doc = PM.EngineDoc;
      doc.barrier.flush();
      const frame = V.engineFrame?.() ?? null;
      return {
        comp: PM.proj.compId as string, session: PM.EngineConnection.session()?.composition ?? null,
        route: s.route as string, reason: s.routeReason as string | null, status: line?.textContent ?? null,
        presenting: !!canvas && canvas.style.visibility === 'visible', legacy: V.el.style.visibility as string,
        revision: doc.barrier.revision as number, frameRevision: frame?.revision ?? null
      };
    });
    const unsupported = async (comp: string) => {
      await expect.poll(async () => { const v = await view(); return { comp: v.comp, session: v.session, route: v.route, presenting: v.presenting }; }, { timeout: 20_000 })
        .toEqual({ comp, session: comp, route: 'legacy', presenting: false });
      await expect.poll(async () => (await view()).status, { timeout: 10_000 }).toMatch(why);
      const v = await view();
      expect(v.reason).toMatch(why);
      expect(v.legacy).toBe('hidden');
      return v;
    };
    const drawn = async (comp: string) => {
      // The engine draws the switched-to composition at the revision it
      // reported the other one at: that frame must reach the stage.
      await expect.poll(async () => { const v = await view(); return { comp: v.comp, session: v.session, route: v.route, presenting: v.presenting, status: v.status, reason: v.reason }; }, { timeout: 20_000 })
        .toEqual({ comp, session: comp, route: 'engine', presenting: true, status: null, reason: null });
      const v = await view();
      expect(v.frameRevision).toBe(v.revision);
      expect(v.legacy).toBe('hidden');
      return v;
    };
    const report = async () => (await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))).presenter;
    const a0 = await unsupported(first);
    const routed0 = (await report()).routedLegacy as number;
    // A drawable composition of the same project, opened (a document
    // batch, not a load): the engine hears the batch before the viewer state.
    const second = await page.evaluate(() => (window as any).PM.Comps.create({ name: 'Plain', w: 640, h: 360, fps: 30, dur: 3 }, { open: true }) as string);
    const b1 = await drawn(second);
    console.log(`switch A ${JSON.stringify(a0)} -> B ${JSON.stringify(b1)}`);
    await page.evaluate((id) => (window as any).PM.Comps.open(id), first);
    await unsupported(first);
    await page.evaluate((id) => (window as any).PM.Comps.open(id), second);
    const b2 = await drawn(second);
    // And it stays drawn: nothing comes back to cover it.
    await page.waitForTimeout(1_000);
    expect(await view()).toMatchObject({ comp: second, route: 'engine', presenting: true, status: null });
    const presenter = await report();
    console.log(`switch B again ${JSON.stringify(b2)}; routedLegacy ${routed0} -> ${presenter.routedLegacy}; routes ${JSON.stringify(presenter.routes)}`);
    expect(presenter.routedLegacy).toBeGreaterThan(routed0);
    expect(session.diagnostics.pageErrors).toEqual([]);
    const errors = session.diagnostics.console.filter((c) => c.type === 'error');
    expect(errors.map((e) => e.text)).toEqual([]);
  });

  test('path masks are drawn by the engine', async ({ session }) => {
    const { page } = session;
    await openTracer(session, { pathMask: true });
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    // The canvas turns visible with its first frame; the frame counts as on
    // screen (and replaces the starting line) once it is revealed.
    await expect.poll(async () => (await engineFrame(page)) !== null, { timeout: 20_000 }).toBe(true);
    expect(await page.evaluate(() => (window as any).PM.Kernel.services.get('viewer').stage.querySelector('[data-engine-status]'))).toBeNull();
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('effects are drawn by the engine with the kernel\'s definition snapshots', async ({ session }) => {
    const { page } = session;
    await openTracer(session, { fx: true });
    await waitRoute(page, 'engine');
    await expect.poll(() => enginePresenting(page), { timeout: 20_000 }).toBe(true);
    expect(await page.evaluate(() => Boolean((window as any).PM.EngineDefinitions))).toBe(true);
    const revision = await mirrorRevision(page);
    await expect.poll(async () => (await engineFrame(page))?.revision ?? -1, { timeout: 20_000 }).toBeGreaterThanOrEqual(revision);
    // Invert at 100%: #FF3300 is drawn as #00CCFF.
    const view = await viewerGeometry(page, 'box');
    const inside = await windowPixel(session, (view.layer.x0 + view.layer.x1) / 2, (view.layer.y0 + view.layer.y1) / 2);
    expect(inside[0]!).toBeLessThanOrEqual(3);
    expect(Math.abs(inside[1]! - 0xcc)).toBeLessThanOrEqual(3);
    expect(Math.abs(inside[2]! - 0xff)).toBeLessThanOrEqual(3);
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
    // The playhead holds where Play put it until the run's first frame is
    // shown (the audio output's latency), never moving back.
    const held: string[] = await page.evaluate(async () => {
      const out: string[] = [];
      for (let i = 0; i < 40; i++) {
        const PM = (window as any).PM; const c = PM.EngineConnection.session()?.clock; const host = PM.EngineConnection.hostClock.hostUs();
        out.push(`${PM.time} ${c?.state} ${c?.epoch}/${c?.seq} m=${c?.mediaTime / 705600000} ahead=${c ? Math.round(c.hostTime - host) : ''}`);
        await new Promise((r) => setTimeout(r, 10));
      }
      return out;
    });
    const ht = held.map((h) => Number(h.split(' ')[0]));
    expect(Math.min(...ht), held.join('\n')).toBeGreaterThanOrEqual(0.5 - 1e-9);
    // (Re-anchoring to the device clock may move it by microseconds.)
    expect(ht.every((t, i) => i === 0 || t >= ht[i - 1]! - 0.002), held.join('\n')).toBe(true);
    await expect.poll(() => page.evaluate(() => (window as any).PM.time as number), { timeout: 5000 }).toBeGreaterThan(0.6);
    // One second of the playhead, sampled in the page: it advances in real
    // time (least squares over the samples) and stays on the engine
    // clock's line (the audio device's).
    const { series, sync } = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const out: { now: number; t: number; line: number }[] = [];
      // A/V sync from frame-callback timing (section 15): for every timed
      // engine frame presented, where the audio clock's line is at the
      // instant the frame reaches the display, against the frame's interval.
      const sync: number[] = [];
      const off = PM.Kernel.services.get('viewer').enginePresentation.onPresented((frame: any) => {
        const clock = PM.EngineConnection.session()?.clock;
        if (!frame.timed || frame.displayUs === null || !clock || clock.state === 'stopped') return;
        const heard = clock.mediaTime + ((frame.displayUs - clock.hostTime) * 705_600_000 * clock.rate.num) / (1e6 * clock.rate.den);
        const end = frame.pts + 705_600_000 / 30;
        const outside = heard < frame.pts ? frame.pts - heard : heard >= end ? heard - end : 0;
        sync.push((outside / 705_600_000) * 1000);
      });
      const start = performance.now();
      while (performance.now() - start < 1000) {
        await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
        const now = performance.now();
        out.push({ now, t: PM.time, line: PM.EngineConnection.transport.now(now)?.time ?? NaN });
      }
      off();
      return { series: out, sync };
    });
    // Frames reach the display inside the interval the audio is heard in
    // (±10 ms, section 15): at most one late or early refresh in the second.
    expect(sync.length).toBeGreaterThan(20);
    expect(sync.filter((ms) => ms > 10).length).toBeLessThanOrEqual(1);
    expect(series.length).toBeGreaterThan(20);
    const n = series.length;
    const mx = series.reduce((s, p) => s + p.now, 0) / n;
    const my = series.reduce((s, p) => s + p.t, 0) / n;
    const rate = series.reduce((s, p) => s + (p.now - mx) * (p.t - my), 0) / series.reduce((s, p) => s + (p.now - mx) ** 2, 0) * 1000;
    expect(rate).toBeGreaterThan(0.98);
    expect(rate).toBeLessThan(1.02);
    // The playhead is the line at its last animation frame: within one
    // refresh (8.3 ms at 120 Hz, 16.7 at 60) plus scheduling.
    for (const p of series) expect(Math.abs(p.line - p.t)).toBeLessThan(0.025);
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
    await page.evaluate(() => { const PM = (window as any).PM; PM.proj.work = [1, 1.5]; PM.touch(); PM.loop = true; PM.setTime(1.2); PM.play(); });
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

  for (const clip of ['prores422.mov', 'h264-aac.mp4']) {
    test(`a ${clip.startsWith('prores') ? 'ProRes' : 'long-GOP H.264'} clip scrubs and plays in engine frames with its audio`, async ({ session }) => {
      const { page } = session;
      await session.openEditor();
      await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
      await page.evaluate(() => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.mkProject({ name: 'Tracer video', w: 640, h: 360, fps: 30, dur: 2, bg: '#000000' }));
        PM.setTime(0);
      });
      const chooser = page.waitForEvent('filechooser');
      await page.evaluate(() => (window as any).PM.pickFiles());
      await (await chooser).setFiles(fixturePath(clip));
      await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'), undefined, { timeout: 30_000 });
      const layerId = await page.evaluate(() => (window as any).PM.proj.layers.find((l: any) => l.type === 'video').id as string);
      await waitRoute(page, 'engine');
      await expect.poll(() => enginePresenting(page), { timeout: 30_000 }).toBe(true);
      // The fixture is red at 0.5 s and green at 1.5 s (fixtures.json).
      const centerColor = async (): Promise<number[]> => {
        const frame = await engineFrame(page);
        const quad = quadBox(frame.overlay.layers.find((l: any) => l.layer === layerId).bounds);
        // A quarter in from a corner: the center carries the anchor handle.
        return windowPixel(session, quad.x0 + (quad.x1 - quad.x0) / 4, quad.y0 + ((quad.y1 - quad.y0) * 3) / 4);
      };
      // The exact frame at `t`: a scrub may show the nearest decoded frame
      // first (a stand-in, docs/engine/scrub.md).
      const showsAt = async (t: number): Promise<void> => {
        await expect.poll(async () => {
          const frame = await engineFrame(page);
          return frame && !frame.standIn ? Math.abs(frame.pts / 705_600_000 - t) : 1;
        }, { timeout: 10_000 }).toBeLessThan(1e-6);
      };
      // A scrub: many paused seeks in a row; the last one's frame stays.
      await page.evaluate(() => {
        const PM = (window as any).PM;
        for (let i = 0; i <= 20; i++) PM.setTime(Math.round((0.2 + i * 0.05) * 30) / 30);
      });
      await showsAt(Math.round(1.2 * 30) / 30);
      await page.evaluate(() => (window as any).PM.setTime(0.5));
      await showsAt(0.5);
      const red = await centerColor();
      expect(red[0]!).toBeGreaterThan(200);
      expect(red[1]!).toBeLessThan(60);
      await page.evaluate(() => (window as any).PM.setTime(1.5));
      await showsAt(1.5);
      const green = await centerColor();
      expect(green[1]!).toBeGreaterThan(200);
      expect(green[0]!).toBeLessThan(60);
      // Playback from 0.2 s runs on the audio clock of the clip's own
      // audio, and the frames shown carry rising times.
      await page.evaluate(() => { const PM = (window as any).PM; PM.loop = false; PM.setTime(0.2); PM.play(); });
      await expect.poll(() => page.evaluate(() => (window as any).PM.time as number), { timeout: 10_000 }).toBeGreaterThan(0.4);
      const pts: number[] = await page.evaluate(async () => {
        const out: number[] = [];
        const V = (window as any).PM.Kernel.services.get('viewer');
        const off = V.enginePresentation.onPresented((f: any) => { if (f.timed) out.push(f.pts / 705_600_000); });
        await new Promise((r) => setTimeout(r, 600));
        off();
        return out;
      });
      expect(pts.length).toBeGreaterThan(8);
      expect(pts.every((t, i) => i === 0 || t > pts[i - 1]!)).toBe(true);
      await page.evaluate(() => (window as any).PM.pause());
      // The run played on the audio device (not the host-clock fallback),
      // which mixed the clip's sound without problems.
      const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
      const audio = Object.values(report.presenter.audio) as any[];
      expect(report.presenter.hostClockRuns).toBe(0);
      expect(audio.length).toBe(1);
      expect(audio[0].error).toBeNull();
      expect(audio[0].device).toBeTruthy();
      expect(audio[0].projects).toBeGreaterThan(0);
      expect(audio[0].problems).toEqual([]);
      expect(report.presenter.renderErrors).toBe(0);
      expect((await status(page)).failures.filter((f) => !f.startsWith('deviceLost'))).toEqual([]);
      expect(session.diagnostics.pageErrors).toEqual([]);
    });
  }

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
