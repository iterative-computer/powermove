/*
 * Rendering a large project with raster wrappers, "Use New Engine" on vs
 * off (the bigrender lane; docs/engine/host-processes.md, "Large projects,
 * rendering").
 *
 * Real users' production projects (hundreds of text and shape layers baked
 * to a key per frame, wrapped by extensions that adjust text geometry, type
 * text out, or pass every layer through) played one or two frames in eight
 * seconds with the engine on while today played over a hundred: every layer
 * a wrapper passed through waited for base pixels prepared two at a time, a
 * wrapper that dropped the preview crop had each layer's whole raster drawn
 * on the CPU and read back every frame, and every channel read prepared its
 * whole keyframe track again. This is the same shape with synthetic data:
 *
 * - flag off: build and save it (shapes and text with a key per frame on
 *   five channels, staggered so about a quarter is visible at a time);
 * - flag off, fresh profile: register the wrappers, open, play four seconds
 *   from 0 and count presented frames, scrub twelve seeks;
 * - flag on, fresh profile: the same. The engine presents at least today's
 *   frames (within one frame per second of noise), and its scrub p95 is
 *   within one 30 fps frame of today's plus a fifth.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-large-project-render.spec.ts --workers=1 --global-timeout=0
 */
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { Page } from 'playwright';

import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const SHAPES = 240;
const TEXTS = 48;
const DURATION = 8;
const FPS = 30;
const SIZE = { w: 1280, h: 720 };
const PLAY_SECONDS = 4;
const SEEKS = 12;

/**
 * Today's wrapper patterns from real setups: an assigned `PM.raster` that
 * calls the raster without the preview crop and returns it for every layer
 * but adjusted text (typography anchoring); a services wrapper that types
 * two text layers out over the base raster (a key per frame); and one that
 * passes everything through after reading the layer.
 */
async function registerWrappers(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('large-render-wrappers');
    const base = api.services.get('raster');
    api.services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      void layer?.fx?.length;
      return base(layer, scale, time, uploaded, crop);
    });
    const inner = api.services.get('raster');
    api.services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      if (layer?.type !== 'text' || !String(layer.name).startsWith('typed')) return inner(layer, scale, time, uploaded, crop);
      const surface = inner(layer, scale, time, undefined, crop);
      if (!surface?.cv) return surface;
      const canvas = document.createElement('canvas');
      canvas.width = surface.cv.width; canvas.height = surface.cv.height;
      const ctx = canvas.getContext('2d')!;
      const shown = Math.max(1, Math.round(canvas.width * Math.min(1, (time % 2) / 2)));
      ctx.drawImage(surface.cv, 0, 0, shown, canvas.height, 0, 0, shown, canvas.height);
      return { ...surface, cv: canvas, key: `typed:${layer.id}:${time.toFixed(4)}:${scale}` };
    });
    const assigned = PM.raster;
    PM.raster = (layer: any, scale = 1, time = PM.time) => assigned(layer, scale, time);
  });
}

async function openFile(session: LaunchedApp, file: string): Promise<number> {
  await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, file);
  const started = Date.now();
  const layers = await session.page.evaluate(async () => { const w = window as any; await w.PM.openProject(); return w.PM.proj.layers.length as number; });
  expect(layers).toBe(SHAPES + TEXTS);
  return (Date.now() - started) / 1000;
}

type Run = { frames: number; distinct: number; distinctPlayheads: number; scrubP95: number; scrubP50: number };

/** Presented frames over PLAY_SECONDS from 0, then SEEKS seeks to a presented frame. */
async function measure(page: Page): Promise<Run> {
  // Warm the first frame (both engines draw their first frame cold).
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.pause(); PM.setTime(0, { force: true });
    await new Promise<void>((resolve) => { const off = PM.bus.on('preview:presented', () => { if (typeof off === 'function') off(); resolve(); }); setTimeout(resolve, 10_000); });
  });
  const play = await page.evaluate(async (seconds) => {
    const PM = (window as any).PM;
    // The time of the picture each event announces (today's compositor
    // and the engine's presentation both name the frame drawn), and the
    // playhead when it came (logged: the engine's worker can put two frames
    // up within one of the playhead's steps).
    const times: number[] = [];
    const playheads: number[] = [];
    const off = PM.bus.on('preview:presented', (payload: { time?: number } | undefined) => {
      times.push(Number.isFinite(payload?.time) ? Number(payload!.time) : PM.time);
      playheads.push(PM.time);
    });
    PM.play();
    await new Promise((r) => setTimeout(r, seconds * 1000));
    PM.pause();
    if (typeof off === 'function') off();
    return { frames: times.length, distinct: new Set(times).size, distinctPlayheads: new Set(playheads).size };
  }, PLAY_SECONDS);
  const scrub = await page.evaluate(async ({ seeks, duration }) => {
    const PM = (window as any).PM; const ms: number[] = [];
    for (let i = 0; i < seeks; i++) {
      const t = ((i * 5) % seeks) / seeks * (duration - 0.1) + 0.05;
      const start = performance.now();
      const presented = new Promise<void>((resolve) => { const off = PM.bus.on('preview:presented', () => { if (typeof off === 'function') off(); resolve(); }); });
      PM.setTime(t, { force: true });
      await Promise.race([presented, new Promise((r) => setTimeout(r, 5000))]);
      ms.push(performance.now() - start);
    }
    const sorted = [...ms].sort((a, b) => a - b);
    return { p50: sorted[Math.floor(sorted.length / 2)]!, p95: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)]!, all: ms.map(v => Math.round(v)) };
  }, { seeks: SEEKS, duration: DURATION });
  console.log(`seeks (ms, in order): ${JSON.stringify(scrub.all)}`);
  return { frames: play.frames, distinct: play.distinct, distinctPlayheads: play.distinctPlayheads, scrubP50: scrub.p50, scrubP95: scrub.p95 };
}

async function run(mode: 'today' | 'engine', file: string): Promise<Run> {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: mode === 'engine' ? '1' : '0' } });
  try {
    await session.openEditor();
    if (mode === 'engine') await session.page.waitForFunction(() => (window as any).PM?.EngineConnection?.status?.().connected === true, null, { timeout: 60_000 });
    await registerWrappers(session.page);
    const seconds = await openFile(session, file);
    console.log(`${mode}: opened in ${seconds.toFixed(1)} s`);
    const result = await measure(session.page);
    console.log(`${mode}: ${JSON.stringify(result)}`);
    if (mode === 'engine') {
      const transport = await session.page.evaluate(() => { const t = (window as any).PM.EngineRender; return { compat: t?.compatStats, lookahead: t?.lookaheadStats ? { ...t.lookaheadStats, recent: undefined, recentAnswers: undefined } : null, snapshots: t?.stats }; });
      const report = await session.app.evaluate(async () => { const e = (globalThis as any).__powermoveNativeEngine?.engine; const r = e ? await e.report() : null; const p = r?.presenter; return p ? { stages: p.stages, compatWaits: p.compatWaits, stalls: p.stalls, lastStall: p.lastStall, slowTurns: p.slowTurns, snapshots: p.snapshots, frames: p.frames, lateSkipped: p.lateSkipped, poolWaits: p.poolWaits, memory: p.memory, externalWaitTimeouts: p.externalWaitTimeouts } : null; });
      console.log(`engine diagnostics: ${JSON.stringify({ transport, report })}`);
      const status = await session.page.evaluate(() => (window as any).PM.EngineConnection.status());
      expect(status.connected).toBe(true);
      expect(status.route).toBe('engine');
    }
    return result;
  } finally {
    const child = session.app.process();
    const closed = session.close().then(() => true, () => true);
    if (!(await Promise.race([closed, new Promise<boolean>((r) => setTimeout(() => r(false), 30_000))]))) child.kill('SIGKILL');
    await closed;
  }
}

test('a large project with raster wrappers plays and scrubs with the engine on at least as well as today', async () => {
  test.setTimeout(1_200_000);
  const dir = await mkdtemp(path.join(os.tmpdir(), 'powermove-large-render-'));
  const file = path.join(dir, 'Large render.pmv');
  try {
    const author = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    try {
      await author.openEditor();
      await author.page.evaluate(({ w, h, dur, fps, shapes, texts }) => {
        const PM = (window as any).PM;
        PM.replaceProject(PM.mkProject({ name: 'Large render', w, h, fps, dur, bg: '#10141c' }));
        const proj = PM.proj;
        const span = dur / 4;
        for (let i = 0; i < shapes; i++) {
          const from = ((i * 7) % 16) / 16 * (dur - span);
          proj.layers.push(PM.mkLayer('shape', {
            name: `shape ${i}`, from, dur: span,
            d: { shape: ['rect', 'ellipse', 'star', 'polygon'][i % 4], w: 30 + (i % 9) * 12, h: 20 + (i % 7) * 10, points: 5, radius: 4, color: ['#3FA9F5', '#F25F5C', '#FFE066', '#70C1B3'][i % 4], stroke: i % 3 ? 0 : 2, strokeColor: '#FFFFFF' }
          }, proj));
        }
        for (let i = 0; i < texts; i++) {
          const from = ((i * 5) % 8) / 8 * (dur - span);
          proj.layers.push(PM.mkLayer('text', {
            name: `${i < 2 ? 'typed' : 'plain'} text ${i}`, from: i < 2 ? 0 : from, dur: i < 2 ? dur : span,
            d: { text: `Ridge ${i} — cove · meadow`, font: 'Helvetica', size: 26 + (i % 4) * 6, color: '#FFFFFF' }
          }, proj));
        }
        // A key per frame on five channels of every layer (a baked export).
        const frames = Math.round(dur * fps);
        proj.layers.forEach((layer: any, index: number) => {
          const channels: Record<string, (f: number) => number> = {
            'position.x': (f) => 60 + ((index * 37 + f * 3) % (w - 120)),
            'position.y': (f) => 40 + ((index * 23 + f * 2) % (h - 80)),
            'scale.x': (f) => 90 + 10 * Math.sin((f + index) / 9),
            rotation: (f) => ((f + index * 5) % 90) - 45,
            opacity: (f) => 55 + 45 * Math.abs(Math.sin((f + index) / 17)),
          };
          for (const [name, value] of Object.entries(channels)) {
            const prop = layer.p[name] ?? (layer.p[name] = PM.P(value(0)));
            prop.kf = Array.from({ length: frames }, (_, f) => PM.KF(f / fps, value(f)));
          }
        });
        PM.touch?.();
        PM.bus.emit('layers');
      }, { w: SIZE.w, h: SIZE.h, dur: DURATION, fps: FPS, shapes: SHAPES, texts: TEXTS });
      await author.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
      expect(await author.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    } finally {
      await author.close();
    }
    const today = await run('today', file);
    const engine = await run('engine', file);
    console.log(`large render: today ${JSON.stringify(today)}, engine ${JSON.stringify(engine)}`);
    // Today's cadence, within one frame per second played of noise.
    expect(engine.frames).toBeGreaterThanOrEqual(today.frames - PLAY_SECONDS);
    expect(engine.distinct).toBeGreaterThanOrEqual(today.distinct - PLAY_SECONDS);
    // Today's scrub, within a fifth and one 30 fps frame.
    expect(engine.scrubP95).toBeLessThanOrEqual(today.scrubP95 * 1.2 + 1000 / 30);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
