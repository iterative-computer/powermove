/*
 * Display timing from requestAnimationFrame timestamps: one refresh grid
 * that the viewer's decisions and the engine's vsync hint both follow
 * (docs/engine/bridge.md, "Pacing at 120 Hz").
 *
 * rAF timestamps are BeginFrame times, coarsened, jittered by about a
 * millisecond, and on an idle machine often up to 2 ms early; a busy
 * renderer skips some. So:
 *
 * - the period is the display's nominal refresh interval when known, else
 *   a line through refresh indices inferred from the median interval;
 * - the phase is locked once from the median of the first samples, then
 *   tracked like a phase-locked loop: each sample within a quarter period
 *   of the grid nudges it by a sixteenth of its error, clamped to a
 *   sixteenth of a period, so jitter and early BeginFrames move it by
 *   microseconds, never by a refresh;
 * - a real phase change of the display link (after a resize or an idle
 *   gap) is a dozen consecutive samples outside that band that agree: the
 *   lock moves to them at once (`jumped`).
 */

export type VsyncFit = {
  periodMs: number;
  /** A vsync time (performance.now ms) on the fitted grid. */
  vsyncMs: number;
  /** The lock moved to a new phase at this sample. */
  jumped: boolean;
  medianResidualMs: number;
  samples: number;
};

const WINDOW = 240;
const RECENT = 12;
/** Loop gain and the largest move one sample makes, as fractions of a period. */
const GAIN = 1 / 16;
const STEP = 1 / 16;

const median = (xs: number[]): number => {
  const sorted = [...xs].sort((a, b) => a - b);
  return sorted[sorted.length >> 1] ?? 0;
};

export class VsyncEstimator {
  private samples: number[] = [];
  private fitted: VsyncFit | null = null;
  private lastPeriodMs = -Infinity;
  /** Consecutive errors outside the lock band (a phase change, if they agree). */
  private outside: number[] = [];
  private residuals: number[] = [];

  /** `nominalHz`: the display's refresh rate when the host knows it. */
  constructor(private nominalHz: number | null = null) {}

  setNominalHz(hz: number | null): void {
    this.nominalHz = hz && hz > 0 ? hz : null;
    this.fitted = null;
  }

  /** Records one rAF timestamp; returns the grid after it. */
  sample(rafMs: number): VsyncFit | null {
    this.samples.push(rafMs);
    if (this.samples.length > WINDOW * 5) this.samples.splice(0, this.samples.length - WINDOW);
    if (!this.fitted) {
      if (this.samples.length < 24) return null;
      this.fitted = this.fit();
      this.lastPeriodMs = rafMs;
      return this.fitted;
    }
    let { periodMs, vsyncMs } = this.fitted;
    // Without a nominal rate the period is refined from the window now and
    // then; the phase carries over on the same grid.
    if (!this.nominalHz && rafMs - this.lastPeriodMs > 500) {
      periodMs = this.period(this.samples.slice(-WINDOW));
      this.lastPeriodMs = rafMs;
    }
    const error = this.fold(rafMs, vsyncMs, periodMs);
    let jumped = false;
    if (Math.abs(error) <= periodMs / 4) {
      this.outside = [];
      vsyncMs += GAIN * Math.max(-STEP * periodMs, Math.min(STEP * periodMs, error));
      this.residuals.push(Math.abs(error));
      if (this.residuals.length > RECENT * 2) this.residuals.shift();
    } else {
      this.outside.push(error);
      // Around the first of them (circularly: a half-period jump folds
      // either way).
      const first = this.outside[0]!;
      const around = this.outside.map((e) => this.fold(e, first, periodMs));
      const spread = Math.max(...around) - Math.min(...around);
      if (this.outside.length >= RECENT && spread < periodMs / 4) {
        vsyncMs += first + median(around);
        this.outside = [];
        jumped = true;
      } else if (this.outside.length >= RECENT) {
        this.outside.shift();
      }
    }
    // Keep the anchor near the newest sample.
    vsyncMs += Math.round((rafMs - vsyncMs) / periodMs) * periodMs;
    this.fitted = { periodMs, vsyncMs, jumped, medianResidualMs: median(this.residuals), samples: Math.min(this.samples.length, WINDOW) };
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
    const approximate = median(deltas);
    if (!(approximate > 0)) return 1000 / 60;
    // Adjacent timestamps are coarsened (120 Hz becomes mostly 8.3 ms,
    // with occasional 8.4 ms). Their median loses the fractional period:
    // infer skipped refreshes first, then fit across the whole window so
    // rounding error cannot accumulate.
    let index = 0;
    const indices = [0];
    const times = [0];
    let elapsed = 0;
    for (const delta of deltas) {
      const steps = Math.max(1, Math.round(delta / approximate));
      index += steps;
      // A display-link phase jump changes phase, not frequency.
      const residual = delta - steps * approximate;
      elapsed += Math.abs(residual) > approximate / 4 ? steps * approximate : delta;
      indices.push(index);
      times.push(elapsed);
    }
    const mx = indices.reduce((a, b) => a + b, 0) / indices.length;
    const my = times.reduce((a, t) => a + t, 0) / times.length;
    let covariance = 0, variance = 0;
    for (let i = 0; i < recent.length; i++) {
      const x = indices[i]! - mx;
      covariance += x * (times[i]! - my);
      variance += x * x;
    }
    return variance > 0 ? covariance / variance : approximate;
  }

  private fold(t: number, ref: number, periodMs: number): number {
    let x = (((t - ref) % periodMs) + periodMs) % periodMs;
    if (x > periodMs / 2) x -= periodMs;
    return x;
  }

  /** The phase from the median of the window (the initial lock). */
  private fit(): VsyncFit {
    const recent = this.samples.slice(-WINDOW);
    const periodMs = this.period(recent);
    const ref = recent[recent.length - 1]!;
    const phase = median(recent.map((t) => this.fold(t, ref, periodMs)));
    return { periodMs, vsyncMs: ref + phase, jumped: false, medianResidualMs: 0, samples: recent.length };
  }
}

/** The vsync at or after `t` on a fitted grid. */
export function nextVsync(fit: VsyncFit, t: number): number {
  const n = Math.ceil((t - fit.vsyncMs) / fit.periodMs - 1e-9);
  return fit.vsyncMs + n * fit.periodMs;
}

/**
 * When to tell the engine about the refresh grid again (amendment 18's
 * vsync-phase hint): the engine plans renders and a Play's start on it, so
 * it follows the viewer's grid continuously: when it is first known, when
 * the period changes by more than 0.1%, or when the phase moves by more
 * than `PHASE_TOLERANCE_US`, at most every `HINT_INTERVAL_MS` unless the
 * lock moved to a new phase.
 */
export const PHASE_TOLERANCE_US = 100;
export const HINT_INTERVAL_MS = 250;

export function hintChanged(last: { hostTime: number; periodUs: number } | null, next: { hostTime: number; periodUs: number }): boolean {
  if (!last) return true;
  if (Math.abs(next.periodUs - last.periodUs) > last.periodUs * 0.001) return true;
  let d = (next.hostTime - last.hostTime) % next.periodUs;
  if (d > next.periodUs / 2) d -= next.periodUs;
  if (d < -next.periodUs / 2) d += next.periodUs;
  return Math.abs(d) > PHASE_TOLERANCE_US;
}
