/*
 * The document host pool against real worker processes: the engine bundle
 * is built once (vite, engine.vite.config.ts) into a temporary directory and
 * forked per project, as the serve host and the hidden MCP host fork it.
 *
 * Host services are the real ProjectSessions (server/sessions.ts) over an
 * in-memory profile store, so a "restart" is a new pool on the same store.
 */
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import type { AgentToolResponseEvent } from '../../shared/ipc';
import { DocumentHostPool } from './pool';
import { DocumentHostError, INSTANCE_STATE_KEY, type DocumentHostServices } from './protocol';
import { BRIDGE, FIXTURE, WASM, buildEngineBundle, hostServices, type TestHost } from './test-host';
import { canonicalProjectHash } from '../../shared/canonical-json';

let bundle: { script: string; dispose(): void } | null = null;
beforeAll(async () => { bundle = await buildEngineBundle(); }, 180_000);
afterAll(() => bundle?.dispose());

const env = { POWERMOVE_PM_WASM: WASM, POWERMOVE_PM_BRIDGE: BRIDGE };
const pools: DocumentHostPool[] = [];
const newPool = (services: DocumentHostServices, options: Partial<ConstructorParameters<typeof DocumentHostPool>[0]> = {}) => {
  const pool = new DocumentHostPool({ services, newEngine: true, script: bundle!.script, env, ...options });
  pools.push(pool);
  return pool;
};
afterEach(async () => { await Promise.all(pools.splice(0).map((pool) => pool.shutdown())); });

let call = 0;
/** The revision each pool last reported per project: an agent passes the revision it read. */
const revisions = new WeakMap<DocumentHostPool, Map<string, number>>();
const ask = async (pool: DocumentHostPool, projectId: string, runId: string, tool: string, args: Record<string, unknown>): Promise<AgentToolResponseEvent> => {
  let seen = revisions.get(pool);
  if (!seen) revisions.set(pool, seen = new Map());
  const response = await pool.call(projectId, { runId, callId: `c${++call}`, tool, arguments: args, baseRevision: seen.get(projectId) ?? 0 });
  if (typeof response.revision === 'number') seen.set(projectId, Math.max(seen.get(projectId) ?? 0, response.revision));
  return response;
};

const doc = (host: TestHost, id: string) => host.sessions.document(id) as any;
const names = (host: TestHost, id: string) => doc(host, id).layers.map((l: any) => l.name);

describe.skipIf(!existsSync(WASM) || !existsSync(BRIDGE))('the document host pool', () => {
  it('runs two projects concurrently, each with its own process, history and attribution', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    host.seed('Pbeta', 'Beta');
    const pool = newPool(host.services);
    const [a, b] = await Promise.all([pool.open('Palpha'), pool.open('Pbeta')]);
    expect(a.engine).toEqual({ evaluator: 'pm-wasm', mode: 'on', fonts: 'registry' });
    expect(new Set(pool.instances().map((i) => i.pid)).size).toBe(2);
    expect(a.instance).not.toBe(b.instance);

    // Calls on both projects at once, with reads on the same project in flight.
    const first = await Promise.all([
      ask(pool, 'Palpha', 'run-a1', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: 'A1' }], label: 'a1' }),
      ask(pool, 'Pbeta', 'run-b1', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'solid', name: 'B1' }], label: 'b1' }),
      ask(pool, 'Palpha', 'run-a2', 'get_project_state', {}),
      ask(pool, 'Pbeta', 'run-b9', 'get_project_state', {}),
    ]);
    const second = await Promise.all([
      ask(pool, 'Palpha', 'run-a2', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'text', name: 'A2', content: { text: 'Hello' } }], label: 'a2' }),
      ask(pool, 'Pbeta', 'run-b1', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: 'B2' }], label: 'b2' }),
    ]);
    const results = [...first, ...second];
    expect(results.map((r) => [r.runId, r.ok, r.error ?? null])).toEqual([
      ['run-a1', true, null], ['run-b1', true, null], ['run-a2', true, null], ['run-b9', true, null],
      ['run-a2', true, null], ['run-b1', true, null],
    ]);
    // Every answer reached the call that asked (attribution by run and call id).
    expect(new Set(results.map((r) => r.callId)).size).toBe(6);

    for (const [projectId, run] of [['Palpha', 'run-a1'], ['Palpha', 'run-a2'], ['Pbeta', 'run-b1']] as const) {
      expect((await ask(pool, projectId, run, '__finish_run', { commit: true })).ok).toBe(true);
    }
    // The host's session has every edit before the answers arrived.
    expect(names(host, 'Palpha').sort()).toEqual(['A1', 'A2']);
    expect(names(host, 'Pbeta').sort()).toEqual(['B1', 'B2']);
    const [sa, sb] = await Promise.all([pool.stats('Palpha'), pool.stats('Pbeta')]);
    expect(sa.historyDepth).toBe(2);
    expect(sb.historyDepth).toBe(1);

    // Rolling back a run on Beta never touches Alpha's history or document.
    await ask(pool, 'Pbeta', 'run-b2', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: 'B3' }], label: 'b3' });
    expect(names(host, 'Pbeta').sort()).toEqual(['B1', 'B2', 'B3']);
    expect((await ask(pool, 'Pbeta', 'run-b2', '__finish_run', { commit: false })).ok).toBe(true);
    expect(names(host, 'Pbeta').sort()).toEqual(['B1', 'B2']);
    expect((await pool.stats('Palpha')).historyDepth).toBe(2);
    expect(names(host, 'Palpha')).toHaveLength(2);
  }, 120_000);

  it('resumes each project and its undo history after a restart and after a crash', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    host.seed('Pbeta', 'Beta');
    const first = newPool(host.services);
    for (let i = 0; i < 3; i++) {
      await ask(first, 'Palpha', `run-${i}`, 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: `Shape ${i}` }], label: `shape ${i}` });
      await ask(first, 'Palpha', `run-${i}`, '__finish_run', { commit: true });
    }
    await ask(first, 'Pbeta', 'run-b', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'solid', name: 'Plate' }], label: 'plate' });
    await ask(first, 'Pbeta', 'run-b', '__finish_run', { commit: true });
    await first.shutdown();
    await host.sessions.flush();
    const saved = host.values.get(INSTANCE_STATE_KEY('Palpha')) as any;
    expect(saved.v).toBe(1);
    expect(saved.history.entries.map((e: any) => e.label)).toEqual(['Agent · shape 0', 'Agent · shape 1', 'Agent · shape 2']);

    // A new host process: the same profile store, sessions read back from the slots.
    const restarted = hostServices(host.values);
    const second = newPool(restarted.services);
    expect((await second.open('Palpha')).restoredHistory).toBe(3);
    expect(names(restarted, 'Palpha')).toEqual(['Shape 2', 'Shape 1', 'Shape 0']);
    expect((await second.open('Pbeta')).restoredHistory).toBe(1);

    // A crash: the next call starts the worker again and it resumes the same history.
    const pid = second.instances().find((i) => i.projectId === 'Palpha')!.pid!;
    process.kill(pid, 'SIGKILL');
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect((await ask(second, 'Palpha', 'run-x', 'get_project_state', {})).ok).toBe(true);
    expect(second.instances().find((i) => i.projectId === 'Palpha')!.pid).not.toBe(pid);
    expect((await second.stats('Palpha')).historyDepth).toBe(3);
    expect((await second.stats('Pbeta')).historyDepth).toBe(1);
  }, 120_000);

  it('imports persisted undo over a changed hash exactly as the desktop does', async () => {
    const host = hostServices(); host.seed('Pforeign', 'Original');
    const first = newPool(host.services);
    await ask(first, 'Pforeign', 'run-h', 'apply_commands', { label: 'local', commands: [{ type: 'add_layer', layerType: 'solid', name: 'Local' }] });
    await ask(first, 'Pforeign', 'run-h', '__finish_run', { commit: true });
    await first.shutdown(); await host.sessions.flush();
    const saved = host.values.get(INSTANCE_STATE_KEY('Pforeign')) as any;
    const stored = host.values.get('project.Pforeign') as any;
    stored.proj.name = 'Foreign edit';
    const restarted = hostServices(host.values);
    const second = newPool(restarted.services);
    expect((await second.open('Pforeign')).restoredHistory).toBe(saved.history.entries.length);
    expect(doc(restarted, 'Pforeign').name).toBe('Foreign edit');
    expect(restarted.log.some(line => line.includes('undo applies on top'))).toBe(true);
  }, 60_000);

  it('applies remote session edits from other members without touching its own undo history', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    const pool = newPool(host.services);
    await pool.open('Palpha');
    host.sessions.join(host.tab, 'Palpha', null);
    host.sessions.patch(host.tab, 'Palpha', [{ path: ['name'], exists: true, value: 'Renamed by a tab' }] as never);
    await new Promise((resolve) => setTimeout(resolve, 300));
    const state = await ask(pool, 'Palpha', 'run-r', 'get_project_state', {});
    expect(JSON.stringify(state.content)).toContain('Renamed by a tab');
    expect((await pool.stats('Palpha')).historyDepth).toBe(0);
  }, 60_000);

  it('never loses an edit made while its batch, or a remote patch, is in flight', async () => {
    // The reviewer's sequence, step by step: T1's batch in flight, a remote
    // patch R arrives, T2 edits, T2's batch in flight, T3 edits. Each host
    // patch round trip waits until the test releases it.
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    host.sessions.join(host.tab, 'Palpha', null);
    const base = host.services.session;
    const gates: Array<() => void> = [];
    let sends = 0;
    host.services.session = {
      ...base,
      patch: async (projectId, patches) => {
        sends += 1;
        await new Promise<void>((resolve) => gates.push(resolve));
        return base.patch(projectId, patches);
      },
    };
    const until = async (what: string, ok: () => boolean | Promise<boolean>) => {
      for (let i = 0; i < 400; i++) { if (await ok()) return; await new Promise((resolve) => setTimeout(resolve, 10)); }
      throw new Error(`timed out waiting for ${what}`);
    };
    const pool = newPool(host.services);
    await pool.open('Palpha');
    const revision = async () => (await pool.stats('Palpha')).revision;
    // Each call reads the revision the worker has now (its edits apply before they reach the host).
    const add = async (run: string, name: string) => {
      const baseRevision = await revision();
      return pool.call('Palpha', { runId: run, callId: `race-${name}`, tool: 'apply_commands', arguments: { commands: [{ type: 'add_layer', layerType: 'shape', name }], label: name }, baseRevision });
    };
    const t1 = add('run-1', 'T1');
    await until("T1's batch in flight", () => sends === 1);
    host.sessions.patch(host.tab, 'Palpha', [{ path: ['name'], exists: true, value: 'Renamed by a tab' }] as never);
    // R reaches the worker over the pool's IPC.
    await new Promise((resolve) => setTimeout(resolve, 200));
    const r1 = await revision();
    const t2 = add('run-2', 'T2');
    await until('T2 edited', async () => (await revision()) > r1);
    gates.shift()!();
    await until("T2's batch in flight", () => sends === 2 && gates.length === 1);
    const r2 = await revision();
    const t3 = add('run-3', 'T3');
    await until('T3 edited', async () => (await revision()) > r2);
    // From here every round trip goes through.
    const release = setInterval(() => { while (gates.length) gates.shift()!(); }, 5);
    try {
      const answers = await Promise.all([t1, t2, t3]);
      expect(answers.map((a) => [a.runId, a.ok, a.error ?? null])).toEqual([['run-1', true, null], ['run-2', true, null], ['run-3', true, null]]);
      // Read-your-writes: every answered edit is in the host's copy, with the tab's.
      expect(names(host, 'Palpha').sort()).toEqual(['T1', 'T2', 'T3']);
      expect(doc(host, 'Palpha').name).toBe('Renamed by a tab');
      expect((await pool.stats('Palpha')).hash).toBe(canonicalProjectHash(doc(host, 'Palpha')));
    } finally { clearInterval(release); }
  }, 60_000);

  it("converges on the host's order when a remote patch writes the same value as an edit in flight", async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    host.sessions.join(host.tab, 'Palpha', null);
    const base = host.services.session;
    let sends = 0;
    host.services.session = {
      ...base,
      patch: async (projectId, patches) => {
        sends += 1;
        // The tab's rename reaches the host first; the agent's batch lands after it, so the agent's value wins there.
        if (sends === 1) host.sessions.patch(host.tab, 'Palpha', [{ path: ['name'], exists: true, value: 'Tab name' }] as never);
        await new Promise((resolve) => setTimeout(resolve, 150));
        return base.patch(projectId, patches);
      },
    };
    const pool = newPool(host.services);
    await pool.open('Palpha');
    const renamed = await ask(pool, 'Palpha', 'run-1', 'apply_commands', { commands: [{ type: 'set_composition', patch: { name: 'Agent name' } }], label: 'rename' });
    expect(renamed.ok).toBe(true);
    await new Promise((resolve) => setTimeout(resolve, 400));
    const stats = await pool.stats('Palpha');
    expect(stats.hash).toBe(canonicalProjectHash(doc(host, 'Palpha')));
    expect(doc(host, 'Palpha').name).toBe('Agent name');
  }, 60_000);

  it('retains a refused batch and sends it with the next successful call', async () => {
    const host = hostServices(); host.seed('Prefused', 'Refused');
    const base = host.services.session;
    let refuse = true;
    host.services.session = { ...base, patch: async (id, patches) => {
      if (refuse) { refuse = false; throw new Error('host temporarily unavailable'); }
      return base.patch(id, patches);
    } };
    const pool = newPool(host.services);
    await expect(ask(pool, 'Prefused', 'run-a', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'solid', name: 'Retained' }] })).rejects.toMatchObject({ code: 'service-unavailable' });
    expect(names(host, 'Prefused')).toEqual([]);
    expect((await ask(pool, 'Prefused', 'run-r', 'get_project_state', {})).ok).toBe(true);
    expect(names(host, 'Prefused')).toEqual(['Retained']);
    expect((await pool.stats('Prefused')).hash).toBe(canonicalProjectHash(doc(host, 'Prefused')));
  }, 60_000);

  it('waits for an edit made during the reconciliation send before answering', async () => {
    await reconciliationWindow(false);
  }, 60_000);

  it('rechecks a same-path remote patch received during the reconciliation send', async () => {
    await reconciliationWindow(true);
  }, 60_000);

  // Seeded schedules over the same windows (order, collisions, refusals, join and send phases): sync-schedule.test.ts.

  it('keeps an edit made during the reconciliation send over a later same-path remote patch', async () => {
    // The edit reaches the host after the remote patch (the host has not seen
    // it yet), so the host must end with it, as it does outside a reconciliation.
    const host = hostServices();
    host.seed('Prace', 'Race');
    host.sessions.join(host.tab, 'Prace', null);
    const base = host.services.session;
    const gates: Array<() => void> = [];
    let sends = 0;
    let joins = 0;
    let releaseJoin: (() => void) | undefined;
    host.services.session = {
      ...base,
      join: async (id) => { const snapshot = await base.join(id); if (++joins === 2) await new Promise<void>(resolve => { releaseJoin = resolve; }); return snapshot; },
      patch: async (id, patches) => { sends++; await new Promise<void>(resolve => gates.push(resolve)); return base.patch(id, patches); },
    };
    const until = async (what: string, ok: () => boolean | Promise<boolean>) => {
      for (let n = 0; n < 500; n++) { if (await ok()) return; await new Promise(resolve => setTimeout(resolve, 5)); }
      throw new Error(`timed out waiting for ${what}`);
    };
    const pool = newPool(host.services);
    await pool.open('Prace');
    const edit = async (name: string, commands: unknown[]) => pool.call('Prace', { runId: name, callId: name, tool: 'apply_commands', arguments: { commands, label: name }, baseRevision: (await pool.stats('Prace')).revision });
    let draining: ReturnType<typeof setInterval> | undefined;
    try {
      const t1 = edit('T1', [{ type: 'set_composition', patch: { name: 'Agent 1' } }]);
      await until('send 1', () => sends === 1);
      host.sessions.patch(host.tab, 'Prace', [{ path: ['name'], exists: true, value: 'Remote 1' }] as never);
      await new Promise(resolve => setTimeout(resolve, 50));
      gates.shift()!();
      await until('reconcile join', () => !!releaseJoin);
      const r0 = (await pool.stats('Prace')).revision;
      const t2 = edit('T2', [{ type: 'add_layer', layerType: 'solid', name: 'During join' }]);
      await until('T2 applied', async () => (await pool.stats('Prace')).revision > r0);
      releaseJoin!();
      await until('reconcile send', () => sends === 2);
      const r = (await pool.stats('Prace')).revision;
      const t3 = edit('T3', [{ type: 'set_composition', patch: { name: 'Agent 3' } }]);
      await until('T3 applied', async () => (await pool.stats('Prace')).revision > r);
      host.sessions.patch(host.tab, 'Prace', [{ path: ['name'], exists: true, value: 'Remote 2' }] as never);
      await new Promise(resolve => setTimeout(resolve, 50));
      draining = setInterval(() => { while (gates.length) gates.shift()!(); }, 2);
      expect((await Promise.all([t1, t2, t3])).map(a => a.ok)).toEqual([true, true, true]);
      expect(doc(host, 'Prace').name).toBe('Agent 3');
      expect(names(host, 'Prace')).toEqual(['During join']);
      expect((await pool.stats('Prace')).hash).toBe(canonicalProjectHash(doc(host, 'Prace')));
    } finally {
      if (draining) clearInterval(draining);
      while (gates.length) gates.shift()!();
      releaseJoin?.();
    }
  }, 60_000);

  it('fails by name, with retry, when pm-wasm is missing and the new engine is on; never a legacy evaluator', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    const workerEnv = { ...env, POWERMOVE_PM_WASM: path.join(path.dirname(bundle!.script), 'missing.wasm') };
    const pool = newPool(host.services, { env: workerEnv });
    const error = await pool.open('Palpha').catch((e) => e);
    expect(error).toBeInstanceOf(DocumentHostError);
    expect([error.code, error.retry]).toEqual(['pm-wasm-unavailable', true]);
    const failed = await ask(pool, 'Palpha', 'run-1', 'get_project_state', {}).catch((e) => e);
    expect(failed.code).toBe('pm-wasm-unavailable');
    // The module appears (a rebuild): the next call starts a worker that uses it.
    workerEnv.POWERMOVE_PM_WASM = WASM;
    expect((await ask(pool, 'Palpha', 'run-1', 'get_project_state', {})).ok).toBe(true);
  }, 60_000);

  it('fails by name when the font service is missing and the new engine is on', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    const pool = newPool(host.services, { env: { ...env, POWERMOVE_PM_BRIDGE: path.join(path.dirname(bundle!.script), 'missing.node') } });
    const error = await pool.open('Palpha').catch((e) => e);
    expect([error.code, error.retry]).toEqual(['font-service-unavailable', true]);
  }, 60_000);

  it("keeps today's evaluator choice with the new engine off", async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    const pool = newPool(host.services, { newEngine: false, env: { ...env, POWERMOVE_PM_WASM: path.join(path.dirname(bundle!.script), 'missing.wasm') } });
    expect((await pool.open('Palpha')).engine.fonts).toBe('none');
    expect((await ask(pool, 'Palpha', 'run-1', 'get_project_state', {})).ok).toBe(true);
  }, 60_000);

  it('applies the shared reader guards: a stamped project needs the new engine, a newer one opens read-only', async () => {
    const host = hostServices();
    host.seed('Pnative', 'Native', { ...structuredClone(FIXTURE), requiresEngine: 'native' });
    host.seed('Pnewer', 'Newer', { ...structuredClone(FIXTURE), minReaderVersion: 99 });
    const legacy = newPool(host.services, { newEngine: false });
    const refused = await legacy.open('Pnative').catch((e) => e);
    expect(refused.code).toBe('requires-new-engine');
    expect(refused.message).toMatch(/Turn on Use New Engine/);
    const engine = newPool(host.services);
    expect((await engine.open('Pnative')).readOnly).toBe(false);
    expect((await engine.open('Pnewer')).readOnly).toBe(true);
    const edited = await ask(engine, 'Pnewer', 'run-n', 'apply_commands', { commands: [{ type: 'add_layer', layerType: 'shape', name: 'Kept local' }], label: 'local' });
    expect(edited.ok).toBe(true);
    await ask(engine, 'Pnewer', 'run-n', '__finish_run', { commit: true });
    expect(names(host, 'Pnewer')).toEqual([]);
    expect(host.values.has(INSTANCE_STATE_KEY('Pnewer'))).toBe(false);
  }, 60_000);

  it('refuses tools that belong to another route', async () => {
    const host = hostServices();
    host.seed('Palpha', 'Alpha');
    const pool = newPool(host.services);
    const error = await ask(pool, 'Palpha', 'run-1', 'export_video', {}).catch((e) => e);
    expect(error.code).toBe('wrong-route');
  }, 30_000);
});

/** Gate the reconcile's own join and send, not just ordinary batch sends. */
async function reconciliationWindow(remoteDuringSend: boolean): Promise<void> {
  const host = hostServices();
  host.seed('Prace', 'Race');
  host.sessions.join(host.tab, 'Prace', null);
  const base = host.services.session;
  const gates: Array<() => void> = [];
  let sends = 0;
  let joins = 0;
  let releaseJoin: (() => void) | undefined;
  host.services.session = {
    ...base,
    join: async (id) => {
      const snapshot = await base.join(id);
      if (++joins === 2) await new Promise<void>(resolve => { releaseJoin = resolve; });
      return snapshot;
    },
    patch: async (id, patches) => {
      sends++;
      await new Promise<void>(resolve => gates.push(resolve));
      return base.patch(id, patches);
    },
  };
  const until = async (what: string, ok: () => boolean | Promise<boolean>) => {
    for (let n = 0; n < 500; n++) { if (await ok()) return; await new Promise(resolve => setTimeout(resolve, 5)); }
    throw new Error(`timed out waiting for ${what}`);
  };
  const pool = newPool(host.services);
  await pool.open('Prace');
  const edit = async (name: string, commands: unknown[]) => pool.call('Prace', {
    runId: name, callId: name, tool: 'apply_commands', arguments: { commands, label: name }, baseRevision: (await pool.stats('Prace')).revision,
  });
  let draining: ReturnType<typeof setInterval> | undefined;
  try {
    const t1 = edit('T1', [{ type: 'set_composition', patch: { name: 'Agent 1' } }]);
    await until('ordinary send', () => sends === 1);
    host.sessions.patch(host.tab, 'Prace', [{ path: ['name'], exists: true, value: 'Remote 1' }] as never);
    await new Promise(resolve => setTimeout(resolve, 30));
    gates.shift()!();
    await until('reconcile join', () => !!releaseJoin);
    const r = (await pool.stats('Prace')).revision;
    const t2 = edit('T2', [{ type: 'set_composition', patch: { name: 'Agent 2' } }]);
    await until('edit during join', async () => (await pool.stats('Prace')).revision > r);
    releaseJoin!();
    await until('reconcile send', () => sends === 2);
    let answered = false;
    const r2 = (await pool.stats('Prace')).revision;
    const t3 = edit('T3', [{ type: 'add_layer', layerType: 'solid', name: 'During reconcile' }]).then(answer => { answered = true; expect(names(host, 'Prace')).toContain('During reconcile'); return answer; });
    await until('edit during reconcile send', async () => (await pool.stats('Prace')).revision > r2);
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(answered).toBe(false);
    if (remoteDuringSend) host.sessions.patch(host.tab, 'Prace', [{ path: ['name'], exists: true, value: 'Remote 2' }] as never);
    await new Promise(resolve => setTimeout(resolve, 20));
    draining = setInterval(() => { while (gates.length) gates.shift()!(); }, 2);
    expect((await Promise.all([t1, t2, t3])).map(a => a.ok)).toEqual([true, true, true]);
    expect(doc(host, 'Prace').name).toBe('Agent 2');
    expect((await pool.stats('Prace')).hash).toBe(canonicalProjectHash(doc(host, 'Prace')));
  } finally {
    if (draining) clearInterval(draining);
    while (gates.length) gates.shift()!();
    releaseJoin?.();
    await pool.shutdown();
  }
}
