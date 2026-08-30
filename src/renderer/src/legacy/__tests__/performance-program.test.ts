import { expect, it } from 'vitest';

import { makePM } from './make-pm';

const runPerformance = process.env.PM_PERF_PROGRAM === '1' ? it : it.skip;
const LAYER_COUNT = Math.max(1, Math.floor(Number(process.env.PM_PERF_LAYERS) || 5_000));
const EDIT_SAMPLES = 15;

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)]!;
}

function elapsed(action: () => void): number {
  const start = performance.now();
  action();
  return performance.now() - start;
}

function performanceEditor() {
  const PM = makePM(
    'core/easing',
    'core/model',
    'core/selection',
    'core/anim',
    'core/history',
    'core/editing',
  );
  PM.proj = PM.mkProject({ name: 'Performance fixture', w: 3840, h: 2160, fps: 30, dur: 60 });
  PM.proj.layers = Array.from({ length: LAYER_COUNT }, (_value, index) => {
    const layer = PM.mkLayer('solid', { name: `Layer ${index}` });
    layer.id = `perf-layer-${index}`;
    layer.from = index % 50;
    layer.dur = 10;
    return layer;
  });
  PM.time = 1;
  return PM;
}

runPerformance('records the large-project performance comparison fixture', () => {
  const PM = performanceEditor();
  const initialProjectBytes = new TextEncoder().encode(JSON.stringify(PM.proj)).byteLength;

  // Warm the exact paths before measuring them.
  PM.L(`perf-layer-${LAYER_COUNT - 1}`);
  PM.Edit.sourceCatalog();

  const lookupMs = elapsed(() => {
    for (let index = 0; index < 10_000; index++) {
      expect(PM.L(`perf-layer-${index % LAYER_COUNT}`)).not.toBeNull();
    }
  });

  const catalogMs = elapsed(() => {
    expect(PM.Edit.sourceCatalog().layers).toHaveLength(LAYER_COUNT);
  });

  const editSamples: number[] = [];
  for (let index = 0; index < EDIT_SAMPLES; index++) {
    editSamples.push(elapsed(() => {
      const result = PM.Edit.apply({
        type: 'set_property',
        target: `perf-layer-${LAYER_COUNT - 1}`,
        path: 'opacity',
        value: 80 + index,
        mode: 'static',
      }, { label: `Performance edit ${index}`, origin: 'inspector' });
      expect(result.ok).toBe(true);
    }));
  }

  const undoSamples: number[] = [];
  for (let index = 0; index < 5; index++) {
    undoSamples.push(elapsed(() => expect(PM.hist.undo()).toBe(true)));
  }
  for (let index = 0; index < 5; index++) expect(PM.hist.redo()).toBe(true);

  const history = typeof PM.hist.stats === 'function'
    ? PM.hist.stats()
    : {
        entries: PM.hist.list().length,
        bytes: initialProjectBytes * 2 * PM.hist.list().length,
        estimated: true,
      };

  const result = {
    layers: LAYER_COUNT,
    initialProjectBytes,
    editMedianMs: median(editSamples),
    editP95Ms: [...editSamples].sort((a, b) => a - b)[Math.ceil(editSamples.length * 0.95) - 1],
    lookup10kMs: lookupMs,
    sourceCatalogMs: catalogMs,
    undoMedianMs: median(undoSamples),
    historyEntries: history.entries,
    historyBytes: history.bytes,
    historyEstimated: !!history.estimated,
  };

  console.log(`PM_PERF_RESULT ${JSON.stringify(result)}`);
  expect(result.initialProjectBytes).toBeGreaterThan(LAYER_COUNT * 100);
});
