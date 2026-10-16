/*
 * Phase 2 performance rows measured in the app with "Use New Engine" on
 * (plan section 25; docs/engine/tracer.md, "Measurements"). Times here are
 * the page's: an edit or a seek is timed from the call to the animation
 * frame that drew the first engine frame carrying it, plus the display
 * latency the presentation predicts (the instant the frame was picked
 * for). Presentation times from the compositor (viz) for the drag and
 * cadence rows come from scripts/frame-bridge-measure.mjs (`tracer-*`
 * runs).
 *
 * Opt in (heavy; one Electron at a time):
 *   PM_TRACER_MEASURE=1 pm-electron-lock bunx playwright test e2e/native-engine-tracer-measure.spec.ts --workers=1
 * Results go to $PM_TRACER_MEASURE_OUT (JSON lines; default
 * /tmp/powermove-tracer-measure.jsonl), never into the repository.
 */
import { existsSync } from 'node:fs';
import { appendFile, mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';

import type { Page } from 'playwright';

import { buildSyntheticProject, type SyntheticSummary } from '../src/main/engine/integration/synthetic-project';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const OUT = process.env.PM_TRACER_MEASURE_OUT ?? path.join(os.tmpdir(), 'powermove-tracer-measure.jsonl');
const TICKS = 705_600_000;
const MEDIA = process.env.PM_TRACER_MEDIA ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/tracer-media');
/** The S1 harness addon (pm-bridge `harness` feature), for the display state. */
const HARNESS = process.env.PM_BRIDGE_HARNESS ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/pm_bridge_harness.node');
const displayState = (): { asleep: boolean; locked: boolean } | null => {
  try {
    return existsSync(HARNESS) ? createRequire(__filename)(HARNESS).displayState() : null;
  } catch {
    return null;
  }
};

test.skip(!process.env.PM_TRACER_MEASURE, 'measurements run on request (PM_TRACER_MEASURE=1)');

type Dist = { n: number; p50?: number; p95?: number; p99?: number; max?: number };
function dist(xs: number[], digits = 2): Dist {
  if (!xs.length) return { n: 0 };
  const s = [...xs].sort((a, b) => a - b);
  const at = (p: number) => +s[Math.min(s.length - 1, Math.floor((s.length - 1) * p + 1e-9))]!.toFixed(digits);
  return { n: s.length, p50: at(0.5), p95: at(0.95), p99: at(0.99), max: at(1) };
}

async function record(row: string, value: unknown): Promise<void> {
  const line = JSON.stringify({ row, at: new Date().toISOString(), cpu: os.cpus()[0]?.model ?? null, display: displayState(), load: os.loadavg()[0], value });
  await appendFile(OUT, `${line}\n`);
  console.log(`[measure] ${row}: ${JSON.stringify(value)}`);
}

const status = (page: Page): Promise<any> => page.evaluate(() => (window as any).PM.EngineConnection.status());

async function engineReady(page: Page, timeout = 90_000): Promise<void> {
  await expect.poll(async () => (await status(page)).route, { timeout }).toBe('engine');
  await expect.poll(() => page.evaluate(() => !!(window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting), { timeout }).toBe(true);
}

async function synthetic(session: LaunchedApp, layers: number, extra: Partial<Parameters<typeof buildSyntheticProject>[0]> = {}): Promise<SyntheticSummary> {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
  const summary = await session.page.evaluate(buildSyntheticProject, { layers, ...extra });
  expect(summary.layers).toBe(layers);
  return summary;
}

test.describe('Use New Engine: Phase 2 measurements', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('the synthetic 500-layer project routes to the engine', async ({ session }) => {
    test.setTimeout(180_000);
    const summary = await synthetic(session, 500);
    await engineReady(session.page);
    const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
    await record('synthetic-500', { summary, composeUsMax: report.presenter.composeUsMax, documentFrames: report.presenter.documentFrames, layerFailures: report.presenter.layerFailures, renderErrors: report.presenter.renderErrors });
    expect(report.presenter.renderErrors).toBe(0);
    expect(report.presenter.layerFailures).toBe(0);
  });

  for (const clip of [
    { file: 'prores422hq-1080p-2997.mov', row: 'scrub-intra-prores422hq-1080p' },
    { file: 'h264-longgop-1080p-2997.mp4', row: 'scrub-longgop-h264-1080p-gop250' },
    { file: 'h264-longgop-2160p-2997.mp4', row: 'scrub-longgop-h264-2160p-gop250' }
  ]) {
    test(`scrub latency: ${clip.row}`, async ({ session }) => {
      test.setTimeout(300_000);
      const file = path.join(MEDIA, clip.file);
      test.skip(!existsSync(file), `generate ${file} first (docs/engine/tracer.md)`);
      const { page } = session;
      await session.openEditor();
      await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
      await page.evaluate(() => {
        const PM = (window as any).PM;
        const project = PM.mkProject({ name: 'Scrub', w: 1920, h: 1080, fps: 30000 / 1001, dur: 30, bg: '#000000' });
        project.fpsRational = { num: 30000, den: 1001 };
        PM.replaceProject(project);
        PM.setTime(0);
      });
      const chooser = page.waitForEvent('filechooser');
      await page.evaluate(() => (window as any).PM.pickFiles());
      await (await chooser).setFiles(file);
      await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'), undefined, { timeout: 120_000 });
      await engineReady(page, 120_000);
      // Random frames across the clip, each seek waiting for its frame: the
      // time from the seek to the animation frame that drew the frame at
      // that time (the presentation worker's draw, on the page's clock).
      const result = await page.evaluate(async ({ count, ticks }) => {
        const PM = (window as any).PM;
        const presentation = PM.Kernel.services.get('viewer').enginePresentation;
        const fps = 30000 / 1001;
        // Inside the clip and the composition (a seek past the end lands on its last frame).
        const frames = Math.floor(Math.min(PM.proj.layers.find((l: any) => l.type === 'video').dur, PM.proj.dur) * fps) - 2;
        let seed = 7;
        const random = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
        const latencies: number[] = [];
        let timeouts = 0;
        const missed: string[] = [];
        for (let i = 0; i < count; i++) {
          const k = 1 + Math.floor(random() * frames);
          const t = k / fps;
          const target = Math.round(t * ticks);
          const ms = await new Promise<number | null>((resolve) => {
            let done = false;
            const off = presentation.onPresented((f: any) => {
              if (done || Math.abs(f.pts - target) > 1) return;
              done = true;
              off();
              resolve(f.rafMs - t0);
            });
            const t0 = performance.now();
            PM.setTime(t);
            setTimeout(() => {
              if (done) return;
              done = true;
              off();
              resolve(null);
            }, 3000);
          });
          if (ms === null) {
            timeouts++;
            const s = PM.EngineConnection.status();
            missed.push(`k=${k} shown=${Math.round((PM.Kernel.services.get('viewer').engineFrame()?.pts ?? -1) / ticks * fps)} route=${s.route} failures=${s.failures.length}`);
          } else latencies.push(ms);
          await new Promise((r) => setTimeout(r, 30));
        }
        // A playhead drag: a seek at every animation frame for 3 s; how far
        // behind the requested time each drawn frame is.
        const lag: number[] = [];
        const shown = new Set<number>();
        let requested = new Map<number, number>();
        const off = presentation.onPresented((f: any) => {
          shown.add(f.pts);
          const at = requested.get(f.pts);
          if (at !== undefined) lag.push(f.rafMs - at);
        });
        const start = performance.now();
        let k = 0;
        while (performance.now() - start < 3000) {
          await new Promise((r) => requestAnimationFrame(r));
          k = (k + 1) % frames;
          const t = (k + 1) / fps;
          requested.set(Math.round(t * ticks), performance.now());
          PM.setTime(t);
        }
        await new Promise((r) => setTimeout(r, 500));
        off();
        const asked = requested.size;
        requested = new Map();
        return { latencies, timeouts, missed, drag: { asked, shown: shown.size, lagMs: lag } };
      }, { count: 60, ticks: TICKS });
      const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
      await record(clip.row, {
        file: clip.file,
        seekToDrawMs: dist(result.latencies),
        timeouts: result.timeouts,
        missed: result.missed.slice(0, 20),
        dragScrub: { asked: result.drag.asked, distinctFramesShown: result.drag.shown, requestToDrawMs: dist(result.drag.lagMs) },
        composeUsMax: report?.presenter?.composeUsMax ?? null,
        renderErrors: report?.presenter?.renderErrors ?? null
      });
      expect(result.timeouts).toBe(0);
    });
  }

  for (const source of [
    { name: 'synthetic-500-silent', clip: null },
    { name: 'prores-clip-with-audio', clip: 'prores422hq-1080p-2997.mov' }
  ]) {
    test(`A/V sync during playback: ${source.name}`, async ({ session }) => {
      test.setTimeout(300_000);
      const { page } = session;
      if (source.clip) {
        const file = path.join(MEDIA, source.clip);
        test.skip(!existsSync(file), `generate ${file} first`);
        await session.openEditor();
        await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
        await page.evaluate(() => {
          const PM = (window as any).PM;
          const project = PM.mkProject({ name: 'Sync', w: 1920, h: 1080, fps: 30000 / 1001, dur: 20, bg: '#000000' });
          project.fpsRational = { num: 30000, den: 1001 };
          PM.replaceProject(project);
          PM.setTime(0);
        });
        const chooser = page.waitForEvent('filechooser');
        await page.evaluate(() => (window as any).PM.pickFiles());
        await (await chooser).setFiles(file);
        await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'), undefined, { timeout: 120_000 });
      } else {
        await synthetic(session, 500, { dur: 30 });
      }
      await engineReady(page, 120_000);
      // 15 s of playback. For every timed engine frame presented: where the
      // audio clock's line is at the instant the frame reaches the display
      // (the instant the presentation picked it for), against the frame's
      // interval (0 inside it); and the signed offset of the line from the
      // frame's start.
      const result = await page.evaluate(async ({ ticks }) => {
        const PM = (window as any).PM;
        const presentation = PM.Kernel.services.get('viewer').enginePresentation;
        const fps = PM.proj.fps;
        const outside: number[] = [];
        const offset: number[] = [];
        let frames = 0;
        let previous = -1;
        let repeats = 0;
        let skips = 0;
        PM.loop = false;
        PM.setTime(0.5);
        PM.play();
        await new Promise((r) => setTimeout(r, 1500));
        const off = presentation.onPresented((f: any) => {
          const clock = PM.EngineConnection.session()?.clock;
          if (!f.timed || f.displayUs === null || !clock || clock.state === 'stopped') return;
          frames++;
          const heard = clock.mediaTime + ((f.displayUs - clock.hostTime) * ticks * clock.rate.num) / (1e6 * clock.rate.den);
          const dur = ticks / fps;
          outside.push(((heard < f.pts ? f.pts - heard : heard >= f.pts + dur ? heard - f.pts - dur : 0) / ticks) * 1000);
          offset.push(((heard - f.pts) / ticks) * 1000);
          const k = Math.round(f.pts / dur);
          if (previous >= 0 && k === previous) repeats++;
          if (previous >= 0 && k > previous + 1) skips += k - previous - 1;
          previous = k;
        });
        await new Promise((r) => setTimeout(r, 15_000));
        off();
        PM.pause();
        return { frames, outside, offset, repeats, skips };
      }, { ticks: TICKS });
      const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
      const audio = Object.values(report?.presenter?.audio ?? {})[0] as any;
      await record(`av-sync-${source.name}`, {
        framesPresented: result.frames,
        outsideIntervalMs: dist(result.outside),
        withinTenMs: result.outside.filter((x) => x <= 10).length,
        lineMinusFrameStartMs: dist(result.offset),
        skippedFrames: result.skips,
        repeatedFrames: result.repeats,
        audio: audio ? { device: audio.device, latencyUs: audio.latencyUs, underruns: audio.underruns, starvedRenders: audio.starvedRenders } : null,
        hostClockRuns: report?.presenter?.hostClockRuns ?? null
      });
      expect(result.frames).toBeGreaterThan(100);
    });
  }

  test('inspector edits during playback of 1,000 layers (page timing)', async ({ session }) => {
    test.setTimeout(300_000);
    await synthetic(session, 1000, { dur: 60 });
    const { page } = session;
    await engineReady(page, 120_000);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const presentation = PM.Kernel.services.get('viewer').enginePresentation;
      PM.loop = false;
      PM.setTime(0);
      PM.play();
      await new Promise((r) => setTimeout(r, 1500));
      const before = presentation.stats();
      const applyMs: number[] = [];
      const toDrawMs: number[] = [];
      const layers = PM.proj.layers.filter((l: any) => l.type === 'solid');
      for (let i = 0; i < 60; i++) {
        const layer = layers[(i * 37) % layers.length];
        const t0 = performance.now();
        PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 30 + (i % 60) }, { label: 'Opacity', origin: 'inspector' });
        await Promise.resolve();
        applyMs.push(performance.now() - t0);
        const revision = PM.EngineDoc.barrier.revision;
        const drawn = await new Promise<number | null>((resolve) => {
          const off = presentation.onPresented((f: any) => {
            if (f.revision < revision) return;
            off();
            resolve(f.rafMs - t0);
          });
          setTimeout(() => { off(); resolve(null); }, 2000);
        });
        if (drawn !== null) toDrawMs.push(drawn);
        await new Promise((r) => setTimeout(r, 250 - (performance.now() - t0) % 250));
      }
      const after = presentation.stats();
      PM.pause();
      return { applyMs, toDrawMs, before, after };
    });
    const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
    const dropped = (s: any) => Object.values(s?.dropped ?? {}).reduce((a: number, b: any) => a + Number(b), 0);
    await record('edit-during-playback-1000-page', {
      applyAndSendMs: dist(result.applyMs, 3),
      editToDrawMs: dist(result.toDrawMs),
      edits: result.applyMs.length,
      drawn: result.toDrawMs.length,
      presenterDroppedDuring: dropped(result.after) - dropped(result.before),
      presenterDroppedByReason: result.after?.dropped ?? null,
      engineLateSkipped: report?.presenter?.lateSkipped ?? null,
      composeUsMax: report?.presenter?.composeUsMax ?? null,
      composeUsMean: report?.presenter?.documentFrames ? Math.round(report.presenter.composeUsTotal / report.presenter.documentFrames) : null
    });
    // Measured, not gated here: every edit is reported, drawn or not.
    expect(result.applyMs.length).toBe(60);
  });
});

test.describe('the current engine, for comparison', () => {
  test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '0' } } });

  test('inspector edits during playback of 1,000 layers (Use New Engine off)', async ({ session }) => {
    test.setTimeout(300_000);
    await session.openEditor();
    const { page } = session;
    await page.evaluate(buildSyntheticProject, { layers: 1000, dur: 60 });
    await page.waitForTimeout(2000);
    const result = await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.loop = false;
      PM.setTime(0);
      PM.play();
      await new Promise((r) => setTimeout(r, 1500));
      const applyMs: number[] = [];
      const layers = PM.proj.layers.filter((l: any) => l.type === 'solid');
      for (let i = 0; i < 60; i++) {
        const layer = layers[(i * 37) % layers.length];
        const t0 = performance.now();
        PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 30 + (i % 60) }, { label: 'Opacity', origin: 'inspector' });
        await Promise.resolve();
        applyMs.push(performance.now() - t0);
        await new Promise((r) => setTimeout(r, 250 - (performance.now() - t0) % 250));
      }
      PM.pause();
      return { applyMs, mirror: !!PM.EngineDoc?.enabled };
    });
    await record('edit-during-playback-1000-page-legacy', { applyMs: dist(result.applyMs, 3), mirrorOn: result.mirror });
    expect(result.applyMs.length).toBe(60);
  });
});

test('cold start: the engine is ready before the window finished loading the last project', async () => {
  test.setTimeout(600_000);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-tracer-cold-'));
  try {
    let session = await launchApp({ userData, env: ON });
    try {
      await synthetic(session, 500);
      await engineReady(session.page, 120_000);
      // Open it the way a user's project is open (a tab of this window), so
      // the next launch opens it again.
      await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        const project = JSON.parse(JSON.stringify(PM.proj));
        PM.Projects.put(PM.proj);
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
        await new Promise((r) => setTimeout(r, 500));
        await PM.flushProject?.();
      });
      expect(await session.page.evaluate(() => (window as any).PM.proj.layers.length)).toBeGreaterThan(400);
      await session.page.waitForTimeout(1000);
    } finally {
      await session.close();
    }
    const runs: unknown[] = [];
    for (let i = 0; i < 3; i++) {
      const launchedAt = Date.now();
      session = await launchApp({ userData, env: ON });
      try {
        expect(await session.page.evaluate(() => (window as any).PM.proj.layers.length)).toBeGreaterThan(400);
        const marks = await session.page.evaluate(async () => {
          const PM = (window as any).PM;
          await PM.projectAssetsReady;
          const deadline = performance.now() + 60_000;
          const mark = (name: string) => performance.getEntriesByName(name)[0]?.startTime ?? null;
          while ((mark('powermove:engine-first-frame') === null || mark('powermove:project-ready') === null) && performance.now() < deadline) await new Promise((r) => setTimeout(r, 20));
          return { timeOrigin: performance.timeOrigin, layers: PM.proj.layers.length, projectReady: mark('powermove:project-ready'), engineConnected: mark('powermove:engine-connected'), engineFirstFrame: mark('powermove:engine-first-frame') };
        });
        const fromLaunch = (ms: number | null) => (ms === null ? null : Math.round(marks.timeOrigin + ms - launchedAt));
        runs.push({
          layers: marks.layers,
          pageLoadAfterLaunchMs: Math.round(marks.timeOrigin - launchedAt),
          projectReadyMs: fromLaunch(marks.projectReady),
          engineConnectedMs: fromLaunch(marks.engineConnected),
          engineFirstFrameMs: fromLaunch(marks.engineFirstFrame),
          connectedBeforeProjectReady: marks.engineConnected !== null && marks.projectReady !== null && marks.engineConnected <= marks.projectReady,
          firstFrameBeforeProjectReady: marks.engineFirstFrame !== null && marks.projectReady !== null && marks.engineFirstFrame <= marks.projectReady
        });
      } finally {
        await session.close();
      }
    }
    await record('cold-start-500', { runs });
  } finally {
    await rm(userData, { recursive: true, force: true });
  }
});
