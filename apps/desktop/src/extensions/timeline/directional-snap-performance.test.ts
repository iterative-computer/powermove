import { appendFileSync } from 'node:fs';
import { expect, it } from 'vitest';
import { createDirectionalSnapper } from './directional-snap';

function largeSelection() {
  return {
    anchors: Array.from({ length: 1000 }, (_, index) => ({ time: (index * 13 + 2) / 30 })),
    targets: Array.from({ length: 20000 }, (_, index) => ({ time: ((index * 7919) % 20000) / 30 })),
  };
}

it.skipIf(process.env.PM_PERF_PROGRAM !== '1')('measures snapping a large clip selection', () => {
  const { anchors, targets } = largeSelection();
  const search = () => createDirectionalSnapper(-.1).resolve(anchors, .123, targets, .05);
  search(); search();
  const samples = Array.from({ length: 9 }, () => {
    const start = performance.now();
    const result = search();
    const elapsed = performance.now() - start;
    expect(result.lock).not.toBeNull();
    return elapsed;
  }).sort((a, b) => a - b);
  const result = JSON.stringify({ anchors: anchors.length, targets: targets.length, medianMs: samples[4], samples });
  if (process.env.PM_PERF_OUTPUT) appendFileSync(process.env.PM_PERF_OUTPUT, result + '\n');
  console.log('PM_TIMELINE_SNAP', result);
});
