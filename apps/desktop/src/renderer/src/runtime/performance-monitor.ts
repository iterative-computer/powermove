export interface PerformanceIssue {
  id: string;
  name: string;
  kind: 'extension' | 'shader' | 'effect';
  ms: number;
  samples: number;
  at: number;
  layerId?: string;
}

/** Bounded telemetry polled by the UI; render callbacks never write runes. */
export class PerformanceMonitor {
  private entries = new Map<string, PerformanceIssue>();
  record(info: Omit<PerformanceIssue, 'ms' | 'samples' | 'at'>, ms: number, now = performance.now()): void {
    if (!Number.isFinite(ms) || ms < 0) return;
    const previous = this.entries.get(info.id);
    const recent = previous && now - previous.at < 15_000;
    this.entries.delete(info.id);
    this.entries.set(info.id, { ...info, ms: recent ? previous.ms * .7 + ms * .3 : ms,
      samples: recent ? previous.samples + 1 : 1, at: now });
    while (this.entries.size > 128) this.entries.delete(this.entries.keys().next().value!);
  }
  issues(now = performance.now()): PerformanceIssue[] {
    return [...this.entries.values()].filter(item => now - item.at < 15_000 &&
      (item.ms >= 50 || item.samples >= 3 && item.ms >= 8)).sort((a, b) => b.ms - a.ms);
  }
  clear(id?: string): void { if (id) this.entries.delete(id); else this.entries.clear(); }
}
export const performanceMonitor = new PerformanceMonitor();
