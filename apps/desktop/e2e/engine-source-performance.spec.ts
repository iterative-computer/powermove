/** Paired source workload, matching pm-perfbench's public fixture and time grid.
 * Each app session imports COPIES distinct remuxes of the fixture (same
 * streams, different container metadata, so import cannot reuse an asset).
 * Every copy is a cold source: its first filmstrip is one cold sample.
 * RUNS alternating off/on/on/off cycles feed the statistic in
 * helpers/source-performance-gate.ts: medians across runs of each per-run
 * statistic, zero slack, and no native run's scrub P95 above today's worst. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import type { CDPSession } from 'playwright';
import { expect, launchApp, repoRoot, test } from './helpers/app';
import { fixturePath } from './helpers/media';
import { sourceFfmpeg } from './helpers/source-media';
import { MIN_RUNS, percentile, sourcePerformanceGate, type PerfSample } from './helpers/source-performance-gate';
import { startSourceTimeline } from './helpers/source-timeline';

const COPIES = 3;
// Fewer runs (diagnostics) cannot pass: the gate itself needs MIN_RUNS.
const RUNS = Number(process.env.PM_SOURCE_PERF_RUNS) || MIN_RUNS;
const spread = (values: number[]) => ({ n: values.length, p50: percentile(values, .5), p90: percentile(values, .9),
  p95: percentile(values, .95), max: Math.max(...values), top: [...values].sort((a, b) => b - a).slice(0, 5) });

test('source filmstrip and monitor scrub cost no more than the current app', async () => {
  test.setTimeout(RUNS * 600_000);
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'performance') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-'));
  await mkdir(directory, { recursive: true });
  const machine = { startedAt: new Date().toISOString(), platform: `${os.platform()} ${os.release()} ${os.arch()}`,
    cpus: os.cpus().length, loadBefore: os.loadavg() };
  const results: Array<PerfSample & { width: number; height: number; loadBefore: number[] }> = [];
  const copies = await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-copies-'));
  try {
    const names = Array.from({ length: COPIES }, (_, k) => `h264-aac-cold-${k}.mp4`);
    for (const [k, name] of names.entries())
      await promisify(execFile)(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', '-i', fixturePath('h264-aac.mp4'),
        '-map', '0', '-c', 'copy', '-metadata', `comment=pm-source-cold-${k}`, '-movflags', '+faststart', path.join(copies, name)], { timeout: 30_000 });
    // This spec itself runs under pm-electron-lock. Build the probe beforehand;
    // collect its native-only counters beside the end-to-end app wall clocks.
    const probe = await promisify(execFile)(path.resolve(repoRoot, '../../engine/target/release/examples/source_media'), [], {
      env: { ...process.env, PM_SOURCE_BENCH_MEDIA: fixturePath('h264-aac.mp4') }, timeout: 60_000
    });
    const counters = probe.stdout.trim().split('\n').map(line => JSON.parse(line));
    await writeFile(path.join(directory, 'native-counters.jsonl'), probe.stdout);
    // Alternating order makes drift visible instead of giving one mode every warm run.
    for (let run = 0; run < RUNS; run++) for (const [round, enabled] of [[0, false], [0, true], [1, true], [1, false]] as const) {
      const loadBefore = os.loadavg();
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      let nativeTrace = '';
      const record = (chunk: Buffer) => { if (nativeTrace.length < 2 * 1024 * 1024) nativeTrace += chunk.toString(); };
      const child = session.app.process();
      let cpu: CDPSession | null = null;
      let timeline: CDPSession | null = null;
      let stopTimeline: (() => Promise<{ events: number; bytes: number; truncated: boolean }>) | null = null;
      child.stdout?.on('data', record); child.stderr?.on('data', record);
      try {
        if (process.env.PM_SOURCE_TIMELINE === '1') {
          timeline = await session.page.context().newCDPSession(session.page);
          stopTimeline = await startSourceTimeline(timeline, path.join(directory, `${enabled ? 'native' : 'legacy'}-${run}-${round}.trace.json`));
        }
        if (process.env.PM_SOURCE_CPU_PROFILE === '1') {
          cpu = await session.page.context().newCDPSession(session.page);
          await cpu.send('Profiler.enable'); await cpu.send('Profiler.start');
        }
        await session.openEditor();
        for (const [copy, name] of names.entries()) {
          const chooser = session.page.waitForEvent('filechooser');
          await session.page.evaluate(() => (window as any).PM.pickFiles());
          await (await chooser).setFiles(path.join(copies, name));
          await session.page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), name);
          if (enabled) await session.page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
          const measured = await session.page.evaluate(async ({ enabled, profile, timeline, run, round, name }) => {
            const PM = (window as any).PM;
            const id = Object.values<any>(PM.proj.assets).find(a => a.name === name)!.id;
            const viewer = PM.Kernel.services.get('viewer');
            const frameTrace: Record<string, number | string | null>[] = [];
            const scrubTrace: Record<string, number | null>[] = [];
            const canvasTrace: { began: number; ended: number; scrub: number | null }[] = [];
            const longTasks: { began: number; duration: number }[] = [];
            const prefix = `pm-source:${enabled ? 'native' : 'legacy'}:${run}:${round}:${name}`;
            const mark = (name: string) => { if (timeline) performance.mark(`${prefix}:${name}`); };
            let activeScrub: number | null = null;
            let frameNumber = 0;
            const observer = timeline && PerformanceObserver.supportedEntryTypes.includes('longtask')
              ? new PerformanceObserver(list => {
                for (const entry of list.getEntries()) longTasks.push({ began: entry.startTime, duration: entry.duration });
              }) : null;
            observer?.observe({ type: 'longtask' });
            let restoreFrame = () => {};
            if (enabled && profile) {
              const source = PM.EngineSourceMedia, frame = source.frame;
              restoreFrame = () => { source.frame = frame; };
              source.frame = async (...args: any[]) => {
                const began = performance.now(), number = frameNumber++, scrub = activeScrub;
                const sample: Record<string, number | string | null> = { number, scrub, began, time: args[1], width: args[2], height: args[3] };
                frameTrace.push(sample);
                mark(`frame:${number}:start`);
                try {
                  const value = await Reflect.apply(frame, source, args) as {
                    mainMs?: number; waitedUs?: number; renderUs?: number; revision?: number;
                  };
                  Object.assign(sample, { ended: performance.now(), elapsedMs: performance.now() - began,
                    mainMs: value.mainMs ?? null, waitedUs: value.waitedUs ?? null, renderUs: value.renderUs ?? null, revision: value.revision ?? null });
                  mark(`frame:${number}:done`);
                  return value;
                } catch (error) {
                  Object.assign(sample, { ended: performance.now(), error: error instanceof Error ? error.name : String(error) });
                  mark(`frame:${number}:failed`);
                  throw error;
                }
              };
            }
            const ticks = 705_600_000;
            const times = (scattered: boolean) => Array.from({ length: 24 }, (_, k) => Math.round(((scattered ? (k * 7) % 24 : k) * 1.9 / 24 + 1 / 60) * ticks) / ticks);
            const painted = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
            const paintedAt = () => new Promise<number>(resolve => requestAnimationFrame(() => resolve(performance.now())));
            const wait = (source: any, event: string, time: number, sample?: Record<string, number | null>, index?: number) => new Promise<void>((resolve, reject) => {
              const done = () => { if (Math.abs(source.currentTime - time) < .00001) {
                if (sample) { sample.eventAt = performance.now(); mark(`scrub:${index}:event`); }
                clearTimeout(timer); source.removeEventListener(event, done); resolve();
              } };
              const timer = setTimeout(() => {
                source.removeEventListener(event, done);
                const state = enabled ? { error: source.error?.message ?? null, rendering: source.rendering, paintAgain: source.paintAgain,
                  currentTime: source.currentTime, stats: PM.EngineSourceMedia.stats } : { readyState: source.readyState, currentTime: source.currentTime };
                reject(new Error(`${event} timed out at ${time}: ${JSON.stringify(state)}`));
              }, 15_000);
              source.addEventListener(event, done);
              if (sample) { sample.setAt = performance.now(); mark(`scrub:${index}:set`); }
              source.currentTime = time;
              if (sample) { sample.setDoneAt = performance.now(); mark(`scrub:${index}:setDone`); }
            });
            const canvasPrototype = CanvasRenderingContext2D.prototype;
            const putImageData = canvasPrototype.putImageData;
            let canvas: HTMLCanvasElement | null = null;
            let previewCleared = false;
            try {
              viewer.preview.show(id);
              const root = document.querySelector<HTMLElement>('#source-preview')!;
              root.style.width = `${320 / devicePixelRatio}px`; root.style.height = `${180 / devicePixelRatio}px`;
              root.style.right = root.style.bottom = 'auto';
              const monitor = enabled ? PM.EngineSourceMedia.monitorValue : root.querySelector('video');
              canvas = enabled && timeline ? monitor.canvas as HTMLCanvasElement : null;
              if (canvas) canvasPrototype.putImageData = function(this: CanvasRenderingContext2D, ...args: any[]) {
                if (this.canvas !== canvas) return Reflect.apply(putImageData, this, args);
                const began = performance.now();
                mark(`canvas:${canvasTrace.length}:start`);
                try { return Reflect.apply(putImageData, this, args); }
                finally {
                  canvasTrace.push({ began, ended: performance.now(), scrub: activeScrub });
                  mark(`canvas:${canvasTrace.length - 1}:done`);
                }
              };
              const until = performance.now() + 15_000;
              while (enabled ? monitor.canvas.width !== 320 || monitor.canvas.height !== 180 : monitor.readyState < 2) {
                if (performance.now() > until) throw new Error('Source monitor did not reach its measured size');
                await painted();
              }
              await painted();
              const scrubMs: number[] = [];
              for (const [index, time] of times(true).entries()) {
                activeScrub = index;
                const began = performance.now();
                const sample = timeline ? { index, time, began, revisionAtSet: enabled ? PM.EngineDoc?.barrier?.revision ?? null : null } : undefined;
                if (sample) { scrubTrace.push(sample); mark(`scrub:${index}:start`); }
                await wait(monitor, enabled ? 'loadeddata' : 'seeked', time, sample, index);
                // Both paths include the next presentation opportunity.
                let rafAt: number | null = null;
                if (sample) rafAt = await paintedAt(); else await painted();
                if (sample) {
                  Object.assign(sample, { rafAt, revisionAtRaf: enabled ? PM.EngineDoc?.barrier?.revision ?? null : null });
                  mark(`scrub:${index}:raf`);
                }
                scrubMs.push(performance.now() - began);
                activeScrub = null;
              }
              viewer.preview.clear(); previewCleared = true;
              const filmstripMs: number[] = [];
              for (let run = 0; run < 3; run++) {
                const began = performance.now();
                if (enabled) {
                  const frames = await PM.EngineSourceMedia.filmstrip(id, times(false), 160, 90);
                  if (frames.length !== 24 || frames.some((frame: Uint8Array) => frame.length !== 160 * 90 * 4)) throw new Error('Incomplete measured filmstrip');
                } else {
                  const video = PM.assets.get(id).el;
                  const canvas = document.createElement('canvas'); canvas.width = 160; canvas.height = 90;
                  const context = canvas.getContext('2d')!;
                  const frames: Uint8Array[] = [];
                  for (const time of times(false)) {
                    await wait(video, 'seeked', time);
                    context.clearRect(0, 0, 160, 90); context.drawImage(video, 0, 0, 160, 90);
                    frames.push(new Uint8Array(context.getImageData(0, 0, 160, 90).data));
                  }
                  if (frames.length !== 24) throw new Error('Incomplete current filmstrip');
                }
                filmstripMs.push(performance.now() - began);
              }
              return { scrubMs, filmstripMs, width: 320, height: 180,
                sourceStats: enabled ? { ...PM.EngineSourceMedia.stats } : null, frameTrace, scrubTrace, canvasTrace, longTasks };
            } finally {
              if (canvas) canvasPrototype.putImageData = putImageData;
              for (const entry of observer?.takeRecords() ?? []) longTasks.push({ began: entry.startTime, duration: entry.duration });
              observer?.disconnect();
              restoreFrame();
              if (!previewCleared) viewer.preview.clear();
            }
          }, { enabled, profile: process.env.PM_SOURCE_PROFILE === '1' || process.env.PM_SOURCE_TIMELINE === '1',
            timeline: process.env.PM_SOURCE_TIMELINE === '1', run, round, name });
          results.push({ run, enabled, round, copy, loadBefore, ...measured });
        }
        expect(session.diagnostics.pageErrors).toEqual([]);
      } finally {
        try {
          if (cpu) {
            const { profile } = await cpu.send('Profiler.stop');
            await writeFile(path.join(directory, `${enabled ? 'native' : 'legacy'}-${run}-${round}.cpuprofile`), JSON.stringify(profile));
          }
        } finally {
          try {
            if (stopTimeline) {
              const meta = await stopTimeline();
              await writeFile(path.join(directory, `${enabled ? 'native' : 'legacy'}-${run}-${round}-timeline.json`), JSON.stringify(meta));
            }
          } finally {
            await cpu?.detach().catch(() => {});
            await timeline?.detach().catch(() => {});
            await session.close();
          }
        }
        child.stdout?.off('data', record); child.stderr?.off('data', record);
        await writeFile(path.join(directory, `${enabled ? 'native' : 'legacy'}-${run}-${round}-stages.log`),
          nativeTrace.split('\n').filter(line => /source-job-timing|pm source (?:filmstrip|job|frame|batch)|pm job renderer/.test(line)).join('\n'));
      }
      await writeFile(path.join(directory, 'app-source-performance.json'), JSON.stringify(results, null, 2));
    }
    const gate = sourcePerformanceGate(results);
    const cold = (enabled: boolean) => results.filter(r => r.enabled === enabled).map(r => r.filmstripMs[0]!);
    const comparison = { gate,
      // Per run, session and copy, so sub-vsync jitter reads apart from a tail.
      distribution: results.map(r => ({ run: r.run, enabled: r.enabled, round: r.round, copy: r.copy, loadBefore: r.loadBefore,
        scrub: spread(r.scrubMs), coldFilmstrip: r.filmstripMs[0] })),
      cold: { legacy: { ...spread(cold(false)), p90: percentile(cold(false), .9) }, native: { ...spread(cold(true)), p90: percentile(cold(true), .9) } } };
    await writeFile(path.join(directory, 'comparison.json'), JSON.stringify({ comparison, counters }, null, 2));
    expect(gate.failures).toEqual([]);
  } finally {
    await rm(copies, { recursive: true, force: true });
    try { await writeFile(path.join(directory, 'machine.json'), JSON.stringify({ ...machine, endedAt: new Date().toISOString(), loadAfter: os.loadavg() }, null, 2)); }
    finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
  }
});
