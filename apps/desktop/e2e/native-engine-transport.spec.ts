/*
 * The new engine's transport in the real app (plan section 15; section 25
 * Phase 3b: "J/K/L and audio scrub functional tests"), behind "Use New
 * Engine": J, K, and L shuttle at 1× and 2× forward and reverse on the audio
 * device's clock with frames on that line, play around the current frame
 * returns to it, audio scrub plays grains while scrubbing and stepping, and
 * the mixer's meters and the timeline's waveforms come from the engine.
 *
 * Run serially, one Electron at a time:
 *   pm-electron-lock bunx playwright test e2e/native-engine-transport.spec.ts --workers=1
 */
import type { Page } from 'playwright';

import { expect, test, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';

const ON = { POWERMOVE_NEW_ENGINE: '1' };

type Clock = { state: string; mediaTime: number; epoch: number; rate: { num: number; den: number }; hostTime: number } | null;

const status = (page: Page): Promise<{ route: string; driving: boolean; clock: Clock; failures: string[] }> =>
  page.evaluate(() => (window as any).PM.EngineConnection.status());

/** A 640×360, 30 fps, 8 s composition of solids (an engine-drawn route). */
async function solidsProject(page: Page): Promise<void> {
  await page.evaluate(() => {
    const PM = (window as any).PM;
    const project = PM.mkProject({ name: 'Transport', w: 640, h: 360, fps: 30, dur: 8, bg: '#203040' });
    const box = PM.mkLayer('solid', { name: 'Box', from: 0, dur: 8, d: { color: '#FF3300', w: 80, h: 40 } }, project);
    box.id = 'box';
    project.layers = [box];
    PM.replaceProject(project);
    PM.loop = false;
  });
}

async function openEngine(session: LaunchedApp): Promise<void> {
  await session.openEditor();
  await session.page.waitForFunction(() => Boolean((window as any).PM?.EngineConnection));
}

async function waitDriving(page: Page): Promise<void> {
  await expect.poll(async () => (await status(page)).route, { timeout: 45_000 }).toBe('engine');
  await expect.poll(async () => (await status(page)).driving, { timeout: 20_000 }).toBe(true);
}

/** Moves focus off any field, so keys reach the editor's bindings. */
async function focusEditor(page: Page): Promise<void> {
  await page.evaluate(() => {
    (document.activeElement as HTMLElement | null)?.blur?.();
    document.body.focus();
  });
}

/**
 * The playhead's rate over `ms` (least squares of PM.time against page time),
 * and A/V sync of the engine frames presented meanwhile: for each timed frame,
 * how far the audio clock's line at the frame's display instant is outside the
 * frame's interval (ms).
 */
async function measureRun(page: Page, ms: number): Promise<{ rate: number; samples: number; sync: number[] }> {
  return page.evaluate(async (ms) => {
    const PM = (window as any).PM;
    const out: { now: number; t: number }[] = [];
    const sync: number[] = [];
    const off = PM.Kernel.services.get('viewer').enginePresentation.onPresented((frame: any) => {
      const clock = PM.EngineConnection.session()?.clock;
      if (!frame.timed || frame.displayUs === null || !clock || clock.state === 'stopped') return;
      const heard = clock.mediaTime + ((frame.displayUs - clock.hostTime) * 705_600_000 * clock.rate.num) / (1e6 * clock.rate.den);
      const end = frame.pts + 705_600_000 / 30;
      const outside = heard < frame.pts ? frame.pts - heard : heard >= end ? heard - end : 0;
      sync.push((outside / 705_600_000) * 1000);
    });
    const start = performance.now();
    while (performance.now() - start < ms) {
      await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      out.push({ now: performance.now(), t: PM.time });
    }
    off();
    const n = out.length;
    const mx = out.reduce((s, p) => s + p.now, 0) / n;
    const my = out.reduce((s, p) => s + p.t, 0) / n;
    const rate = (out.reduce((s, p) => s + (p.now - mx) * (p.t - my), 0) / out.reduce((s, p) => s + (p.now - mx) ** 2, 0)) * 1000;
    return { rate, samples: n, sync };
  }, ms);
}

test.describe('Use New Engine: J/K/L, play around, and audio scrub', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('J, K, and L shuttle at 1× and 2× both ways on the audio clock, with frames on its line', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    await solidsProject(page);
    await waitDriving(page);
    await page.evaluate(() => (window as any).PM.setTime(2));
    await focusEditor(page);

    // Paused, J keeps its After Effects meaning: nothing plays.
    await page.keyboard.press('j');
    await page.waitForTimeout(300);
    expect(await page.evaluate(() => (window as any).PM.playing)).toBe(false);
    await page.evaluate(() => (window as any).PM.setTime(4));

    const runs: Record<string, { rate: number; outside: number; frames: number; worstMs: number }> = {};
    const shuttleTo = async (key: string, expected: number, label: string): Promise<void> => {
      await page.keyboard.press(key);
      await expect.poll(async () => (await status(page)).clock?.rate.num, { timeout: 10_000 }).toBe(expected);
      await expect.poll(async () => (await status(page)).clock?.state, { timeout: 5_000 }).not.toBe('stopped');
      const run = await measureRun(page, 700);
      runs[label] = { rate: run.rate, outside: run.sync.filter((ms) => ms > 10).length, frames: run.sync.length, worstMs: Math.max(0, ...run.sync) };
      expect(run.samples).toBeGreaterThan(15);
      // The playhead moves at the rate (least squares over 0.7 s).
      expect(Math.abs(run.rate - expected), `${label}: ${run.rate}`).toBeLessThan(0.06 * Math.abs(expected));
      // Frames are presented on the audio clock's line (section 15, ±10 ms):
      // at most one frame of the run outside its interval.
      expect(run.sync.length, label).toBeGreaterThan(8);
      expect(run.sync.filter((ms) => ms > 10).length, `${label}: ${run.sync.map((v) => v.toFixed(1)).join(' ')}`).toBeLessThanOrEqual(1);
    };
    // Space plays at 1×; L again: 2×; J: backward at 1×; J again: 2× backward.
    await page.keyboard.press('Space');
    await expect.poll(async () => (await status(page)).clock?.rate.num, { timeout: 10_000 }).toBe(1);
    await shuttleTo('l', 2, '2× forward');
    await shuttleTo('j', -1, '1× reverse');
    await shuttleTo('j', -2, '2× reverse');
    await shuttleTo('l', 1, '1× forward');
    // K stops on a frame.
    await page.keyboard.press('k');
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 5_000 }).toBe('stopped');
    expect(await page.evaluate(() => (window as any).PM.playing)).toBe(false);
    const stopped = await page.evaluate(() => (window as any).PM.time as number);
    expect(Math.abs(stopped * 30 - Math.round(stopped * 30))).toBeLessThan(1e-6);

    // Control-J plays backward from a pause, and the run stops at the start.
    await page.evaluate(() => (window as any).PM.setTime(0.6));
    await focusEditor(page);
    await page.keyboard.press('Control+j');
    await expect.poll(async () => (await status(page)).clock?.rate.num, { timeout: 10_000 }).toBe(-1);
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing), { timeout: 10_000 }).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.time)).toBeLessThan(1 / 30 + 1e-6);

    // 2× forward runs to the end of the work area and stops there.
    await page.evaluate(() => (window as any).PM.setTime(7));
    await page.evaluate(() => (window as any).PM.shuttle(2));
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing), { timeout: 10_000 }).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.time)).toBeGreaterThan(7.9);

    // Every run played on the audio device's clock.
    const report = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.report('interactive'));
    expect(report.presenter.hostClockRuns).toBe(0);
    const audio = Object.values(report.presenter.audio) as any[];
    expect(audio.length).toBe(1);
    expect(audio[0].error).toBeNull();
    expect(audio[0].problems).toEqual([]);
    console.log(`J/K/L runs: ${JSON.stringify(runs)}`);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('play around the current frame plays the seconds around it and returns', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    await solidsProject(page);
    await waitDriving(page);
    await page.evaluate(() => (window as any).PM.setTime(3));
    await focusEditor(page);
    await page.evaluate(() => {
      const w = window as any;
      w.__seen = [];
      w.__watch = setInterval(() => w.__seen.push(w.PM.time), 50);
    });
    await page.keyboard.press('Control+Shift+Space');
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 10_000 }).toBe('playing');
    await expect.poll(() => page.evaluate(() => (window as any).PM.playing), { timeout: 10_000 }).toBe(false);
    const seen: number[] = await page.evaluate(() => { const w = window as any; clearInterval(w.__watch); return w.__seen; });
    // It started two seconds before, passed the frame, and came back to it.
    expect(Math.min(...seen)).toBeLessThan(1.5);
    expect(Math.max(...seen)).toBeGreaterThan(4.2);
    const back = await page.evaluate(() => (window as any).PM.time as number);
    expect(Math.abs(back - 3)).toBeLessThan(1 / 30 + 1e-6);
    expect((await status(page)).clock?.state).toBe('stopped');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('audio scrub plays grains while scrubbing and stepping; meters and waveforms come from the engine', async ({ session }) => {
    const { page } = session;
    await openEngine(session);
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'Scrub', w: 640, h: 360, fps: 30, dur: 2, bg: '#000000' }));
      PM.setTime(0);
      PM.loop = false;
    });
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(fixturePath('tone.wav'));
    await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'audio'), undefined, { timeout: 30_000 });
    await waitDriving(page);

    // The waveform envelope comes from the engine's decode: 100 buckets a
    // second of the 1.5 s tone.
    await expect.poll(() => page.evaluate(() => {
      const PM = (window as any).PM;
      const layer = PM.proj.layers.find((l: any) => l.type === 'audio');
      const asset = PM.assets.get(layer.d.asset);
      return asset?.peaksSource === 'engine' ? asset.peaks.length : 0;
    }), { timeout: 20_000 }).toBe(150);
    const peak = await page.evaluate(() => {
      const PM = (window as any).PM;
      const asset = PM.assets.get(PM.proj.layers.find((l: any) => l.type === 'audio').d.asset);
      return Math.max(...Array.from(asset.peaks as Float32Array));
    });
    expect(peak).toBeGreaterThan(0.1);

    // Scrub audio is a switch in Settings › General › Engine.
    await page.evaluate(() => (window as any).PM.cmd('settings'));
    const toggle = page.getByRole('button', { name: 'Scrub audio' });
    await expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-pressed', 'true');
    await page.keyboard.press('Escape');
    expect(await page.evaluate(() => (window as any).PM.audioScrub())).toBe(true);

    // A grain is heard through the engine's levels: the meters' data while
    // the device plays it (the viewer's line does not move for a grain).
    // Returns the loudest message's media time and peak, or null when no
    // levels came.
    const grainAt = async (action: string): Promise<{ media: number; peak: number } | null> =>
      page.evaluate(async (action) => {
        const PM = (window as any).PM;
        const heard: { media: number; peak: number }[] = [];
        const off = PM.EngineConnection.audio.onLevels((l: any) => heard.push({ media: l.mediaTime / 705_600_000, peak: Math.max(...l.master.peak) }));
        if (action === 'scrub') PM.setTime(0.8);
        else if (action === 'step') PM.step(3);
        else PM.setTime(1.2);
        await new Promise((r) => setTimeout(r, 1000));
        off();
        if (!heard.length) return null;
        return heard.reduce((a, b) => (b.peak > a.peak ? b : a));
      }, action);
    // A scrub while paused: an audible grain at the new position; the
    // playhead and the clock stay.
    const scrubbed = await grainAt('scrub');
    expect(scrubbed).not.toBeNull();
    expect(scrubbed!.peak).toBeGreaterThan(0.05);
    expect(Math.abs(scrubbed!.media - 0.8)).toBeLessThan(0.1);
    expect(await page.evaluate(() => (window as any).PM.time)).toBeCloseTo(0.8, 6);
    expect((await status(page)).clock?.state).toBe('stopped');
    // A step plays a grain at the stepped frame.
    const stepped = await grainAt('step');
    expect(stepped).not.toBeNull();
    expect(stepped!.peak).toBeGreaterThan(0.05);
    expect(Math.abs(stepped!.media - (0.8 + 3 / 30))).toBeLessThan(0.1);
    // Off: scrubs are silent (no grain, no levels).
    await page.evaluate(() => (window as any).PM.setAudioScrub(false));
    await page.waitForTimeout(200);
    expect(await grainAt('silent')).toBeNull();

    // While playing, the mixer's meters read the engine's levels of the tone.
    await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0.1); PM.play(); });
    await expect.poll(async () => (await status(page)).clock?.state, { timeout: 10_000 }).toBe('playing');
    await expect.poll(() => page.evaluate(() => {
      const meters = (window as any).PM.Audio.meters;
      const out = { peak: [0, 0], rms: [0, 0] };
      return meters.running() && meters.read(meters.MASTER, out) ? out.peak[0] : 0;
    }), { timeout: 5_000 }).toBeGreaterThan(0.05);
    // A monitor solo of another strip silences the master meter.
    await page.evaluate(() => (window as any).PM.Audio.setMonitorSolo(['no-such-layer']));
    await expect.poll(() => page.evaluate(() => {
      const meters = (window as any).PM.Audio.meters;
      const out = { peak: [1, 1], rms: [1, 1] };
      meters.read(meters.MASTER, out);
      return out.peak[0];
    }), { timeout: 5_000 }).toBeLessThan(1e-4);
    await page.evaluate(() => { const PM = (window as any).PM; PM.Audio.setMonitorSolo([]); PM.pause(); });
    expect((await status(page)).failures.filter((f) => !f.startsWith('deviceLost'))).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
