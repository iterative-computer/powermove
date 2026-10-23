/* The desktop harness's part of engine/crates/pm-perfbench/baselines.json.
 * It uses the same units and down-only update rule as the Rust ratchet. */

export const MAIN_THREAD_RUNNER = 'desktop-vitest';

export type MainThreadBench = {
  runner: string;
  units: number;
  unit: string;
  thread: Record<string, number>;
  all_threads: Record<string, number>;
};

export type PerfBaselines = {
  benches: Record<string, MainThreadBench>;
  [key: string]: unknown;
};

export function compareAndLowerMainThread(
  file: PerfBaselines,
  name: string,
  units: number,
  counts: Record<string, number>,
  update: boolean
): { next: PerfBaselines; regressions: string[] } {
  const previous = file.benches[name];
  const regressions: string[] = [];
  if (previous && (previous.runner !== MAIN_THREAD_RUNNER || previous.units !== units || previous.unit !== 'frames')) {
    regressions.push(`${name}: measured ${units} frames with ${MAIN_THREAD_RUNNER}, baseline is ${previous.units} ${previous.unit} with ${previous.runner}`);
    return { next: file, regressions };
  }
  for (const [counter, count] of Object.entries(counts)) {
    if (!Number.isSafeInteger(count) || count < 0) throw new Error(`${name} ${counter}: expected a nonnegative integer count`);
    const held = previous?.thread[counter];
    if (!previous || count > (held ?? 0)) regressions.push(`${name} ${counter}: ${count} > baseline ${held ?? (previous ? 0 : 'missing')}`);
  }
  if (previous) {
    for (const counter of Object.keys(previous.thread)) {
      if (!(counter in counts)) regressions.push(`${name} ${counter}: measured counter is missing`);
    }
  }
  if (!update) return { next: file, regressions };
  const thread: Record<string, number> = previous ? { ...previous.thread } : {};
  for (const [counter, count] of Object.entries(counts)) {
    const held = thread[counter];
    if ((!previous || held !== undefined) && (held === undefined || count < held)) thread[counter] = count;
  }
  return {
    next: {
      ...file,
      benches: {
        ...file.benches,
        [name]: previous ? { ...previous, thread } : { runner: MAIN_THREAD_RUNNER, units, unit: 'frames', thread, all_threads: {} }
      }
    },
    regressions: previous ? regressions : []
  };
}
