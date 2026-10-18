/*
 * Phase 3a performance rows measured in the app with "Use New Engine" on
 * (plan section 25; docs/engine/phase3a-gate.md):
 *
 * - "UHD ProRes 422 HQ, 3 layers with blur: real time at 23.976, 29.97,
 *   59.94 on Pro; zero drops over 60 s; same with linear blending on".
 *   Three 3840 × 2160 ProRes 422 HQ layers, each a different 4 s section of
 *   one 12 s clip (three decodes per frame), each with a Gaussian blur, in
 *   a composition of a whole number of frames (96, 120, 240: about 4 s) at
 *   the clip's rate, played looping for 60 s, then
 *   again after turning on "Blend in linear light" (if the composition
 *   still routes to the engine; its route is recorded either way). Every
 *   timed frame the presentation shows is counted: a composition frame
 *   skipped between two shown frames is a drop, and so is anything the
 *   presenter reports dropped (a frame replaced by a newer revision of
 *   itself is not: it is shown).
 * - Routing evidence for the canvas text typing row: a canvas text layer
 *   routes to the current engine (`route` and `routeReason` as the engine
 *   connection reports them).
 * - The memory row ("within leases; no growth over a 1-hour playback
 *   loop"): the 1,000-layer synthetic project the engine draws, looping
 *   for $PM_P3A_MEMORY_SECONDS (3600 for the row; the test is skipped
 *   without it), sampling every process's memory (Electron's
 *   `app.getAppMetrics()`: the engines by service name) and the engines'
 *   last lease usage against their grants every 30 s.
 *
 * The clips are generated outside the repository (docs/engine/phase3a-gate.md):
 *   ffmpeg -f lavfi -i "testsrc2=size=3840x2160:rate=30000/1001,noise=alls=12:allf=t" -t 12 \
 *     -c:v prores_videotoolbox -profile:v hq -pix_fmt p210le \
 *     -color_primaries bt709 -color_trc bt709 -colorspace bt709 prores422hq-2160p-2997.mov
 * (and 24000/1001 → -23976, 60000/1001 → -5994) in $PM_P3A_MEDIA. The
 * clips are copied into the disposable profile's media store, as an import
 * does by default (PM_P3A_LINK=1 links them instead).
 *
 * Opt in (heavy; one Electron at a time):
 *   PM_P3A_MEASURE=1 pm-electron-lock bunx playwright test e2e/native-engine-p3a-measure.spec.ts --workers=1 --global-timeout=0
 * Results go to $PM_P3A_MEASURE_OUT (JSON lines; default
 * /tmp/powermove-p3a-measure.jsonl), never into the repository.
 */
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { appendFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import type { Page } from 'playwright';

import { buildSyntheticProject } from '../src/main/engine/integration/synthetic-project';
import { expect, test } from './helpers/app';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const OUT = process.env.PM_P3A_MEASURE_OUT ?? path.join(os.tmpdir(), 'powermove-p3a-measure.jsonl');
const MEDIA = process.env.PM_P3A_MEDIA ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/scratch/p3agate');
const SECONDS = Number(process.env.PM_P3A_SECONDS ?? 60);
const MEMORY_SECONDS = Number(process.env.PM_P3A_MEMORY_SECONDS ?? 0);
/** Link the clips instead of copying them into the profile's media store (PM_P3A_LINK=1). */
const LINK_MEDIA = process.env.PM_P3A_LINK === '1';

test.skip(!process.env.PM_P3A_MEASURE, 'measurements run on request (PM_P3A_MEASURE=1)');

async function record(row: string, value: unknown): Promise<void> {
  const line = JSON.stringify({ row, at: new Date().toISOString(), cpu: os.cpus()[0]?.model ?? null, load: os.loadavg(), value });
  await appendFile(OUT, `${line}\n`);
  console.log(`[measure] ${row}: ${JSON.stringify(value)}`);
}

/** Whether an FFmpeg process is converting this clip (main's rendition for the legacy viewer). */
function converting(name: string): boolean {
  try {
    return execFileSync('/bin/ps', ['-axo', 'command='], { encoding: 'utf8' }).split('\n').some((line) => /ffmpeg/i.test(line) && line.includes(name));
  } catch {
    return false;
  }
}

const status = (page: Page): Promise<any> => page.evaluate(() => (window as any).PM.EngineConnection.status());

/** Waits until the engine connection has decided this composition's route. */
async function settledRoute(page: Page, timeout = 120_000): Promise<any> {
  const deadline = Date.now() + timeout;
  let last: any = null;
  while (Date.now() < deadline) {
    last = await status(page);
    if (last?.route === 'engine') {
      const presenting = await page.evaluate(() => !!(window as any).PM.Kernel.services.get('viewer').enginePresentation?.presenting);
      if (presenting) return last;
    } else if (last?.route === 'legacy') {
      // Give a routing decision time to settle (a route can pass through legacy while loading).
      await page.waitForTimeout(3000);
      return await status(page);
    }
    await page.waitForTimeout(250);
  }
  return last;
}

const RATES = [
  { name: '23.976', num: 24000, den: 1001, file: 'prores422hq-2160p-23976.mov' },
  { name: '29.97', num: 30000, den: 1001, file: 'prores422hq-2160p-2997.mov' },
  { name: '59.94', num: 60000, den: 1001, file: 'prores422hq-2160p-5994.mov' }
];

test.describe('Use New Engine: Phase 3a measurements', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  for (const rate of RATES) {
    test(`UHD ProRes 422 HQ, 3 layers with blur, ${rate.name} fps, linear blending off and on`, async ({ session }) => {
      test.setTimeout(2 * SECONDS * 1000 + 1_800_000);
      const file = path.join(MEDIA, rate.file);
      test.skip(!existsSync(file), `generate ${file} first (see the header)`);
      const { page } = session;
      await session.openEditor();
      await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
      await page.evaluate(({ num, den, linkMedia }) => {
        const PM = (window as any).PM;
        if (linkMedia) PM.HostMedia?.setProjectStorage?.('link');
        // A whole number of frames (about 4 s), so the loop wraps on a frame boundary.
        const project = PM.mkProject({ name: 'UHD ProRes', w: 3840, h: 2160, fps: num / den, dur: (Math.round((4 * num) / den) * den) / num, bg: '#000000' });
        project.fpsRational = { num, den };
        PM.replaceProject(project);
        PM.setTime(0);
      }, { num: rate.num, den: rate.den, linkMedia: LINK_MEDIA });
      const importStart = Date.now();
      const chooser = page.waitForEvent('filechooser');
      await page.evaluate(() => (window as any).PM.pickFiles());
      await (await chooser).setFiles(file);
      await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'), undefined, { timeout: 900_000 });
      // Main converts a ProRes original into a rendition for the legacy
      // viewer: let it finish so its encoder does not share the machine with
      // the measurement.
      const idle = Date.now() + 900_000;
      while (Date.now() < idle && converting(rate.file)) await page.waitForTimeout(2000);
      const importSeconds = (Date.now() - importStart) / 1000;
      const built = await page.evaluate(() => {
        const PM = (window as any).PM;
        const first = PM.proj.layers.find((l: any) => l.type === 'video');
        const asset = first.d.asset;
        // Two more layers of the same clip, 3.9 s and 7.8 s into it (every
        // layer ends inside the 12 s clip).
        for (const trim of [3.9, 7.8]) {
          PM.Edit.apply({ type: 'add_layer', layerType: 'video', name: `UHD ${trim}`, content: { asset, fit: 'contain', w: 3840, h: 2160, trim, speed: 1 } }, { origin: 'panel', label: 'Add layer' });
        }
        const videos = PM.proj.layers.filter((l: any) => l.type === 'video');
        for (const layer of videos) {
          PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0, duration: PM.proj.dur } }, { origin: 'panel', label: 'Timing' });
          PM.Edit.apply({ type: 'add_effect', target: layer.id, effect: 'blur', parameters: { amount: 24 } }, { origin: 'panel', label: 'Add effect' });
        }
        // Lower layers show through: each upper layer at 60% opacity.
        videos.slice(1).forEach((layer: any) => PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'opacity', value: 60 }, { origin: 'panel', label: 'Opacity' }));
        return {
          layers: PM.proj.layers.map((l: any) => ({ type: l.type, trim: l.d?.trim, from: l.from, dur: l.dur, fx: (l.fx ?? []).map((f: any) => f.type), asset: l.d?.asset })),
          fps: PM.proj.fps,
          dur: PM.proj.dur,
          asset: PM.assets?.get?.(asset) ? { storage: PM.assets.get(asset).storage ?? null, rendition: PM.assets.get(asset).rendition ?? null } : null
        };
      });
      expect(built.layers.filter((l: any) => l.type === 'video')).toHaveLength(3);
      for (const linear of [false, true]) {
        const row = `uhd-prores-3-blur-${rate.name}${linear ? '-linear' : ''}`;
        if (linear) {
          // Settings › Project › Color › "Blend in linear light", as the switch applies it.
          await page.evaluate(() => {
            const PM = (window as any).PM;
            const next = { mode: 'standard', linearBlending: true, precision: 'float16', blendDomain: 'hdrSafe' };
            const apply = () => { PM.proj.color = next; };
            if (PM.hist?.do) PM.hist.do('Blend in linear light', apply);
            else apply();
            PM.touch?.();
            PM.bus?.emit?.('project');
            PM.invalidate?.('render');
          });
          await page.waitForTimeout(3000);
        }
        const routed = await settledRoute(page);
        const project = await page.evaluate(() => ({ color: (window as any).PM.proj.color ?? null, requiresEngine: (window as any).PM.proj.requiresEngine ?? null }));
        if (routed?.route !== 'engine') {
          await record(row, { built, importSeconds, project, route: routed?.route ?? null, routeReason: routed?.routeReason ?? null, failures: routed?.failures ?? null });
          continue;
        }
        const result = await page.evaluate(async ({ seconds, ticks }) => {
          const PM = (window as any).PM;
          const presentation = PM.Kernel.services.get('viewer').enginePresentation;
          const fps = PM.proj.fps;
          const frameTicks = ticks / fps;
          const frames = Math.round(PM.proj.dur * fps);
          PM.loop = true;
          PM.setTime(0);
          PM.play();
          // Warm up: the decoders and the first loop.
          await new Promise((r) => setTimeout(r, 3000));
          const before = presentation.stats();
          let shown = 0;
          let previous = -1;
          let skipped = 0;
          let repeats = 0;
          const skips: number[] = [];
          const intervals: number[] = [];
          let lastDisplay: number | null = null;
          const off = presentation.onPresented((f: any) => {
            if (!f.timed) return;
            shown++;
            const k = Math.round(f.pts / frameTicks) % frames;
            if (previous >= 0) {
              const step = (k - previous + frames) % frames;
              if (step === 0) repeats++;
              else if (step > 1) { skipped += step - 1; if (skips.length < 20) skips.push(k); }
            }
            previous = k;
            if (f.displayUs != null) {
              if (lastDisplay !== null) intervals.push((f.displayUs - lastDisplay) / 1000);
              lastDisplay = f.displayUs;
            }
          });
          const t0 = performance.now();
          await new Promise((r) => setTimeout(r, seconds * 1000));
          off();
          const elapsed = (performance.now() - t0) / 1000;
          const after = presentation.stats();
          PM.pause();
          const sorted = intervals.sort((a, b) => a - b);
          const at = (p: number) => sorted.length ? +sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))]!.toFixed(2) : null;
          return { elapsed, expected: Math.round(elapsed * fps), frames, shown, skipped, skips, repeats, before, after, intervalMs: { n: sorted.length, p50: at(0.5), p99: at(0.99), max: at(1) } };
        }, { seconds: SECONDS, ticks: 705_600_000 });
        const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
        const dropped = (s: any) => Object.entries(s?.dropped ?? {}).filter(([reason]) => reason !== 'replaced').reduce((a: number, [, b]: [string, any]) => a + Number(b), 0);
        await record(row, {
          seconds: result.elapsed,
          expectedFrames: result.expected,
          compositionFrames: result.frames,
          shownFrames: result.shown,
          skippedFrames: result.skipped,
          skippedAt: result.skips,
          repeatedFrames: result.repeats,
          presenterDropped: dropped(result.after) - dropped(result.before),
          presenterDroppedByReason: result.after?.dropped ?? null,
          displayIntervalMs: result.intervalMs,
          engine: {
            lateSkipped: report?.presenter?.lateSkipped ?? null,
            renderErrors: report?.presenter?.renderErrors ?? null,
            layerFailures: report?.presenter?.layerFailures ?? null,
            composeUsMax: report?.presenter?.composeUsMax ?? null,
            composeUsMean: report?.presenter?.documentFrames ? Math.round(report.presenter.composeUsTotal / report.presenter.documentFrames) : null
          },
          importSeconds,
          project,
          built
        });
        expect(result.shown).toBeGreaterThan(0);
      }
    });
  }

  test('routing: a canvas text layer', async ({ session }) => {
    test.setTimeout(300_000);
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Typing', w: 1920, h: 1080, fps: 30, dur: 10, bg: '#000000' }));
      PM.Edit.apply({ type: 'add_layer', id: 'p3a-title', layerType: 'text', content: { text: 'Typing' } }, { origin: 'panel', label: 'Add text' });
    });
    const text = await settledRoute(page);
    await record('route-canvas-text', { route: text?.route ?? null, routeReason: text?.routeReason ?? null, failures: text?.failures ?? null });
    expect(text?.route).toBeTruthy();
  });

  test('memory: a playback loop of the 1,000-layer project', async ({ session }) => {
    test.skip(!MEMORY_SECONDS, 'set PM_P3A_MEMORY_SECONDS (3600 for the row)');
    test.setTimeout(MEMORY_SECONDS * 1000 + 600_000);
    const { page } = session;
    await session.openEditor();
    await page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
    const summary = await page.evaluate(buildSyntheticProject, { layers: 1000, images: 2, dur: 90 });
    expect(summary.layers).toBe(1000);
    const routed = await settledRoute(page);
    expect(routed?.route).toBe('engine');
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.loop = true;
      PM.setTime(0);
      PM.play();
    });
    const sample = async (): Promise<unknown> => {
      const main = await session.app.evaluate(({ app }) => {
        const engine = (globalThis as any).__powermoveNativeEngine?.engine;
        const leases: Record<string, unknown> = {};
        for (const process of ['interactive', 'job']) {
          const state = engine?.memory?.leases?.get?.(process);
          leases[process] = state ? { leaseBytes: state.current.bytes, reason: state.current.reason, usedBytes: state.lastUsage?.total ?? null, categories: state.lastUsage?.categories ?? null } : null;
        }
        return {
          processes: app.getAppMetrics().map((m: any) => ({ pid: m.pid, type: m.type, service: m.serviceName ?? m.name ?? null, workingSetKiB: m.memory?.workingSetSize ?? null, peakKiB: m.memory?.peakWorkingSetSize ?? null, privateKiB: m.memory?.privateBytes ?? null })),
          leases,
          globalBytes: engine?.memory?.global ?? null
        };
      });
      const page = await session.page.evaluate(() => {
        const PM = (window as any).PM;
        const stats = PM.Kernel.services.get('viewer').enginePresentation?.stats?.() ?? null;
        return { route: PM.EngineConnection.status().route, playing: !!PM.playing, presented: stats?.presented ?? null, dropped: stats?.dropped ?? null, heapBytes: (performance as any).memory?.usedJSHeapSize ?? null };
      });
      return { t: Date.now(), ...main, page };
    };
    const samples: unknown[] = [await sample()];
    const start = Date.now();
    while (Date.now() - start < MEMORY_SECONDS * 1000) {
      await page.waitForTimeout(Math.min(30_000, MEMORY_SECONDS * 1000 - (Date.now() - start)));
      samples.push(await sample());
    }
    await page.evaluate(() => (window as any).PM.pause());
    const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
    await record('memory-playback-loop-1000', { seconds: MEMORY_SECONDS, summary, samples, presenter: { lateSkipped: report?.presenter?.lateSkipped ?? null, renderErrors: report?.presenter?.renderErrors ?? null, layerFailures: report?.presenter?.layerFailures ?? null, documentFrames: report?.presenter?.documentFrames ?? null } });
    expect(samples.length).toBeGreaterThan(1);
  });
});
