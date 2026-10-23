import { mkdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';
import {
  BUDGET_HEIGHT, BUDGET_VERTICES, BUDGET_WIDTH, addFourthBudgetPlacement,
  budgetMemorySample, installBudgetScene, measureBudgetFrame, percentile,
} from './helpers/scene3d-budget-parity';

const denseModes = process.env.PM_SCENE3D_BUDGET_DENSE === undefined
  ? [false, true] : [process.env.PM_SCENE3D_BUDGET_DENSE === '1'];
/** Where measurements go; `--repeat-each` runs get their own `run-N`. */
const outputRoot = process.env.PM_SCENE3D_BUDGET_OUT ?? '/tmp/ne-w5/scene3d-r8-budget';
/** Opt-in: the native engine's resource inventory and the page's WASM/JS
 * heap at each memory sample (POWERMOVE_ENGINE_ALLOCATION_INVENTORY). */
const inventory = process.env.PM_SCENE3D_BUDGET_INVENTORY === '1';
/** PM_SCENE3D_BUDGET_RUN numbers runs that separate invocations interleave. */
const outputDir = (dense: boolean) => {
  const run = process.env.PM_SCENE3D_BUDGET_RUN ?? String(test.info().repeatEachIndex);
  return run !== '0' || process.env.PM_SCENE3D_BUDGET_OUT
    ? `${outputRoot}/run-${run}/${dense ? 'dense' : 'sparse'}`
    : `${outputRoot}/${dense ? 'dense' : 'sparse'}`;
};

/** Both routes are sampled with the viewer showing the current document:
 * the native viewer presents asynchronously (its first pool is made after
 * the editor opens), the current viewer on its next animation frames. */
async function settled(session: LaunchedApp, engine: boolean) {
  if (engine) await expect.poll(() => session.page.evaluate(() => {
    const PM = (window as any).PM;
    const p = PM.Kernel.services.get('viewer')?.enginePresentation;
    return !!p?.presenting && p.latest?.revision >= PM.EngineDoc.barrier.revision;
  }), { timeout: 30_000 }).toBe(true);
  await session.page.evaluate(() => new Promise<void>(resolve =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
}

async function inventorySample(session: LaunchedApp, engine: boolean) {
  if (!inventory) return null;
  const host = engine ? await session.app.evaluate(async () => {
    const e = (globalThis as any).__powermoveNativeEngine?.engine;
    return { engines: e?.status?.() ?? null, interactive: await e?.report?.('interactive').catch((error: unknown) => String(error)) ?? null };
  }) : null;
  const page = await session.page.evaluate(() => {
    const PM = (window as any).PM;
    return { wasm: PM.EngineDoc?.stats?.().wasm ?? null, jsHeapBytes: (performance as any).memory?.usedJSHeapSize ?? null,
      jsHeapTotalBytes: (performance as any).memory?.totalJSHeapSize ?? null };
  });
  return { host, page };
}

async function captureStage(session: LaunchedApp, file: string) {
  await session.page.evaluate(() => new Promise<void>(resolve =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  const bounds = await session.page.locator('#stage').evaluate(stage => {
    const r = stage.getBoundingClientRect();
    return { x: Math.ceil(r.x), y: Math.ceil(r.y), width: Math.floor(r.width) - 2, height: Math.floor(r.height) - 2 };
  });
  const png = await session.app.evaluate(async ({ BrowserWindow }, bounds) => {
    const window = BrowserWindow.getAllWindows().find(w => !w.isDestroyed() && w.webContents.getURL().startsWith('app:'))!;
    return (await window.webContents.capturePage(bounds)).toPNG().toString('base64');
  }, bounds);
  const bytes = Buffer.from(png, 'base64');
  await writeFile(file, bytes);
  // A row through the models avoids toolbar/cursor ink. Group gaps below
  // 1% of the image width: dense mode has tiny gaps between its triangles.
  const row = execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'png_pipe', '-i', 'pipe:0',
    '-vf', 'crop=iw:1:0:floor(ih*0.55)', '-frames:v', '1', '-f', 'rawvideo', '-pix_fmt', 'rgba', 'pipe:1'],
  { input: bytes, maxBuffer: 16 << 20 });
  const modelColumns: number[] = [];
  for (let i = 0; i < row.length; i += 4) {
    if (row[i]! > 80 && row[i]! > row[i + 1]! * 1.4 && row[i]! > row[i + 2]! * 1.4) modelColumns.push(i / 4);
  }
  return { bounds, modelColumns, modelBands: occupiedBands(modelColumns, Math.max(1, Math.round(row.length / 4 * .01))) };
}

async function lane(engine: boolean, dense: boolean) {
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0',
      ...(inventory ? { POWERMOVE_ENGINE_ALLOCATION_INVENTORY: '1' } : {}) } });
    await session.openEditor();
    await settled(session, engine);
    const baseline = await budgetMemorySample(session.app);
    const inventories: Record<string, unknown> = { baseline: await inventorySample(session, engine) };
    const samples: { phase: string; atMs: number; processes: Awaited<ReturnType<typeof budgetMemorySample>> }[] = [];
    let phase = 'import';
    let pending: Promise<void> | null = null;
    const poll = () => {
      if (pending) return;
      pending = budgetMemorySample(session!.app).then(processes => {
        samples.push({ phase, atMs: Date.now(), processes });
      }).catch(() => undefined).then(() => { pending = null; });
    };
    const timer = setInterval(poll, 50);
    try {
      const model = await installBudgetScene(session.page, 3, dense);
      await settled(session, engine);
      phase = 'accepted frames';
      const imported = await budgetMemorySample(session.app);
      inventories.imported = await inventorySample(session, engine);
      const accepted = await measureBudgetFrame(session.page, engine);
      const out = outputDir(dense);
      await mkdir(out, { recursive: true });
      if (engine) await expect.poll(() => session!.page.evaluate(() => {
        const PM = (window as any).PM;
        const p = PM.Kernel.services.get('viewer')?.enginePresentation;
        return !!p?.presenting && p.latest?.revision >= PM.EngineDoc.barrier.revision;
      }), { timeout: 30_000 }).toBe(true);
      const stageBefore = await captureStage(session, `${out}/${engine ? 'native' : 'legacy'}-stage-before.png`);
      phase = 'fourth placement';
      const added = await addFourthBudgetPlacement(session.page, model.assetId);
      const refused = added.ok ? await measureBudgetFrame(session.page, engine, 1) : null;
      const stageRefusal = await captureStage(session, `${out}/${engine ? 'native' : 'legacy'}-stage-refusal.png`);
      phase = 'steady';
      const steady = await budgetMemorySample(session.app);
      inventories.steady = await inventorySample(session, engine);
      // A refused scene is reported once per change, never redrawn at every
      // presenter turn (each failed attempt also held GPU allocations).
      const renderErrors = engine ? await session.app.evaluate(async () =>
        ((await (globalThis as any).__powermoveNativeEngine?.engine?.report?.('interactive')) as any)?.presenter?.renderErrors ?? null) : null;
      const jsHeapBytes = await session.page.evaluate(() =>
        (performance as any).memory?.usedJSHeapSize ?? null);
      return { engine, model, accepted, stageBefore, stageRefusal, refusal: { added, frame: refused },
        memory: { baseline, imported, steady, samples, jsHeapBytes, inventories }, renderErrors,
        rendererPageErrors: [...session.diagnostics.pageErrors],
        rendererErrors: session.diagnostics.console.filter(record => record.type === 'error'),
      };
    } finally {
      clearInterval(timer);
      await pending;
    }
  } finally {
    // launchApp owns and removes its isolated userData directory here.
    await session?.close();
  }
}

function occupiedBands(columns: number[], gap = 1) {
  let bands = 0, prior = -2;
  for (const x of columns) {
    if (x > prior + gap) bands++;
    prior = x;
  }
  return bands;
}

function memoryTotals(processes: Awaited<ReturnType<typeof budgetMemorySample>>) {
  const sum = (key: 'workingSetKiB' | 'peakWorkingSetKiB' | 'privateKiB') =>
    processes.some(process => process[key] !== null)
      ? processes.reduce((total, process) => total + (process[key] ?? 0), 0) : null;
  return { workingSetKiB: sum('workingSetKiB'),
    processLifetimePeakKiB: sum('peakWorkingSetKiB'), privateKiB: sum('privateKiB') };
}

for (const dense of denseModes) test(`600k-vertex glTF placements draw until the legacy 2M limit (${dense ? 'dense' : 'sparse'})`, async () => {
  const OUT = outputDir(dense);
  test.setTimeout(dense ? 480_000 : 300_000);
  const loadBefore = os.loadavg();
  const legacy = await lane(false, dense);
  const native = await lane(true, dense);
  await mkdir(OUT, { recursive: true });
  for (const result of [legacy, native]) {
    if (result.accepted.ok && result.accepted.png) {
      await writeFile(`${OUT}/${result.engine ? 'native' : 'legacy'}.png`,
        Buffer.from(result.accepted.png.split(',')[1]!, 'base64'));
    }
    if (result.refusal.frame?.ok && result.refusal.frame.png) {
      await writeFile(`${OUT}/${result.engine ? 'native' : 'legacy'}-refusal.png`,
        Buffer.from(result.refusal.frame.png.split(',')[1]!, 'base64'));
    }
  }
  const pixelsA = legacy.accepted.pixels;
  const pixelsB = native.accepted.pixels;
  let withinTwo = 0, maximum = 0;
  if (pixelsA && pixelsB) for (let i = 0; i < pixelsA.length; i += 4) {
    const diff = Math.max(...[0, 1, 2, 3].map(k => Math.abs(pixelsA[i + k]! - pixelsB[i + k]!)));
    if (diff <= 2) withinTwo++;
    maximum = Math.max(maximum, diff);
  }
  const fractionWithin2CV = withinTwo / (BUDGET_WIDTH * BUDGET_HEIGHT);
  const refusedA = legacy.refusal.frame?.pixels, refusedB = native.refusal.frame?.pixels;
  let refusalWithinTwo = 0;
  if (refusedA && refusedB) for (let i = 0; i < refusedA.length; i += 4) {
    if ([0, 1, 2, 3].every(k => Math.abs(refusedA[i + k]! - refusedB[i + k]!) <= 2)) refusalWithinTwo++;
  }
  const refusalFractionWithin2CV = refusedA && refusedB
    ? refusalWithinTwo / (BUDGET_WIDTH * BUDGET_HEIGHT) : null;
  const summarize = (result: typeof legacy) => ({
    ...result,
    memory: { ...result.memory,
      totals: { baseline: memoryTotals(result.memory.baseline),
        imported: memoryTotals(result.memory.imported),
        steady: memoryTotals(result.memory.steady),
        sampledPeakWorkingSetKiB: Math.max(0, ...result.memory.samples.map(sample =>
          memoryTotals(sample.processes).workingSetKiB ?? 0)),
      } },
    accepted: { ...result.accepted, pixels: undefined, png: undefined,
      frameP50Ms: percentile(result.accepted.timesMs, .5),
      frameP95Ms: percentile(result.accepted.timesMs, .95),
      frameP99Ms: percentile(result.accepted.timesMs, .99),
      occupiedBands: occupiedBands(result.accepted.occupiedColumns) },
    refusal: { ...result.refusal,
      frame: result.refusal.frame && { ...result.refusal.frame, pixels: undefined, png: undefined } },
  });
  await writeFile(`${OUT}/measure.json`, JSON.stringify({
    loadBefore, loadAfter: os.loadavg(),
    modelVertices: BUDGET_VERTICES, dense, width: BUDGET_WIDTH, height: BUDGET_HEIGHT,
    fractionWithin2CV, maximumCV: maximum, refusalFractionWithin2CV,
    legacy: summarize(legacy), native: summarize(native),
  }, null, 2));

  expect(legacy.model.geometryVertices).toBe(BUDGET_VERTICES);
  expect(native.model.geometryVertices).toBe(BUDGET_VERTICES);
  expect(legacy.model.assetVertices).toBe(BUDGET_VERTICES);
  expect(native.model.assetVertices).toBe(BUDGET_VERTICES);
  expect(legacy.model.geometryTriangles).toBe(200_000);
  expect(native.model.geometryTriangles).toBe(200_000);
  expect(legacy.model.totalPlacedVertices).toBe(1_800_000);
  expect(native.model.totalPlacedVertices).toBe(1_800_000);
  expect(native.renderErrors).not.toBeNull();
  expect(native.renderErrors!).toBeLessThanOrEqual(8);
  expect(legacy.accepted.ok).toBe(true);
  expect(native.accepted.ok).toBe(true);
  expect(legacy.accepted.webglContextLost).toBe(false);
  expect(native.accepted.webglContextLost).toBe(false);
  expect(legacy.accepted.occupied).toBeGreaterThan(100);
  expect(native.accepted.occupied).toBeGreaterThan(100);
  expect(occupiedBands(legacy.accepted.occupiedColumns)).toBeGreaterThanOrEqual(3);
  expect(occupiedBands(native.accepted.occupiedColumns)).toBeGreaterThanOrEqual(3);
  expect(fractionWithin2CV).toBeGreaterThanOrEqual(.99934); // existing S7 3D bar
  const legacyRefusal = [legacy.refusal.added.error, legacy.refusal.frame?.error,
    ...legacy.rendererErrors.map(record => record.text)].filter(Boolean).join('\n');
  const nativeRefusal = [native.refusal.added.error, native.refusal.frame?.error,
    ...native.rendererErrors.map(record => record.text)].filter(Boolean).join('\n');
  expect(legacyRefusal).toContain('two million vertices');
  expect(nativeRefusal.includes('two million vertices') ||
    (native.refusal.frame?.ok && native.refusal.frame.occupied === 0)).toBe(true);
  if (native.refusal.frame?.ok) {
    expect(native.refusal.frame.occupied).toBe(legacy.refusal.frame?.occupied ?? 0);
  }
  if (refusalFractionWithin2CV !== null) expect(refusalFractionWithin2CV).toBeGreaterThanOrEqual(.99934);
  for (const result of [legacy, native]) {
    expect(result.stageBefore.modelBands).toBe(3);
    expect(result.stageRefusal.modelBands).toBe(3);
    expect(result.stageRefusal.modelColumns).toEqual(result.stageBefore.modelColumns);
  }
});
