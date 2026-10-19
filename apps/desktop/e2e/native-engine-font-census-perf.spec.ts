import { appendFile } from 'node:fs/promises';
import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

// Opt-in benchmark: the same 1000-layer timeline and playback in today's
// app and with the native engine, recording the census's own work too.
test('font census cost on a large timeline during playback', async () => {
  test.skip(process.env.PM_FONT_CENSUS_MEASURE !== '1', 'opt-in playback measurement');
  test.setTimeout(300_000);
  const rows = [];
  for (const enabled of ['0', '1']) {
    const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: enabled } });
    try {
      await session.openEditor();
      await session.page.evaluate(() => {
        const PM = (window as any).PM;
        const p = PM.mkProject({ name: 'Large font census timeline', w: 1280, h: 720, dur: 60, fps: 30 });
        p.layers = Array.from({ length: 1000 }, (_, i) => PM.mkLayer('solid', {
          name: `Timeline layer ${i}`, d: { color: '#446688', w: 1, h: 1 },
          p: { 'position.x': i % 1280, 'position.y': i % 720 },
        }, p));
        PM.replaceProject(p); PM.time = 0; PM.invalidate();
      });
      await session.page.waitForTimeout(2000);
      const row = await session.page.evaluate(async () => {
        const PM = (window as any).PM;
        const before = PM.EngineDoc?.stats().fontCensus;
        const intervals: number[] = [];
        let previous = performance.now();
        const started = previous;
        PM.play();
        await new Promise<void>(resolve => {
          const tick = (now: number) => {
            intervals.push(now - previous); previous = now;
            if (now - started >= 10_000) resolve(); else requestAnimationFrame(tick);
          };
          requestAnimationFrame(tick);
        });
        PM.pause();
        const after = PM.EngineDoc?.stats().fontCensus;
        intervals.sort((a,b) => a-b);
        const nominal = intervals[Math.floor(intervals.length / 2)]!;
        return { frames: intervals.length, p50: nominal, p95: intervals[Math.ceil(intervals.length * 0.95)-1], longFrames: intervals.filter(v => v > nominal*1.5).length, before, after, engine: PM.EngineConnection?.status() };
      });
      if (enabled === '1') {
        expect(row.after).not.toBeNull();
        expect(row.engine?.route).toBe('engine');
      }
      rows.push({ enabled, ...row });
    } finally { await session.close(); }
  }
  await appendFile(process.env.PM_FONT_CENSUS_OUT ?? '/tmp/powermove-font-census.jsonl', JSON.stringify(rows) + '\n');
});
