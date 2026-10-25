import { describe, expect, it } from 'vitest';
import { FRAME_MATCH } from './source-frame-correctness';
import { sourcePerformanceGate, type CampaignCorrectness, type PerfSample } from './source-performance-gate';

const accepted: CampaignCorrectness = { campaign: 'c', build: 'b', fixtures: [],
  evidence: { campaign: 'c', build: 'b', tolerances: FRAME_MATCH, pass: true, fixtures: [] } };

const quiet = [12, 8.3, 8.3, 8.3, 8.4];

function run(run: number, legacy: number[], native: number[], cold = [40, 20], load = 2): PerfSample[] {
  const sample = { workload: 'h264-320', run, round: 0, copy: 0, loadBefore: [load, load, load] };
  return [
    { ...sample, enabled: false, scrubMs: legacy, filmstripMs: [cold[0]!, 30] },
    { ...sample, enabled: true, scrubMs: native, filmstripMs: [cold[1]!, 0.2] }
  ];
}

describe('sourcePerformanceGate', () => {
  it('compares medians of per-run statistics, not one run against one run', () => {
    const samples = [
      ...run(0, quiet, [12, 8.3, 8.4, 8.4, 8.4]), // one run's 0.1 ms vsync jitter alone
      ...run(1, quiet, quiet), ...run(2, quiet, quiet), ...run(3, quiet, quiet), ...run(4, quiet, quiet)
    ];
    expect(sourcePerformanceGate(samples, accepted).failures).toEqual([]);
  });

  it('fails a native run whose P95 exceeds today\'s worst run, and too few runs', () => {
    const samples = [...run(0, quiet, [12, 8.3, 8.3, 8.3, 16.6]), ...run(1, quiet, quiet), ...run(2, quiet, quiet), ...run(3, quiet, quiet)];
    const gate = sourcePerformanceGate(samples, accepted);
    expect(gate.failures).toContain('h264-320: 4 runs; the gate needs 5');
    expect(gate.failures.some(f => f.startsWith('h264-320: run 0 scrub P95 16.6'))).toBe(true);
  });

  it('fails when the native median of a statistic is slower', () => {
    const samples = [0, 1, 2, 3, 4].flatMap(r => run(r, [12, 8.3], [13, 8.4], [40, 45]));
    expect(sourcePerformanceGate(samples, accepted).failures).toEqual(expect.arrayContaining([
      'h264-320: median firstScrub: native 13 > today 12',
      'h264-320: median scrubP50: native 8.4 > today 8.3',
      'h264-320: median coldMedian: native 45 > today 40']));
  });

  it('fails a tail the P95 hides: more steady scrubs over one frame and a slower P99', () => {
    const steady = (slow: number) => [12, ...Array.from({ length: 40 }, (_, k) => k < slow ? 16.7 : 8.3)];
    // Two over-frame scrubs in forty sit above P95 but below the median of runs.
    const samples = [0, 1, 2, 3, 4].flatMap(r => run(r, steady(r === 0 ? 1 : 0), steady(1)));
    const gate = sourcePerformanceGate(samples, accepted);
    expect(gate.workloads['h264-320']!.medians.native.scrubP95).toBe(8.3);
    expect(gate.failures).toEqual(expect.arrayContaining([
      'h264-320: median scrubP99: native 16.7 > today 8.3',
      'h264-320: median over16: native 1 > today 0',
      'h264-320: 5 steady scrubs over 16 ms > today 1']));
  });

  it('fails a session measured under load instead of averaging it in', () => {
    const samples = [0, 1, 2, 3, 4].flatMap(r => run(r, quiet, quiet, [40, 20], r === 2 ? 30 : 2));
    expect(sourcePerformanceGate(samples, accepted).failures).toEqual(['session today run 2 round 0 at load 30.0 >= 12', 'session native run 2 round 0 at load 30.0 >= 12']);
  });

  it('gates each workload on its own', () => {
    const samples = [0, 1, 2, 3, 4].flatMap(r => [...run(r, quiet, quiet),
      ...run(r, quiet, [12, 8.3, 8.3, 8.3, 25]).map(s => ({ ...s, workload: 'hevc-4k' }))]);
    const failures = sourcePerformanceGate(samples, accepted).failures;
    expect(failures.length).toBeGreaterThan(0);
    expect(failures.every(f => f.startsWith('hevc-4k: '))).toBe(true);
  });

  it('refuses timings without this campaign\'s original-media correctness evidence', () => {
    const samples = [0, 1, 2, 3, 4].flatMap(r => run(r, quiet, quiet));
    expect(sourcePerformanceGate(samples, { ...accepted, evidence: null }).failures)
      .toEqual(['original-media correctness: no evidence for this campaign']);
    expect(sourcePerformanceGate(samples, { ...accepted, build: 'rebuilt' }).failures)
      .toEqual(['original-media correctness: evidence is for another build']);
  });
});
