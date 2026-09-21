// @vitest-environment happy-dom
import { appendFileSync } from 'node:fs';
import { expect, it, vi } from 'vitest';
import { createTimelineRuntime } from './timeline';
import { fakePowermoveAPI } from './fake-api.test-helper';

function fixture(count: number) {
  const harness = fakePowermoveAPI(vi);
  const layers = Array.from({ length: count }, (_, index) => ({
    id: `layer-${index}`, name: `Layer ${index}`, type: index % 10 === 0 ? 'group' : 'solid',
    group: index % 10 === 0 ? null : `layer-${index - index % 10}`,
    collapsed: true, p: {},
  }));
  harness.state.project.layers = layers;
  const runtime: any = createTimelineRuntime(harness.api);
  return { harness, layers, runtime, rebuild: () => harness.emit('selection', harness.state.selection) };
}

it('builds grouped rows with bounded work instead of rescanning every layer per group', () => {
  const { layers, runtime, rebuild } = fixture(1000);
  let reads = 0;
  for (const layer of layers) {
    const group = layer.group;
    Object.defineProperty(layer, 'group', { get: () => { reads++; return group; } });
  }
  try {
    rebuild();
    expect(runtime.rows.map((row: any) => row.L.id)).toEqual(layers.map(layer => layer.id));
    expect(runtime.rows.map((row: any) => row.i)).toEqual(layers.map((_, index) => index));
    expect(reads).toBeLessThan(layers.length * 4);
  } finally { runtime.dispose(); }
});

it('keeps source indices and depth-first order with nested, orphaned, and cyclic memberships', () => {
  const { harness, runtime, rebuild } = fixture(0);
  const layer = (id: string, type = 'solid', group: string | null = null) => ({ id, name: id, type, group, collapsed: true, p: {} });
  harness.state.project.layers = [
    layer('child', 'solid', 'nested'), layer('other'), layer('group', 'group'),
    layer('nested', 'group', 'group'), layer('sibling', 'solid', 'group'),
    layer('orphan', 'solid', 'missing'), layer('cycle-a', 'group', 'cycle-b'), layer('cycle-b', 'group', 'cycle-a'),
  ];
  try {
    rebuild();
    expect(runtime.rows.map((row: any) => [row.L.id, row.i])).toEqual([
      ['other', 1], ['group', 2], ['nested', 3], ['child', 0], ['sibling', 4],
      ['orphan', 5], ['cycle-a', 6], ['cycle-b', 7],
    ]);
  } finally { runtime.dispose(); }
});

it.skipIf(process.env.PM_PERF_PROGRAM !== '1')('measures rebuilding a large grouped timeline', () => {
  for (const count of [1000, 5000, 10000]) {
    const { runtime, rebuild } = fixture(count);
    try {
      rebuild();
      const samples = Array.from({ length: 7 }, () => {
        const start = performance.now(); rebuild(); return performance.now() - start;
      }).sort((a, b) => a - b);
      const result = JSON.stringify({ layers: count, medianMs: samples[3], samples });
      if (process.env.PM_PERF_OUTPUT) appendFileSync(process.env.PM_PERF_OUTPUT, result + '\n');
      console.log('PM_TIMELINE_ROWS', result);
      expect(runtime.rows).toHaveLength(count);
    } finally { runtime.dispose(); }
  }
});
