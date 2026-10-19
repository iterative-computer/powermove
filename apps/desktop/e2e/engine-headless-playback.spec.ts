/*
 * The native session owns time/audio even when the viewer panel is closed
 * (orchestrator decision A: Play works as today, through an engine session
 * with no view, never routed to the current engine), follows the open
 * composition, and a viewer that opens mid-play joins the running device
 * clock. The current engine's own join is measured with the same probe.
 *
 *   pm-electron-lock bunx playwright test e2e/engine-headless-playback.spec.ts --workers=1
 */
import os from 'node:os';
import path from 'node:path';

import type { Page } from 'playwright';

import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';

/** Enables the viewer mid-play and samples the playhead every 10 ms: how far
 *  `PM.time` strays from wall time (the page's main thread mounting the
 *  viewer), and, with the engine, how far the engine's line strays. */
async function joinViewer(page: Page): Promise<{ before: { id: number | null; epoch: number | null }; id: number | null; epoch: number | null; playing: boolean; uiError: number; lineError: number; catchUp: number | null }> {
  return page.evaluate(async () => {
    const PM = (window as any).PM, connection = PM.EngineConnection, s = connection?.session?.() ?? null;
    const line = () => connection?.transport.now(performance.now())?.time ?? null;
    const before = { id: s?.id ?? null, epoch: s?.clock?.epoch ?? null, time: PM.time as number, line: line(), now: performance.now() };
    await PM.Kernel.api('headless-proof').extensions.setEnabled('viewer', true);
    const samples: Array<{ time: number; line: number | null; now: number }> = [];
    for (let n = 0; n < 30; n++) { await new Promise(r => setTimeout(r, 10)); samples.push({ time: PM.time, line: line(), now: performance.now() }); }
    let uiError = 0, lineError = 0;
    let previous: { time: number; line: number | null; now: number } = before;
    for (const sample of samples) {
      const wall = (sample.now - previous.now) / 1000;
      uiError = Math.max(uiError, Math.abs(sample.time - previous.time - wall));
      if (sample.line !== null && previous.line !== null) lineError = Math.max(lineError, Math.abs(sample.line - previous.line - wall));
      previous = sample;
    }
    // After the mount, the playhead is back on the line within two frames.
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    const at = line();
    return { before: { id: before.id, epoch: before.epoch }, id: s?.id ?? null, epoch: s?.clock?.epoch ?? null, playing: PM.playing as boolean, uiError, lineError, catchUp: at === null ? null : Math.abs(PM.time - at) };
  });
}

test.describe('Use New Engine', () => {
test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('plays, shuttles, seeks and loops without a viewer; follows the open composition; opening joins the running device clock', async ({ session }) => {
  test.setTimeout(120_000);
  let { page } = session;
  await session.openEditor();
  await page.waitForFunction(() => Boolean((window as any).PM.EngineConnection));
  // The stock workspace keeps its viewer slot. Unload the contribution to
  // exercise a real viewerless launch, as customized workspaces can have.
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Kernel.api('headless-proof').extensions.setEnabled('viewer', false);
  });
  await page.waitForFunction(() => !(window as any).PM.Kernel.services.get('viewer'));
  await session.relaunch();
  page = session.page;
  // openEditor() intentionally requires a viewer; this case has none.
  await page.waitForFunction(() => Boolean((window as any).PM?.ProjectsScreen));
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.Kernel.loader.builtinsReady;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'No viewer' }) }));
    PM.ProjectsScreen.hide();
  });
  await page.waitForFunction(() => Boolean((window as any).PM.EngineConnection) && !(window as any).PM.Kernel.services.get('viewer'));
  // The premise: the relaunch kept the viewer contribution off (a viewer
  // that mounts later would attach presentation and void the test).
  expect(await page.evaluate(() => (window as any).PM.Kernel.api('headless-proof').extensions.list().find((e: any) => e.id === 'viewer')?.enabled)).toBe(false);
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ name: 'Headless playback', w: 320, h: 180, fps: 30, dur: 6 }));
    const input = document.createElement('input'); input.id = 'headless-audio'; input.type = 'file'; input.hidden = true;
    document.body.append(input);
  });
  await page.locator('#headless-audio').setInputFiles(fixturePath('tone.wav'));
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    const input = document.getElementById('headless-audio') as HTMLInputElement;
    const asset = await PM.assets.add(input.files![0], { newAsset: true, silent: true });
    // Keep tone and transport running beyond viewer module/GPU startup.
    for (let n = 0; n < 4; n++) PM.proj.layers.push(PM.mkLayer('audio', { from: n * 1.5, dur: 1.5, d: { asset: asset.id, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 } }, PM.proj));
    PM.proj.work = [0.2, 1.2]; PM.touch();
    PM.bus.emit('layers'); PM.bus.emit('project'); PM.bus.emit('assets');
    const proof: any = (window as any).__headless = { levels: 0, peak: 0 };
    proof.off = PM.EngineConnection.audio.onLevels((l: any) => {
      proof.levels++; proof.peak = Math.max(proof.peak, ...l.master.peak);
    });
    if (PM.Kernel.services.get('viewer') || !PM.EngineConnection.session()?.headless) throw new Error('a viewer is attached: the premise of this test does not hold');
    PM.loop = false; PM.setTime(0.1); PM.play();
  });
  try {
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0.4);
  } catch (error) {
    console.log('headless start diagnostics', JSON.stringify(await page.evaluate(() => {
      const PM = (window as any).PM, s = PM.EngineConnection.session();
      return { time: PM.time, playing: PM.playing, status: PM.EngineConnection.status(), headless: s?.headless, viewer: Boolean(PM.Kernel.services.get('viewer')), pending: PM.EngineConnection.transport.pending, proof: (window as any).__headless };
    })));
    console.log('headless native diagnostics', JSON.stringify(await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))));
    const pid = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid);
    // PM_SAMPLE_ON_FAIL=1: a 3 s stack sample of the interactive engine.
    if (process.env.PM_SAMPLE_ON_FAIL && pid) {
      const { execFileSync } = await import('node:child_process');
      const file = path.join(os.tmpdir(), `powermove-engine-sample-${Date.now()}.txt`);
      execFileSync('/usr/bin/sample', [String(pid), '3', '-file', file]);
      console.log(`engine stack sample: ${file}`);
    }
    throw error;
  }
  await expect.poll(() => page.evaluate(() => (window as any).__headless.peak)).toBeGreaterThan(0.05);
  const hidden = await page.evaluate(() => {
    const PM = (window as any).PM, s = PM.EngineConnection.session();
    return { id: s.id, frames: s.frames, headless: s.headless, levels: (window as any).__headless.levels };
  });
  expect(hidden).toMatchObject({ frames: 0, headless: true });
  expect(hidden.levels).toBeGreaterThan(0);
  // Reverse, seek and the work-area loop work without presentation.
  await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.setTime(1); PM.shuttle(-1); });
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().clock?.rate.num)).toBe(-1);
  await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeLessThan(0.8);
  await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.loop = true; PM.setTime(1.05); PM.play(); });
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM; return PM.playing && PM.time >= 0.2 && PM.time < 0.8;
  })).toBe(true);
  // Put the join inside one non-looping run: loop boundaries legitimately
  // change the audio epoch and must not be confused with a viewer restart.
  await page.evaluate(() => {
    const PM = (window as any).PM; PM.pause(); PM.loop = false;
    PM.proj.work = [0, 6]; PM.touch(); PM.bus.emit('project');
    PM.setTime(0.2); PM.play();
  });
  await expect.poll(() => page.evaluate(() => {
    const PM = (window as any).PM, c = PM.EngineConnection.session().clock;
    return c?.state === 'playing' && c.loopRange === null && PM.time > 0.3 && PM.time < 0.7;
  })).toBe(true);
  const join = await joinViewer(page);
  console.log(`headless join: ${JSON.stringify(join)}`);
  expect(join.id).toBe(hidden.id);
  // The same device run: the viewer joins it, nothing restarts or seeks.
  expect(join.epoch).toBe(join.before.epoch);
  expect(join.playing).toBe(true);
  // The engine's line runs on through the mount (half a frame at 30 fps)
  // and the playhead is back on it within two animation frames. `uiError`
  // is the page's main thread mounting the viewer, measured against the
  // current engine's own join below.
  expect(join.lineError).toBeLessThan(1 / 60);
  expect(join.catchUp).not.toBeNull();
  expect(join.catchUp!).toBeLessThan(1 / 30);
  const maxJoinError = join.uiError;
  await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.session().frames)).toBeGreaterThan(0);
  // Closing again retires presentation, not the running session or device.
  const closedAgain = await page.evaluate(async () => {
    const PM = (window as any).PM, s = PM.EngineConnection.session();
    await PM.Kernel.api('headless-proof').extensions.setEnabled('viewer', false);
    await new Promise(r => setTimeout(r, 100)); // release in-flight descriptors
    const before = { frames: s.frames, time: PM.time, levels: (window as any).__headless.levels, epoch: s.clock.epoch };
    await new Promise(r => setTimeout(r, 300));
    return { before, id: s.id, headless: s.headless, frames: s.frames, time: PM.time,
      levels: (window as any).__headless.levels, peak: (window as any).__headless.peak, epoch: s.clock.epoch, playing: PM.playing };
  });
  console.log(`headless close: ${JSON.stringify(closedAgain)}`);
  expect(closedAgain.id).toBe(hidden.id);
  expect(closedAgain.headless).toBe(true);
  expect(closedAgain.playing).toBe(true);
  expect(closedAgain.epoch).toBe(closedAgain.before.epoch);
  expect(closedAgain.frames).toBe(closedAgain.before.frames);
  expect(closedAgain.levels).toBeGreaterThan(closedAgain.before.levels);
  expect(closedAgain.time - closedAgain.before.time).toBeGreaterThan(0.2);
  await page.evaluate(() => (window as any).PM.pause());
  // Another composition of the same project (a timeline tab: a document
  // batch, not a load). Play plays it, not the one before: it is silent, so
  // the engine starts it on the host clock at once and hears nothing.
  const silentBefore = (await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))).presenter.silentRuns;
  const comps = await page.evaluate(() => {
    const PM = (window as any).PM, first = PM.proj.compId as string;
    const second = PM.Comps.create({ name: 'Silent', w: 320, h: 180, fps: 30, dur: 3 }, { open: true }) as string;
    return { first, second, open: PM.proj.compId as string };
  });
  expect(comps.open).toBe(comps.second);
  const quiet = await page.evaluate(async () => {
    const PM = (window as any).PM, proof = (window as any).__headless;
    await new Promise(r => setTimeout(r, 200)); // the stop's silent level message
    proof.peak = 0;
    PM.loop = false; PM.setTime(0.1); PM.play();
    await new Promise(r => setTimeout(r, 900));
    const s = PM.EngineConnection.session();
    const out = { time: PM.time, playing: PM.playing, peak: proof.peak, composition: s.composition, headless: s.headless, clock: s.clock?.state };
    PM.pause();
    return out;
  });
  console.log(`headless second composition: ${JSON.stringify(quiet)}`);
  expect(quiet).toMatchObject({ composition: comps.second, headless: true, playing: true, clock: 'playing' });
  expect(quiet.time).toBeGreaterThan(0.6);
  expect(quiet.peak).toBeLessThan(0.01);
  const silentAfter = (await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))).presenter.silentRuns;
  expect(silentAfter).toBe(silentBefore + 1);
  // And back: the first composition's tone plays again.
  await page.evaluate((first) => {
    const PM = (window as any).PM;
    PM.Comps.open(first);
    (window as any).__headless.peak = 0;
    PM.loop = false; PM.setTime(0.1); PM.play();
  }, comps.first);
  await expect.poll(() => page.evaluate(() => (window as any).__headless.peak)).toBeGreaterThan(0.05);
  expect(await page.evaluate(() => (window as any).PM.EngineConnection.session().composition)).toBe(comps.first);
  await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); (window as any).__headless.off(); });
  const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
  expect(report.presenter.hostClockRuns).toBe(0);
  const audio: any = Object.values(report.presenter.audio)[0];
  expect(audio.deviceCallbacks).toBeGreaterThan(20);
  expect(audio.problems).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
  console.log(`headless playback: ${JSON.stringify({ initial: hidden, maxJoinError, closedAgain, deviceCallbacks: audio.deviceCallbacks, hostClockRuns: report.presenter.hostClockRuns, stalls: report.presenter.stalls, stallMs: report.presenter.stallUsMax / 1000, lastStall: report.presenter.lastStall })}`);
});
});

test.describe('Use New Engine off (the current engine)', () => {
  test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '0' } } });

  test('the current engine\'s join: the same probe while the viewer opens mid-play', async ({ session }) => {
    test.setTimeout(90_000);
    const { page } = session;
    await session.openEditor();
    await page.evaluate(async () => { await (window as any).PM.Kernel.api('headless-proof').extensions.setEnabled('viewer', false); });
    await page.waitForFunction(() => !(window as any).PM.Kernel.services.get('viewer'));
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Join baseline', w: 320, h: 180, fps: 30, dur: 6 }));
      PM.loop = false; PM.setTime(0.2); PM.play();
    });
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0.4);
    const join = await joinViewer(page);
    console.log(`current engine join: ${JSON.stringify(join)}`);
    expect(join.playing).toBe(true);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
