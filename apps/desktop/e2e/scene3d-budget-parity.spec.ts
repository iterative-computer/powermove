import { mkdir, writeFile } from 'node:fs/promises';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';
import {
  BUDGET_HEIGHT, BUDGET_VERTICES, BUDGET_WIDTH, addFourthBudgetPlacement,
  budgetMemorySample, installBudgetScene, measureBudgetFrame, percentile,
} from './helpers/scene3d-budget-parity';

const dense = process.env.PM_SCENE3D_BUDGET_DENSE === '1';
const OUT = `/tmp/ne-w5/scene3d-r8-budget/${dense ? 'dense' : 'sparse'}`;

async function lane(engine: boolean) {
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: engine ? '1' : '0' } });
    await session.openEditor();
    const baseline = await budgetMemorySample(session.app);
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
      phase = 'accepted frames';
      const imported = await budgetMemorySample(session.app);
      const accepted = await measureBudgetFrame(session.page, engine);
      phase = 'fourth placement';
      const added = await addFourthBudgetPlacement(session.page, model.assetId);
      const refused = added.ok ? await measureBudgetFrame(session.page, engine, 1) : null;
      phase = 'steady';
      const steady = await budgetMemorySample(session.app);
      const jsHeapBytes = await session.page.evaluate(() =>
        (performance as any).memory?.usedJSHeapSize ?? null);
      return { engine, model, accepted, refusal: { added, frame: refused },
        memory: { baseline, imported, steady, samples, jsHeapBytes },
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

function occupiedBands(columns: number[]) {
  let bands = 0, prior = -2;
  for (const x of columns) {
    if (x > prior + 1) bands++;
    prior = x;
  }
  return bands;
}

function memoryTotals(processes: Awaited<ReturnType<typeof budgetMemorySample>>) {
  const sum = (key: 'workingSetKiB' | 'peakWorkingSetKiB' | 'privateKiB') =>
    processes.reduce((total, process) => total + (process[key] ?? 0), 0);
  return { workingSetKiB: sum('workingSetKiB'),
    processLifetimePeakKiB: sum('peakWorkingSetKiB'), privateKiB: sum('privateKiB') };
}

test('600k-vertex glTF placements draw in both lanes until the legacy 2M scene limit', async () => {
  test.setTimeout(dense ? 480_000 : 300_000);
  const legacy = await lane(false);
  const native = await lane(true);
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
  const summarize = (result: typeof legacy) => ({
    ...result,
    memory: { ...result.memory,
      totals: { baseline: memoryTotals(result.memory.baseline),
        imported: memoryTotals(result.memory.imported),
        steady: memoryTotals(result.memory.steady),
        sampledPeakWorkingSetKiB: Math.max(0, ...result.memory.samples.map(sample =>
          memoryTotals(sample.processes).workingSetKiB)),
      } },
    accepted: { ...result.accepted, pixels: undefined, png: undefined,
      frameP50Ms: percentile(result.accepted.timesMs, .5),
      frameP95Ms: percentile(result.accepted.timesMs, .95),
      occupiedBands: occupiedBands(result.accepted.occupiedColumns) },
    refusal: { ...result.refusal,
      frame: result.refusal.frame && { ...result.refusal.frame, pixels: undefined, png: undefined } },
  });
  await writeFile(`${OUT}/measure.json`, JSON.stringify({
    modelVertices: BUDGET_VERTICES, dense, width: BUDGET_WIDTH, height: BUDGET_HEIGHT,
    fractionWithin2CV, maximumCV: maximum,
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
});
