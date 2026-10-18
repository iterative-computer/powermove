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
