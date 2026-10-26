/** One writer per window; each project owns its debounce and acknowledged revision. */
export type SaveOutcome = boolean | 'cancelled';
export type SavePhase = 'pending' | 'saving' | 'saved' | 'failure';
type Entry = { revision: number; persisted: number; first: number; due: number; phase: SavePhase; error?: string };
export class AutosaveQueue {
  private entries = new Map<string, Entry>();
  private timer?: ReturnType<typeof setTimeout>;
  private tail: Promise<unknown> = Promise.resolve();
  private queued = new Map<string, number>();
  constructor(private readonly write: (id: string, explicit: boolean) => Promise<SaveOutcome>,
    private readonly changed: (id: string, state: Readonly<Entry>) => void,
    private readonly ready: (id: string) => boolean = () => true,
    private readonly delay = 750, private readonly maximum = 5000,
    private readonly clock = { setTimeout, clearTimeout }) {}
  edit(id: string): void {
    const now = Date.now(), old = this.entries.get(id);
    const entry: Entry = old || { revision: 0, persisted: 0, first: now, due: now, phase: 'pending' };
    if (entry.revision === entry.persisted || entry.phase === 'failure') entry.first = now;
    entry.revision++; entry.due = Math.min(now + this.delay, entry.first + this.maximum);
    entry.phase = this.queued.has(id) ? 'saving' : 'pending'; entry.error = undefined;
    this.entries.set(id, entry); this.changed(id, entry); this.arm();
  }
  forget(id: string): void { this.entries.delete(id); this.arm(); }
  private arm(): void {
    this.clock.clearTimeout(this.timer); this.timer = undefined;
    const waiting = [...this.entries].filter(([id, e]) => e.revision > e.persisted && e.phase !== 'failure' && !this.queued.has(id));
    if (!waiting.length) return;
    const due = Math.min(...waiting.map(([, e]) => e.due));
    this.timer = this.clock.setTimeout(() => {
      this.timer = undefined;
      for (const [id, e] of waiting) if (e.due <= Date.now()) {
        // A drag is a transaction. Persist its completed patch on release.
        if (!this.ready(id)) { e.due = Date.now() + 100; continue; }
        void this.enqueue(id, false);
      }
      this.arm();
    }, Math.max(0, due - Date.now()));
  }
  private enqueue(id: string, explicit: boolean, write?: () => Promise<SaveOutcome>): Promise<boolean> {
    if (!explicit && this.queued.has(id)) return Promise.resolve(false);
    this.queued.set(id, (this.queued.get(id) || 0) + 1);
    const operation = this.tail.then(async () => {
      let entry = this.entries.get(id);
      if (!entry && explicit) { entry = { revision: 0, persisted: 0, first: Date.now(), due: Date.now(), phase: 'pending' }; this.entries.set(id, entry); }
      if (!entry && !explicit) return false;
      const revision = entry?.revision ?? 0;
      if (entry) { entry.phase = 'saving'; this.changed(id, entry); }
      let ok = false, cancelled = false;
      try { const outcome = await (write ? write() : this.write(id, explicit)); ok = outcome === true; cancelled = outcome === 'cancelled'; }
      catch (error) { if (entry) entry.error = error instanceof Error ? error.message : String(error); }
      if (entry && this.entries.get(id) === entry) {
        if (ok) { entry.persisted = revision; entry.phase = entry.revision === revision ? 'saved' : 'pending'; }
        else entry.phase = cancelled ? (entry.persisted === entry.revision ? 'saved' : 'pending') : 'failure';
        this.changed(id, entry);
      }
      return ok;
    });
    this.tail = operation.then(() => undefined, () => undefined);
    return operation.finally(() => { const count = (this.queued.get(id) || 1) - 1; if (count) this.queued.set(id, count); else this.queued.delete(id); this.arm(); });
  }
  /** Cmd+S waits behind an active writer, then captures the latest document. */
  flush(id: string, write?: () => Promise<SaveOutcome>): Promise<boolean> { return this.enqueue(id, true, write); }
  state(id: string): Readonly<Entry> | undefined { return this.entries.get(id); }
}
