/*
 * What the engine path costs the window's main thread, counted (always on,
 * a few integer adds per message): the work today's compositor did inside
 * its animation frame, which the engine must not exceed (bigrender round 3).
 *
 * - `entries`: main-thread callbacks into the engine path, by kind: `port`
 *   (one task per message from the engine), `turn` (the raster override's
 *   answers, one per animation frame while playing, render-transport.ts),
 *   `presented` (frame events for a presented frame), `ipc` (main's asks
 *   for export frames);
 * - `posted` and `received`: messages to and from the engine on the
 *   window's port, and their bytes;
 * - `presented`: frames the engine connection announced as on screen.
 *
 * Counts, unlike times, do not move with the machine's load: the per-frame
 * ratios (`perFrame`) are what the deterministic budget test
 * (main-thread-budget.test.ts) holds to its checked-in baseline, and what
 * the measurement harness reads around playback (`PM.EngineCounters`).
 * `ms` is the main-thread time of each kind (informational).
 */
export type EntryKind = 'port' | 'turn' | 'presented' | 'ipc';

export type CountersSnapshot = {
  entries: Record<EntryKind, number>;
  ms: Record<EntryKind, number>;
  posted: number;
  postedBytes: number;
  received: number;
  receivedBytes: number;
  presented: number;
};

const kinds = (): Record<EntryKind, number> => ({ port: 0, turn: 0, presented: 0, ipc: 0 });

export class MainThreadCounters {
  private readonly c: CountersSnapshot = { entries: kinds(), ms: kinds(), posted: 0, postedBytes: 0, received: 0, receivedBytes: 0, presented: 0 };

  constructor(private readonly now: () => number = () => performance.now()) {}

  /** Runs `fn` as one main-thread entry of `kind`, timed. */
  entry<T>(kind: EntryKind, fn: () => T): T {
    this.c.entries[kind]++;
    const began = this.now();
    try {
      return fn();
    } finally {
      this.c.ms[kind] += this.now() - began;
    }
  }

  posted(bytes: number): void {
    this.c.posted++;
    this.c.postedBytes += bytes;
  }

  received(bytes: number): void {
    this.c.received++;
    this.c.receivedBytes += bytes;
  }

  presentedFrame(): void {
    this.c.presented++;
  }

  snapshot(): CountersSnapshot {
    return { ...this.c, entries: { ...this.c.entries }, ms: { ...this.c.ms } };
  }
}

/** Per presented frame between two snapshots (`after` minus `before`). */
export function perFrame(before: CountersSnapshot, after: CountersSnapshot): Record<string, number> | null {
  const frames = after.presented - before.presented;
  if (frames <= 0) return null;
  const per = (n: number): number => Math.round((n / frames) * 1000) / 1000;
  const entries = (Object.keys(after.entries) as EntryKind[]).reduce((sum, k) => sum + after.entries[k] - before.entries[k], 0);
  const ms = (Object.keys(after.ms) as EntryKind[]).reduce((sum, k) => sum + after.ms[k] - before.ms[k], 0);
  return {
    frames,
    entries: per(entries),
    portEntries: per(after.entries.port - before.entries.port),
    turns: per(after.entries.turn - before.entries.turn),
    posted: per(after.posted - before.posted),
    postedBytes: per(after.postedBytes - before.postedBytes),
    received: per(after.received - before.received),
    receivedBytes: per(after.receivedBytes - before.receivedBytes),
    ms: per(ms)
  };
}

/** The window's counters (one engine connection per window). */
export const mainThreadCounters = new MainThreadCounters();
