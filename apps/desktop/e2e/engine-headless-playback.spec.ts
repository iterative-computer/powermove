/* The native session owns time/audio even when the viewer panel is closed. */
import { expect, test } from './helpers/app';
import { fixturePath } from './helpers/media';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('plays, shuttles, seeks and loops without a viewer; opening joins the running device clock', async ({ session }) => {
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
    PM.loop = false; PM.setTime(0.1); PM.play();
  });
  try {
    await expect.poll(() => page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(0.4);
  } catch (error) {
    console.log('headless start diagnostics', JSON.stringify(await page.evaluate(() => {
      const PM = (window as any).PM, s = PM.EngineConnection.session();
      return { time: PM.time, playing: PM.playing, status: PM.EngineConnection.status(), headless: s?.headless, pending: PM.EngineConnection.transport.pending, proof: (window as any).__headless };
    })));
    console.log('headless native diagnostics', JSON.stringify(await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'))));
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
  const join = await page.evaluate(async () => {
    const PM = (window as any).PM, s = PM.EngineConnection.session();
    const before = { id: s.id, epoch: s.clock.epoch, time: PM.time, now: performance.now() };
    await PM.Kernel.api('headless-proof').extensions.setEnabled('viewer', true);
    const samples: Array<{ time: number; now: number }> = [];
    for (let n = 0; n < 30; n++) { await new Promise(r => setTimeout(r, 10)); samples.push({ time: PM.time, now: performance.now() }); }
    return { before, samples, id: s.id, epoch: s.clock.epoch, playing: PM.playing };
  });
  expect(join.id).toBe(hidden.id);
  expect(join.epoch).toBe(join.before.epoch);
  expect(join.playing).toBe(true);
  let previous = join.before, maxJoinError = 0;
  for (const sample of join.samples) {
    const error = Math.abs(sample.time - previous.time - (sample.now - previous.now) / 1000);
    maxJoinError = Math.max(maxJoinError, error);
    expect(error).toBeLessThan(0.08);
    previous = { ...previous, ...sample };
  }
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
  await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); (window as any).__headless.off(); });
  const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
  expect(report.presenter.hostClockRuns).toBe(0);
  const audio: any = Object.values(report.presenter.audio)[0];
  expect(audio.deviceCallbacks).toBeGreaterThan(20);
  expect(audio.problems).toEqual([]);
  expect(session.diagnostics.pageErrors).toEqual([]);
  console.log(`headless playback: ${JSON.stringify({ initial: hidden, maxJoinError, closedAgain, deviceCallbacks: audio.deviceCallbacks, hostClockRuns: report.presenter.hostClockRuns })}`);
});
