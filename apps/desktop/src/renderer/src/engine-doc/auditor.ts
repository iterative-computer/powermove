/*
 * The shadow-replay auditor (docs/rust-engine.md, section 6, item 5; test
 * builds): at frame boundaries the patch stream applied to a JS shadow must
 * equal `PM.Comps.canonicalize(PM.proj)` under sorted-key serialization, and
 * pm-wasm must hold the same document (its canonical hash equals the
 * shadow's).
 *
 * A failure means a write the barrier did not see (the shadow differs from
 * the live document) or a stream pm-wasm applied differently (the hashes
 * differ). The auditor records where, then repairs the mirror so the next
 * frame starts clean.
 */
import { canonicalHash } from '../../../shared/canonical-json';
import { diffOps } from '../../../shared/patch-ids';
import type { DocumentBarrier } from './barrier';

export interface AuditFailure {
  kind: 'missed-write' | 'mirror';
  revision: number;
  /** Paths where the live document left the shadow (missed writes). */
  paths: string[];
  detail: string;
  at: number;
}

export interface AuditStats {
  audits: number;
  failures: number;
  missedWrites: number;
  mirrorMismatches: number;
  ms: number;
  samples: AuditFailure[];
}

const MAX_SAMPLES = 100;

export class ShadowAuditor {
  readonly stats: AuditStats = { audits: 0, failures: 0, missedWrites: 0, mirrorMismatches: 0, ms: 0, samples: [] };
  private lastMirrorRevision = -1;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(
    private barrier: () => DocumentBarrier | null,
    private report: (failure: AuditFailure) => void = () => {},
  ) {}

  /** Audits now: flushes, compares, and repairs. Returns whether all was equal. */
  audit(): boolean {
    const barrier = this.barrier();
    if (!barrier || barrier.wasm.dead) return true;
    const started = performance.now();
    this.stats.audits += 1;
    let ok = true;
    try {
      barrier.flush();
      const compared = barrier.shadowMatches();
      if (!compared.ok) {
        ok = false;
        const paths = barrier.repair('structuralDiff');
        this.fail({ kind: 'missed-write', revision: barrier.revision, paths: paths.slice(0, 20), detail: `${paths.length} path(s)`, at: Date.now() });
        this.stats.missedWrites += 1;
      }
      if (barrier.revision !== this.lastMirrorRevision) {
        const shadowHash = barrier.shadow === undefined ? '' : canonicalHash(barrier.shadow);
        const mirrorHash = barrier.wasm.hash();
        this.lastMirrorRevision = barrier.revision;
        if (shadowHash !== mirrorHash) {
          ok = false;
          this.stats.mirrorMismatches += 1;
          const mirror = JSON.parse(barrier.wasm.toJson() || 'null');
          const paths = diffOps(mirror, barrier.shadow).map(op => op.path.map(String).join('.')).slice(0, 20);
          this.fail({ kind: 'mirror', revision: barrier.revision, paths, detail: `${shadowHash.slice(0, 12)} ≠ ${mirrorHash.slice(0, 12)}`, at: Date.now() });
          barrier.resync();
          this.lastMirrorRevision = barrier.revision;
        }
      }
    } catch (error) {
      console.warn('[engine-doc] audit failed to run', error);
    } finally {
      this.stats.ms += performance.now() - started;
    }
    return ok;
  }

  private fail(failure: AuditFailure) {
    this.stats.failures += 1;
    if (this.stats.samples.length < MAX_SAMPLES) this.stats.samples.push(failure);
    this.report(failure);
  }

  /** Audits at task boundaries, at most every `ms` milliseconds. */
  start(ms = 100): void {
    if (this.timer) return;
    this.timer = setInterval(() => this.audit(), ms);
    (this.timer as any)?.unref?.();
  }

  stop(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }
}
