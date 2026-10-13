/*
 * Reporting for the native evaluator's test modes: one `[pm-native]` line per
 * event, through the host log (main's stderr in the desktop app, so a test
 * harness can collect them) and the console. Events are the first mismatches
 * of each dual-answer operation, auditor failures, consistency-check
 * mismatches, Proxy violations, and a periodic summary.
 */
export type ReportEvent =
  | { type: 'dual'; op: string; path: string; where: Record<string, unknown>; native: string; legacy: string }
  | { type: 'audit'; kind: string; revision: number; paths: string[]; detail: string; site?: string }
  | { type: 'consistency'; reason: string; revision: number; missed: string[]; ms: number }
  | { type: 'violation'; path: string; site: string; count: number }
  | { type: 'summary'; [key: string]: unknown }
  | { type: 'ready' | 'dead' | 'error'; [key: string]: unknown };

import { bridge as hostBridge } from '../kernel/bridge';

export type Sink = (line: string) => void;

/** Below the host log's per-line limit (`LIMITS.logChars`, 8,000) with the prefix. */
const MAX_LINE = 7_900;

export function hostSink(): Sink {
  // The kernel takes the preload's bridge off `window` at boot
  // (kernel/capture-bridge.ts); the captured one is read per line.
  return (line: string) => {
    try { (hostBridge() as any)?.log?.('warn', line); } catch { /* the bridge is gone */ }
    console.warn(line);
  };
}

export class Reporter {
  private lines = 0;
  constructor(private sink: Sink, private limit = 2000) {}

  emit(event: ReportEvent): void {
    if (this.lines >= this.limit && event.type !== 'summary') return;
    this.lines += 1;
    // Wall-clock time, so a test run can attribute events to tests.
    event = { ...event, at: Date.now() } as ReportEvent;
    let text: string;
    try { text = JSON.stringify(event); } catch { text = String(event.type); }
    // Summaries stay whole JSON within the host log's line limit: the least
    // needed members go first.
    if (event.type === 'summary' && text.length > MAX_LINE) {
      const slim: Record<string, unknown> = { ...event };
      for (const key of ['violations', 'wasm', 'native', 'barrier']) {
        if (text.length <= MAX_LINE) break;
        slim[key] = '(dropped: line limit)';
        try { text = JSON.stringify(slim); } catch { break; }
      }
    }
    this.sink(`[pm-native] ${text.length > MAX_LINE ? text.slice(0, MAX_LINE) + '…' : text}`);
  }
}
