/*
 * The memory and GPU arbiter in main (contracts 8, 13, 14; plan section 19
 * "Memory and GPU").
 *
 * Memory: one global budget split into a lease per engine. Leases change by
 * epoch; a shrink is in force only once the engine acknowledges it with
 * usage that fits, so main never grows one engine into memory the other
 * has not given back. During playback the job engine's lease shrinks and
 * the interactive engine's grows; at the end of playback (or when the
 * interactive engine dies) the split is restored.
 *
 * Hardware codec sessions are counted per kind across both engines and
 * capped per codec (S8: sessions share one fixed per-codec throughput).
 *
 * The arbiter is pure bookkeeping: it returns the leases to send and the
 * host connection sends them.
 */
import type { EngineProcess, HardwareSessionGrant, HardwareSessionKind, HardwareSessionRequest, Lease, LeaseReason, LeaseRequest, LeaseUsage } from '../../shared/engine-protocol';

const GiB = 1024 ** 3;

/** One global budget for both engines: half of physical memory, and never
 *  more than physical memory minus 4 GiB for macOS, the renderers, and the
 *  user's other apps (RFC 08 open question 1; the memory gate revisits it). */
export function globalBudget(physicalBytes: number): number {
  const budget = Math.min(Math.floor(physicalBytes / 2), physicalBytes - 4 * GiB);
  return Math.max(budget, 1 * GiB);
}


/** Shares of the global budget in per-mille (integers, so main and the
 *  CLI host, pm-host, compute identical byte counts). */
export const IDLE_SPLIT = { interactive: 600, job: 400 } as const;
export const PLAYBACK_SPLIT = { interactive: 850, job: 150 } as const;
export const ACK_DEADLINE_MS = 500;

type LeaseState = {
  current: Lease;
  acknowledged: boolean;
  grantedAt: number;
  lastUsage: LeaseUsage | null;
  /** What the engine may still hold from earlier leases until it
   *  acknowledges this one. */
  previous: number;
};

export type LeaseGrant = { process: EngineProcess; lease: Lease };

export class LeaseError extends Error {
  override name = 'LeaseError';
}

export class MemoryArbiter {
  private leases = new Map<EngineProcess, LeaseState>();
  private epochs = new Map<EngineProcess, number>([['interactive', 0], ['job', 0]]);
  private playing = false;
  /** Grants waiting for the other engine to acknowledge a shrink. */
  private deferred = new Map<EngineProcess, { bytes: number; reason: LeaseReason }>();

  constructor(readonly global: number, private readonly now: () => number = () => Date.now()) {}

  private target(process: EngineProcess, playing = this.playing): number {
    const share = (playing ? PLAYBACK_SPLIT : IDLE_SPLIT)[process];
    return Math.floor(this.global / 1000) * share + Math.floor(((this.global % 1000) * share) / 1000);
  }

  private other(process: EngineProcess): EngineProcess {
    return process === 'interactive' ? 'job' : 'interactive';
  }

  /** Bytes the other engine may still be holding: its lease, or the
   *  previous larger lease until it acknowledges a shrink. */
  private committed(process: EngineProcess): number {
    const state = this.leases.get(process);
    if (!state) return 0;
    return state.acknowledged ? state.current.bytes : Math.max(state.current.bytes, state.previous);
  }

  private make(process: EngineProcess, bytes: number, reason: LeaseReason): LeaseGrant {
    // Epochs keep increasing across restarts of the same engine: a new
    // launch starts a new ledger, and a higher epoch is always valid.
    const epoch = (this.epochs.get(process) ?? 0) + 1;
    this.epochs.set(process, epoch);
    const lease: Lease = { process, epoch, bytes: Math.max(0, Math.floor(bytes)), reason, ackDeadlineMs: ACK_DEADLINE_MS };
    const previous = this.committed(process);
    this.leases.set(process, { current: lease, acknowledged: false, grantedAt: this.now(), lastUsage: null, previous });
    return { process, lease };
  }

  /** Grants `bytes` now if it fits next to what the other engine holds;
   *  otherwise grants what fits and remembers the rest until the other
   *  engine acknowledges its shrink. */
  private grantUpTo(process: EngineProcess, bytes: number, reason: LeaseReason): LeaseGrant {
    const room = this.global - this.committed(this.other(process));
    if (bytes > room) this.deferred.set(process, { bytes, reason });
    else this.deferred.delete(process);
    return this.make(process, Math.min(bytes, Math.max(room, 0)), reason);
  }

  /** A freshly connected engine's first lease. If the other engine grew
   *  past its share while this one was down (`request` gives it what is
   *  free), it is shrunk back to its share first (`memoryPressure`), and
   *  this engine's grant grows to its share once that shrink is
   *  acknowledged; otherwise the newcomer would sit near zero until
   *  playback next toggled. */
  initial(process: EngineProcess): LeaseGrant[] {
    const grants: LeaseGrant[] = [];
    const other = this.other(process);
    const held = this.leases.get(other);
    if (held && held.current.bytes > this.target(other)) {
      this.deferred.delete(other);
      grants.push(this.make(other, this.target(other), 'memoryPressure'));
    }
    grants.push(this.grantUpTo(process, this.target(process), 'initial'));
    return grants;
  }

  current(process: EngineProcess): Lease | null {
    return this.leases.get(process)?.current ?? null;
  }

  isAcknowledged(process: EngineProcess): boolean {
    return this.leases.get(process)?.acknowledged ?? false;
  }

  /** A usage report (an acknowledgement answer or a periodic event).
   *  Returns grants unblocked by an acknowledged shrink. */
  report(usage: LeaseUsage): LeaseGrant[] {
    const state = this.leases.get(usage.process);
    if (!state) throw new LeaseError(`usage from the ${usage.process} engine before any lease`);
    // A periodic report can cross a newer grant in flight: it is stale, not wrong.
    if (usage.epoch < state.current.epoch) return [];
    if (usage.epoch > state.current.epoch) throw new LeaseError(`usage for epoch ${usage.epoch} but the current epoch is ${state.current.epoch}`);
    const sum = Object.values(usage.categories).reduce((a, b) => a + b, 0);
    if (sum !== usage.total) throw new LeaseError(`usage total ${usage.total} does not equal the category sum ${sum}`);
    state.lastUsage = usage;
    if (usage.ack) {
      if (usage.total > state.current.bytes) throw new LeaseError(`acknowledged epoch ${usage.epoch} with ${usage.total} bytes over a ${state.current.bytes}-byte lease`);
      state.acknowledged = true;
    }
    const other = this.other(usage.process);
    const waiting = this.deferred.get(other);
    if (usage.ack && waiting) return [this.grantUpTo(other, waiting.bytes, waiting.reason)];
    return [];
  }

  /** Playback started in the interactive engine: shrink the job engine
   *  first; the interactive engine grows once the job engine has given
   *  the memory back. */
  playbackStarted(): LeaseGrant[] {
    if (this.playing) return [];
    this.playing = true;
    const grants: LeaseGrant[] = [];
    if (this.leases.has('job')) grants.push(this.make('job', this.target('job'), 'playbackStarted'));
    if (this.leases.has('interactive')) grants.push(this.grantUpTo('interactive', this.target('interactive'), 'playbackStarted'));
    return grants;
  }

  /** Playback stopped (or the interactive engine exited while playing):
   *  shrink the interactive engine back, then restore the job engine. */
  playbackStopped(reason: LeaseReason = 'playbackStopped'): LeaseGrant[] {
    if (!this.playing) return [];
    this.playing = false;
    const grants: LeaseGrant[] = [];
    if (this.leases.has('interactive')) grants.push(this.make('interactive', this.target('interactive'), reason));
    if (this.leases.has('job')) grants.push(this.grantUpTo('job', this.target('job'), reason));
    return grants;
  }

  get isPlaying(): boolean {
    return this.playing;
  }

  /** `requestLease`: an engine wants more. It gets what is free next to
   *  the other engine's commitment, never less than its current lease. */
  request(request: LeaseRequest): LeaseGrant {
    const state = this.leases.get(request.process);
    if (!state) throw new LeaseError(`the ${request.process} engine has no lease yet`);
    const room = this.global - this.committed(this.other(request.process));
    const bytes = Math.max(state.current.bytes, Math.min(request.wanted, room));
    return this.make(request.process, bytes, 'requested');
  }

  /** The engine exited: its memory is free. Returns grants for the
   *  survivor (playback ends if the interactive engine died). */
  exited(process: EngineProcess): LeaseGrant[] {
    this.leases.delete(process);
    this.deferred.delete(process);
    const grants = process === 'interactive' ? this.playbackStopped('peerExited') : [];
    const other = this.other(process);
    const waiting = this.deferred.get(other);
    if (waiting) grants.push(this.grantUpTo(other, waiting.bytes, waiting.reason));
    return grants;
  }

  /** Engines whose shrink is overdue (logged; the memory gate counts them). */
  overdue(): EngineProcess[] {
    const now = this.now();
    return [...this.leases.entries()]
      .filter(([, s]) => !s.acknowledged && now - s.grantedAt > s.current.ackDeadlineMs)
      .map(([process]) => process);
  }
}

/** Concurrent hardware sessions per codec kind across both engines (S8
 *  recommendation 8; S6 sets the reference machine's real limits). */
export const HARDWARE_LIMITS: Readonly<Record<HardwareSessionKind, number>> = {
  decode: 16,
  encode: 2,
  proResDecode: 8,
  proResEncode: 2
};

export class HardwareArbiter {
  private held = new Map<string, number>();

  constructor(private readonly limits: Readonly<Record<HardwareSessionKind, number>> = HARDWARE_LIMITS) {}

  private key(process: EngineProcess, kind: HardwareSessionKind): string {
    return `${process}:${kind}`;
  }

  heldBy(process: EngineProcess, kind: HardwareSessionKind): number {
    return this.held.get(this.key(process, kind)) ?? 0;
  }

  total(kind: HardwareSessionKind): number {
    return this.heldBy('interactive', kind) + this.heldBy('job', kind);
  }

  /** Acquires up to `delta` sessions (what is free under the cap) or
   *  releases `-delta`. The grant is the requester's new total. */
  request(process: EngineProcess, request: HardwareSessionRequest): HardwareSessionGrant {
    if (!Number.isSafeInteger(request.delta) || request.delta === 0) throw new Error('a session request changes the count');
    if (!(request.kind in this.limits)) throw new Error(`unknown hardware session kind ${String(request.kind)}`);
    const mine = this.heldBy(process, request.kind);
    const next = request.delta > 0
      ? mine + Math.min(request.delta, Math.max(0, this.limits[request.kind] - this.total(request.kind)))
      : Math.max(0, mine + request.delta);
    this.held.set(this.key(process, request.kind), next);
    return { kind: request.kind, held: next };
  }

  /** The engine exited: its sessions are gone with it. */
  releaseAll(process: EngineProcess): void {
    for (const kind of Object.keys(this.limits) as HardwareSessionKind[]) this.held.delete(this.key(process, kind));
  }
}
