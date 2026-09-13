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
    PM.setTime(0);
    PM.perf.auto = true; PM.perf.ms = 100; PM.quality = 1;
    PM.play();
  });
  await page.waitForFunction(() => (window as any).PM.quality < 1);
  const playing = await page.evaluate(() => {
    const PM = (window as any).PM;
    return { playing: PM.playing, previewActive: PM.Preview.active, ...PM.Audio.inspect() };
  });
  expect(playing.playing).toBe(true);
  expect(playing.previewActive).toBe(false);
  expect(playing.running).toBe(true);
  expect(playing.voices.length).toBeGreaterThan(0);
  await page.evaluate(() => (window as any).PM.pause());
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
  expect(session.diagnostics.pageErrors).toEqual([]);
});
