/*
 * Crash handling for engine processes (contract 7, plan section 19 "Crash
 * handling"): exponential-backoff restarts and crash attribution.
 *
 * Each engine reports the attributable items it is working on
 * (`inFlight`). When it dies, those are the suspects; an item suspected in
 * two crashes is marked failed for the session, reported once, and sent to
 * the restarted engine so it refuses the item instead of crashing again.
 * The attempt count resets once an engine has stayed up for a healthy
 * interval.
 */
import type { CrashReport, EngineProcess, ErrorScope } from '../../shared/engine-protocol';

export const CRASHES_BEFORE_MARKED_FAILED = 2;
export const HEALTHY_INTERVAL_MS = 60_000;

/** 250 ms doubling per attempt, capped at 30 s (pm_protocol `restart_backoff_ms`). */
export function restartBackoffMs(attempt: number): number {
  const shift = Math.min(Math.max(attempt - 1, 0), 16);
  return Math.min(250 * 2 ** shift, 30_000);
}

export function isAttributable(scope: ErrorScope): boolean {
  return scope.kind === 'asset' || scope.kind === 'effect' || scope.kind === 'definition' || scope.kind === 'layer';
}

function scopeKey(scope: ErrorScope): string {
  switch (scope.kind) {
    case 'asset': return `asset\0${scope.asset}`;
    case 'effect': return `effect\0${scope.layer}\0${scope.effect}\0${scope.definition}`;
    case 'definition': return `definition\0${scope.definition}`;
    case 'layer': return `layer\0${scope.layer}`;
    default: return `other\0${JSON.stringify(scope)}`;
  }
}

/** Host-side attribution for one app session (both engines share it: an
 *  item that crashes one engine is suspect in the other too). */
export class CrashLedger {
  private counts = new Map<string, { scope: ErrorScope; count: number }>();

  /** Records one crash; returns the items that became failed with it. */
  record(suspects: readonly ErrorScope[]): ErrorScope[] {
    const newlyFailed: ErrorScope[] = [];
    const seen = new Set<string>();
    for (const scope of suspects) {
      if (!isAttributable(scope)) continue;
      const key = scopeKey(scope);
      if (seen.has(key)) continue;
      seen.add(key);
      const entry = this.counts.get(key) ?? { scope, count: 0 };
      entry.count++;
      this.counts.set(key, entry);
      if (entry.count === CRASHES_BEFORE_MARKED_FAILED) newlyFailed.push(scope);
    }
    return newlyFailed;
  }

  isFailed(scope: ErrorScope): boolean {
    return (this.counts.get(scopeKey(scope))?.count ?? 0) >= CRASHES_BEFORE_MARKED_FAILED;
  }

  /** Every item failed so far (sent to a restarted engine). */
  failed(): ErrorScope[] {
    return [...this.counts.values()].filter((e) => e.count >= CRASHES_BEFORE_MARKED_FAILED).map((e) => e.scope);
  }
}

/** Consecutive-crash bookkeeping for one engine process. */
export class RestartPolicy {
  private attempt = 0;
  private startedAt: number | null = null;

  constructor(private readonly now: () => number = () => Date.now()) {}

  started(): void {
    this.startedAt = this.now();
  }

  /** The engine exited unexpectedly: the attempt number and its delay. */
  crashed(): { attempt: number; delayMs: number } {
    const healthy = this.startedAt !== null && this.now() - this.startedAt >= HEALTHY_INTERVAL_MS;
    this.attempt = healthy ? 1 : this.attempt + 1;
    this.startedAt = null;
    return { attempt: this.attempt, delayMs: restartBackoffMs(this.attempt) };
  }

  get attempts(): number {
    return this.attempt;
  }
}

const SIGNALS: Readonly<Record<number, string>> = { 1: 'SIGHUP', 2: 'SIGINT', 4: 'SIGILL', 5: 'SIGTRAP', 6: 'SIGABRT', 8: 'SIGFPE', 9: 'SIGKILL', 10: 'SIGBUS', 11: 'SIGSEGV', 13: 'SIGPIPE', 15: 'SIGTERM' };

/** Describes how a utility process ended, for the log. Electron reports
 *  `WEXITSTATUS` for a normal exit and `WTERMSIG` for a signal in the same
 *  number, so a small status names the signal it would be. */
export function describeExit(code: number | null): string {
  if (code === null) return 'exited';
  const signal = SIGNALS[code];
  return signal ? `exit status ${code} (${signal} if signalled)` : `exit status ${code}`;
}

export function crashReport(process: EngineProcess, attempt: number, exit: string, suspects: ErrorScope[], markedFailed: ErrorScope[], restartInMs: number): CrashReport {
  return { process, attempt, exit, suspects, markedFailed, restartInMs };
}
