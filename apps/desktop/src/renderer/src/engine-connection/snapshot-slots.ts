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
/** Main's limit of result bytes outstanding, which it reserves at each
 *  request's output size (main/engine/snapshots.ts MAX_OUTSTANDING_BYTES):
 *  a layer raster reserves its largest canvas, 8192², so only four fit.
 *  Waiting here, in order, replaces a refusal and its retry backoff (a
 *  playback base waited 0.5 to 1.5 s on them, measured). */
export const SNAPSHOT_BYTES = 1024 ** 3;

const RANK: Record<SlotClass, number> = { sync: 0, async: 1, idle: 2 };

export class SnapshotSlots {
  private busy = 0;
  private bytes = 0;
  private readonly waiting: Array<{ rank: number; seq: number; bytes: number; go: () => void }> = [];
  private seq = 0;

  constructor(private readonly capacity = SNAPSHOT_SLOTS, private readonly maxBytes = SNAPSHOT_BYTES) {}

  get inFlight(): number { return this.busy; }
  get queued(): number { return this.waiting.length; }

  /** Whether a request of `bytes` fits beside those in flight (one always does). */
  private fits(bytes: number): boolean {
    return this.busy < this.capacity && (this.busy === 0 || this.bytes + bytes <= this.maxBytes);
  }

  /** Runs `fn` once a slot and its result `bytes` are free, and frees them when `fn` settles. */
  async run<T>(cls: SlotClass, fn: () => Promise<T>, bytes = 0): Promise<T> {
    if (this.waiting.length || !this.fits(bytes)) {
      await new Promise<void>((go) => {
        this.waiting.push({ rank: RANK[cls], seq: this.seq++, bytes, go });
        this.waiting.sort((a, b) => a.rank - b.rank || a.seq - b.seq);
      });
    } else {
      this.busy++;
      this.bytes += bytes;
    }
    try {
      return await fn();
    } finally {
      this.busy--;
      this.bytes -= bytes;
      // The queue's order holds: its head goes first, as soon as it fits.
      while (this.waiting.length && this.fits(this.waiting[0]!.bytes)) {
        const next = this.waiting.shift()!;
        this.busy++;
        this.bytes += next.bytes;
        next.go();
      }
    }
  }
}
