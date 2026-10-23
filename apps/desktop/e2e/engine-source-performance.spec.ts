/** Paired source workload, matching pm-perfbench's public fixture and time grid. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { expect, launchApp, repoRoot, test } from './helpers/app';
import { fixturePath, importFixture } from './helpers/media';

test('source filmstrip and monitor scrub cost no more than the current app', async () => {
  test.setTimeout(540_000);
  const evidence = process.env.PM_SOURCE_EVIDENCE;
  const directory = evidence ? path.join(evidence, 'performance') : await mkdtemp(path.join(os.tmpdir(), 'pm-source-performance-'));
  await mkdir(directory, { recursive: true });
  const machine = { startedAt: new Date().toISOString(), platform: `${os.platform()} ${os.release()} ${os.arch()}`,
    cpus: os.cpus().length, loadBefore: os.loadavg() };
  const results: Array<{ enabled: boolean; round: number; scrubMs: number[]; filmstripMs: number[]; width: number; height: number }> = [];
  try {
    // This spec itself runs under pm-electron-lock. Build the probe beforehand;
    // collect its native-only counters beside the end-to-end app wall clocks.
    const probe = await promisify(execFile)(path.resolve(repoRoot, '../../engine/target/release/examples/source_media'), [], {
      env: { ...process.env, PM_SOURCE_BENCH_MEDIA: fixturePath('h264-aac.mp4') }, timeout: 60_000
    });
    const counters = probe.stdout.trim().split('\n').map(line => JSON.parse(line));
    await writeFile(path.join(directory, 'native-counters.jsonl'), probe.stdout);
    // Alternating order makes drift visible instead of giving one mode every warm run.
    for (const [round, enabled] of [[0, false], [0, true], [1, true], [1, false]] as const) {
      const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled ? '1' : '0' } });
      try {
        await session.openEditor();
        await importFixture(session.page, 'h264-aac.mp4');
        await session.page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.name === 'h264-aac.mp4'));
        if (enabled) await session.page.waitForFunction(() => !!(window as any).PM.EngineRender?.ready);
        const measured = await session.page.evaluate(async enabled => {
          const PM = (window as any).PM;
          const id = Object.values<any>(PM.proj.assets).find(a => a.name === 'h264-aac.mp4')!.id;
          const viewer = PM.Kernel.services.get('viewer');
          const ticks = 705_600_000;
          const times = (scattered: boolean) => Array.from({ length: 24 }, (_, k) => Math.round(((scattered ? (k * 7) % 24 : k) * 1.9 / 24 + 1 / 60) * ticks) / ticks);
          const painted = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          const wait = (source: any, event: string, time: number) => new Promise<void>((resolve, reject) => {
            const done = () => { if (Math.abs(source.currentTime - time) < .00001) { clearTimeout(timer); source.removeEventListener(event, done); resolve(); } };
            const timer = setTimeout(() => { source.removeEventListener(event, done); reject(new Error(`${event} timed out at ${time}`)); }, 15_000);
            source.addEventListener(event, done); source.currentTime = time;
          });
          viewer.preview.show(id);
          const root = document.querySelector<HTMLElement>('#source-preview')!;
          root.style.width = `${320 / devicePixelRatio}px`; root.style.height = `${180 / devicePixelRatio}px`;
          root.style.right = root.style.bottom = 'auto';
          const monitor = enabled ? PM.EngineSourceMedia.monitorValue : root.querySelector('video');
          const until = performance.now() + 15_000;
          while (enabled ? monitor.canvas.width !== 320 || monitor.canvas.height !== 180 : monitor.readyState < 2) {
            if (performance.now() > until) throw new Error('Source monitor did not reach its measured size');
            await painted();
          }
          await painted();
          const scrubMs: number[] = [];
          for (const time of times(true)) {
            const began = performance.now();
            await wait(monitor, enabled ? 'loadeddata' : 'seeked', time);
            await painted(); // Both paths include the next presentation opportunity.
            scrubMs.push(performance.now() - began);
          }
          viewer.preview.clear();
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
          return { scrubMs, filmstripMs, width: 320, height: 180 };
        }, enabled);
        results.push({ enabled, round, ...measured });
        expect(session.diagnostics.pageErrors).toEqual([]);
      } finally { await session.close(); }
      await writeFile(path.join(directory, 'app-source-performance.json'), JSON.stringify(results, null, 2));
    }
    const percentile = (values: number[], p: number) => [...values].sort((a, b) => a - b)[Math.round((values.length - 1) * p)]!;
    const values = (enabled: boolean, key: 'scrubMs' | 'filmstripMs') => results.filter(r => r.enabled === enabled).flatMap(r => r[key]);
    const comparison = {
      monitor: { legacyP50: percentile(values(false, 'scrubMs'), .5), nativeP50: percentile(values(true, 'scrubMs'), .5),
        legacyP95: percentile(values(false, 'scrubMs'), .95), nativeP95: percentile(values(true, 'scrubMs'), .95) },
      filmstrip: { legacyMedian: percentile(values(false, 'filmstripMs'), .5), nativeMedian: percentile(values(true, 'filmstripMs'), .5),
        legacyFirst: results.find(r => !r.enabled)!.filmstripMs[0], nativeFirst: results.find(r => r.enabled)!.filmstripMs[0] }
    };
    await writeFile(path.join(directory, 'comparison.json'), JSON.stringify({ comparison, counters }, null, 2));
    expect(comparison.monitor.nativeP50).toBeLessThanOrEqual(comparison.monitor.legacyP50);
    expect(comparison.monitor.nativeP95).toBeLessThanOrEqual(comparison.monitor.legacyP95);
    expect(comparison.filmstrip.nativeMedian).toBeLessThanOrEqual(comparison.filmstrip.legacyMedian);
    expect(comparison.filmstrip.nativeFirst).toBeLessThanOrEqual(comparison.filmstrip.legacyFirst!);
  } finally {
    try { await writeFile(path.join(directory, 'machine.json'), JSON.stringify({ ...machine, endedAt: new Date().toISOString(), loadAfter: os.loadavg() }, null, 2)); }
    finally { if (!evidence) await rm(directory, { recursive: true, force: true }); }
  }
});
