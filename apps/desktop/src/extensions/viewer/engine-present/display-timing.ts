/*
 * Display timing from requestAnimationFrame timestamps (S1's tuned viewer,
 * docs/engine/spikes/S1.md "Display-link phase").
 *
 * rAF timestamps are BeginFrame times, coarsened and jittered by about a
 * millisecond, and a busy renderer skips some, so a line fitted through
 * them is pulled off by outliers. Instead:
 *
 * - the period is the display's nominal refresh interval when known, else
 *   the median interval;
 * - the phase is the median of every sample's offset from the newest one,
 *   folded into one period;
 * - the display link's phase can jump by about half a period (after a
 *   resize or an idle gap): the newest 12 samples win when they disagree
 *   with the long median by more than a quarter period.
 */

export type VsyncFit = {
  periodMs: number;
  /** A vsync time (performance.now ms) on the fitted grid. */
  vsyncMs: number;
  /** The newest samples moved the phase by more than a quarter period. */
  jumped: boolean;
  medianResidualMs: number;
  samples: number;
};

const WINDOW = 240;
const RECENT = 12;

const median = (xs: number[]): number => {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[sorted.length >> 1] ?? 0;
};

export class VsyncEstimator {
  private samples: number[] = [];
  private fitted: VsyncFit | null = null;
  private lastFitMs = -Infinity;

  /** `nominalHz`: the display's refresh rate when the host knows it. */
  constructor(private nominalHz: number | null = null) {}

  setNominalHz(hz: number | null): void {
    this.nominalHz = hz && hz > 0 ? hz : null;
    this.fitted = null;
  }

  /** Records one rAF timestamp; refits every 500 ms, or at once on a phase jump. */
  sample(rafMs: number): VsyncFit | null {
    this.samples.push(rafMs);
    if (this.samples.length > WINDOW * 5) this.samples.splice(0, this.samples.length - WINDOW);
    if (this.samples.length < 24) return this.fitted;
    const due = rafMs - this.lastFitMs > 500 || (rafMs - this.lastFitMs > 100 && this.phaseJumped());
    if (due) {
      this.fitted = this.fit();
      this.lastFitMs = rafMs;
    }
    return this.fitted;
  }

  /** The current fit without new samples. */
  current(): VsyncFit | null {
    return this.fitted;
  }

  private period(recent: number[]): number {
    if (this.nominalHz) return 1000 / this.nominalHz;
    const deltas: number[] = [];
    for (let i = 1; i < recent.length; i++) deltas.push(recent[i]! - recent[i - 1]!);
    return median(deltas);
  }

  private fold(t: number, ref: number, periodMs: number): number {
    let x = (((t - ref) % periodMs) + periodMs) % periodMs;
    if (x > periodMs / 2) x -= periodMs;
    return x;
  }

  fit(): VsyncFit {
    const recent = this.samples.slice(-WINDOW);
    const periodMs = this.period(recent);
    const ref = recent[recent.length - 1]!;
    const offsets = recent.map((t) => this.fold(t, ref, periodMs));
    let phase = median(offsets);
    const shortPhase = median(offsets.slice(-RECENT));
    let d = shortPhase - phase;
    if (d > periodMs / 2) d -= periodMs;
    if (d < -periodMs / 2) d += periodMs;
    const jumped = Math.abs(d) > periodMs / 4;
    if (jumped) phase = shortPhase;
    const residuals = offsets.slice(jumped ? -RECENT : 0).map((x) => Math.abs(x - phase));
    return { periodMs, vsyncMs: ref + phase, jumped, medianResidualMs: median(residuals), samples: recent.length };
  }

  /** True when the newest samples sit more than a quarter period off the fit. */
  private phaseJumped(): boolean {
    const f = this.fitted;
    if (!f || this.samples.length < RECENT) return false;
    const offsets = this.samples.slice(-RECENT).map((t) => this.fold(t, f.vsyncMs, f.periodMs));
    return Math.abs(median(offsets)) > f.periodMs / 4;
  }
}

/** The vsync at or after `t` on a fitted grid. */
export function nextVsync(fit: VsyncFit, t: number): number {
  const n = Math.ceil((t - fit.vsyncMs) / fit.periodMs - 1e-9);
  return fit.vsyncMs + n * fit.periodMs;
}

/**
 * When to tell the engine about the refresh grid again (amendment 18's
 * vsync-phase hint). The hint moves the engine's render starts, so it is
 * sent when it is first known, when the period changes by more than 0.1%,
 * or when the phase moves by more than `PHASE_TOLERANCE_US`, and not more
 * than once a second unless the display link's phase jumped.
 */
export const PHASE_TOLERANCE_US = 250;
export const HINT_INTERVAL_MS = 1000;

export function hintChanged(last: { hostTime: number; periodUs: number } | null, next: { hostTime: number; periodUs: number }): boolean {
  if (!last) return true;
  if (Math.abs(next.periodUs - last.periodUs) > last.periodUs * 0.001) return true;
  let d = (next.hostTime - last.hostTime) % next.periodUs;
  if (d > next.periodUs / 2) d -= next.periodUs;
  if (d < -next.periodUs / 2) d += next.periodUs;
  return Math.abs(d) > PHASE_TOLERANCE_US;
}
