/*
 * OPENWRITE-1: an open checkpoints the project once, as JSON text, and checks
 * (rather than rewrites) its later checkpoints. Crash safety, in the real app:
 * a large synthetic project (above the size at which checkpoints are checked)
 * is opened, and the whole app is SIGKILLed at seeded random moments during
 * and after the open. Each profile is then launched again and asked for the
 * project. It must be either absent (killed before the first checkpoint
 * reached disk) or exactly the document a clean open stores, and the store
 * must load without quarantining a file. With either engine setting.
 *
 * The exhaustive version of this, every prefix of the file operations of an
 * open's checkpoint sequence, is the unit test
 * src/renderer/src/legacy/__tests__/project-checkpoint-crash.test.ts.
 *
 *   pm-electron-lock bunx playwright test e2e/project-open-crash-recovery.spec.ts --workers=1 --global-timeout=0
 */
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const LAYERS = 100;
const DURATION = 12;
const FPS = 30;

/** Seeded kill moments (ms after the open starts), plus one after it settles. */
function killMoments(seed: number, count: number): number[] {
  const out: number[] = [];
  for (let i = 0; i < count; i++) {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0;
    // Most moments fall in the open's first seconds, where its checkpoint is.
    out.push(40 + Math.floor(((seed % 10_000) / 10_000) ** 2 * 6000));
  }
  return out.sort((a, b) => a - b);
}

async function closeSoon(session: LaunchedApp): Promise<void> {
  const child = session.app.process();
  const closed = session.close().then(() => true, () => true);
  const done = await Promise.race([closed, new Promise<boolean>(resolve => setTimeout(() => resolve(false), 30_000))]);
  if (!done) child.kill('SIGKILL');
  await closed;
}

async function kill(session: LaunchedApp): Promise<void> {
  const child = session.app.process();
  const exited = new Promise<void>(resolve => { if (child.exitCode !== null || child.signalCode !== null) resolve(); else child.once('exit', () => resolve()); });
  child.kill('SIGKILL');
  await exited;
  // Chromium's children leave once their browser process is gone.
  await new Promise(resolve => setTimeout(resolve, 1500));
}

/** The stored project named `Crash` as Projects.get recovers it (slot plus journal), or null. */
async function recovered(userData: string, engine: boolean): Promise<{ json: string | null; quarantined: string[] }> {
  const session = await launchApp({ userData, env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' } });
  try {
    await session.page.waitForFunction(() => !!(window as any).PM?.Projects, null, { timeout: 120_000 });
    const json = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      const meta = PM.Projects.list().find((item: any) => item.name === 'Crash');
      const project = meta && PM.Projects.get(meta.id);
      if (!project) return null;
      const { id: _id, ...rest } = project;
      return JSON.stringify(rest);
    });
    const quarantined = (await readdir(path.join(userData, 'store'))).filter(name => name.includes('.corrupt-'));
    return { json, quarantined };
  } finally { await closeSoon(session); }
}

test('a crash at any moment of a large open recovers nothing or the opened document', async () => {
  test.setTimeout(3_600_000);
  const dir = await mkdtemp(path.join(os.tmpdir(), 'pm-open-crash-'));
  const file = path.join(dir, 'Crash.pmv');
  try {
    // Build and save the project.
    const author = await launchApp({ userData: path.join(dir, 'author'), env: { POWERMOVE_NEW_ENGINE: '0' } });
    try {
      await author.openEditor();
      const chars = await author.page.evaluate(({ layers, dur, fps }) => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Crash', w: 960, h: 540, fps, dur, bg: '#182028' });
        PM.replaceProject(project);
        const proj = PM.proj, frames = Math.round(dur * fps);
        for (let i = 0; i < layers; i++) {
          const shape = PM.mkLayer('shape', { name: `marker ${i}`, from: 0, dur, d: { shape: i % 2 ? 'ellipse' : 'rect', w: 40 + (i % 7) * 6, h: 30, color: '#3FA9F5', stroke: 0 } }, proj);
          for (const [name, base] of [['position.x', 80], ['position.y', 60], ['rotation', 0], ['opacity', 70], ['scale.x', 100], ['scale.y', 100], ['skew', 0]] as const) {
            const prop = shape.p[name] ?? (shape.p[name] = PM.P(base));
            prop.kf = Array.from({ length: frames }, (_, f) => PM.KF(f / fps, base + ((i * 13 + f * 3) % 50)));
          }
          proj.layers.push(shape);
        }
        PM.touch?.();
        PM.bus.emit('layers');
        return JSON.stringify(proj).length;
      }, { layers: LAYERS, dur: DURATION, fps: FPS });
      // Above the size at which repeated checkpoints are checked (projects.ts).
      expect(chars).toBeGreaterThan(4 << 20);
      await author.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, file);
      expect(await author.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    } finally { await closeSoon(author); }

    // The document a clean open stores.
    const clean = path.join(dir, 'clean');
    let reference = '';
    {
      const session = await launchApp({ userData: clean, env: { POWERMOVE_NEW_ENGINE: '0' } });
      try {
        await session.openEditor();
        await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, file);
        await session.page.evaluate(async () => {
          const PM = (window as any).PM;
          await PM.openProject();
          await PM.projectAssetsReady;
          await PM.flushProject();
        });
      } finally { await closeSoon(session); }
      const stored = await recovered(clean, false);
      expect(stored.quarantined).toEqual([]);
      expect(stored.json).not.toBeNull();
      reference = stored.json!;
    }

    const outcomes: Array<{ engine: boolean; ms: number; recovered: boolean }> = [];
    const runs = [...killMoments(20261022, 9).map(ms => ({ engine: false, ms })), ...killMoments(7, 4).map(ms => ({ engine: true, ms })),
      { engine: false, ms: -1 }, { engine: true, ms: -1 }];
    for (const [index, run] of runs.entries()) {
      const userData = path.join(dir, `kill-${index}`);
      const session = await launchApp({ userData, env: { POWERMOVE_NEW_ENGINE: run.engine ? '1' : '0' } });
      try {
        await session.openEditor();
        await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, file);
        await session.page.evaluate(() => {
          const w = window as any;
          w.__crashOpen = w.PM.openProject().then(() => w.PM.projectAssetsReady).then(() => new Promise(resolve => setTimeout(resolve, 3000)));
        });
        if (run.ms >= 0) await new Promise(resolve => setTimeout(resolve, run.ms));
        else await session.page.evaluate(() => (window as any).__crashOpen);
      } finally { await kill(session); }
      const result = await recovered(userData, run.engine);
      expect(result.quarantined, `kill ${index}`).toEqual([]);
      if (result.json !== null) expect(result.json, `kill ${index} (${run.ms} ms, engine ${run.engine}) recovered another document`).toBe(reference);
      if (run.ms < 0) expect(result.json, `kill ${index}: a settled open must be recovered`).not.toBeNull();
      outcomes.push({ engine: run.engine, ms: run.ms, recovered: result.json !== null });
      await rm(userData, { recursive: true, force: true });
    }
    console.log(`open crash outcomes ${JSON.stringify(outcomes)}`);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
