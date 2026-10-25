/*
 * Admission for a window's asynchronous engine snapshots.
 *
 * Main serves at most `MAX_PER_WINDOW` snapshots per window at once and
 * refuses the rest (`refused`, main/engine/snapshots.ts). Today's
 * `render.renderFrameTo` and the app's captures never fail for being busy,
 * so the window queues its snapshots here instead of sending more than main
 * admits: the most urgent class first (a raster a viewer frame waits for,
 * then API and export frames, then the UI's idle thumbnails), in arrival
 * order within a class. On a large project the idle thumbnails and the
 * compatibility rasters of one frame took all eight slots, and an
 * extension's `renderFrameToAsync` failed (C1, the intermediate project).
 */
export type SlotClass = 'sync' | 'async' | 'idle';

/** Main's per-window limit (main/engine/snapshots.ts MAX_PER_WINDOW). */
export const SNAPSHOT_SLOTS = 8;

const RANK: Record<SlotClass, number> = { sync: 0, async: 1, idle: 2 };

export class SnapshotSlots {
  private busy = 0;
  private readonly waiting: Array<{ rank: number; seq: number; go: () => void }> = [];
  private seq = 0;

  constructor(private readonly capacity = SNAPSHOT_SLOTS) {}

  get inFlight(): number { return this.busy; }
  get queued(): number { return this.waiting.length; }

  /** Runs `fn` once a slot is free, and frees it when `fn` settles. */
  async run<T>(cls: SlotClass, fn: () => Promise<T>): Promise<T> {
    if (this.busy >= this.capacity || this.waiting.length) {
      await new Promise<void>((go) => {
        this.waiting.push({ rank: RANK[cls], seq: this.seq++, go });
        this.waiting.sort((a, b) => a.rank - b.rank || a.seq - b.seq);
      });
    } else this.busy++;
    try {
      return await fn();
    } finally {
      const next = this.waiting.shift();
      if (next) next.go(); // the slot passes on
      else this.busy--;
    }
  }
}
