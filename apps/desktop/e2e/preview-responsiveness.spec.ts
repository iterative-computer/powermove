import { writeFileSync } from 'node:fs';
import { expect, test } from './helpers/app';

test('profiles playback and bidirectional scrubbing with a populated timeline and inspector', async ({ session }, info) => {
  test.setTimeout(90_000);
  await session.openEditor();
  await session.page.setViewportSize({ width: 1440, height: 1000 });
  await session.page.evaluate(() => {
    const PM = (window as any).PM;
    PM.pause();
    const project = PM.mkProject({ name: 'Preview responsiveness', w: 1280, h: 720, dur: 8, fps: 30, bg: '#182439' });
    for (let i = 0; i < 1000; i++) {
      const layer = PM.mkLayer('shape', {
        d: { shape: i % 2 ? 'ellipse' : 'rect', w: 53, h: 43, radius: 7, color: i % 2 ? '#ee7733' : '#44aadd' },
        p: { 'position.x': 25 + i % 24 * 51, 'position.y': 25 + Math.floor(i / 24) * 66, opacity: 65 },
      }, project);
      project.layers.push(layer);
    }
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: project }));
    PM.ProjectsScreen.hide();
    PM.perf.auto = false; PM.quality = 1;
    for (const layer of PM.proj.layers) PM.animate(layer, 'rotation', [{ t: 0, v: 0 }, { t: 8, v: 180 }]);
    PM.selectLayers(PM.proj.layers.slice(0, 12).map((layer: any) => layer.id));
    PM.touch(); PM.bus.emit('layers'); PM.setTime(0, { force: true });
  });
  await session.page.waitForTimeout(500);
  const cdp = await session.page.context().newCDPSession(session.page);
  await cdp.send('Profiler.enable'); await cdp.send('Profiler.start');
  const result = await session.page.evaluate(async () => {
    const PM = (window as any).PM, render = PM.GL.render;
    let phase = 'playback', last = 0, cycleChecks = 0, playbackCycleChecks = 0;
    const wouldCycle = PM.wouldCycle;
    PM.wouldCycle = (...args: any[]) => { cycleChecks++; return wouldCycle(...args); };
    const frames: any[] = [], scrubs: any[] = [], engineCosts: any[] = [];
    const native = Boolean(PM.EngineConnection);
    const offPresented = native ? PM.bus.on('preview:presented', (frame: any) => {
      const now = performance.now();
      frames.push({ phase, time: frame.time, ms: null, gap: last ? now - last : 0, presented: true });
      const stats = PM.EngineStats?.latest;
      if (stats && engineCosts.at(-1)?.frames !== stats.frames) {
        engineCosts.push({ phase, frames: stats.frames, meanMs: stats.frameUsAvg / 1000 });
      }
      last = now;
    }) : null;
    if (!native) PM.GL.render = (...args: any[]) => {
      const start = performance.now();
      const presented = render(...args);
      frames.push({ phase, time: args[0], ms: performance.now() - start, gap: last ? start - last : 0, presented });
      last = start; return presented;
    };
    const serialized = JSON.stringify(PM.proj);
    try {
      PM.play(); await new Promise(resolve => setTimeout(resolve, 2400)); PM.pause();
      playbackCycleChecks = cycleChecks;
      phase = 'scrub'; last = 0;
      for (const time of [0, 7, .1, 6, 2, 5, 0, ...Array.from({ length: 30 }, (_, i) => i * 7 / 30)]) {
        const start = performance.now(); PM.setTime(time);
        const setMs = performance.now() - start;
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        scrubs.push({ time, setMs, elapsed: performance.now() - start });
      }
      await new Promise(resolve => setTimeout(resolve, 200));
    } finally { PM.pause(); offPresented?.(); PM.GL.render = render; PM.wouldCycle = wouldCycle; }
    const rotation = document.querySelector<HTMLInputElement>('[data-channel="rotation"] [role="spinbutton"]');
    return { frames, scrubs, engineCosts, cycleChecks, playbackCycleChecks, frameSource: native ? 'native-presentation' : 'legacy-render',
      inspectorRotation: rotation ? parseFloat(rotation.value) : null,
      expectedRotation: PM.ev(PM.proj.layers[0], 'rotation', PM.time), preserved: serialized === JSON.stringify(PM.proj), quality: PM.quality };
  });
  const profile = (await cdp.send('Profiler.stop')).profile;
  writeFileSync(info.outputPath('preview.cpuprofile'), JSON.stringify(profile));
  writeFileSync(info.outputPath('preview.json'), JSON.stringify(result));
  if (process.env.PM_PREVIEW_METRICS) {
    writeFileSync(process.env.PM_PREVIEW_METRICS, JSON.stringify(result));
    writeFileSync(process.env.PM_PREVIEW_METRICS + '.cpuprofile', JSON.stringify(profile));
  }
  for (const phase of ['playback', 'scrub']) {
    const frames = result.frames.filter(frame => frame.phase === phase);
    const times = frames.map(frame => frame.ms).filter((ms): ms is number => typeof ms === 'number').sort((a, b) => a - b);
    const gaps = frames.slice(1).map(frame => frame.gap).sort((a, b) => a - b);
    console.log('PREVIEW', JSON.stringify({ phase, frameSource: result.frameSource,
      timing: result.frameSource === 'native-presentation' ? 'presentation-cadence' : 'render-call-duration',
      cycleChecks: result.cycleChecks, count: frames.length, median: times[Math.floor(times.length / 2)] ?? null,
      p95: times[Math.floor(times.length * .95)] ?? null, gap95: gaps[Math.floor(gaps.length * .95)] }));
  }
  // Native rolling means are engine stats, separate from per-frame cadence;
  // they cannot be relabeled as a render-duration percentile.
  if (result.frameSource === 'native-presentation') console.log('PREVIEW_ENGINE_COST', JSON.stringify(result.engineCosts));
  expect(result.preserved).toBe(true);
  expect(result.quality).toBe(1);
  if (process.env.PM_PREVIEW_BASELINE !== '1') expect(result.playbackCycleChecks).toBeLessThan(100_000);
  expect(result.inspectorRotation).not.toBeNull();
  expect(result.inspectorRotation).toBe(Math.round(result.expectedRotation));
  expect(result.frames.filter(frame => frame.phase === 'playback').length).toBeGreaterThan(20);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
