/* Paired P5 viewer mount: PM.time and performance.now() on the same page
 * clock, fresh hidden profiles, one Electron at a time. Opt in with
 * PM_MOUNT_MEASURE=1; PM_MOUNT_PAIRS defaults to 5. The enclosing command
 * must run under pm-electron-lock. Evidence stays outside the repository. */
import { appendFile, statfs } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

const out = process.env.PM_MOUNT_OUT ?? '/tmp/powermove-mount-paired.jsonl';
const stats = (xs: number[]) => {
  const sorted = [...xs].sort((a, b) => a - b);
  const q = (p: number) => sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * p) - 1)] ?? null;
  return { samples: sorted.length, p50: q(.5), p95: q(.95), p99: q(.99), max: q(1) };
};

test('P5 mid-play viewer mount paired with today', async () => {
  test.skip(process.env.PM_MOUNT_MEASURE !== '1', 'set PM_MOUNT_MEASURE=1');
  test.setTimeout(1_800_000);
  const pairs = Number(process.env.PM_MOUNT_PAIRS ?? 5);
  expect(Number.isInteger(pairs) && pairs > 0 && pairs <= 30).toBe(true);
  const results: any[] = [];
  for (let pair = 0; pair < pairs; pair++) for (const native of pair % 2 ? [true, false] : [false, true]) {
    const quietUntil = Date.now() + 15 * 60_000;
    while (os.loadavg()[0] > 8 && Date.now() < quietUntil) await new Promise(r => setTimeout(r, 30_000));
    if (os.loadavg()[0] > 8) throw new Error('Load stayed above 8; mount row is unmeasured');
    const disk = await statfs('/System/Volumes/Data');
    if (disk.bavail * disk.bsize < 40 * 1024 ** 3) throw new Error('Disk below 40 GiB; do not launch');
    const load = [os.loadavg()[0]];
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' } });
    const timer = setInterval(() => load.push(os.loadavg()[0]), 250);
    try {
      const display = () => session.app.evaluate(({}, harnessPath) => process.getBuiltinModule('module').createRequire(harnessPath)(harnessPath).displayState(), process.env.POWERMOVE_PM_BRIDGE_HARNESS ?? path.join(os.homedir(), 'Library/Caches/powermove-engine/pm_bridge_harness.node'));
      const before = await display();
      if (before.asleep || before.locked) {
        await appendFile(out, JSON.stringify({ pair, mode: native ? 'native' : 'today', invalid: 'display asleep or session locked', before, load }) + '\n');
        test.skip(true, 'Display asleep or session locked; no performance pass');
      }
      await session.openEditor();
      await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        await PM.Kernel.api('mount-measure').extensions.setEnabled('viewer', false);
      });
      // A viewerless launch prepares its drawing before playback begins.
      await session.relaunch();
      await session.page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen));
      await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        await PM.Kernel.loader.builtinsReady;
        window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Mount probe', w: 640, h: 360, fps: 30, dur: 60 }) }));
        PM.ProjectsScreen.hide();
        PM.loop = false; PM.setTime(2); PM.play();
      });
      await session.page.waitForFunction(() => !(window as any).PM.Kernel.services.get('viewer') && (window as any).PM.time > 2.2);
      const result = await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        const samples: { time: number; now: number; line: number | null }[] = [];
        const sample = () => samples.push({ time: PM.time, now: performance.now(), line: PM.EngineConnection?.transport.now(performance.now())?.time ?? null });
        sample();
        await PM.Kernel.api('mount-measure').extensions.setEnabled('viewer', true);
        for (let n = 0; n < 60; n++) { await new Promise(r => setTimeout(r, 10)); sample(); }
        PM.pause();
        return samples;
      });
      const after = await display();
      const errors = result.slice(1).map((s, i) => Math.abs(s.time - result[i].time - (s.now - result[i].now) / 1000) * 1000);
      const lineErrors = result.slice(1).flatMap((s, i) => s.line === null || result[i].line === null ? [] : [Math.abs(s.line - result[i].line! - (s.now - result[i].now) / 1000) * 1000]);
      const row = { pair, mode: native ? 'native' : 'today', uiMs: stats(errors), lineMs: stats(lineErrors), samples: result, before, after, load, invalid: after.asleep || after.locked || Math.max(...load) > 8 ? 'display or load guard changed' : null };
      results.push(row); await appendFile(out, JSON.stringify(row) + '\n');
      expect(row.invalid).toBeNull();
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { clearInterval(timer); await session.close(); }
  }
  const maxima = (mode: string) => stats(results.filter(r => r.mode === mode).map(r => r.uiMs.max));
  const summary = { kind: 'paired-mount-summary', today: maxima('today'), native: maxima('native') };
  await appendFile(out, JSON.stringify(summary) + '\n');
  console.log(JSON.stringify(summary));
  expect(summary.native.p95).toBeLessThanOrEqual(summary.today.p95!);
});
