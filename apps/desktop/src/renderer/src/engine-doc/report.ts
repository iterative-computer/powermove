/*
 * Reporting for the native evaluator's test modes: one `[pm-native]` line per
 * event, through the host log (main's stderr in the desktop app, so a test
 * harness can collect them) and the console. Events are the first mismatches
 * of each dual-answer operation, auditor failures, consistency-check
 * mismatches, Proxy violations, and a periodic summary.
 */
export type ReportEvent =
  | { type: 'dual'; op: string; path: string; where: Record<string, unknown>; native: string; legacy: string }
  | { type: 'audit'; kind: string; revision: number; paths: string[]; detail: string }
  | { type: 'consistency'; reason: string; revision: number; missed: string[]; ms: number }
  | { type: 'violation'; path: string; site: string; count: number }
  | { type: 'summary'; [key: string]: unknown }
  | { type: 'ready' | 'dead' | 'error'; [key: string]: unknown };

export type Sink = (line: string) => void;

export function hostSink(): Sink {
  const bridge = (globalThis as any).powermove;
  return (line: string) => {
    try { bridge?.log?.('warn', line); } catch { /* the bridge is gone */ }
    console.warn(line);
  };
}

export class Reporter {
  private lines = 0;
  constructor(private sink: Sink, private limit = 2000) {}

  emit(event: ReportEvent): void {
    if (this.lines >= this.limit && event.type !== 'summary') return;
    this.lines += 1;
    let text: string;
    try { text = JSON.stringify(event); } catch { text = String(event.type); }
    this.sink(`[pm-native] ${text.length > 4000 ? text.slice(0, 4000) + '…' : text}`);
  }
}
