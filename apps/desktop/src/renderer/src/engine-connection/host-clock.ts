/*
 * performance.now() on the engines' host clock (contract 9 `HostMicros`:
 * `mach_absolute_time` in µs, which Chromium's monotonic clock also reads).
 *
 * The two differ by a constant per page. Main answers `hostTime` with its
 * own reading; the offset is taken from the round trip with the least
 * delay (its error is at most half that round trip), refreshed now and
 * then because a page's clock and the host's never drift but a busy main
 * thread makes single samples noisy.
 */

export type HostTimeSource = () => Promise<number>;

/** One measurement: the offset and the round trip it came from. */
export type OffsetSample = { offsetUs: number; rttUs: number };

/** The offset (host µs − performance.now() µs) of the best of `samples`. */
export function bestOffset(samples: readonly OffsetSample[]): OffsetSample | null {
  let best: OffsetSample | null = null;
  for (const s of samples) if (Number.isFinite(s.offsetUs) && (!best || s.rttUs < best.rttUs)) best = s;
  return best;
}

export class HostClock {
  private best: OffsetSample | null = null;
  private running: Promise<void> | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(private readonly hostTime: HostTimeSource, private readonly now: () => number = () => performance.now()) {}

  /** host µs − performance.now() µs, once measured. */
  offsetUs(): number | null {
    return this.best ? this.best.offsetUs : null;
  }

  /** The round trip of the measurement in use (its error is at most half). */
  errorUs(): number | null {
    return this.best ? this.best.rttUs / 2 : null;
  }

  /** performance.now() ms as host µs. */
  hostUs(ms: number = this.now()): number | null {
    return this.best ? ms * 1000 + this.best.offsetUs : null;
  }

  /** Host µs as performance.now() ms. */
  pageMs(hostUs: number): number | null {
    return this.best ? (hostUs - this.best.offsetUs) / 1000 : null;
  }

  /** Measures `rounds` round trips and keeps the best seen so far. */
  measure(rounds = 8): Promise<void> {
    if (this.running) return this.running;
    this.running = (async () => {
      const samples: OffsetSample[] = [];
      for (let i = 0; i < rounds; i++) {
        const before = this.now();
        let host: number;
        try {
          host = await this.hostTime();
        } catch {
          break;
        }
        const after = this.now();
        if (!Number.isFinite(host)) break;
        samples.push({ offsetUs: host - ((before + after) / 2) * 1000, rttUs: (after - before) * 1000 });
      }
      const found = bestOffset(this.best ? [this.best, ...samples] : samples);
      // A fresh round may only improve the estimate (the offset is constant).
      if (found) this.best = found;
    })().finally(() => {
      this.running = null;
    });
    return this.running;
  }

  /** Measures now and every `everyMs`. */
  start(everyMs = 10_000): void {
    void this.measure();
    if (!this.timer) this.timer = setInterval(() => void this.measure(4), everyMs);
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }
}
