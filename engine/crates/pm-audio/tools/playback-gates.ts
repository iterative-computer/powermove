/** Paired hidden-app A7/A8 gates: 16 speed-1.5 voices; 60 s work-area
 * looping, then an edit and Play in the same renderer task. Each latency
 * sample seeks to 4 s and allows 250 ms to prepare before the edit, so it
 * measures edit-then-play rather than a simultaneous cold seek. Set
 * PM_AUDIO_START_EDIT=gain|speed|none, PM_AUDIO_LOOP_SECONDS (default 60),
 * PM_AUDIO_LOAD_THREADS (default 0), PM_AUDIO_START_VOICES (default 16).
 * PM_AUDIO_START_DISTINCT=trim gives every voice its own trim (61.3 ms
 * apart), so no two voices share a stretched signal: each needs its own
 * vocoder pre-roll and native starts cannot reuse an immutable shared head
 * (asserted per sample through prefillReadyUs).
 * PM_AUDIO_START_WAIT_MS=0 alternates 2 s / 3 s same-task cold seeks.
 * PM_AUDIO_START_ORDER=today-native|native-today counterbalances paired rows.
 * PM_AUDIO_CONTROL_READY_WAIT_MS adds a post-edit wait for explicitly
 * prepared controls and requires the immutable first output to predate Play.
 * Each run continues 120 ms after its first clock so late-voice omission
 * spans include re-entry; PM_AUDIO_START_OBSERVE_MS overrides this window.
 * PM_AUDIO_START_SPEED (default 1.5) sets every clip's speed (a speed edit
 * toggles it by +0.1). PM_AUDIO_CLIP_SECONDS (default 10) sets the clips'
 * and project's length. PM_AUDIO_START_KEYFRAMED=gain keys each clip's gain
 * every PM_AUDIO_KEY_STEP seconds (default 1) along the whole clip, and a
 * gain edit changes the level of each clip's key nearest the play position. PM_AUDIO_START_LAYER=video plays the file as a
 * video layer's soundtrack (embedded audio). PM_AUDIO_START_TARGETS lists
 * the seek targets (seconds, comma separated; the defaults are 4, or 2 and
 * 3 for cold seeks), cycled per sample. PM_AUDIO_LOOP_RANGE (default 4,6)
 * sets the work area. PM_AUDIO_SETTLE_MS waits after the warm Play (both
 * apps) so whole-file decodes finish before measuring.
 * All load processes are joined on exit. Run under pm-electron-lock.
 * bun engine/crates/pm-audio/tools/playback-gates.ts <wav> <outside-repo.json> [runs]
 * Build the app and native addons first. The corpus Node bridge hosts
 * Playwright's Electron driver. This measures scheduled device presentation,
 * not acoustic loopback.
 */
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { closeActiveLegacyApps, launchLegacyApp } from '../../../corpus/src/app';

const [file, output, count = '30'] = process.argv.slice(2);
if (!file || !output) throw new Error('wav and outside-repository output required');
// Use the packaged copy so the harness also works after build-target cleanup.
process.env.POWERMOVE_PM_BRIDGE ??= resolve(import.meta.dir, '../../../../apps/desktop/native/pm_bridge.node');
const voices = Number(process.env.PM_AUDIO_START_VOICES ?? 16);
const edit = process.env.PM_AUDIO_START_EDIT ?? 'gain';
const loopSeconds = Number(process.env.PM_AUDIO_LOOP_SECONDS ?? 60);
const stressThreads = Number(process.env.PM_AUDIO_LOAD_THREADS ?? 0);
const waitMs = Number(process.env.PM_AUDIO_START_WAIT_MS ?? 250);
const readyWaitMs = Number(process.env.PM_AUDIO_CONTROL_READY_WAIT_MS ?? 0);
if (!Number.isFinite(readyWaitMs) || readyWaitMs < 0) throw new Error('PM_AUDIO_CONTROL_READY_WAIT_MS must be nonnegative');
const observeMs = Number(process.env.PM_AUDIO_START_OBSERVE_MS ?? 120);
const distinct = process.env.PM_AUDIO_START_DISTINCT ?? 'none';
const order = process.env.PM_AUDIO_START_ORDER ?? 'today-native';
const speed = Number(process.env.PM_AUDIO_START_SPEED ?? 1.5);
const clipSeconds = Number(process.env.PM_AUDIO_CLIP_SECONDS ?? 10);
const keyframed = process.env.PM_AUDIO_START_KEYFRAMED ?? 'none';
const keyStep = Number(process.env.PM_AUDIO_KEY_STEP ?? 1);
const layerKind = process.env.PM_AUDIO_START_LAYER ?? 'audio';
const loopRange = (process.env.PM_AUDIO_LOOP_RANGE ?? '4,6').split(',').map(Number);
const settleMs = Number(process.env.PM_AUDIO_SETTLE_MS ?? 0);
if (!(speed > 0) || !(clipSeconds > 0) || !(keyStep > 0) || !(settleMs >= 0)) throw new Error('invalid speed, clip length, key step or settle time');
if (keyframed !== 'none' && keyframed !== 'gain') throw new Error('PM_AUDIO_START_KEYFRAMED must be none or gain');
if (layerKind !== 'audio' && layerKind !== 'video') throw new Error('PM_AUDIO_START_LAYER must be audio or video');
if (loopRange.length !== 2 || !(loopRange[1] > loopRange[0])) throw new Error('PM_AUDIO_LOOP_RANGE must be start,end');
const activeStress = new Set<Bun.Subprocess>();
let stoppingStress: Promise<void> | undefined;
function stopStress(): Promise<void> {
  return stoppingStress ??= (async () => {
    const children = [...activeStress];
    activeStress.clear();
    for (const child of children) { try { child.kill(); } catch { /* already exited */ } }
    await Promise.allSettled(children.map(child => child.exited));
  })().finally(() => { stoppingStress = undefined; });
}
let signalCleanup = false;
for (const [signal, code] of [['SIGTERM', 143], ['SIGINT', 130], ['SIGHUP', 129]] as const) {
  process.on(signal, () => {
    if (signalCleanup) return;
    signalCleanup = true;
    void (async () => {
      try {
        try { await stopStress(); }
        finally { await closeActiveLegacyApps(); }
      }
      finally { process.exit(code); }
    })();
  });
}
if (distinct !== 'none' && distinct !== 'trim') throw new Error('PM_AUDIO_START_DISTINCT must be none or trim');
if (order !== 'today-native' && order !== 'native-today') throw new Error('PM_AUDIO_START_ORDER must be today-native or native-today');
const trimStep = distinct === 'trim' ? 0.0613 : 0;
const targets = process.env.PM_AUDIO_START_TARGETS ? process.env.PM_AUDIO_START_TARGETS.split(',').map(Number) : waitMs === 0 ? [2, 3] : [4];
// Play from past the work area's end starts at its start (today and native).
if (!targets.length || targets.some(t => !(t >= 0 && t < clipSeconds && t < loopRange[1]))) throw new Error('PM_AUDIO_START_TARGETS must lie within the clips and before the work area end');
const result: Record<string, unknown> = { method: 'PM.play → scheduled program sample at device output; no acoustic loopback', seekToPlayMs: waitMs, observeMs, voices, speed, clipSeconds, keyframed, keyStep, layerKind, loopRange, settleMs, edit, loopSeconds, stressThreads, distinct, trims: Array.from({ length: voices }, (_, i) => i * trimStep), targets };
result.controlReadyWaitMs = readyWaitMs;
const modes = process.env.PM_AUDIO_START_MODE === 'today' ? [false] : process.env.PM_AUDIO_START_MODE === 'native' ? [true] : order === 'today-native' ? [false, true] : [true, false];
result.modeOrder = modes.map(native => native ? 'native' : 'today');
const idlePoolFailed = (timing: any) => !timing?.idleStartupArm?.startsWith('Armed')
  || timing?.idleLease?.active !== true || timing?.idleLease?.conflict !== false
  || timing?.idlePoolSincePlay?.conflicts !== 0
  || ['armFailures', 'armTimeouts', 'computationOverruns', 'constraintOverruns', 'restoreFailures', 'hintFailures', 'constraintReadbackFailures']
    .some(key => timing?.idlePool?.[key] !== 0);
function activeAudio(report: any, sessionId: number) {
  if (!Number.isSafeInteger(sessionId) || sessionId <= 0) throw new Error(`invalid active native session ID: ${sessionId}`);
  const matches = Object.entries(report.presenter.audio as Record<string, any>)
    .filter(([key]) => key.endsWith(`:${sessionId}`));
  if (matches.length !== 1) throw new Error(`expected exactly one active audio entry for session ${sessionId}; found ${matches.length}`);
  const [key, audio] = matches[0];
  if (report.presenter.routes[key] !== 'engine') throw new Error(`active audio route ${key} is ${report.presenter.routes[key] ?? 'missing'}, expected engine`);
  return { key, audio, sessionId, hostClockRuns: report.presenter.hostClockRuns, silentRuns: report.presenter.silentRuns };
}
for (const native of modes) {
  process.env.POWERMOVE_NEW_ENGINE = native ? '1' : '0';
  const session = await launchLegacyApp();
  console.error(`A7/A8 measurement: ${native ? 'native' : 'today'} launched`);
  try {
    const page = {
      evaluate: session.evaluate,
      waitForFunction: session.waitFor,
      waitForTimeout: (ms: number) => Bun.sleep(ms),
    };
    await page.waitForFunction(() => {
      const doc = (window as any).PM?.EngineDoc;
      return Boolean(doc && (!doc.enabled || doc.ready));
    });
    await page.evaluate(({ clipSeconds }) => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ name: 'A7-A8 measurement', w: 160, h: 90, fps: 30, dur: clipSeconds }));
      PM.loop = false;
      const original = AudioBufferSourceNode.prototype.start;
      (window as any).__a6starts = [];
      AudioBufferSourceNode.prototype.start = function (...args: Parameters<typeof original>) {
        const ctx = this.context as AudioContext;
        const called = performance.now();
        const when = Math.max(args[0] ?? 0, ctx.currentTime);
        const stamp = ctx.getOutputTimestamp();
        (window as any).__a6starts.push({ ctx, called, when, stamp, baseLatency: ctx.baseLatency, outputLatency: ctx.outputLatency });
        return original.apply(this, args);
      };
    }, { clipSeconds });
    await page.evaluate(() => {
      const input = document.createElement('input');
      input.type = 'file'; input.id = 'a6-media'; input.style.display = 'none';
      document.body.appendChild(input);
    });
    await session.setInputFiles('#a6-media', [file]);
    const installed = await page.evaluate(async ({ voices, trimStep, speed, clipSeconds, keyframed, keyStep, layerKind, loopRange }) => {
      const PM = (window as any).PM;
      const input = document.getElementById('a6-media') as HTMLInputElement;
      const asset = await PM.assets.add(input.files![0], { newAsset: true, silent: true });
      // Gain keys along the whole clip, alternating levels, linear.
      const gainOf = (base: number) => keyframed !== 'gain' ? base : {
        v: base, expr: null,
        kf: Array.from({ length: Math.floor(clipSeconds / keyStep) + 1 }, (_, k) => ({
          t: k * keyStep, v: base * (k % 2 ? 0.5 : 1), i: `g${k}`, inInterp: 'linear', outInterp: 'linear',
          inEase: { speed: 0, influence: 16.666666666666668 }, outEase: { speed: 0, influence: 16.666666666666668 } })),
      };
      for (let i = 0; i < voices; i++) {
        const layer = layerKind === 'video'
          ? PM.mkLayer('video', { from: 0, dur: clipSeconds, d: { asset: asset.id, fit: 'contain', w: 160, h: 90, trim: i * trimStep, speed, embeddedAudio: true, audioGain: gainOf(1 / voices) } }, PM.proj)
          : PM.mkLayer('audio', { from: 0, dur: clipSeconds, d: { asset: asset.id, gain: gainOf(1 / voices), trim: i * trimStep, fadeIn: 0, fadeOut: 0, speed } }, PM.proj);
        PM.proj.layers.push(layer);
      }
      PM.proj.work = loopRange;
      PM.touch();
      PM.bus.emit('layers'); PM.bus.emit('project');
      PM.invalidate();
      return { projectId: PM.proj.id, layerCount: PM.proj.layers.length };
    }, { voices, trimStep, speed, clipSeconds, keyframed, keyStep, layerKind, loopRange });
    if (native) {
      await page.waitForFunction(() => {
        const PM = (window as any).PM;
        const active = PM.EngineConnection?.session();
        const snapshot = PM.EngineDoc?.snapshot();
        return Number.isSafeInteger(active?.id) && active.id > 0
          && snapshot && Number.isSafeInteger(snapshot.revision)
          && active.applied >= snapshot.revision;
      });
      await page.waitForFunction(() => (window as any).PM.EngineConnection.status().driving);
      await page.waitForFunction(() => Boolean((window as any).PM.EngineConnection.status().hostOffsetUs));
    } else if (layerKind === 'audio') {
      await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.audioBuffer));
    }
    const fixture = await page.evaluate(() => {
      const PM = (window as any).PM;
      return { projectId: PM.proj?.id, layerCount: PM.proj?.layers?.length,
        sessionId: PM.EngineConnection?.session()?.id ?? null,
        revision: PM.EngineDoc?.snapshot()?.revision ?? null };
    });
    if (typeof installed.projectId !== 'string' || !installed.projectId || installed.layerCount !== voices
      || fixture.projectId !== installed.projectId || fixture.layerCount !== installed.layerCount
      || (native && (!Number.isSafeInteger(fixture.sessionId) || fixture.sessionId <= 0
        || !Number.isSafeInteger(fixture.revision)))) {
      throw new Error(`fixture was not installed on the active document: ${JSON.stringify(fixture)}`);
    }
    const activeNativeSessionId: number | null = native ? fixture.sessionId : null;
    const assertFixture = async () => {
      const actual = await page.evaluate(() => {
        const PM = (window as any).PM;
        return { projectId: PM.proj?.id, layerCount: PM.proj?.layers?.length,
          sessionId: PM.EngineConnection?.session()?.id ?? null };
      });
      if (actual.projectId !== fixture.projectId || actual.layerCount !== voices
        || (native && actual.sessionId !== activeNativeSessionId)) {
        throw new Error(`measurement fixture changed: ${JSON.stringify({ expected: fixture, actual })}`);
      }
    };
    result[native ? 'nativeFixture' : 'todayFixture'] = fixture;
    // Keep unrelated renderer/GPU telemetry out of the per-start IPC path.
    // The final validation still checks every reported audio entry.
    const nativeReport = async (allAudio = false) => session.evaluateMain(async (_electron, { sessionId, allAudio }) => {
      const { presenter } = await (globalThis as any).__powermoveNativeEngine.engine.report('interactive');
      const audio = allAudio ? presenter.audio : Object.fromEntries(
        Object.entries(presenter.audio).filter(([key]) => key.endsWith(`:${sessionId}`)));
      return { presenter: { audio, routes: presenter.routes, hostClockRuns: presenter.hostClockRuns, silentRuns: presenter.silentRuns } };
    }, { sessionId: activeNativeSessionId, allAudio });
    // First play opens/resumes the device; subsequent measured runs are warm.
    await page.evaluate(() => (window as any).PM.play());
    await page.waitForTimeout(300);
    // A soundtrack decodes when first played today.
    if (!native) await page.waitForFunction(() => [...(window as any).PM.assets.map.values()].some((a: any) => a.audioBuffer));
    if (settleMs > 0) await page.waitForTimeout(settleMs);
    if (native) {
      const warmDeadline = performance.now() + 60_000;
      for (;;) {
        const report = await nativeReport();
        const matches = Object.keys(report.presenter.audio).filter(key => key.endsWith(`:${activeNativeSessionId}`));
        if (matches.length) {
          const bound = activeAudio(report, activeNativeSessionId!);
          if (typeof bound.audio.device === 'string' && bound.audio.device.length > 0 && bound.audio.deviceRate > 0
            && bound.audio.projects > 0 && bound.audio.deviceCallbacks > 0 && bound.audio.startTiming != null) {
            result.nativeBinding = { sessionId: bound.sessionId, audioKey: bound.key,
              hostClockRuns: bound.hostClockRuns, silentRuns: bound.silentRuns,
              projectId: fixture.projectId, layerCount: fixture.layerCount };
            break;
          }
        }
        if (performance.now() >= warmDeadline) throw new Error(`active native audio was not ready after warm Play: ${JSON.stringify({ fixture, matches })}`);
        await Bun.sleep(25);
      }
    }
    await assertFixture();
    await page.evaluate(() => (window as any).PM.pause());
    let loop: any;
    try {
      for (let i = 0; i < stressThreads; i++) activeStress.add(Bun.spawn(['/bin/sh', '-c', 'while :; do :; done']));
      const status = async () => native ? activeAudio(await nativeReport(), activeNativeSessionId!).audio : await page.evaluate(() => {
        const starts = (window as any).__a6starts;
        const ctx = starts.at(-1)?.ctx;
        return ctx ? { baseLatency: ctx.baseLatency, stats: JSON.parse(JSON.stringify(ctx.playbackStats)) } : null;
      });
      if (loopSeconds > 0) {
        await page.evaluate(t => { const PM = (window as any).PM; PM.loop = true; PM.setTime(t); }, loopRange[0]);
        await Bun.sleep(1500);
        await page.evaluate(() => (window as any).PM.play());
        await Bun.sleep(1000);
        const before: any = await status();
        const snapshots: any[] = [];
        for (let i = 0; i < loopSeconds; i++) { await Bun.sleep(1000); snapshots.push(await status()); }
        const after = snapshots.at(-1);
        loop = { before, after, snapshots, seconds: loopSeconds };
        await page.evaluate(() => { const PM = (window as any).PM; PM.pause(); PM.loop = false; });
        if (native) Object.assign(loop, { underruns: after.underruns - before.underruns, starvedRenders: after.starvedRenders - before.starvedRenders, overloads: after.deviceOverloads - before.deviceOverloads });
        else Object.assign(loop, { underruns: after.stats.underrunEvents - before.stats.underrunEvents });
        result[native ? 'nativeLoop' : 'todayLoop'] = loop;
        await writeFile(output, JSON.stringify(result, null, 2));
        if (native && (loop.underruns !== 0 || loop.starvedRenders !== 0 || loop.overloads !== 0 || after.startTiming?.programUnderruns !== 0
          || (process.platform === 'darwin' && (after.startTiming?.workerSchedulingFailures !== 0 || after.startTiming?.producerMachError !== null || after.startTiming?.producerHintError !== null
            || after.startTiming?.schedulingComputationOverruns !== 0 || after.startTiming?.schedulingConstraintOverruns !== 0 || after.startTiming?.schedulingRestoreFailures !== 0 || after.startTiming?.schedulingHintFailures !== 0 || after.startTiming?.schedulingConstraintReadbackFailures !== 0 || after.startTiming?.schedulingIntervalFailures !== 0 || after.startTiming?.schedulingIntervalExpired !== 0 || after.startTiming?.profileWorkerLogDrops !== 0 || after.startTiming?.profileWorkerFlushFailures !== 0 || idlePoolFailed(after.startTiming))))) {
          throw new Error(`loop delivery or real-time scheduling failed: ${JSON.stringify({ underruns: loop.underruns, timing: after.startTiming })}`);
        }
      }
      const samples: any[] = [];
      // Keep completed and failing raw starts in the catch report too.
      result[native ? 'native' : 'today'] = { samples, n: 0, partial: true };
      for (let run = 0; run < Number(count); run++) {
        await assertFixture();
        const target = targets[run % targets.length];
        if (waitMs > 0) {
          await page.evaluate(t => { const PM = (window as any).PM; PM.pause(); PM.setTime(t); }, target);
          await page.waitForTimeout(waitMs);
        }
        const preEditAudio: any = native ? activeAudio(await nativeReport(), activeNativeSessionId!).audio : null;
        const sample = await page.evaluate(async ({ native, target, inline, edit, run, readyWaitMs, speed, projectId, voices, sessionId }) => {
          const PM = (window as any).PM;
          if (PM.proj?.id !== projectId || PM.proj?.layers?.length !== voices
            || (native && PM.EngineConnection?.session()?.id !== sessionId)) {
            throw new Error('measurement fixture or active native session changed before timed Play');
          }
          const starts = (window as any).__a6starts as any[];
          starts.length = 0;
          const oldEpoch = PM.EngineConnection?.status().clock?.epoch;
          // Seek and Play in the same task: no paused preparation time.
          if (inline) { PM.pause(); PM.setTime(target); }
          for (const layer of PM.proj.layers) {
            const key = layer.type === 'video' ? 'audioGain' : 'gain';
            const level = (run % 2 ? 0.5 : 1) / PM.proj.layers.length;
            if (edit === 'gain' && typeof layer.d[key] === 'object') {
              // A keyframe edit: the key at the play position changes level.
              const k = layer.d[key].kf.reduce((best: any, k: any) => Math.abs(k.t - target) < Math.abs(best.t - target) ? k : best);
              k.v = level;
            } else if (edit === 'gain') layer.d[key] = level;
            if (edit === 'speed') layer.d.speed = run % 2 ? speed + 0.1 : speed;
          }
          if (edit !== 'none') { PM.touch(); PM.bus.emit('layers'); PM.invalidate(); }
          // Only the separately named prepared control rows enable this.
          // The A8 edit/Play rows execute no await between edit and Play.
          if (readyWaitMs > 0) await new Promise(resolve => setTimeout(resolve, readyWaitMs));
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
                observedMs: first.called - requested, baseLatency: first.baseLatency, outputLatency: first.outputLatency,
                scheduledVoices: starts.length, voiceStartSpreadMs: (Math.max(...starts.map(s => s.when)) - Math.min(...starts.map(s => s.when))) * 1000 };
            }
            await new Promise(r => setTimeout(r, 1));
          }
        }, { native, target, inline: waitMs === 0, edit, run, readyWaitMs, speed,
          projectId: fixture.projectId, voices, sessionId: activeNativeSessionId });
        const captured: any = { ...sample, preEditAudio };
        samples.push(captured);
        result[native ? 'native' : 'today'].n = samples.length;
        await page.waitForTimeout(observeMs);
        await page.evaluate(() => (window as any).PM.pause());
        if (native) {
          // The presenter and audio have independent epoch counters. Match
          // this request on the shared host clock, after the 33 ms status poll.
          const deadline = performance.now() + 1000;
          for (;;) {
            const postEditAudio: any = await status();
            const timing = postEditAudio?.startTiming;
            if (timing && timing.requestedUs >= sample.requestedHostUs) {
              const deviceOverloads = postEditAudio.deviceOverloads - preEditAudio.deviceOverloads;
              Object.assign(captured, { timing, postEditAudio, deviceOverloads,
                ingressRequestMs: postEditAudio.ingressRequestUs == null ? null : (postEditAudio.ingressRequestUs - sample.requestedHostUs) / 1000,
                producerRequestMs: (timing.requestedUs - sample.requestedHostUs) / 1000 });
              if (deviceOverloads !== 0) {
                throw new Error(`device overloaded during start: ${JSON.stringify(captured)}`);
              }
              if (timing.omittedPrefixFrames !== 0 || timing.startSpreadFrames !== 0 || timing.safetyNetFires !== 0 || timing.programUnderruns !== 0 || timing.historyClaims !== 0
                || timing.voiceStartSamples.length !== voices) {
                throw new Error(`ordinary start must be complete and simultaneous: ${JSON.stringify(timing)}`);
              }
              if (process.platform === 'darwin' && (!['device-workgroup', 'custom-audio-interval', 'mach-time-constraint'].includes(timing.producerScheduling)
                || ((speed !== 1 || timing.idlePoolSincePlay?.jobClaims > 0) && timing.workerWorkgroupJoins + timing.workerMachJoins === 0) || timing.workerSchedulingFailures !== 0 || timing.producerMachError !== null || timing.producerHintError !== null || timing.schedulingComputationOverruns !== 0 || timing.schedulingConstraintOverruns !== 0 || timing.schedulingRestoreFailures !== 0 || timing.schedulingHintFailures !== 0 || timing.schedulingConstraintReadbackFailures !== 0 || timing.schedulingIntervalFailures !== 0 || timing.schedulingIntervalExpired !== 0 || timing.profileWorkerLogDrops !== 0 || timing.profileWorkerFlushFailures !== 0 || idlePoolFailed(timing))) {
                throw new Error(`audio threads did not enter a real-time schedule: ${JSON.stringify(timing)}`);
              }
              // Distinct signals must not be served by a shared immutable head.
              if (distinct !== 'none' && timing.prefillReadyUs !== 0) {
                throw new Error(`distinct voices used a shared head: ${JSON.stringify(timing)}`);
              }
              if (readyWaitMs > 0 && !(timing.prefillReadyUs > 0 && timing.prefillReadyUs <= timing.requestedUs)) {
                throw new Error(`prepared control first output was not ready before Play: ${JSON.stringify(timing)}`);
              }
              break;
            }
            if (performance.now() >= deadline) throw new Error('first-buffer timing missing');
            await Bun.sleep(10);
          }
        }
      }
      const audio = native ? await (async () => {
        const report = await nativeReport(true);
        const bound = activeAudio(report, activeNativeSessionId!);
        if (bound.key !== (result.nativeBinding as any).audioKey) throw new Error(`active audio key changed: ${bound.key}`);
        return { hostClockRuns: report.presenter.hostClockRuns, silentRuns: report.presenter.silentRuns,
          activeAudioKey: bound.key, activeSessionId: bound.sessionId,
          audio: Object.values(report.presenter.audio) };
      })() : null;
      if (audio && (audio.hostClockRuns !== 0 || !audio.audio.length || audio.audio.some((a: any) => a.error || a.problems.length))) {
        throw new Error(`native device-clock validation failed: ${JSON.stringify(audio)}`);
      }
      const sorted = samples.map(s => s.outputMs).sort((a, b) => a - b);
      const p = (q: number) => sorted[Math.ceil(sorted.length * q) - 1];
      result[native ? 'native' : 'today'] = { samples, n: samples.length, p50: p(.5), p95: p(.95), p99: p(.99), min: sorted[0], max: sorted.at(-1), pageErrors: session.pageErrors, audio };
      await writeFile(output, JSON.stringify(result, null, 2));
    } finally { await stopStress(); }
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
