import { execFile } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';

import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

const fixtures = [
  { name: 'short-audio', video: 3, audio: .75, delay: 0, stored: .5 },
  { name: 'long-audio', video: .8, audio: 3, delay: 0, stored: 9 },
  { name: 'delayed-pcm', video: 3, audio: 1, delay: 1, stored: 9 },
];

for (const fixture of fixtures) test(`original ${fixture.name} controls paused timing, playback and end`, async ({ session }) => {
  test.setTimeout(90_000);
  const directory = await mkdtemp(path.join(tmpdir(), 'pm-source-timing-'));
  const name = `${fixture.name}.mov`, file = path.join(directory, name);
  try {
    // Separate input lengths deliberately preserve unequal track endpoints.
    // PCM has no encoder delay, so the MOV timestamp is the authored offset.
    await promisify(execFile)(ffmpegBinary, [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-f', 'lavfi', '-i', `color=c=red:s=64x64:r=10:d=${fixture.video}`,
      '-itsoffset', String(fixture.delay), '-f', 'lavfi', '-i',
      `aevalsrc=0.125|-0.125:s=48000:d=${fixture.audio}`,
      '-map', '0:v:0', '-map', '1:a:0',
      '-c:v', 'h264_videotoolbox', '-pix_fmt', 'yuv420p', '-c:a', 'pcm_s16le', file,
    ]);
    await session.openEditor();
    const { page } = session;
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => {
      const input = document.createElement('input'); input.type = 'file';
      input.onchange = async () => {
        try { await (window as any).PM.importFiles([...input.files!], { placement: null }); }
        finally { input.remove(); }
      };
      document.body.append(input); input.click();
    });
    await (await chooser).setFiles(file);
    await page.waitForFunction(name => {
      const PM = (window as any).PM;
      return Object.values<any>(PM.proj.assets).some(asset => asset.name === name)
        && [...PM.assets.map.values()].some((asset: any) => asset.name === name);
    }, name);

    const authored = await page.evaluate(({ name, stored }) => {
      const PM = (window as any).PM;
      const meta = Object.values<any>(PM.proj.assets).find(asset => asset.name === name);
      // Simulate an imported project whose old browser rendition supplied
      // incorrect facts. Opening a source must not rewrite authored content.
      meta.dur = stored; meta.hasAudio = false;
      const layer = PM.mkLayer('video', { from: .5, dur: .25, d: { asset: meta.id, trim: .125 } }, PM.proj);
      PM.proj.layers = [layer]; PM.touch();
      const readAuthored = () => {
        const asset = PM.proj.assets[meta.id], clip = PM.proj.layers.find((item: any) => item.id === layer.id);
        return { duration: asset.dur, hasAudio: asset.hasAudio, from: clip.from, durationInTimeline: clip.dur, trim: clip.d.trim };
      };
      PM.Kernel.services.get('viewer').preview.show(meta.id);
      const source = PM.EngineSourceMedia.monitorValue;
      const state = (window as any).__sourceTiming = {
        source, readAuthored, ended: 0, plays: 0, samples: [] as Array<{ time: number; peak: number }>,
      };
      source.addEventListener('play', () => { state.plays++; });
      source.addEventListener('ended', () => { state.ended++; });
      source.addEventListener('timeupdate', () => {
        if (!source.paused && source.levels.peak.length) {
          state.samples.push({ time: source.currentTime, peak: Math.max(...source.levels.peak) });
          if (state.samples.length > 256) state.samples.shift();
        }
      });
      return readAuthored();
    }, { name, stored: fixture.stored });

    const status = () => page.evaluate(() => {
      const state = (window as any).__sourceTiming, source = state.source;
      return { duration: source.duration, time: source.currentTime, paused: source.paused,
        ended: state.ended, plays: state.plays, error: source.error?.message ?? null };
    });
    await expect.poll(status, { message: 'paused initial monitor receives original endpoint without Play' })
      .toEqual({ duration: 3, time: 0, paused: true, ended: 0, plays: 0, error: null });
    await expect(page.locator('#source-preview .sp-scrub')).toHaveAttribute('aria-valuemax', '3');
    expect(await page.evaluate(() => (window as any).__sourceTiming.readAuthored())).toEqual(authored);

    // A paused seek clamps to the original endpoint, without emitting ended.
    await page.evaluate(() => { (window as any).__sourceTiming.source.currentTime = 20; });
    await expect.poll(status).toEqual({ duration: 3, time: 3, paused: true, ended: 0, plays: 0, error: null });
    await expect(page.locator('#source-preview .sp-scrub')).toHaveAttribute('aria-valuenow', '3');

    await page.evaluate(async name => {
      const state = (window as any).__sourceTiming;
      state.source.currentTime = name === 'long-audio' ? 1.25 : 0;
      state.samples = [];
      await state.source.play();
    }, fixture.name);

    // These are the live native monitor's output levels, not a second offline
    // decode of the same file. The native tests cover exact PCM/seek alignment.
    await expect.poll(() => page.evaluate(name => {
      const samples = (window as any).__sourceTiming.samples as Array<{ time: number; peak: number }>;
      const start = name === 'delayed-pcm' ? 1.1 : name === 'long-audio' ? 1.35 : .1;
      const end = name === 'short-audio' ? .65 : 2.5;
      return samples.some(sample => sample.time > start && sample.time < end && sample.peak > .05);
    }, fixture.name), { message: 'original PCM reaches the source monitor despite stored hasAudio=false' }).toBe(true);

    if (fixture.name === 'delayed-pcm') {
      const lead = await page.evaluate(() => ((window as any).__sourceTiming.samples as Array<{ time: number; peak: number }>)
        .filter(sample => sample.time > .15 && sample.time < .7));
      expect(lead.length, 'the delayed PCM retains a silent lead-in').toBeGreaterThan(0);
      expect(lead.every(sample => sample.peak < .00001)).toBe(true);
    } else if (fixture.name === 'short-audio') {
      await page.evaluate(() => {
        const state = (window as any).__sourceTiming;
        state.source.currentTime = 1.25;
        state.samples = [];
      });
      await expect.poll(() => page.evaluate(() => {
        const state = (window as any).__sourceTiming;
        return !state.source.paused && state.samples.some((sample: { time: number; peak: number }) =>
          sample.time > 1.4 && sample.time < 2.5 && sample.peak < .00001);
      }), { message: 'seeking beyond audio EOF keeps video playback advancing in silence' }).toBe(true);
    }

    await page.evaluate(() => { (window as any).__sourceTiming.source.currentTime = 2.85; });
    await expect.poll(status, { message: 'playback stops once, exactly at the original endpoint' })
      .toEqual({ duration: 3, time: 3, paused: true, ended: 1, plays: 1, error: null });
    await expect(page.locator('#source-preview .sp-scrub')).toHaveAttribute('aria-valuenow', '3');
    expect(await page.evaluate(() => (window as any).__sourceTiming.readAuthored())).toEqual(authored);
    await page.keyboard.press('Escape');
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
