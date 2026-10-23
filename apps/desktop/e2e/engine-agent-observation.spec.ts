import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { loadavg } from 'node:os';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { launchApp } from './helpers/app';

/** Runs hidden under pm-electron-lock; the user's desktop is never captured. */
test('agent observations batch five 720 px frames during UHD project playback without pausing', async () => {
  test.setTimeout(180_000);
  const addon = process.env['POWERMOVE_ENGINE_ADDON'] ?? path.resolve(__dirname, '../native/pm_node.node');
  const addonSha256 = createHash('sha256').update(readFileSync(addon)).digest('hex');
  if (process.env['POWERMOVE_SHIPPING_ADDON_SHA256']) expect(addonSha256).toBe(process.env['POWERMOVE_SHIPPING_ADDON_SHA256']);
  const loadStart = loadavg()[0]!; let loadPeak = loadStart;
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  const sampling = setInterval(() => { loadPeak = Math.max(loadPeak, loadavg()[0]!); }, 1000); sampling.unref();
  try {
    await session.openEditor();
    // The job utility starts on demand: the warm-up capture below starts it.
    await expect.poll(() => session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine?.engine?.status().interactive.state), { timeout: 30_000 }).toBe('ready');
    const result = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Agent observation', w: 3840, h: 2160, fps: 30, dur: 60 });
      PM.bus.emit('project'); PM.touch();
      PM.Edit.apply([{ type: 'add_layer', layerType: 'solid', name: 'Red', content: { color: '#e83a5e', w: 1200, h: 800 }, duration: 60 }], { label: 'Setup', origin: 'user' });
      await PM.EngineDoc.whenReady;
      const times = [0, 0.2, 0.4, 0.6, 0.8];
      await PM.AgentHarness.capture(times, 720); // warm the job lane
      const oldPause = PM.pause, oldSnapshot = PM.Export.snapshot, oldAsync = PM.Export.snapshotAsync;
      let pauses = 0;
      PM.pause = (...args: any[]) => { pauses++; return oldPause(...args); };
      PM.Export.snapshot = PM.Export.snapshotAsync = () => { throw new Error('legacy capture called'); };
      const ms: number[] = [];
      try {
        PM.play();
        const start = PM.time;
        for (let trial = 0; trial < 20; trial++) {
          const before = performance.now();
          const frames = await PM.AgentHarness.capture(times, 720);
          if (frames.images.length !== 5 || frames.images.some((frame: string) => !frame.startsWith('data:image/jpeg;base64,'))) throw new Error('Incomplete engine observation');
          if (!PM.playing) throw new Error('Playback stopped during an agent capture');
          ms.push(performance.now() - before);
        }
        const visual = await PM.AgentHarness.observeVisual(times);
        if (visual.visualError || visual.images.length !== 5) throw new Error(visual.visualError || 'observeVisual lost frames');
        return { ms, pauses, playing: PM.playing, elapsedTimeline: PM.time - start };
      } finally { PM.pause = oldPause; PM.Export.snapshot = oldSnapshot; PM.Export.snapshotAsync = oldAsync; PM.pause(); }
    });
    const sorted = result.ms.toSorted((a, b) => a - b);
    const p95 = sorted[Math.ceil(sorted.length * .95) - 1]!;
    console.log(JSON.stringify({ agentObservation: { addonSha256, loadStart, loadPeak, trials: sorted.length, p50Ms: sorted[9], p95Ms: p95, maxMs: sorted.at(-1), pauses: result.pauses, elapsedTimeline: result.elapsedTimeline } }));
    expect(result.pauses).toBe(0);
    expect(result.playing).toBe(true);
    expect(result.elapsedTimeline).toBeGreaterThan(0);
    expect(p95).toBeLessThanOrEqual(10_000);
  } finally { clearInterval(sampling); await session.close(); }
});
