/*
 * Async captures of a Blender composition with "Use New Engine" on
 * (docs/engine/rfcs/16-render-transport.md, "Blender frames"): today's
 * `PM.Export.snapshotAsync` (the agent's look and observe, and
 * `render.snapshotAsync`) runs `prepareFrame`, which has Blender render
 * the frame at the capture's size, and then captures it. With the engine
 * on, the engine's Blender stage (the one behind the paused viewer's
 * Rendered shading) renders it in the installed Blender and the capture
 * draws its surfaces; never native shading in their place, and never the
 * current engine. A synchronous capture before any preparation fails with
 * today's words; after it, it draws the prepared frame. Playback pauses
 * around the capture and resumes, as today. The capture is compared with
 * today's (Use New Engine off), with today's own Blender surfaces of the
 * frame composited per layer (recorded at main's IPC, as
 * engine-blender.spec.ts does), and with the engine's native shading of the
 * same frame. Skipped without Blender 4.5 or newer.
 *
 * Asserted: the engine's pixels equal today's Blender surfaces composited
 * (the export gate's bound, engine-blender.spec.ts). Logged, not asserted:
 * the distance to today's capture itself, whose compositor draws the
 * surfaces about a pixel larger with a light edge (today's still export
 * does the same, docs/engine/blender.md). That difference is reported to
 * the orchestrator as a decision (docs/engine/bridge.md); it is not an
 * accepted difference.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/engine-blender-captures.spec.ts --workers=1
 *
 * PM_BLENDER_CAPTURE_KEEP=<dir> keeps today's, the engine's, and the
 * native captures as PNGs for a side-by-side.
 */
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/** RGBA8 of a PNG, through FFmpeg. */
const rgba = (file: string) => execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'], { maxBuffer: 64 << 20 });

const W = 192, H = 108;
const NOT_PREPARED = 'The Blender frame is not prepared. Render this frame before capturing it.';

/** A Blender project: a camera, a box and a sphere, EEVEE (as engine-blender.spec.ts). */
async function project(session: LaunchedApp): Promise<boolean> {
  return session.page.evaluate(async ({ W, H }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('blender-capture-proof');
    const status = await api.scene3d.blenderStatus();
    if (!status.available) return false;
    PM.proj = PM.mkProject({ name: 'Blender capture', w: W, h: H, fps: 24, dur: 1 });
    PM.bus.emit('project');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.6, z: 5 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'box', source: { primitive: 'box' }, p: { x: -0.8, ry: 30 }, material: { p: { color: '#FF7733' } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, p: { x: 0.8 }, material: { p: { color: '#3388FF', roughness: 0.2 } } } });
    const r = api.scene3d.setRendering({ enabled: true, engine: 'eevee', samples: 8, previewSamples: 4, previewScale: 1, device: 'cpu' });
    if (!r.ok) throw new Error(r.message);
    PM.setTime(0);
    return true;
  }, { W, H });
}

/** RGBA of a data URL, decoded in the page. */
const decode = (session: LaunchedApp, url: string): Promise<number[]> => session.page.evaluate(async (url) => {
  const image = new Image();
  image.src = url;
  await image.decode();
  const canvas = document.createElement('canvas');
  canvas.width = image.width; canvas.height = image.height;
  const ctx = canvas.getContext('2d')!;
  ctx.drawImage(image, 0, 0);
  return Array.from(ctx.getImageData(0, 0, image.width, image.height).data);
}, url);

/** Mean and 99th-percentile per-pixel max channel difference, and the share within `n`. */
function compare(a: number[], b: number[], n = 8): { mean: number; p99: number; within: number } {
  const diffs: number[] = [];
  let sum = 0;
  for (let i = 0; i < a.length; i += 4) {
    let m = 0;
    for (let c = 0; c < 3; c++) m = Math.max(m, Math.abs(a[i + c]! - b[i + c]!));
    diffs.push(m);
    sum += m;
  }
  diffs.sort((x, y) => x - y);
  return { mean: sum / diffs.length, p99: diffs[Math.floor(diffs.length * 0.99)]!, within: diffs.filter((d) => d <= n).length / diffs.length };
}

async function keep(name: string, url: string): Promise<void> {
  const dir = process.env['PM_BLENDER_CAPTURE_KEEP'];
  if (!dir) return;
  await mkdir(dir, { recursive: true });
  await writeFile(path.join(dir, name), Buffer.from(url.split(',')[1]!, 'base64'));
}

test('an async capture of a Blender composition renders it with Blender first, as today', async () => {
  test.setTimeout(300_000);
  // Today: the current engine's capture (prepareFrame, then the compositor),
  // and today's Blender surfaces of it.
  let today: number[] = [];
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-blender-capture-'));
  const composite = new Float64Array(W * H * 3);
  {
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    try {
      await session.openEditor();
      test.skip(!(await project(session)), 'Install Blender 4.5 or newer to run the Blender capture integration');
      const before = await session.page.evaluate(() => { try { return (window as any).PM.Export.snapshot(0, 192); } catch (e) { return `threw: ${(e as Error).message}`; } });
      expect(before).toBe(`threw: ${NOT_PREPARED}`);
      await session.app.evaluate(({ ipcMain }) => {
        const ipc = ipcMain as any, original = ipc._invokeHandlers.get('blender:run'), files: { id: string; data: string }[] = [];
        (globalThis as any).__blenderSurfaces = files;
        ipc.removeHandler('blender:run');
        ipc.handle('blender:run', async (event: unknown, job: any) => {
          const result = await original(event, job);
          if (job.operation === 'render') for (const f of result.files) files.push({ id: f.id, data: Buffer.from(f.data).toString('base64') });
          return result;
        });
      });
      const url: string = await session.page.evaluate(() => (window as any).PM.Export.snapshotAsync(0, 192));
      expect(url.startsWith('data:image/jpeg')).toBe(true);
      await keep('today.jpg', url);
      today = await decode(session, url);
      const recorded: { id: string; data: string }[] = await session.app.evaluate(() => (globalThis as any).__blenderSurfaces);
      expect(recorded.map((f) => f.id).sort()).toEqual(['ball', 'box']);
      // Composited per layer over the black background, straight alpha, as
      // today's compositor premultiplies them.
      for (const f of recorded) {
        const file = path.join(root, `today-${f.id}.png`);
        await writeFile(file, Buffer.from(f.data, 'base64'));
        const l = rgba(file);
        expect(l.length).toBe(W * H * 4);
        for (let i = 0; i < W * H; i++) {
          const a = l[i * 4 + 3]! / 255;
          for (let c = 0; c < 3; c++) composite[i * 3 + c] = (l[i * 4 + c]! / 255) * a + composite[i * 3 + c]! * (1 - a);
        }
      }
    } finally {
      await session.close();
    }
  }
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    await session.page.waitForFunction(() => Boolean((window as any).PM?.EngineRender));
    expect(await project(session)).toBe(true);
    const { page } = session;
    // Today's synchronous capture before any preparation: today's words.
    await expect.poll(() => page.evaluate(() => { try { return (window as any).PM.Export.snapshot(0, 192); } catch (e) { return `threw: ${(e as Error).message}`; } }), { timeout: 30_000 }).toBe(`threw: ${NOT_PREPARED}`);
    // Playing when the agent looks: today pauses, captures, and resumes.
    await page.evaluate(() => (window as any).PM.play());
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const started = performance.now();
      const playingBefore = PM.playing;
      const url: string = await PM.Export.snapshotAsync(0, 192);
      return { url, ms: performance.now() - started, playingBefore, playingAfter: PM.playing, blender: PM.EngineRender.stats };
    });
    expect(result.playingBefore).toBe(true);
    expect(result.playingAfter).toBe(true);
    await page.evaluate(() => (window as any).PM.pause());
    expect(result.url.startsWith('data:image/jpeg')).toBe(true);
    await keep('engine.jpg', result.url);
    const engine = await decode(session, result.url);
    expect(engine.length).toBe(W * H * 4);
    // The same through the kernel's apiVersion 4 sibling, and the
    // synchronous capture now draws the prepared frame (today's).
    const again = await page.evaluate(async () => {
      const PM = (window as any).PM, api = PM.Kernel.api('blender-capture-proof');
      const asynced: string = await api.render.snapshotAsync(0, 192);
      let sync = '';
      try { sync = PM.Export.snapshot(0, 192); } catch (e) { sync = `threw: ${(e as Error).message}`; }
      return { asynced, sync };
    });
    expect(again.sync.startsWith('data:image/jpeg')).toBe(true);
    const asynced = compare(await decode(session, again.asynced), engine);
    const sync = compare(await decode(session, again.sync), engine);
    // The engine's native shading of the same frame, for contrast.
    const native = await page.evaluate(async ({ W, H }) => {
      const PM = (window as any).PM;
      const snap = await PM.EngineRender.snapshot({ time: 0, width: W, height: H, draft3d: true });
      return PM.EngineRender.toCanvas(snap).toDataURL('image/jpeg', 0.74) as string;
    }, { W, H });
    await keep('native.jpg', native);
    const surfaces: number[] = [];
    for (let i = 0; i < W * H; i++) surfaces.push(...[0, 1, 2].map((c) => Math.round(composite[i * 3 + c]! * 255)), 255);
    const nativePx = await decode(session, native);
    // The engine's pixels before JPEG (the same prepared frame, opaque RGBA8).
    const raw: number[] = await page.evaluate(async ({ W, H }) => {
      const snap = await (window as any).PM.EngineRender.snapshot({ time: 0, width: W, height: H, prepareBlender: true });
      return Array.from(snap.data as Uint8ClampedArray);
    }, { W, H });
    const exact = compare(raw, surfaces, 2);
    let max = 0;
    for (let i = 0; i < raw.length; i += 4) for (let c = 0; c < 3; c++) max = Math.max(max, Math.abs(raw[i + c]! - surfaces[i + c]!));
    const dir = process.env['PM_BLENDER_CAPTURE_KEEP'];
    if (dir) await writeFile(path.join(dir, 'surfaces.rgba'), Buffer.from(surfaces));
    const ours = compare(engine, today), draft = compare(nativePx, today);
    const oursToSurfaces = compare(engine, surfaces), todayToSurfaces = compare(today, surfaces), draftToSurfaces = compare(nativePx, surfaces);
    console.log(`Blender capture: ${result.ms.toFixed(0)} ms through the engine's Blender stage`);
    console.log(`  against today's capture: engine mean ${ours.mean.toFixed(2)} CV, p99 ${ours.p99} CV, ${(100 * ours.within).toFixed(2)}% within 8 CV; native shading mean ${draft.mean.toFixed(2)} CV, p99 ${draft.p99} CV`);
    console.log(`  against today's Blender surfaces composited: engine mean ${oursToSurfaces.mean.toFixed(2)} CV, p99 ${oursToSurfaces.p99} CV, ${(100 * oursToSurfaces.within).toFixed(2)}% within 8 CV; today's capture mean ${todayToSurfaces.mean.toFixed(2)} CV, p99 ${todayToSurfaces.p99} CV, ${(100 * todayToSurfaces.within).toFixed(2)}% within 8 CV; native shading mean ${draftToSurfaces.mean.toFixed(2)} CV`);
    console.log(`  the engine's pixels before JPEG against today's Blender surfaces composited: ${(100 * exact.within).toFixed(3)}% within 2 CV, max ${max} CV`);
    console.log(`  render.snapshotAsync and the prepared sync capture against the engine's capture: mean ${asynced.mean.toFixed(2)} and ${sync.mean.toFixed(2)} CV`);
    // The engine's capture is Blender's frame: today's Blender surfaces
    // composited (the export gate's bound: today's 8-bit dithered surfaces
    // against the pinned 16-bit ones), and far from native shading.
    expect(exact.within).toBeGreaterThanOrEqual(0.999);
    expect(max).toBeLessThanOrEqual(4);
    expect(draftToSurfaces.mean).toBeGreaterThan(oursToSurfaces.mean * 3);
    expect(asynced.mean).toBeLessThanOrEqual(1);
    expect(sync.mean).toBeLessThanOrEqual(1);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
