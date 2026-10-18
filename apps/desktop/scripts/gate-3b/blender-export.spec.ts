import { execFile } from 'node:child_process';
import { mkdtemp, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { expect, test } from '@playwright/test';

import { launchApp, type LaunchedApp } from '../../e2e/helpers/app';

/* Section 25's Phase 3b performance row "3D export, 3 models with shadows,
   Blender EEVEE, 16 samples: at least today's re-measured baseline; memory
   gate with this export running". One 1080p composition (three primitive
   models on a ground slab, a shadow-casting sun, EEVEE at 16 samples) is
   exported to H.264 MP4 by today's exporter (Use New Engine off) and by the
   job engine (on), each in a fresh hidden app. Wall clock from Export to
   done; while each export runs, the app's process tree (main, renderer,
   engines, Blender workers) is sampled every 500 ms for resident memory,
   and the kernel's memory-pressure level is read. A measurement, not a
   check: it runs only with PM_GATE_3B=1, serially under the Electron lock.
   Blender 4.5 or newer must be installed. */

const run = promisify(execFile);
const W = 1920, H = 1080, FPS = 24;
const SECONDS = Number(process.env['PM_GATE_3D_SECONDS']) || 1;
const SAMPLES = 16;

async function ready(session: LaunchedApp): Promise<void> {
  await expect.poll(() => session.app.evaluate(() => {
    const status = (globalThis as { __powermoveNativeEngine?: { engine: { status(): Record<string, { state: string }> } | null } }).__powermoveNativeEngine?.engine?.status();
    return status ? `${status['interactive']!.state}/${status['job']!.state}` : 'missing';
  }), { timeout: 60_000 }).toBe('ready/ready');
}

/** Three models with shadows: a box, a sphere, and a torus that turn over a
 * ground slab, lit by a sun that casts shadows; EEVEE at 16 samples. */
async function project(session: LaunchedApp): Promise<boolean> {
  return session.page.evaluate(async ({ W, H, FPS, SECONDS, SAMPLES }) => {
    const PM = (window as any).PM, api = PM.Kernel.api('gate-3b-blender-export');
    const status = await api.scene3d.blenderStatus();
    if (!status.available) return false;
    PM.pause();
    PM.proj = PM.mkProject({ name: '3D export gate', w: W, h: H, fps: FPS, dur: SECONDS });
    PM.proj.work = [0, SECONDS];
    PM.bus.emit('project');
    const ok = (r: any) => { if (r && r.ok === false) throw new Error(r.message); return r; };
    ok(api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 2.2, z: 7, targetY: 0.2 } } }));
    ok(api.scene3d.edit({ operation: 'add_light', light: { id: 'sun', type: 'sun', castShadow: true, p: { x: -3, y: 6, z: 4, intensity: 3 } } }));
    // Shadows: the environment's `shadows` and every model's and the sun's
    // `castShadow` default to true (core/scene3d/schema.ts).
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'ground', source: { primitive: 'box' }, p: { y: -0.75, sx: 8, sy: 0.1, sz: 6 }, material: { p: { color: '#B8B8B0', roughness: 0.8 } } } }));
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'box', source: { primitive: 'box' }, p: { x: -1.8, ry: 30 }, material: { p: { color: '#FF7733' } } } }));
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'ball', source: { primitive: 'sphere' }, p: { x: 0 }, material: { p: { color: '#3388FF', roughness: 0.2, metalness: 0.4 } } } }));
    ok(api.scene3d.edit({ operation: 'add_object', object: { id: 'ring', source: { primitive: 'torus' }, p: { x: 1.8, rx: 60 }, material: { p: { color: '#44CC88', roughness: 0.4 } } } }));
    // Every model moves, so every frame is its own Blender frame.
    for (const [id, path, from, to] of [['box', 'rotation.y', 30, 120], ['ring', 'rotation.y', 0, 90], ['ball', 'position.y', 0, 0.6]] as const) {
      const layer = PM.proj.layers.find((l: any) => l.d?.data?.object?.id === id);
      if (!layer) throw new Error(`no layer for ${id}`);
      PM.Edit.apply({ type: 'replace_keyframes', target: layer.id, path, keyframes: [{ time: 0, value: from }, { time: SECONDS, value: to }] });
    }
    ok(api.scene3d.setRendering({ enabled: true, engine: 'eevee', samples: SAMPLES, previewSamples: 4, previewScale: 0.5 }));
    PM.setTime(0);
    return true;
  }, { W, H, FPS, SECONDS, SAMPLES });
}

type Sample = { rss: number; blender: number; pressure: number };

/** Resident bytes of `root` and its descendants, Blender's share, and the
 * kernel's memory-pressure level (1 normal, 2 warning, 4 critical). */
async function sample(root: number): Promise<Sample> {
  const { stdout } = await run('ps', ['-A', '-o', 'pid=,ppid=,rss=,comm=']);
  const rows = stdout.trim().split('\n').map((line) => {
    const m = line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(.*)$/);
    return m ? { pid: Number(m[1]), ppid: Number(m[2]), rss: Number(m[3]) * 1024, comm: m[4]! } : null;
  }).filter((r): r is NonNullable<typeof r> => r !== null);
  const tree = new Set([root]);
  for (let grew = true; grew;) {
    grew = false;
    for (const r of rows) if (!tree.has(r.pid) && tree.has(r.ppid)) { tree.add(r.pid); grew = true; }
  }
  let rss = 0, blender = 0;
  for (const r of rows) {
    if (!tree.has(r.pid)) continue;
    rss += r.rss;
    if (/blender/i.test(r.comm)) blender += r.rss;
  }
  const level = await run('sysctl', ['-n', 'kern.memorystatus_vm_pressure_level']).then((o) => Number(o.stdout.trim())).catch(() => NaN);
  return { rss, blender, pressure: level };
}

async function measure(engine: boolean, root: string): Promise<{ fps: number; seconds: number; peak: Sample; bytes: number } | null> {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' } });
  const { app, page } = session;
  try {
    if (engine) await ready(session);
    await session.openEditor();
    if (!(await project(session))) return null;
    if (engine) {
      await page.evaluate(() => {
        const w = window as any;
        w.__routed = [];
        w.__powermoveTestBridge.nativeEngine.onExportStatus((s: any) => { if (s.routed) w.__routed.push(s.error ?? 'routed'); });
      });
    }
    const output = path.join(root, `${engine ? 'engine' : 'today'}.mp4`);
    await app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = (async () => ({ canceled: false, filePath })) as typeof dialog.showSaveDialog;
    }, output);
    const pid = app.process().pid!;
    const peak: Sample = { rss: 0, blender: 0, pressure: 0 };
    let sampling = true;
    const sampler = (async () => {
      while (sampling) {
        const s = await sample(pid).catch(() => null);
        if (s) {
          peak.rss = Math.max(peak.rss, s.rss);
          peak.blender = Math.max(peak.blender, s.blender);
          peak.pressure = Math.max(peak.pressure, Number.isFinite(s.pressure) ? s.pressure : 0);
        }
        await new Promise((r) => setTimeout(r, 500));
      }
    })();
    const seconds = await page.evaluate(async ({ FPS }) => {
      const PM = (window as any).PM;
      const start = performance.now();
      const result = await PM.Export.run({ format: 'mp4', scale: 1, fps: FPS, range: 'all', quality: 'high', mblur: false, alpha: false, audio: false });
      if (result?.cancelled || result?.error) throw new Error(`mp4: ${result.error ?? 'cancelled'}`);
      while (PM.Export.busy) await new Promise((resolve) => setTimeout(resolve, 20));
      return (performance.now() - start) / 1000;
    }, { FPS });
    sampling = false;
    await sampler;
    if (engine) expect(await page.evaluate(() => (window as any).__routed)).toEqual([]);
    const bytes = (await stat(output)).size;
    const frames = FPS * SECONDS;
    const fps = frames / seconds;
    console.log(`MEASURE 3d export ${engine ? 'job engine' : "today's exporter"} ${W}x${H} ${frames} frames EEVEE ${SAMPLES} samples: ${fps.toFixed(3)} fps (${seconds.toFixed(2)} s), ${bytes} bytes; peak app tree RSS ${(peak.rss / 2 ** 20).toFixed(0)} MiB (Blender ${(peak.blender / 2 ** 20).toFixed(0)} MiB), max memory-pressure level ${peak.pressure}`);
    expect(session.diagnostics.pageErrors).toEqual([]);
    return { fps, seconds, peak, bytes };
  } finally {
    await session.close();
  }
}

test('3D export with Blender EEVEE: the job engine against today\'s exporter', async () => {
  test.skip(process.env['PM_GATE_3B'] !== '1', 'measurement: set PM_GATE_3B=1');
  test.setTimeout(1_800_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-gate-3b-3d-'));
  try {
    const today = await measure(false, root);
    test.skip(today === null, 'Install Blender 4.5 or newer to run the 3D export measurement');
    const engine = await measure(true, root);
    if (today && engine) console.log(`MEASURE 3d export ratio: job engine ${(engine.fps / today.fps).toFixed(2)}× today's exporter`);
  } finally {
    if (process.env['PM_GATE_3B_KEEP'] === '1') console.log(`kept ${root}`);
    else await rm(root, { recursive: true, force: true });
  }
});
