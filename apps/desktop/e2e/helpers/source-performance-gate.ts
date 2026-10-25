/** The source performance gate's statistic (method, not threshold).
 *
 * One run is one alternating off/on/on/off cycle of app sessions; each
 * workload is gated on its own. Scrub latency is quantised to vsync, so one
 * run against one run is noise at that resolution. Per mode and run the gate
 * computes, from each copy's first seek after opening (startup excluded) and its steady scrubs
 * (every later one): first-scrub median, steady P50, P95 and P99, the count
 * of steady scrubs over one 60 Hz frame (16 ms), the filmstrip median and the
 * cold first-filmstrip median. With zero slack and at least five runs:
 *   - the median across runs of each native statistic <= today's;
 *   - the pooled count over 16 ms, native <= today's;
 *   - every native run's steady P95 <= today's worst run's, within this
 *     invocation;
 *   - every session started under the load limit (a loaded session is no
 *     measurement, so it fails rather than averages in);
 *   - the campaign's untimed original-media correctness evidence
 *     (source-frame-correctness.ts) covers this build and exactly the measured
 *     fixtures, targets and geometry, and every displayed frame matched. */
import { correctnessFailures, type CampaignFixture, type SourceFrameCorrectness } from './source-frame-correctness';

export type PerfSample = {
  workload: string; run: number; enabled: boolean; round: number; copy: number;
  scrubMs: number[]; filmstripMs: number[]; loadBefore: number[];
};
type Stats = {
  firstScrub: number; scrubP50: number; scrubP95: number; scrubP99: number; over16: number;
  filmstripMedian: number; coldMedian: number;
};

export const MIN_RUNS = 5;
export const MAX_LOAD = 12;
export const FRAME_MS = 16;
export const percentile = (values: number[], p: number) => [...values].sort((a, b) => a - b)[Math.round((values.length - 1) * p)]!;
const KEYS = ['firstScrub', 'scrubP50', 'scrubP95', 'scrubP99', 'over16', 'filmstripMedian', 'coldMedian'] as const;

function stats(samples: PerfSample[]): Stats {
  const steady = samples.flatMap(s => s.scrubMs.slice(1));
  return {
    firstScrub: percentile(samples.map(s => s.scrubMs[0]!), .5),
    scrubP50: percentile(steady, .5),
    scrubP95: percentile(steady, .95),
    scrubP99: percentile(steady, .99),
    over16: steady.filter(ms => ms > FRAME_MS).length,
    filmstripMedian: percentile(samples.flatMap(s => s.filmstripMs), .5),
    coldMedian: percentile(samples.map(s => s.filmstripMs[0]!), .5)
  };
}

function gateWorkload(workload: string, samples: PerfSample[], failures: string[]) {
  const runs = [...new Set(samples.map(s => s.run))].sort((a, b) => a - b);
  const perRun = runs.map(run => ({ run,
    legacy: stats(samples.filter(s => s.run === run && !s.enabled)),
    native: stats(samples.filter(s => s.run === run && s.enabled)) }));
  const median = (mode: 'legacy' | 'native') =>
    Object.fromEntries(KEYS.map(key => [key, percentile(perRun.map(r => r[mode][key]), .5)])) as Stats;
  const medians = { legacy: median('legacy'), native: median('native') };
  const pooled = { legacy: stats(samples.filter(s => !s.enabled)), native: stats(samples.filter(s => s.enabled)) };
  const worstLegacyP95 = Math.max(...perRun.map(r => r.legacy.scrubP95));
  if (runs.length < MIN_RUNS) failures.push(`${workload}: ${runs.length} runs; the gate needs ${MIN_RUNS}`);
  for (const key of KEYS)
    if (medians.native[key] > medians.legacy[key]) failures.push(`${workload}: median ${key}: native ${medians.native[key]} > today ${medians.legacy[key]}`);
  if (pooled.native.over16 > pooled.legacy.over16)
    failures.push(`${workload}: ${pooled.native.over16} steady scrubs over ${FRAME_MS} ms > today ${pooled.legacy.over16}`);
  for (const r of perRun)
    if (r.native.scrubP95 > worstLegacyP95) failures.push(`${workload}: run ${r.run} scrub P95 ${r.native.scrubP95} > today's worst run ${worstLegacyP95}`);
  return { runs: runs.length, perRun, medians, pooled, worstLegacyP95 };
}

export type CampaignCorrectness = { evidence: SourceFrameCorrectness | null; campaign: string; build: string; fixtures: CampaignFixture[] };

export function sourcePerformanceGate(samples: PerfSample[], correctness: CampaignCorrectness) {
  const failures = correctnessFailures(correctness.evidence, correctness.campaign, correctness.build, correctness.fixtures);
  const loaded = new Set(samples.filter(s => s.loadBefore[0]! >= MAX_LOAD)
    .map(s => `${s.enabled ? 'native' : 'today'} run ${s.run} round ${s.round} at load ${s.loadBefore[0]!.toFixed(1)}`));
  for (const session of loaded) failures.push(`session ${session} >= ${MAX_LOAD}`);
  const workloads = Object.fromEntries([...new Set(samples.map(s => s.workload))]
    .map(workload => [workload, gateWorkload(workload, samples.filter(s => s.workload === workload), failures)]));
  return { workloads, failures };
}
