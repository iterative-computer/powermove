/** Hidden real-app play-start measurement. Run under pm-electron-lock:
 * bun engine/crates/pm-audio/tools/play-start.ts <wav> <outside-repo.json> [runs]
 * App and native addons must already be built. Uses the corpus Node bridge
 * because Bun cannot host Playwright's Electron inspector driver.
 * Measures scheduled device presentation, not acoustic loopback.
 */
import { writeFile } from 'node:fs/promises';
import { launchLegacyApp } from '../../../corpus/src/app';

const [file, output, count = '30'] = process.argv.slice(2);
if (!file || !output) throw new Error('wav and outside-repository output required');
const result: Record<string, unknown> = { method: 'PM.play → scheduled program sample at device output; no acoustic loopback' };
const modes = process.env.PM_AUDIO_START_MODE === 'today' ? [false] : process.env.PM_AUDIO_START_MODE === 'native' ? [true] : [false, true];
for (const native of modes) {
  process.env.POWERMOVE_NEW_ENGINE = native ? '1' : '0';
  const session = await launchLegacyApp();
  console.error(`A6 measurement: ${native ? 'native' : 'today'} launched`);
  try {
    const page = {
      evaluate: session.evaluate,
      waitForFunction: session.waitFor,
      waitForTimeout: (ms: number) => Bun.sleep(ms),
    };
    await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'A6 start measurement', w: 160, h: 90, fps: 30, dur: 10 }));
      PM.loop = false;
      const original = AudioBufferSourceNode.prototype.start;
      (window as any).__a6starts = [];
      AudioBufferSourceNode.prototype.start = function (...args: Parameters<typeof original>) {
        const ctx = this.context as AudioContext;
        const called = performance.now();
        const when = Math.max(args[0] ?? 0, ctx.currentTime);
        const stamp = ctx.getOutputTimestamp();
        (window as any).__a6starts.push({ called, when, stamp, baseLatency: ctx.baseLatency, outputLatency: ctx.outputLatency });
        return original.apply(this, args);
      };
    });
    await page.evaluate(() => {
      const input = document.createElement('input');
      input.type = 'file'; input.id = 'a6-media'; input.style.display = 'none';
      document.body.appendChild(input);
    });
    await session.setInputFiles('#a6-media', [file]);
    await page.evaluate(async () => {
      const PM = (window as any).PM;
      const input = document.getElementById('a6-media') as HTMLInputElement;
      const asset = await PM.assets.add(input.files![0], { newAsset: true, silent: true });
      const layer = PM.mkLayer('audio', { from: 0, dur: 10, d: { asset: asset.id, gain: 1, trim: 0, fadeIn: 0, fadeOut: 0 } }, PM.proj);
      PM.proj.layers.push(layer);
      PM.touch();
      PM.bus.emit('layers'); PM.bus.emit('project');
      PM.invalidate();
    });
    if (native) {
      await page.waitForFunction(() => (window as any).PM.EngineConnection.status().driving);
      await page.waitForFunction(() => Boolean((window as any).PM.EngineConnection.status().hostOffsetUs));
    } else {
      await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.audioBuffer));
    }
    // First play opens/resumes the device; subsequent measured runs are warm.
    await page.evaluate(() => (window as any).PM.play());
    await page.waitForTimeout(300);
    await page.evaluate(() => (window as any).PM.pause());
    const samples = [];
    for (let run = 0; run < Number(count); run++) {
      const target = run % 2 === 0 ? 0 : 3;
      await page.evaluate(t => { const PM = (window as any).PM; PM.pause(); PM.setTime(t); }, target);
      await page.waitForTimeout(250);
      const sample = await page.evaluate(async ({ native, target }) => {
        const PM = (window as any).PM;
        const starts = (window as any).__a6starts as any[];
        starts.length = 0;
        const oldEpoch = PM.EngineConnection?.status().clock?.epoch;
        const requested = performance.now();
        PM.play();
        const deadline = requested + 5000;
        for (;;) {
          if (performance.now() > deadline) throw new Error('play-start timed out');
          if (native) {
            const s = PM.EngineConnection.status();
            const clock = s.clock;
            if (clock && clock.epoch !== oldEpoch && clock.state === 'playing') {
              const firstUs = clock.hostTime - (clock.mediaTime / 705600000 - target) * 1e6;
              return { target, presenterEpoch: clock.epoch, requestedHostUs: requested * 1000 + s.hostOffsetUs, outputMs: (firstUs - s.hostOffsetUs) / 1000 - requested,
                observedMs: performance.now() - requested, offsetErrorUs: s.hostOffsetErrorUs,
                outputLatencyUs: clock.outputLatencyUs, sampleRate: clock.sampleRate };
            }
          } else if (starts.length) {
            const first = starts[0];
            if (!first.stamp.performanceTime || !first.stamp.contextTime) throw new Error('WebAudio output timestamp unavailable');
            return { target, outputMs: first.stamp.performanceTime + (first.when - first.stamp.contextTime) * 1000 - requested,
              observedMs: first.called - requested, baseLatency: first.baseLatency, outputLatency: first.outputLatency };
          }
          await new Promise(r => setTimeout(r, 1));
        }
      }, { native, target });
      await page.evaluate(() => (window as any).PM.pause());
      if (native) {
        // The presenter and audio have independent epoch counters. Match
        // this request on the shared host clock, after the 33 ms status poll.
        const deadline = performance.now() + 1000;
        for (;;) {
          const timing = await session.evaluateMain(async () => {
            const report = await (globalThis as any).__powermoveNativeEngine.engine.report('interactive');
            return (Object.values(report.presenter.audio)[0] as any)?.startTiming;
          });
          if (timing && timing.requestedUs >= sample.requestedHostUs) {
            Object.assign(sample, { timing });
            break;
          }
          if (performance.now() >= deadline) throw new Error('first-buffer timing missing');
          await Bun.sleep(10);
        }
      }
      samples.push(sample);
    }
    const audio = native ? await session.evaluateMain(async () => {
      const report = await (globalThis as any).__powermoveNativeEngine.engine.report('interactive');
      return { hostClockRuns: report.presenter.hostClockRuns, audio: Object.values(report.presenter.audio) };
    }) : null;
    if (audio && (audio.hostClockRuns !== 0 || !audio.audio.length || audio.audio.some((a: any) => a.error || a.problems.length))) {
      throw new Error(`native device-clock validation failed: ${JSON.stringify(audio)}`);
    }
    const sorted = samples.map(s => s.outputMs).sort((a, b) => a - b);
    const p = (q: number) => sorted[Math.ceil(sorted.length * q) - 1];
    result[native ? 'native' : 'today'] = { samples, n: samples.length, p50: p(.5), p95: p(.95), p99: p(.99), min: sorted[0], max: sorted.at(-1), pageErrors: session.pageErrors, audio };
    await writeFile(output, JSON.stringify(result, null, 2));
  } catch (error) {
    result.failure = { error: String(error), status: await session.evaluate(() => ({
      engine: (window as any).PM.EngineConnection?.status(),
      enabled: (window as any).PM.EngineDoc?.enabled,
      engineDocument: (window as any).PM.EngineDoc?.status?.(),
    })).catch(String), pageErrors: session.pageErrors, consoleErrors: session.consoleErrors };
    await writeFile(output, JSON.stringify(result, null, 2));
    throw error;
  } finally { await session.close(); }
}
console.log(JSON.stringify(result, null, 2));
