/*
 * Overlay geometry at the presented frame's revision (plan section 18,
 * item 6: "handles never lead the pixels during gestures").
 *
 * While the engine presents, the document can be a revision or two ahead
 * of the frame on screen (a drag's edit is in flight to the engine). The
 * viewer records the selection geometry it computed for each document
 * revision it drew at, and draws the geometry of the newest recorded
 * revision the presented frame includes, so the box and handles move with
 * the pixels, never ahead of them. Geometry is recorded per key (the
 * selected layers and the time), so a selection change or a seek never
 * shows another selection's box.
 */

export class GeometryAtRevision<T> {
  private entries: { revision: number; key: string; value: T }[] = [];

  constructor(private readonly capacity = 32) {}

  /** The geometry `value` holds at document `revision` for `key`. */
  record(revision: number, key: string, value: T): void {
    const last = this.entries.at(-1);
    if (last && last.revision === revision && last.key === key) {
      last.value = value;
      return;
    }
    // Revisions only grow; an older one means the history restarted (a resync of a new mirror).
    if (last && revision < last.revision) this.entries = [];
    this.entries.push({ revision, key, value });
    if (this.entries.length > this.capacity) this.entries.shift();
  }

  /**
   * What to draw for `key` when the frame on screen shows `frameRevision`:
   * the newest geometry recorded at or before it, or `current` when the
   * frame is up to date or nothing that old is recorded for this key.
   */
  at(frameRevision: number | null, key: string, current: T): { value: T; revision: number | null } {
    const last = this.entries.at(-1);
    if (frameRevision === null || !last || frameRevision >= last.revision) return { value: current, revision: last?.revision ?? null };
    for (let i = this.entries.length - 1; i >= 0; i--) {
      const entry = this.entries[i]!;
      if (entry.key !== key) return { value: current, revision: last.revision };
      if (entry.revision <= frameRevision) return { value: entry.value, revision: entry.revision };
    }
    return { value: current, revision: last.revision };
  }

  clear(): void {
    this.entries = [];
  }
}
