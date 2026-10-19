import { execFileSync } from 'node:child_process';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/* Exports with Blender layers in the job engine (plan section 17,
   contract 11). Existing projects retain today's PNG8/dither and straight
   ImageBitmap compositing; level 2 keeps pinned PNG16. The job engine's
   still is compared directly with today's still at the Blender corpus
   bounds. A composite of today's raw surfaces is diagnostic only: it
   does not reproduce today's ImageBitmap upload at the fringes. The
   viewer test checks paused Blender previews and native playback without
   renderer-side Blender work. Skipped without Blender 4.5 or newer. */

const ffprobe = ffmpegBinary.replace(/ffmpeg$/, 'ffprobe');
const probe = (file: string) => JSON.parse(execFileSync(ffprobe, ['-v', 'error', '-show_entries', 'format_tags=encoder:stream=codec_name,pix_fmt,nb_frames,width,height', '-of', 'json', file], { encoding: 'utf8' }));
/** RGBA8 of a PNG, through FFmpeg. */
const rgba = (file: string) => execFileSync(ffmpegBinary, ['-v', 'error', '-i', file, '-f', 'rawvideo', '-pix_fmt', 'rgba', '-'], { maxBuffer: 64 << 20 });

const W = 192, H = 108;

async function ready(session: LaunchedApp): Promise<void> {
  await expect.poll(() => session.app.evaluate(() => {
    const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
    return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
  }), { timeout: 30_000 }).toBe('ready/ready');
}

/** A Blender project: a camera, a sun, a box and a sphere, EEVEE. */
async function project(session: LaunchedApp): Promise<boolean> {
  return session.page.evaluate(async ({ W, H }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('blender-export-proof');
    const status = await api.scene3d.blenderStatus();
    if (!status.available) return false;
    PM.proj = PM.mkProject({ name: 'Blender export', w: W, h: H, fps: 24, dur: 0.25 });
    PM.proj.work = [0, 0.25];
    PM.bus.emit('project');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.6, z: 5 } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'box', source: { primitive: 'box' }, p: { x: -0.8, ry: 30 }, material: { p: { color: '#FF7733' } } } });
    api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, p: { x: 0.8 }, material: { p: { color: '#3388FF', roughness: 0.2 } } } });
    // The box turns: every frame has its own Blender frame.
    const box = PM.proj.layers.find((l: any) => l.d?.data?.object?.source?.primitive === 'box');
    PM.Edit.apply({ type: 'replace_keyframes', target: box.id, path: 'rotation.y', keyframes: [{ time: 0, value: 30 }, { time: 0.25, value: 75 }] });
    const r = api.scene3d.setRendering({ enabled: true, engine: 'eevee', samples: 8, previewSamples: 4, previewScale: 1, device: 'cpu' });
    if (!r.ok) throw new Error(r.message);
    PM.setTime(0);
    return true;
  }, { W, H });
}

async function exportTo(session: LaunchedApp, file: string, options: Record<string, unknown>): Promise<unknown> {
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, file);
  return session.page.evaluate(async (options) => (window as any).PM.Export.run(options), options);
}

test('exports with Blender layers render in the job engine and look like today\'s Blender export', async () => {
  test.setTimeout(300_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-engine-blender-'));
  const still = { format: 'still', scale: 1, fps: 24, range: 'all', quality: 'high', mblur: false, audio: false };
  try {
    // Today: the current exporter's Blender still.
    let today = '';
    {
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
      try {
        await session.openEditor();
        test.skip(!(await project(session)), 'Install Blender 4.5 or newer to run the Blender export integration');
        // Record today's surfaces at main's Blender IPC.
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
        today = path.join(root, 'today.png');
        expect(await exportTo(session, today, still)).toEqual({ cancelled: false });
        const recorded: { id: string; data: string }[] = await session.app.evaluate(() => (globalThis as any).__blenderSurfaces);
        expect(recorded.map((f) => f.id).sort()).toEqual(['ball', 'box']);
        for (const f of recorded) await writeFile(path.join(root, `today-${f.id}.png`), Buffer.from(f.data, 'base64'));
      } finally {
        await session.close();
      }
    }
    // The job engine: the same still, then a movie.
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    try {
      await ready(session);
      await session.openEditor();
      expect(await project(session)).toBe(true);
      const engine = path.join(root, 'engine.png');
      expect(await exportTo(session, engine, still)).toEqual({ cancelled: false });
      const mp4 = path.join(root, 'engine.mp4');
      expect(await exportTo(session, mp4, { ...still, format: 'mp4' })).toEqual({ cancelled: false });
      const movie = probe(mp4);
      // AVAssetWriter's file (the job engine), not the current exporter's FFmpeg mux.
      expect(movie.format.tags?.encoder ?? '').not.toMatch(/^Lavf/);
      expect(movie.streams[0]).toMatchObject({ codec_name: 'h264', width: W, height: H, nb_frames: '6' });
      // Blender rendered the surfaces through host services.
      const userData = await session.app.evaluate(({ app }) => app.getPath('userData'));
      // Each frame's snapshot differs (the box turns), so both layers have a
      // surface per frame; the still at 0 reused frame 0's.
      const surfaces = (await readdir(path.join(userData, 'Engine Cache', 'Blender Surfaces', 'surfaces'))).filter((n) => n.endsWith('.png'));
      console.log(`Blender surfaces in the cache: ${surfaces.length}`);
      expect(surfaces.length).toBe(2 * 6);
      // Today's Blender look: the same models, framing, AgX, and compositing.
      // Diagnostics: PM_BLENDER_E2E_KEEP=<dir> keeps both stills and the
      // job engine's Blender cache area.
      const keep = process.env['PM_BLENDER_E2E_KEEP'];
      if (keep) {
        const { copyFile, cp, mkdir } = await import('node:fs/promises');
        await mkdir(keep, { recursive: true });
        for (const f of [today, engine]) await copyFile(f, path.join(keep, path.basename(f)));
        await cp(path.join(userData, 'Engine Cache', 'Blender Surfaces'), path.join(keep, 'area'), { recursive: true });
      }
      // Ideal premultiplied raw-surface composite, diagnostic only. The
      // actual ImageBitmap upload is covered by the direct still comparison.
      const layers = ['box', 'ball'].map((id) => rgba(path.join(root, `today-${id}.png`)));
      const pixels = W * H;
      const composite = new Float64Array(pixels * 3);
      for (const l of layers) {
        for (let i = 0; i < pixels; i++) {
          const a = l[i * 4 + 3]! / 255;
          for (let c = 0; c < 3; c++) composite[i * 3 + c] = (l[i * 4 + c]! / 255) * a + composite[i * 3 + c]! * (1 - a);
        }
      }
      const compare = (file: string) => {
        const px = rgba(file);
        let within2 = 0, max = 0;
        for (let i = 0; i < pixels; i++) {
          let m = 0;
          for (let c = 0; c < 3; c++) m = Math.max(m, Math.abs(px[i * 4 + c]! - Math.round(composite[i * 3 + c]! * 255)));
          if (m <= 2) within2++;
          max = Math.max(max, m);
        }
        return { within2: within2 / pixels, max };
      };
      const covered = layers.reduce((n, l) => n + l.filter((v, i) => i % 4 === 3 && v > 0).length, 0);
      const ours = compare(engine), theirs = compare(today);
      console.log(`Blender look: job engine still ${(100 * ours.within2).toFixed(3)}% within 2 CV of today's surfaces composited (max ${ours.max} CV); today's own still ${(100 * theirs.within2).toFixed(3)}% (max ${theirs.max} CV); ${covered} covered surface pixels`);
      expect(covered).toBeGreaterThan(pixels * 0.05);
      const actual = rgba(engine), reference = rgba(today);
      let matching = 0, maximum = 0;
      for (let i = 0; i < pixels; i++) {
        let difference = 0;
        for (let c = 0; c < 3; c++) difference = Math.max(difference, Math.abs(actual[i * 4 + c]! - reference[i * 4 + c]!));
        if (difference <= 2) matching++;
        maximum = Math.max(maximum, difference);
      }
      console.log(`Blender still vs today's still: ${(100 * matching / pixels).toFixed(3)}% within 2 CV, max ${maximum} CV`);
      expect(matching / pixels).toBeGreaterThanOrEqual(0.999);
      expect(maximum).toBeLessThanOrEqual(4);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await session.close();
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

/* The paused viewer's Blender shading (plan section 8, contract 11): with
   Use New Engine on, the engine draws the viewer of a Blender composition
   (native shading in Solid), and while paused in Material Preview it has
   host services render each layer's preview surface (EEVEE, preview
   samples and scale) in the installed Blender and draws them per layer:
   the viewer's pixels are the surfaces composited. Playback draws natively
   and starts no Blender job; pausing at a new time renders that frame.
   Skipped without Blender 4.5 or newer. */
test('the paused viewer draws Blender surfaces in Material Preview, and native shading while playing', async () => {
  test.setTimeout(300_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-viewer-blender-'));
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    const { page, app } = session;
    await ready(session);
    await session.openEditor();
    test.skip(!(await project(session)), 'Install Blender 4.5 or newer to run the Blender viewer integration');
    // Installed as the editor installs a project, so the viewer's session
    // mirrors it to the engine.
    await page.evaluate(() => { const PM = (window as any).PM; PM.replaceProject(PM.proj); PM.Projects?.put?.(PM.proj); PM.setTime(0); });
    // Today's own preview runs at main's IPC (they would double Blender's work).
    await app.evaluate(({ ipcMain }) => {
      const ipc = ipcMain as any, original = ipc._invokeHandlers.get('blender:run');
      (globalThis as any).__todayPreviewRuns = 0;
      ipc.removeHandler('blender:run');
      ipc.handle('blender:run', async (event: unknown, job: any) => {
        if (job.operation === 'render') (globalThis as any).__todayPreviewRuns++;
        return original(event, job);
      });
    });
    const report = async () => (await app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))) as any;
    const preview = async () => (await report()).presenter.blenderPreview as { runs: number; jobs: number; rendered: number; cached: number; failed: number; lastError: string | null; device: string | null; surfaces: number };
    const route = async () => {
      const s = await page.evaluate(() => { const PM = (window as any).PM; return { ...PM.EngineConnection.status(), service: !!PM.Kernel.services.get('engine.viewer'), presenting: !!PM.Kernel.services.get('viewer')?.enginePresentation }; });
      const r = await report();
      return `${s.route} (${JSON.stringify(s)}; presenter ${JSON.stringify(r?.presenter?.routes ?? {})} sessions ${JSON.stringify(r?.sessions ?? null)})`;
    };
    await expect.poll(route, { timeout: 45_000 }).toMatch(/^engine /);
    await expect.poll(() => page.evaluate(() => document.getElementById('engine-present')?.style.visibility), { timeout: 20_000 }).toBe('visible');
    expect((await preview()).runs).toBe(0);
    const frameTag = () => page.evaluate(() => (window as any).PM.Kernel.services.get('viewer')?.engineFrame?.() ?? null);
    // Material Preview, paused at 0.
    await page.evaluate(() => (window as any).PM.Kernel.api('blender-viewer').scene3d.viewport.run('shading', 'material'));
    await expect.poll(async () => { const p = await preview(); return p.failed ? `failed: ${p.lastError}` : p.surfaces; }, { timeout: 180_000 }).toBe(2);
    const first = await preview();
    console.log(`viewer Blender preview: ${JSON.stringify(first)}`);
    expect(first.jobs).toBe(1);
    expect(first.rendered + first.cached).toBe(2);
    expect(first.device).toBeTruthy();
    // The frame drawn after the surfaces arrived.
    const drawn = (await report()).presenter.documentFrames as number;
    await expect.poll(async () => (await report()).presenter.documentFrames, { timeout: 10_000 }).toBeGreaterThanOrEqual(drawn);
    await page.waitForTimeout(500);
    // The surfaces composited over the composition's black, from the
    // Blender Surfaces area (the preview is the composition's size).
    const userData = await app.evaluate(({ app }) => app.getPath('userData'));
    const dir = path.join(userData, 'Engine Cache', 'Blender Surfaces', 'surfaces');
    const files = (await readdir(dir)).filter((n) => n.endsWith('.png'));
    expect(files.length).toBe(2);
    const layers = files.map((f) => rgba(path.join(dir, f)));
    expect(layers.every((l) => l.length === W * H * 4)).toBe(true);
    const composite = new Float64Array(W * H * 3), alpha = new Float64Array(W * H);
    for (const l of layers) {
      for (let i = 0; i < W * H; i++) {
        const a = l[i * 4 + 3]! / 255;
        alpha[i] = a + alpha[i]! * (1 - a);
        for (let c = 0; c < 3; c++) composite[i * 3 + c] = (l[i * 4 + c]! / 255) * a + composite[i * 3 + c]! * (1 - a);
      }
    }
    // Flat, opaque pixels of the models (3 × 3 neighbourhoods within 2 CV):
    // the viewer scales the composition up, so edges are interpolated.
    const at = (x: number, y: number, c: number) => Math.round(composite[(y * W + x) * 3 + c]! * 255);
    const points: [number, number][] = [];
    for (let y = 2; y < H - 2; y += 3) {
      for (let x = 2; x < W - 2; x += 3) {
        if (alpha[y * W + x]! < 0.999) continue;
        let flat = true;
        for (let dy = -1; dy <= 1 && flat; dy++) for (let dx = -1; dx <= 1 && flat; dx++) for (let c = 0; c < 3; c++) if (Math.abs(at(x + dx, y + dy, c) - at(x, y, c)) > 2 || alpha[(y + dy) * W + x + dx]! < 0.999) flat = false;
        if (flat) points.push([x, y]);
      }
    }
    expect(points.length).toBeGreaterThan(20);
    const viewport = (await frameTag()).overlay.viewport as { x: number; y: number; width: number; height: number };
    const sample = async (list: [number, number][]) => {
      const out: number[][] = [];
      for (const [x, y] of list) {
        const css = { x: viewport.x + ((x + 0.5) / W) * viewport.width, y: viewport.y + ((y + 0.5) / H) * viewport.height };
        const origin = await page.evaluate(() => { const r = (window as any).PM.Kernel.services.get('viewer').stage.getBoundingClientRect(); return { x: r.left, y: r.top }; });
        out.push(await app.evaluate(async ({ BrowserWindow }, p) => {
          const w = BrowserWindow.getAllWindows().find((b) => !b.isDestroyed() && b.webContents.getURL().startsWith('app:')) ?? BrowserWindow.getAllWindows()[0]!;
          const bitmap = (await w.webContents.capturePage({ x: Math.round(p.x), y: Math.round(p.y), width: 1, height: 1 })).toBitmap();
          return [bitmap[2]!, bitmap[1]!, bitmap[0]!];
        }, { x: origin.x + css.x, y: origin.y + css.y }));
      }
      return out;
    };
    const step = Math.max(1, Math.floor(points.length / 24));
    const chosen = points.filter((_, i) => i % step === 0).slice(0, 24);
    const shown = await sample(chosen);
    const error = (px: number[][]) => px.map((p, i) => Math.max(...[0, 1, 2].map((c) => Math.abs(p[c]! - at(chosen[i]![0], chosen[i]![1], c)))));
    const material = error(shown);
    // Solid shading of the same frame (native three.js shading) for scale.
    await page.evaluate(() => (window as any).PM.Kernel.api('blender-viewer').scene3d.viewport.run('shading', 'solid'));
    await page.waitForTimeout(1_000);
    const solid = error(await sample(chosen));
    const within = material.filter((e) => e <= 6).length / material.length;
    console.log(`viewer pixels vs the Blender surfaces composited at ${chosen.length} points: Material Preview max ${Math.max(...material)} CV (${(100 * within).toFixed(1)}% within 6 CV); Solid (native) max ${Math.max(...solid)} CV`);
    expect(within).toBeGreaterThanOrEqual(0.95);
    expect(Math.max(...solid)).toBeGreaterThan(Math.max(...material));
    // Playback draws natively: no Blender job starts while it plays.
    await page.evaluate(() => (window as any).PM.Kernel.api('blender-viewer').scene3d.viewport.run('shading', 'material'));
    await page.waitForTimeout(1_000);
    const beforePlay = await preview();
    await page.evaluate(() => { const PM = (window as any).PM; PM.loop = true; PM.play(); });
    await page.waitForTimeout(1_500);
    expect((await preview()).runs).toBe(beforePlay.runs);
    // Paused at another time: that frame renders (the box turns).
    await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.setTime(0.125); });
    await expect.poll(async () => (await preview()).surfaces, { timeout: 180_000 }).toBeGreaterThanOrEqual(4);
    const after = await preview();
    expect(after.failed).toBe(0);
    const today = await app.evaluate(() => (globalThis as any).__todayPreviewRuns as number);
    console.log(`viewer Blender preview after playback: ${JSON.stringify(after)}; today's preview runs at main's IPC: ${today}`);
    // The engine draws the viewer, so today's preview does not run Blender too.
    expect(today).toBe(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally {
    await session.close();
    await rm(root, { recursive: true, force: true });
  }
});
