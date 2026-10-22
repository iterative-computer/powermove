import { expect, launchApp, test } from './helpers/app';

/** A v1–3 extension calls the synchronous project, animation and render API
 * while the interactive utility is absent. The no-crash pass uses the exact
 * same serialized input and edit sequence, so the final JSON must match byte
 * for byte, including the ordered edit log. Run under pm-electron-lock with
 * --workers=1; each launch uses and removes its own temporary profile. */
const STEPS = 24;
const PAUSE_AT = 5;
const MAX_SYNC_MS = 30_000;

type Result = {
  initial: string;
  final: string;
  reads: number;
  steps: number;
  revisions: number[];
  frames: { step: number; hash: number }[];
  errors: string[];
  fallbacks: Record<string, number>;
  failures: unknown[];
  backend: string;
  heapMemory: number;
  longestSyncMs: number;
  trigger?: { generation: number; connected: boolean; beforeGeneration: number; ms: number };
  observedOutage: boolean;
};

async function run(crash: boolean, initial?: string): Promise<Result> {
  const session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
  try {
    await session.openEditor();
    await session.page.waitForFunction(() => {
      const PM = (window as any).PM;
      return PM.EngineDoc?.ready && PM.EngineConnection?.status().connected;
    });
    const seed = await session.page.evaluate(async ({ source, steps, pauseAt, maxSyncMs }) => {
      const PM = (window as any).PM;
      const raw = source ? JSON.parse(source) : PM.mkProject({ name: 'Native recovery loop', w: 96, h: 54, fps: 24, dur: 1 });
      if (!source) {
        const layer = PM.mkLayer('solid', { d: { color: '#e83a5e', w: 48, h: 32 } }, raw);
        layer.p['position.x'].v = 10;
        layer.p.opacity.v = 50;
        raw.layers.push(layer);
        // Cross the remote native-document threshold without costly raster/media data.
        raw.metadata = { padding: 'x'.repeat(131_072) };
      }
      const initial = JSON.stringify(raw);
      await PM.EngineDoc.prepare(raw);
      PM.proj = raw;
      PM.pause(); PM.setTime(0, { force: true });
      PM.doc.notify('project', []); PM.doc.notify('layers', []); PM.invalidate('all');
      await PM.EngineDoc.beforeSave('recovery loop seed');
      if (PM.EngineDoc.wasm.backend !== 'native') throw new Error(`Expected native document; got ${PM.EngineDoc.wasm.backend}`);
      const api = PM.Kernel.api('native-recovery-loop');
      const compat = api.host.pm;
      const layer = api.project.get().layers[0];
      const baseRevision = api.project.revision();
      const baselineGeneration = PM.EngineConnection.status().generation;
      const state: any = {
        initial, api, layer, baseRevision, baselineGeneration,
        steps: 0, reads: 0, revisions: [], frames: [], errors: [],
        released: false, done: false, observedOutage: false, longestSyncMs: 0,
      };
      const check = (render: boolean) => {
        const start = performance.now();
        const status = PM.EngineConnection.status();
        if (!status.connected && status.generation === baselineGeneration) state.observedOutage = true;
        const project = api.project.latest();
        if (project !== raw || api.project.get() !== raw || compat.proj !== raw || project.layers[0] !== layer) throw new Error('Document reference identity changed');
        const revision = api.project.revision();
        const expectedRevision = baseRevision + state.steps;
        if (revision !== expectedRevision) throw new Error(`Revision ${revision} != ${expectedRevision}`);
        const expectedValue = state.steps ? 10 + state.steps : 10;
        const value = api.anim.ev(layer, 'position.x', 0);
        const property = api.anim.evP(layer, layer.p['position.x'], 0, 'position.x');
        if (value !== expectedValue || property !== expectedValue) throw new Error(`Evaluation ${value}/${property} != ${expectedValue}`);
        const opacity = api.anim.ev(layer, 'opacity', 0);
        if (opacity !== 50 + state.steps || layer.p.opacity.v !== opacity) throw new Error(`Queued document write ${opacity} != ${50 + state.steps}`);
        if (render) {
          const canvas = api.render.renderFrameTo(0, 96, 54, { mblur: false });
          const pixels = canvas.getContext('2d').getImageData(0, 0, 96, 54).data;
          let hash = 2166136261 >>> 0;
          for (const byte of pixels) hash = Math.imul(hash ^ byte, 16777619) >>> 0;
          state.frames.push({ step: state.steps, hash });
        }
        state.reads++;
        state.longestSyncMs = Math.max(state.longestSyncMs, performance.now() - start);
        if (state.longestSyncMs > maxSyncMs) throw new Error(`Synchronous operation exceeded ${maxSyncMs} ms`);
      };
      const step = (render = false) => {
        if (state.steps >= steps) { state.done = true; return; }
        const n = state.steps + 1;
        const previousUid = PM.uid;
        const previousNow = Date.now;
        PM.uid = (prefix: string) => `${prefix}-recovery-${n}`;
        Date.now = () => 1_700_000_000_000 + n;
        try {
          const edit = api.project.apply({ type: 'set_property', target: layer.id, path: 'position.x', value: 10 + n, time: 0, mode: 'static' }, { label: 'Recovery edit' });
          if (!edit.ok) throw new Error(`Ordered edit ${n} failed: ${edit.message}`);
        } finally { PM.uid = previousUid; Date.now = previousNow; }
        // Deprecated v1–3 host escape hatch: a synchronous raw mutation with
        // the document barrier still has to replay in order with typed edits.
        compat.doc.write([['layers', 0, 'p', 'opacity', 'v']], () => { layer.p.opacity.v = 50 + n; });
        compat.doc.flush();
        state.steps = n;
        state.revisions.push(api.project.revision());
        check(render || n % 6 === 0);
        if (n === steps) state.done = true;
      };
      const tick = () => {
        if (state.done) return;
        try {
          if (state.steps === pauseAt && !state.released) check(false);
          else step();
        } catch (error) {
          state.errors.push(String((error as Error)?.stack ?? error));
          state.done = true;
        }
      };
      state.resume = (duringCrash: boolean) => {
        if (state.steps !== pauseAt) throw new Error(`Resume at unexpected step ${state.steps}`);
        state.released = true;
        if (duringCrash) {
          // This executes immediately after SIGKILL, before awaiting any
          // recovery signal. A changed generation proves the race was missed.
          const status = PM.EngineConnection.status();
          const start = performance.now();
          tick();
          state.trigger = { generation: status.generation, connected: status.connected, beforeGeneration: baselineGeneration, ms: performance.now() - start };
        }
      };
      state.result = () => ({
        initial, final: JSON.stringify(PM.proj), reads: state.reads,
        steps: state.steps, revisions: state.revisions, frames: state.frames,
        errors: state.errors, fallbacks: { ...PM.EngineDoc.api.stats.fallbacks },
        failures: PM.EngineDoc.failures(), backend: PM.EngineDoc.wasm.backend,
        heapMemory: PM.EngineDoc.wasm.heap().memory,
        longestSyncMs: state.longestSyncMs, trigger: state.trigger,
        observedOutage: state.observedOutage,
      });
      (window as any).__nativeRecoveryLoop = state;
      state.timer = window.setInterval(tick, 12);
      return { initial, backend: PM.EngineDoc.wasm.backend, generation: baselineGeneration };
    }, { source: initial ?? null, steps: STEPS, pauseAt: PAUSE_AT, maxSyncMs: MAX_SYNC_MS });
    await session.page.waitForFunction((pauseAt) => (window as any).__nativeRecoveryLoop?.steps === pauseAt, PAUSE_AT, { timeout: 30_000 });
    if (crash) {
      const pid = await session.app.evaluate(() => (globalThis as any).__powermoveNativeEngine.engine.status().interactive.pid as number);
      process.kill(pid, 'SIGKILL');
      // Do not wait for connected/ready/recovery here: resume() performs a
      // synchronous write, evaluator read and render against the dead utility.
      await session.page.evaluate(() => (window as any).__nativeRecoveryLoop.resume(true));
    } else {
      await session.page.evaluate(() => (window as any).__nativeRecoveryLoop.resume(false));
    }
    await session.page.waitForFunction(() => (window as any).__nativeRecoveryLoop?.done, null, { timeout: 90_000 });
    const result = await session.page.evaluate(() => {
      const state = (window as any).__nativeRecoveryLoop;
      window.clearInterval(state.timer);
      return state.result();
    }) as Result;
    expect(result.initial).toBe(seed.initial);
    return result;
  } finally {
    await session.page.evaluate(() => {
      const state = (window as any).__nativeRecoveryLoop;
      if (state?.timer) window.clearInterval(state.timer);
    }).catch(() => {});
    await session.close();
  }
}

test('v1–3 synchronous document and render calls preserve ordered edits across SIGKILL', async () => {
  test.setTimeout(210_000);
  const control = await run(false);
  const crashed = await run(true, control.initial);
  for (const result of [control, crashed]) {
    expect(result.errors).toEqual([]);
    expect(result.steps).toBe(STEPS);
    expect(result.reads).toBeGreaterThan(STEPS);
    expect(result.revisions).toEqual(Array.from({ length: STEPS }, (_, i) => i + 1 + (result.revisions[0] - 1)));
    expect(result.fallbacks).toEqual({});
    expect(result.failures).toEqual([]);
    expect(result.backend).toBe('native');
    expect(result.heapMemory).toBe(0);
    expect(result.longestSyncMs).toBeLessThan(MAX_SYNC_MS);
  }
  expect(crashed.trigger).toBeDefined();
  expect(crashed.trigger!.generation).toBe(crashed.trigger!.beforeGeneration);
  expect(crashed.trigger!.ms).toBeLessThan(MAX_SYNC_MS);
  expect(crashed.revisions).toEqual(control.revisions);
  expect(crashed.frames).toEqual(control.frames);
  expect(crashed.final).toBe(control.final);
});
