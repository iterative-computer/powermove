/** Opt-in, bounded timings on the page clock. No document content or IDs. */
export type RenderTraceEvent = { stage: string; at: number; ms: number; revision: number; time: number; count?: number };

export class RenderPerfTrace {
  enabled = false;
  private events: RenderTraceEvent[] = [];
  private cursor = 0;

  constructor(private readonly capacity = 8192) {}

  record(stage: string, revision: number, time = 0, began = performance.now(), count?: number): void {
    if (!this.enabled) return;
    const event = { stage, at: began, ms: performance.now() - began, revision, time, ...(count === undefined ? {} : { count }) };
    if (this.events.length < this.capacity) this.events.push(event);
    else { this.events[this.cursor] = event; this.cursor = (this.cursor + 1) % this.capacity; }
  }

  take(): RenderTraceEvent[] {
    const events = [...this.events.slice(this.cursor), ...this.events.slice(0, this.cursor)];
    this.events = []; this.cursor = 0;
    return events;
  }
}
