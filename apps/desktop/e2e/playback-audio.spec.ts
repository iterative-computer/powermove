import { expect, test } from './helpers/app';
import { importFixture } from './helpers/media';

async function openAudioProject(page: any) {
  await page.waitForFunction(() => Boolean((window as any).PM.GL.gl));
  await page.evaluate(() => {
    const PM = (window as any).PM;
    window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Audio playback', w: 160, h: 90, dur: 1, fps: 30 }) }));
    PM.ProjectsScreen.hide();
  });
  await importFixture(page, 'tone.wav');
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'audio'));
}

test('adaptive preview quality keeps the soundtrack playing', async ({ session }) => {
  const { page } = session;
  await openAudioProject(page);
  if (await page.evaluate(() => Boolean((window as any).PM.EngineConnection))) {
    test.setTimeout(120_000);
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    // The engine draws, so the compositor cannot be slowed from the page.
    // Apply real render pressure instead: add blurred full-frame layers until
    // Full quality cannot present the 30 fps soundtrack project in real time.
    const budget = await page.evaluate(() => {
      const PM = (window as any).PM;
      const budget = PM.Memory.budget('preview');
      // Remembered frames would correctly be excluded as evidence.
      PM.Memory.setBudget('preview', 0);
      PM.loop = true; PM.proj.work = [0, PM.proj.dur];
      PM.perf.auto = false; PM.quality = 1; PM.bus.emit('quality');
      return budget;
    });
    const presentedRate = () => page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.setTime(0); PM.play();
      await new Promise(resolve => setTimeout(resolve, 500));
      let frames = 0;
      const off = PM.bus.on('preview:presented', () => { if (!PM.EngineConnection.status().presentedCached) frames++; });
      await new Promise(resolve => setTimeout(resolve, 1500));
      off?.(); PM.pause();
      return frames / 1.5;
    });
    try {
      let layers = 0, rate = await presentedRate();
      while (rate >= 24 && layers < 512) {
        await page.evaluate(() => {
          const PM = (window as any).PM;
          for (let i = 0; i < 16; i++) {
            const layer = PM.mkLayer('solid', { name: 'Pressure', d: { w: PM.proj.w, h: PM.proj.h, color: i % 2 ? '#3366ff' : '#ff6633' } }, PM.proj);
            const blur = PM.mkEffect('blur'); blur.p.amount.v = 120; layer.fx.push(blur);
            PM.proj.layers.push(layer);
          }
          PM.ProjectIndex?.invalidate?.(); PM.touch(); PM.bus.emit('layers');
        });
        layers += 16;
        rate = await presentedRate();
      }
      expect(rate, `${layers} blurred layers`).toBeLessThan(24);
      await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.perf.auto = true; PM.quality = 1; PM.play(); });
      await page.waitForFunction(() => (window as any).PM.quality < 1, undefined, { timeout: 20_000 });
      const playing = await page.evaluate(() => ({ playing: (window as any).PM.playing, previewActive: (window as any).PM.Preview.active }));
      expect(playing.playing).toBe(true);
      expect(playing.previewActive).toBe(false);
      // The soundtrack keeps sounding after the step down.
      const notBefore = await page.evaluate(() => performance.now() + 350);
      await nativeSounding(page, 0, notBefore);
    } finally {
      await page.evaluate((budget) => { const PM = (window as any).PM; PM.pause(); PM.Memory.setBudget('preview', budget); }, budget);
    }
    expect(session.diagnostics.pageErrors).toEqual([]);
    return;
  }
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const render = PM.GL.render;
    const budget = PM.Memory.budget('preview');
    const pressure = { frames: 0, renderMs: 0, restore() {
      PM.GL.render = render;
      PM.Memory.setBudget('preview', budget);
    } };
    (window as any).__audioQualityPressure = pressure;
    // Exercise the real timing/adaptation path. Prepared frame reuse would
    // correctly exclude these samples, and play() resets prior measurements.
    PM.Memory.setBudget('preview', 0);
    PM.GL.render = function (this: any, ...args: any[]) {
      const started = performance.now();
      const result = render.apply(this, args);
      if (PM.playing && PM.quality === 1) {
        while (performance.now() - started < 45) { /* controlled render pressure */ }
        pressure.frames++;
        pressure.renderMs += performance.now() - started;
      }
      return result;
    };
    PM.setTime(0);
    PM.perf.auto = true; PM.quality = 1;
    PM.play();
  });
  try {
    await page.waitForFunction(() => (window as any).PM.quality < 1);
    const playing = await page.evaluate(() => {
      const PM = (window as any).PM;
      const pressure = (window as any).__audioQualityPressure;
      return {
        playing: PM.playing, previewActive: PM.Preview.active,
        pressureFrames: pressure.frames, averageRenderMs: pressure.renderMs / pressure.frames,
        ...PM.Audio.inspect(),
      };
    });
    expect(playing.pressureFrames).toBeGreaterThan(5);
    expect(playing.averageRenderMs).toBeGreaterThanOrEqual(45);
    expect(playing.playing).toBe(true);
    expect(playing.previewActive).toBe(false);
    expect(playing.running).toBe(true);
    expect(playing.voices.length).toBeGreaterThan(0);
  } finally {
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.pause();
      (window as any).__audioQualityPressure.restore();
      delete (window as any).__audioQualityPressure;
    });
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});

/** Native audio is mixed by the engine: it sounds when fresh meter levels
 * with a non-zero peak arrive while the engine drives the transport. */
async function nativeSounding(page: any, after: number, notBefore = 0): Promise<number> {
  const reading = await page.waitForFunction(({ after, notBefore }: { after: number; notBefore: number }) => {
    const PM = (window as any).PM;
    const audio = PM.EngineConnection.audio;
    const latest = audio.latest();
    if (performance.now() < notBefore || audio.levelsReceived <= after || !latest || PM.EngineConnection.status().driving !== true || !PM.Audio.meters.running()
      || !latest.levels.master.peak.some((peak: number) => peak > 0)) return false;
    return { sequence: audio.levelsReceived, voices: PM.Audio.inspect().voices.length };
  }, { after, notBefore });
  const result = await reading.jsonValue();
  if (!result) throw new Error('Native audio wait resolved without a meter reading');
  // The page's own Web Audio voices stay silent: the engine plays the mix.
  expect(result.voices).toBe(0);
  return result.sequence;
}

test('cached playback stops its own audio and hands back to normal playback', async ({ session }) => {
  const { page } = session;
  await openAudioProject(page);
  if (await page.evaluate(() => Boolean((window as any).PM.EngineConnection))) {
    await expect.poll(() => page.evaluate(() => (window as any).PM.EngineConnection.status().route), { timeout: 45_000 }).toBe('engine');
    await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.proj.work = [0, 1];
      await PM.Preview.cache();
    });
    const cached = await page.evaluate(() => ({ active: (window as any).PM.Preview.active, frames: (window as any).PM.Preview.count }));
    expect(cached.active).toBe(true);
    expect(cached.frames).toBe(30);
    // The cached preview presents remembered frames with its soundtrack.
    await page.waitForFunction(() => (window as any).PM.EngineConnection.status().presentedCached);
    const cachedAudio = await nativeSounding(page, 0);
    await page.evaluate(() => (window as any).PM.Preview.stop());
    expect(await page.evaluate(() => (window as any).PM.Preview.active)).toBe(false);
    await page.waitForFunction(() => !(window as any).PM.Audio.meters.running());
    const notBefore = await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.setTime(0); PM.play(); PM.bus.emit('quality');
      return performance.now() + 350;
    });
    await nativeSounding(page, cachedAudio, notBefore);
    await page.evaluate(() => (window as any).PM.pause());
    expect(session.diagnostics.pageErrors).toEqual([]);
    return;
  }
  await page.evaluate(async () => {
    const PM = (window as any).PM;
    PM.proj.work = [0, 1];
    await PM.Preview.cache();
  });
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const cached = { active: PM.Preview.active, frames: PM.Preview.count, audio: PM.Audio.inspect() };
    PM.Preview.stop();
    const stopped = PM.Audio.inspect();
    PM.setTime(0); PM.play(); PM.bus.emit('quality');
    const resumed = PM.Audio.inspect();
    PM.pause();
    return { cached, stopped, resumed };
  });
  expect(result.cached.active).toBe(true);
  expect(result.cached.frames).toBe(30);
  expect(result.cached.audio.running).toBe(true);
  expect(result.stopped.running).toBe(false);
  expect(result.stopped.voices).toHaveLength(0);
  expect(result.resumed.running).toBe(true);
  expect(result.resumed.voices.length).toBeGreaterThan(0);
  expect(session.diagnostics.pageErrors).toEqual([]);
});

test('the restored project keeps playing audio through quality changes', async ({ session }) => {
  await openAudioProject(session.page);
  const id = await session.page.evaluate(async () => {
    const PM = (window as any).PM;
    await PM.flushProject();
    return PM.proj.id;
  });
  await session.relaunch();
  await session.page.waitForFunction(id => (window as any).PM.proj.id === id && [...(window as any).PM.assets.map.values()].some((a: any) => a.audioBlob?.size), id);
  await session.page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0); PM.play(); });
  const native = await session.page.evaluate(() => Boolean((window as any).PM.EngineConnection));
  if (native) {
    const sounding = async (after: number, notBefore = 0) => {
      const reading = await session.page.waitForFunction(({ after, notBefore }) => {
        const PM = (window as any).PM;
        const audio = PM.EngineConnection.audio;
        const latest = audio.latest();
        if (performance.now() < notBefore || audio.levelsReceived <= after || !latest || PM.EngineConnection.status().driving !== true || !PM.Audio.meters.running()
          || !latest.levels.master.peak.some((peak: number) => peak > 0)) return false;
        return { sequence: audio.levelsReceived, voices: PM.Audio.inspect().voices.length,
          datasetVoices: Number(document.documentElement.dataset.audioVoices || 0) };
      }, { after, notBefore });
      const result = await reading.jsonValue();
      if (!result) throw new Error('Native audio wait resolved without a meter reading');
      expect(result.voices).toBe(0);
      expect(result.datasetVoices).toBe(0);
      return result.sequence;
    };
    const before = await sounding(0);
    const notBefore = await session.page.evaluate(() => {
      (window as any).PM.bus.emit('quality');
      // Outlast the meter's 250 ms stale interval before accepting fresh levels.
      return performance.now() + 350;
    });
    let sequence = await sounding(before, notBefore);
    for (let index = 0; index < 2; index++) sequence = await sounding(sequence);
    await session.page.evaluate(() => (window as any).PM.pause());
    await session.page.waitForFunction(() => !(window as any).PM.Audio.meters.running());
  } else {
    await session.page.waitForFunction(() => (window as any).PM.Audio.inspect().voices.length > 0);
    const result = await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.bus.emit('quality');
      const result = PM.Audio.inspect();
      PM.pause();
      return result;
    });
    expect(result.running).toBe(true);
    expect(result.voices.length).toBeGreaterThan(0);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
