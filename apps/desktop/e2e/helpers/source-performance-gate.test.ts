import { describe, expect, it } from 'vitest';
import { sourcePerformanceGate, type PerfSample } from './source-performance-gate';

function run(run: number, legacy: number[], native: number[], cold = [40, 20]): PerfSample[] {
  return [
    { run, enabled: false, round: 0, copy: 0, scrubMs: legacy, filmstripMs: [cold[0]!, 30] },
    { run, enabled: true, round: 0, copy: 0, scrubMs: native, filmstripMs: [cold[1]!, 0.2] }
  ];
}

describe('sourcePerformanceGate', () => {
  it('compares medians of per-run statistics, not one run against one run', () => {
    const quiet = [8.3, 8.3, 8.3, 8.4];
    const samples = [
      ...run(0, quiet, [8.3, 8.4, 8.4, 8.4]), // one run's 0.1 ms vsync jitter alone
      ...run(1, quiet, quiet), ...run(2, quiet, quiet), ...run(3, quiet, quiet), ...run(4, quiet, quiet)
    ];
    expect(sourcePerformanceGate(samples).failures).toEqual([]);
  });

  it('fails a native run whose P95 exceeds today\'s worst run, and too few runs', () => {
    const quiet = [8.3, 8.3, 8.3, 8.4];
    const samples = [...run(0, quiet, [8.3, 8.3, 8.3, 16.6]), ...run(1, quiet, quiet), ...run(2, quiet, quiet), ...run(3, quiet, quiet)];
    const gate = sourcePerformanceGate(samples);
    expect(gate.failures).toContain('4 runs; the gate needs 5');
    expect(gate.failures.some(f => f.startsWith('run 0 scrub P95 16.6'))).toBe(true);
  });

  it('fails when the native median of a statistic is slower', () => {
    const samples = [0, 1, 2, 3, 4].flatMap(r => run(r, [8.3], [8.4], [40, 45]));
    expect(sourcePerformanceGate(samples).failures).toEqual(expect.arrayContaining([
      'median scrubP50: native 8.4 > today 8.3', 'median coldMedian: native 45 > today 40']));
  });
});
