/*
 * Seeded schedules over the worker's session sync (contract 15, invariant 3:
 * read-your-writes before every answer). A real worker process; the host's
 * join and patch round trips are each held until the scheduler lets them go.
 *
 * Each seed drives one schedule step by step, choosing at every step among:
 *   - a tool call that edits a colliding or a disjoint value (the
 *     composition's name, its duration, a new layer, a layer's name),
 *   - a remote patch from a tab on the same values (by path, so it may land
 *     on a layer the worker added),
 *   - releasing or refusing the oldest held round trip (a batch send, a
 *     reconciliation's own send, or a reconciliation's join).
 * Per round trip the seed also picks where the host holds it: before it
 * applies (or snapshots), or after (the ack or the copy is in flight). So the
 * seed varies the order of edits and remote patches, which paths collide,
 * refused batches and joins, and whether an edit or a remote patch lands in
 * a reconciliation's join or in its send.
 *
 * Checked at every answer, in the test process, the moment it arrives: the
 * host has received, from the worker, a state holding the call's value or a
 * later local value of the same slot. At the end (every hold released): the
 * worker and the host hash alike, and every refused call's edit reached the
 * host unless a later call wrote the same slot.
 */
import { existsSync } from 'node:fs';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import type { AgentToolResponseEvent } from '../../shared/ipc';
import type { Patch } from '../../shared/patch';
import { canonicalProjectHash } from '../../shared/canonical-json';
import { DocumentHostPool } from './pool';
import { BRIDGE, WASM, buildEngineBundle, hostServices } from './test-host';

let bundle: { script: string; dispose(): void } | null = null;
beforeAll(async () => { bundle = await buildEngineBundle(); }, 180_000);
afterAll(() => bundle?.dispose());

/** mulberry32: a small seeded generator. */
function generator(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Hold { kind: 'send' | 'join'; where: 'before' | 'after'; reconcile: boolean; release(): void; refuse(): void }
interface Write { slot: string; value: unknown; call: number }

const MAX_CALLS = 7;
const MAX_REMOTES = 5;
const SEEDS = Number(process.env['POWERMOVE_SYNC_SEEDS'] ?? 64);
const SEED_BASE = Number(process.env['POWERMOVE_SYNC_SEED_BASE'] ?? 0x5eed0000);

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** One seeded schedule; returns the failures it found (empty when the invariants held). */
async function schedule(seed: number): Promise<{ failures: string[]; trace: string[]; counts: Record<string, number> }> {
  const rng = generator(seed);
  const pick = <T>(items: readonly T[]): T => items[Math.floor(rng() * items.length)]!;
  const projectId = `Psched${seed}`;
  const host = hostServices();
  host.seed(projectId, 'Schedule');
  host.sessions.join(host.tab, projectId, null);
  const base = host.services.session;
  const holds: Hold[] = [];
  /** States the host reached by applying a worker's batch (copies). */
  const fromWorker: any[] = [];
  let joins = 0;
  let draining = false;
  const counts: Record<string, number> = {
    sends: 0, joins: 0, refusedSends: 0, refusedJoins: 0, remotes: 0, calls: 0, okAnswers: 0, failedAnswers: 0,
    callsDuringJoin: 0, remotesDuringJoin: 0, callsDuringReconcileSend: 0, remotesDuringReconcileSend: 0,
  };
  /** Which reconciliation phase is held right now (for the coverage counts). */
  const phase = () => ({ join: holds.some((h) => h.kind === 'join'), send: holds.some((h) => h.kind === 'send' && h.reconcile) });
  /** Set when a reconciliation's join returns: the next send is that reconciliation's own (when it has edits to send). */
  let afterJoin = false;
  const hold = (kind: Hold['kind'], where: Hold['where'], reconcile = false) => new Promise<'go' | 'refuse'>((resolve) => {
    if (draining) return resolve('go');
    holds.push({ kind, where, reconcile, release: () => resolve('go'), refuse: () => resolve('refuse') });
  });
  const record = () => fromWorker.push(structuredClone(host.sessions.document(projectId)));
  host.services.session = {
    ...base,
    join: async (id) => {
      if (++joins === 1) return base.join(id); // the open
      counts.joins! += 1;
      const where = rng() < 0.5 ? 'before' : 'after';
      if (where === 'before') {
        if (await hold('join', where) === 'refuse') { counts.refusedJoins! += 1; throw new Error('join refused by the schedule'); }
        afterJoin = true;
        return base.join(id);
      }
      const snapshot = await base.join(id);
      await hold('join', where);
      afterJoin = true;
      return snapshot;
    },
    patch: async (id, patches) => {
      counts.sends! += 1;
      const reconcile = afterJoin;
      afterJoin = false;
      const where = rng() < 0.5 ? 'before' : 'after';
      if (where === 'before') {
        if (await hold('send', where, reconcile) === 'refuse') { counts.refusedSends! += 1; throw new Error('batch refused by the schedule'); }
        const seq = await base.patch(id, patches);
        record();
        return seq;
      }
      const seq = await base.patch(id, patches);
      record();
      await hold('send', where, reconcile);
      return seq;
    },
  };

  const failures: string[] = [];
  const trace: string[] = [];
  const writes: Write[] = [];
  const added: string[] = [];
  const calls: Array<Promise<void>> = [];
  const refused: Write[][] = [];
  let answered = 0;
  const slotOf = (state: any, slot: string): unknown => {
    if (slot === 'name') return state?.name;
    if (slot === 'dur') return state?.dur;
    if (slot.startsWith('add:')) return (state?.layers ?? []).some((l: any) => l?.id === slot.slice(4)) ? true : undefined;
    if (slot.startsWith('layer:')) return (state?.layers ?? []).find((l: any) => l?.id === slot.slice(6))?.name;
    return undefined;
  };
  /** The call's value, or a value a later call wrote locally to the same slot. */
  const acceptable = (write: Write) => new Set(writes.filter((w) => w.slot === write.slot && w.call >= write.call).map((w) => JSON.stringify(w.value)));
  const reached = (write: Write) => {
    const ok = acceptable(write);
    return fromWorker.some((state) => ok.has(JSON.stringify(slotOf(state, write.slot))));
  };

  const pool$ = new DocumentHostPool({ services: host.services, newEngine: true, script: bundle!.script, env: { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE } });
  await pool$.open(projectId);
  const revision = async () => (await pool$.stats(projectId)).revision;

  /** Lets IPC and the worker run until nothing moves (holds, answers, revision), bounded. */
  const settle = async () => {
    let last = '';
    let still = 0;
    for (let i = 0; i < 120 && still < 3; i++) {
      await sleep(4);
      const now = `${holds.length}/${counts.sends}/${counts.joins}/${answered}/${await revision()}`;
      still = now === last ? still + 1 : 0;
      last = now;
    }
  };

  const startCall = async (n: number) => {
    const kinds = ['name', 'dur', 'add', ...(added.length ? ['layer'] : [])] as const;
    const kind = pick(kinds);
    let command: Record<string, unknown>;
    let write: Write;
    if (kind === 'name') { const value = `W${n}`; command = { type: 'set_composition', patch: { name: value } }; write = { slot: 'name', value, call: n }; }
    else if (kind === 'dur') { const value = 10 + n; command = { type: 'set_composition', patch: { duration: value } }; write = { slot: 'dur', value, call: n }; }
    else if (kind === 'add') { const id = `L${n}`; command = { type: 'add_layer', layerType: 'solid', id, name: `A${n}` }; write = { slot: `add:${id}`, value: true, call: n }; added.push(id); }
    else { const id = pick(added); const value = `N${n}`; command = { type: 'set_layer', target: id, patch: { name: value } }; write = { slot: `layer:${id}`, value, call: n }; }
    writes.push(write);
    trace.push(`call ${n} ${write.slot}=${JSON.stringify(write.value)}`);
    counts.calls! += 1;
    if (phase().join) counts.callsDuringJoin! += 1;
    if (phase().send) counts.callsDuringReconcileSend! += 1;
    const before = await revision();
    const answer = pool$.call(projectId, { runId: `run-${seed}`, callId: `c${n}`, tool: 'apply_commands', arguments: { commands: [command], label: `c${n}` }, baseRevision: before });
    calls.push(answer.then((response: AgentToolResponseEvent) => {
      answered += 1;
      if (!response.ok) { failures.push(`call ${n} answered not ok: ${response.error}`); return; }
      counts.okAnswers! += 1;
      // Read-your-writes, checked the moment the answer arrives.
      if (!reached(write)) failures.push(`call ${n} (${write.slot}=${JSON.stringify(write.value)}) answered ok, but the host never had it`);
    }, (error) => {
      answered += 1;
      counts.failedAnswers! += 1;
      if (error?.code !== 'service-unavailable' || error?.retry !== true) failures.push(`call ${n} failed unexpectedly: ${error?.code} ${error?.message}`);
      else refused.push([write]);
    }));
    // Its edit applies before it syncs (bounded: it may wait behind the worker's queue).
    for (let i = 0; i < 100 && (await revision()) === before; i++) await sleep(4);
  };

  const remote = (n: number) => {
    const doc = host.sessions.document(projectId) as any;
    const layers: any[] = doc?.layers ?? [];
    const kinds = ['name', 'dur', ...(layers.length ? ['layer'] : [])] as const;
    const kind = pick(kinds);
    let patch: Patch;
    if (kind === 'name') patch = { path: ['name'], exists: true, value: `R${n}` };
    else if (kind === 'dur') patch = { path: ['dur'], exists: true, value: 50 + n };
    else patch = { path: ['layers', Math.floor(rng() * layers.length), 'name'], exists: true, value: `RL${n}` };
    trace.push(`remote ${n} ${patch.path.join('.')}=${JSON.stringify(patch.value)}`);
    counts.remotes! += 1;
    if (phase().join) counts.remotesDuringJoin! += 1;
    if (phase().send) counts.remotesDuringReconcileSend! += 1;
    host.sessions.patch(host.tab, projectId, [patch] as never);
  };

  let nCall = 0;
  let nRemote = 0;
  try {
    for (let step = 0; step < 60; step++) {
      const options: string[] = [];
      if (nCall < MAX_CALLS) options.push('call', 'call', 'call');
      if (nRemote < MAX_REMOTES) options.push('remote', 'remote');
      if (holds.length) options.push('release', 'release', 'release', 'release');
      if (holds.some((h) => h.where === 'before')) options.push('refuse');
      if (!options.length) break;
      const action = pick(options);
      if (action === 'call') await startCall(++nCall);
      else if (action === 'remote') remote(++nRemote);
      else if (action === 'release') { const h = holds.shift()!; trace.push(`release ${h.kind} (${h.where})`); h.release(); }
      else { const i = holds.findIndex((h) => h.where === 'before'); const h = holds.splice(i, 1)[0]!; trace.push(`refuse ${h.kind}`); h.refuse(); }
      await settle();
    }
    // Drain: every hold goes through from here.
    draining = true;
    while (holds.length) holds.shift()!.release();
    const drain = setInterval(() => { while (holds.length) holds.shift()!.release(); }, 2);
    try {
      await Promise.all(calls);
      // A refused edit is sent with the next call.
      const final = await pool$.call(projectId, { runId: `run-${seed}`, callId: 'final', tool: 'get_project_state', arguments: {}, baseRevision: await revision() });
      if (!final.ok) failures.push(`final read failed: ${final.error}`);
      await settle();
    } finally { clearInterval(drain); }
    const stats = await pool$.stats(projectId);
    const hostDoc = host.sessions.document(projectId);
    if (stats.hash !== canonicalProjectHash(hostDoc)) failures.push(`worker and host diverged (worker ${stats.hash}, host ${canonicalProjectHash(hostDoc)})`);
    for (const [write] of refused) {
      const later = writes.some((w) => w.slot === write!.slot && w.call > write!.call);
      if (!later && !reached(write!)) failures.push(`refused call ${write!.call} (${write!.slot}=${JSON.stringify(write!.value)}) never reached the host`);
    }
  } finally {
    draining = true;
    while (holds.length) holds.shift()!.release();
    await pool$.shutdown();
  }
  return { failures, trace, counts };
}

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('seeded session sync schedules', () => {
  it(`keeps read-your-writes and converges under ${SEEDS} seeded schedules`, async () => {
    const totals: Record<string, number> = {};
    const report: string[] = [];
    for (let i = 0; i < SEEDS; i++) {
      const seed = SEED_BASE + i;
      const { failures, trace, counts } = await schedule(seed);
      for (const [k, v] of Object.entries(counts)) totals[k] = (totals[k] ?? 0) + v;
      if (failures.length) report.push(`seed ${seed.toString(16)}:\n  ${failures.join('\n  ')}\n  trace: ${trace.join(' | ')}`);
    }
    process.stdout.write(`[sync-schedule] ${SEEDS} seeds from ${SEED_BASE.toString(16)}: ${JSON.stringify(totals)}\n`);
    expect(report).toEqual([]);
    // The schedules really exercised reconciliations, refusals and remote collisions.
    expect(totals.joins).toBeGreaterThan(0);
    expect(totals.refusedSends).toBeGreaterThan(0);
    expect(totals.refusedJoins).toBeGreaterThan(0);
    for (const key of ['callsDuringJoin', 'remotesDuringJoin', 'callsDuringReconcileSend', 'remotesDuringReconcileSend']) expect(totals[key], key).toBeGreaterThan(0);
  }, 600_000);
});
