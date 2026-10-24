/** The source performance gate's statistic (method, not threshold).
 *
 * One run is one alternating off/on/on/off cycle of app sessions. Scrub
 * latency is quantised to vsync (8.3 ms here), so one run against one run
 * is noise at that resolution. Per mode and run the gate computes scrub
 * P50 and P95, the filmstrip median and the cold first-filmstrip median,
 * then requires, with zero slack and at least five runs:
 *   - the median across runs of each native statistic <= today's;
 *   - every native run's scrub P95 <= today's worst run's scrub P95.
 * The pooled distributions are reported beside it. */
export type PerfSample = { run: number; enabled: boolean; round: number; copy: number; scrubMs: number[]; filmstripMs: number[] };
type Stats = { scrubP50: number; scrubP95: number; filmstripMedian: number; coldMedian: number };

export const MIN_RUNS = 5;
export const percentile = (values: number[], p: number) => [...values].sort((a, b) => a - b)[Math.round((values.length - 1) * p)]!;

function stats(samples: PerfSample[]): Stats {
  return {
    scrubP50: percentile(samples.flatMap(s => s.scrubMs), .5),
    scrubP95: percentile(samples.flatMap(s => s.scrubMs), .95),
    filmstripMedian: percentile(samples.flatMap(s => s.filmstripMs), .5),
    coldMedian: percentile(samples.map(s => s.filmstripMs[0]!), .5)
  };
}

export function sourcePerformanceGate(samples: PerfSample[]) {
  const runs = [...new Set(samples.map(s => s.run))].sort((a, b) => a - b);
  const perRun = runs.map(run => ({ run,
    legacy: stats(samples.filter(s => s.run === run && !s.enabled)),
    native: stats(samples.filter(s => s.run === run && s.enabled)) }));
  const keys = ['scrubP50', 'scrubP95', 'filmstripMedian', 'coldMedian'] as const;
  const median = (mode: 'legacy' | 'native') =>
    Object.fromEntries(keys.map(key => [key, percentile(perRun.map(r => r[mode][key]), .5)])) as Stats;
  const medians = { legacy: median('legacy'), native: median('native') };
  const worstLegacyP95 = Math.max(...perRun.map(r => r.legacy.scrubP95));
  const failures: string[] = [];
  if (runs.length < MIN_RUNS) failures.push(`${runs.length} runs; the gate needs ${MIN_RUNS}`);
  for (const key of keys)
    if (medians.native[key] > medians.legacy[key]) failures.push(`median ${key}: native ${medians.native[key]} > today ${medians.legacy[key]}`);
  for (const r of perRun)
    if (r.native.scrubP95 > worstLegacyP95) failures.push(`run ${r.run} scrub P95 ${r.native.scrubP95} > today's worst run ${worstLegacyP95}`);
  const pooled = { legacy: stats(samples.filter(s => !s.enabled)), native: stats(samples.filter(s => s.enabled)) };
  return { runs: runs.length, perRun, medians, worstLegacyP95, pooled, failures };
}
