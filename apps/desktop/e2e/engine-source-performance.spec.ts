/** Paired source workloads: pm-perfbench's public fixture on a 320x180
 * monitor, and 1080p H.264, 4K 10-bit HEVC and 1080p ProRes 422 on a
 * 1280x720 monitor. Each app session imports COPIES distinct remuxes of every
 * workload (same streams, different container metadata, so import cannot
 * reuse an asset); each copy's first filmstrip is one cold sample and its
 * first scrub is the cold monitor. RUNS alternating off/on/on/off cycles feed
 * helpers/source-performance-gate.ts. Each session waits for the machine's
 * 1-minute load to drop under MAX_LOAD and records it; the gate fails a
 * session that never got there. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { expect, launchApp, repoRoot, test } from './helpers/app';
import { fixturePath } from './helpers/media';
import { sourceFfmpeg } from './helpers/source-media';
import { MAX_LOAD, MIN_RUNS, percentile, sourcePerformanceGate, type PerfSample } from './helpers/source-performance-gate';

const COPIES = 3;
// Fewer runs (diagnostics) cannot pass: the gate itself needs MIN_RUNS.
const RUNS = Number(process.env.PM_SOURCE_PERF_RUNS) || MIN_RUNS;
const LOAD_WAIT_MS = 10 * 60_000;
const exec = promisify(execFile);
const spread = (values: number[]) => ({ n: values.length, p50: percentile(values, .5), p95: percentile(values, .95),
  p99: percentile(values, .99), max: Math.max(...values), top: [...values].sort((a, b) => b - a).slice(0, 5) });

const synthetic = (size: string, codec: string[]) =>
  ['-f', 'lavfi', '-i', `testsrc2=size=${size}:rate=30:duration=2`, ...codec];
const WORKLOADS = [
  { name: 'h264-160', monitor: [320, 180], extension: 'mp4', encode: null },
  { name: 'h264-1080', monitor: [1280, 720], extension: 'mp4',
    encode: synthetic('1920x1080', ['-c:v', 'libx264', '-preset', 'veryfast', '-g', '30', '-pix_fmt', 'yuv420p']) },
  { name: 'hevc10-4k', monitor: [1280, 720], extension: 'mp4',
    encode: synthetic('3840x2160', ['-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'log-level=error:keyint=30',
      '-pix_fmt', 'yuv420p10le', '-tag:v', 'hvc1']) },
  { name: 'prores-1080', monitor: [1280, 720], extension: 'mov',
    encode: synthetic('1920x1080', ['-c:v', 'prores_ks', '-profile:v', '2', '-pix_fmt', 'yuv422p10le']) }
] as const;

async function quietLoad() {
  const until = Date.now() + LOAD_WAIT_MS;
  while (os.loadavg()[0]! >= MAX_LOAD && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 15_000));
  return os.loadavg();
}

test('source filmstrip and monitor scrub cost no more than the current app', async () => {
  test.setTimeout(RUNS * (4 * LOAD_WAIT_MS + 1_200_000));
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'performance') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-'));
  await mkdir(directory, { recursive: true });
  const machine = { startedAt: new Date().toISOString(), platform: `${os.platform()} ${os.release()} ${os.arch()}`,
    cpus: os.cpus().length, loadBefore: os.loadavg() };
  const results: PerfSample[] = [];
  const copies = await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-copies-'));
  try {
    const imports: { workload: (typeof WORKLOADS)[number]; copy: number; name: string }[] = [];
    for (const workload of WORKLOADS) {
      const base = workload.encode ? path.join(copies, `${workload.name}.${workload.extension}`) : fixturePath('h264-aac.mp4');
      if (workload.encode) await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', ...workload.encode, base], { timeout: 300_000 });
      for (let copy = 0; copy < COPIES; copy++) {
        const name = `${workload.name}-cold-${copy}.${workload.extension}`;
        await exec(sourceFfmpeg(), ['-hide_banner', '-loglevel', 'error', '-nostdin', '-y', '-i', base, '-map', '0', '-c', 'copy',
          '-metadata', `comment=pm-source-cold-${workload.name}-${copy}`, '-movflags', '+faststart', path.join(copies, name)], { timeout: 60_000 });
        imports.push({ workload, copy, name });
      }
    }
    // This spec itself runs under pm-electron-lock. Build the probe beforehand;
    // collect its native-only counters beside the end-to-end app wall clocks.
    const probe = await exec(path.resolve(repoRoot, '../../engine/target/release/examples/source_media'), [], {
      env: { ...process.env, PM_SOURCE_BENCH_MEDIA: fixturePath('h264-aac.mp4') }, timeout: 60_000
    });
    const counters = probe.stdout.trim().split('\n').map(line => JSON.parse(line));
    // Alternating order makes drift visible instead of giving one mode every warm run.
    for (let run = 0; run < RUNS; run++) for (const [round, enabled] of [[0, false], [0, true], [1, true], [1, false]] as const) {
      const loadBefore = await quietLoad();
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      try {
        await session.openEditor();
        for (const { workload, copy, name } of imports) {
          const chooser = session.page.waitForEvent('filechooser');
          await session.page.evaluate(() => (window as any).PM.pickFiles());
          await (await chooser).setFiles(path.join(copies, name));
          await session.page.waitForFunction(name => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === name), name);
          if (enabled) await session.page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
          const measured = await session.page.evaluate(async ({ enabled, name, monitorSize: [width, height] }) => {
            const PM = (window as any).PM;
            const id = Object.values<any>(PM.proj.assets).find(a => a.name === name)!.id;
            const viewer = PM.Kernel.services.get('viewer');
            const ticks = 705_600_000;
            const times = (scattered: boolean) => Array.from({ length: 24 }, (_, k) => Math.round(((scattered ? (k * 7) % 24 : k) * 1.9 / 24 + 1 / 60) * ticks) / ticks);
            const painted = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
            const wait = (source: any, event: string, time: number) => new Promise<void>((resolve, reject) => {
              const done = () => { if (Math.abs(source.currentTime - time) < .00001) {
                clearTimeout(timer); source.removeEventListener(event, done); resolve();
              } };
              const timer = setTimeout(() => {
                source.removeEventListener(event, done);
                const state = enabled ? { error: source.error?.message ?? null, rendering: source.rendering,
                  currentTime: source.currentTime, stats: PM.EngineSourceMedia.stats } : { readyState: source.readyState, currentTime: source.currentTime };
                reject(new Error(`${event} timed out at ${time}: ${JSON.stringify(state)}`));
              }, 15_000);
              source.addEventListener(event, done);
              source.currentTime = time;
            });
            let previewCleared = false;
            try {
              viewer.preview.show(id);
              const root = document.querySelector<HTMLElement>('#source-preview')!;
              root.style.width = `${width / devicePixelRatio}px`; root.style.height = `${height / devicePixelRatio}px`;
              root.style.right = root.style.bottom = 'auto';
              const monitor = enabled ? PM.EngineSourceMedia.monitorValue : root.querySelector('video');
              const until = performance.now() + 15_000;
              while (enabled ? monitor.paintedSize?.width !== width || monitor.paintedSize?.height !== height : monitor.readyState < 2) {
                if (performance.now() > until) throw new Error('Source monitor did not reach its measured size');
                await painted();
              }
              await painted();
              const scrubMs: number[] = [];
              for (const time of times(true)) {
                const began = performance.now();
                await wait(monitor, enabled ? 'loadeddata' : 'seeked', time);
                // Both paths include the next presentation opportunity.
                await painted();
                scrubMs.push(performance.now() - began);
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
              return { scrubMs, filmstripMs };
            } finally {
              if (!previewCleared) viewer.preview.clear();
            }
          }, { enabled, name, monitorSize: workload.monitor });
          results.push({ workload: workload.name, run, enabled, round, copy, loadBefore, ...measured });
        }
        expect(session.diagnostics.pageErrors).toEqual([]);
      } finally {
        await session.close();
      }
      await writeFile(path.join(directory, 'app-source-performance.json'), JSON.stringify(results, null, 2));
    }
    const gate = sourcePerformanceGate(results);
    // Per run, session and copy, so sub-vsync jitter reads apart from a tail.
    const distribution = results.map(r => ({ workload: r.workload, run: r.run, enabled: r.enabled, round: r.round, copy: r.copy,
      loadBefore: r.loadBefore, firstScrub: r.scrubMs[0], steady: spread(r.scrubMs.slice(1)), coldFilmstrip: r.filmstripMs[0] }));
    await writeFile(path.join(directory, 'comparison.json'), JSON.stringify({ gate, distribution, counters }, null, 2));
    expect(gate.failures).toEqual([]);
  } finally {
    await rm(copies, { recursive: true, force: true });
    try { await writeFile(path.join(directory, 'machine.json'), JSON.stringify({ ...machine, endedAt: new Date().toISOString(), loadAfter: os.loadavg() }, null, 2)); }
    finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
  }
});
