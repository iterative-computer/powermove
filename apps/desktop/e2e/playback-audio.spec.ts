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

test('cached playback stops its own audio and hands back to normal playback', async ({ session }) => {
  const { page } = session;
  await openAudioProject(page);
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
