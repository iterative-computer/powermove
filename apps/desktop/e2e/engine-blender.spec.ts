import { execFileSync } from 'node:child_process';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

/* Exports with Blender layers in the job engine (plan section 17's Blender
   stage, contract 11): with Use New Engine on, the export sheet's job runs
   in the job engine, which builds each frame's snapshot, has host services
   run the pinned worker in the installed Blender (16-bit RGBA through AgX),
   and composites the surfaces; the file is the job engine's (not Lavf), the
   surfaces are in Engine Cache/Blender Surfaces, and the frame looks like
   today's Blender export of the same project (the current exporter, Use
   New Engine off): the job engine's still equals today's Blender surfaces
   of the same frame (today's 8-bit worker, recorded at main's IPC)
   composited per layer, within 2 CV (today's 8-bit dithered PNGs against
   the pinned 16-bit ones). Today's own still is measured against the same
   composite and reported (the current exporter's still path draws its
   surfaces up to a pixel off; docs/engine/blender.md). Skipped without
   Blender 4.5 or newer. */

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
      // Today's surfaces composited per layer over the (black) background,
      // straight alpha, as today's compositor premultiplies them.
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
      expect(ours.within2).toBeGreaterThanOrEqual(0.999);
      expect(ours.max).toBeLessThanOrEqual(4);
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally {
      await session.close();
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
