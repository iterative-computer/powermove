import { afterEach, expect, it, vi } from 'vitest';
import { canonicalProject, canonicalStringify } from '../../../shared/canonical-json';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';
import { RemotePmWasm } from './remote';
import { loadTestModule, RealKernelBridge } from './test-helpers/real-kernel';
import { projectTexts } from '../legacy/core/project-text';
import * as jsonSlices from '../legacy/core/json-slices';

const host = vi.hoisted(() => ({ native: null as RealKernelBridge | null }));
vi.mock('../kernel/bridge', async original => ({ ...await original<typeof import('../kernel/bridge')>(), bridge: () => ({ nativeEngine: { documentKernel: host.native } }) }));
afterEach(() => { host.native = null; });

function project(PM: ReturnType<typeof makePM>, id: string, value: number, large = false): any {
  const raw = PM.mkProject({ name: id }); raw.id = id;
  const layer = PM.mkLayer('solid', {}, raw);
  layer.p['position.x'].v = value;
  raw.layers.push(layer);
  // A bounded fixture crosses the same census threshold as animation arrays.
  if (large) raw.metadata = { samples: Array.from({ length: 4100 }, (_, i) => `sample-${i}-${'x'.repeat(16)}`) };
  return raw;
}

async function editor(automaticRecovery = false) {
  const bridge = await RealKernelBridge.create(); host.native = bridge;
  if (automaticRecovery) Object.defineProperty(bridge, 'automaticRecovery', { value: true });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const old = project(PM, 'old-project', 11); PM.proj = old; PM.time = 0;
  const reports: string[] = [];
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, fonts: null, loadModule: loadTestModule, sink: line => reports.push(line) });
  expect(await doc.whenReady).toBe(true);
  return { PM, doc, bridge, old, reports };
}

it('keeps the facade, shadow, dirty writes and handles across main-owned automatic recovery', async () => {
  const setup = await editor(true), { PM, doc, bridge } = setup;
  const raw = await publishLarge(setup), layer = raw.layers[0];
  await doc.reconnect(1);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(42);
  const instance = doc.wasm! as RemotePmWasm, barrier = doc.barrier!, shadow = barrier.shadow;
  const revision = barrier.revision, context = instance.contextRevision;
  const interned = bridge.calls.filter(call => call.op === 'intern').length;
  const begin = vi.spyOn(bridge, 'begin'), commit = vi.spyOn(bridge, 'commit');
  PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { layer.p['position.x'].v = 84; });
  const calls = bridge.calls.length;
  await doc.reconnect(2);
  expect(bridge.calls).toHaveLength(calls);
  expect(doc.wasm).toBe(instance); expect(doc.barrier).toBe(barrier); expect(barrier.shadow).toBe(shadow);
  expect(barrier.revision).toBe(revision); expect(barrier.pending).toBe(true);
  expect(doc.api!.ev(layer, 'position.x', 0.25)).toBe(84);
  expect(barrier.pending).toBe(false);
  expect(bridge.calls.filter(call => call.op === 'intern')).toHaveLength(interned);
  const call = bridge.call;
  const mocked = vi.spyOn(bridge, 'call');
  mocked.mockReturnValueOnce(JSON.stringify({ ok: false, code: 'unavailable', error: 'Final recovery timeout' }));
  expect(() => instance.fontReport()).toThrow('Final recovery timeout');
  expect(instance.dead).toBe(true); expect(instance.mirrorDiverged).toBe(false);
  mocked.mockImplementation(call);
  doc.ready = false;
  const waiting = vi.fn(); doc.onReady(waiting);
  const failedCalls = mocked.mock.calls.length;
  await doc.reconnect(3);
  expect(mocked.mock.calls).toHaveLength(failedCalls);
  expect(doc.ready).toBe(true); expect(waiting).toHaveBeenCalledOnce();
  expect(doc.wasm).toBe(instance); expect(doc.barrier).toBe(barrier); expect(barrier.shadow).toBe(shadow);
  expect(instance.contextRevision).toBe(context);
  expect(doc.api!.ev(layer, 'position.x', 0.5)).toBe(84);
  expect(PM.proj).toBe(raw); expect(PM.proj.layers[0]).toBe(layer);
  expect(begin).not.toHaveBeenCalled(); expect(commit).not.toHaveBeenCalled();
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
});
async function publishLarge(setup: Awaited<ReturnType<typeof editor>>, id = 'native-project', value = 42) {
  const raw = project(setup.PM, id, value, true);
  await setup.doc.prepare(raw); setup.PM.proj = raw;
  expect(setup.doc.wasm).toBeInstanceOf(RemotePmWasm);
  return raw;
}

it('refreshes an expired native patch journal without serializing a whole-document snapshot', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const raw = await publishLarge(setup);
  const snapshot = vi.spyOn(doc.wasm!, 'toJsonBytes');
  const hash = vi.spyOn(doc.wasm!, 'hash');
  for (let i = 0; i < 129; i++) {
    PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { raw.layers[0].p['position.x'].v = i; });
    PM.doc.flush();
  }
  const resume = await doc.refreshResume!();
  expect(resume).toMatchObject({ revision: doc.barrier!.revision, batches: [] });
  expect(resume!.hash).toBe(bridge.live.hash());
  expect(bridge.calls.map(call => call.op)).toEqual(expect.arrayContaining(['refresh', 'checkpoint']));
  expect(snapshot).not.toHaveBeenCalled();
  expect(hash).not.toHaveBeenCalled();
  const calls = bridge.calls.length;
  expect(await doc.refreshResume!()).toEqual(resume);
  expect(bridge.calls).toHaveLength(calls);
});

it('keeps old raw project identities and synchronous queries until publication, then activates the exact prepared project', async () => {
  const setup = await editor(), { PM, doc, bridge, old } = setup;
  const raw = project(PM, 'prepared-project', 81, true), layer = raw.layers[0];
  const gate = bridge.holdCommit(), preparing = doc.prepare(raw);
  await gate.entered;
  expect(PM.proj).toBe(old);
  expect(doc.api!.ev(old.layers[0], 'position.x', 0)).toBe(11);
  expect(bridge.calls.filter(call => call.op === 'activate')).toHaveLength(0);
  gate.release(); await preparing;
  expect(PM.proj).toBe(old);
  expect(doc.api!.ev(old.layers[0], 'position.x', 0)).toBe(11);
  PM.proj = raw;
  expect(PM.proj).toBe(raw);
  expect(PM.proj.layers[0]).toBe(layer);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(81);
  expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
  expect(bridge.currentDocument).toBe(raw.id);
  expect(bridge.hasPrepared).toBe(false);
  expect(doc.failures()).toEqual([]);
});

it('advances activation past edits to the still-published native document while another ingest is pending', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const live = await publishLarge(setup);
  const next = project(PM, 'next-project', 88, true);
  const gate = bridge.holdCommit(), preparing = doc.prepare(next);
  await gate.entered;
  const preparedRevision = JSON.parse(bridge.call('{"op":"poll"}')).value.revision;
  for (const value of [23, 24]) {
    PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { live.layers[0].p['position.x'].v = value; });
    PM.doc.flush();
  }
  const editedRevision = doc.barrier!.revision;
  expect(editedRevision).toBeGreaterThan(preparedRevision);
  expect(PM.proj).toBe(live);
  expect(doc.api!.ev(live.layers[0], 'position.x', 0)).toBe(24);
  gate.release(); await preparing;
  expect(PM.proj).toBe(live);
  PM.proj = next;
  expect(doc.barrier!.revision).toBeGreaterThan(editedRevision);
  expect(doc.wasm!.revision()).toBe(doc.barrier!.revision);
  expect(bridge.live.revision()).toBe(doc.barrier!.revision);
  expect(doc.api!.ev(next.layers[0], 'position.x', 0)).toBe(88);
  expect(bridge.calls.filter(call => call.op === 'activate').at(-1)!.args).toMatchObject({ revision: editedRevision + 1, preparedRevision });
});

it('refuses a prepared snapshot whose tree differs from its stream and retains the published project', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const live = await publishLarge(setup);
  const revision = doc.barrier!.revision;
  bridge.corruptHash = true;
  await expect(doc.prepare(project(PM, 'bad-source', 99, true))).rejects.toThrow('does not match its canonical hash');
  expect(PM.proj).toBe(live);
  expect(doc.barrier!.revision).toBe(revision);
  expect(doc.api!.ev(live.layers[0], 'position.x', 0)).toBe(42);
  expect(bridge.hasPrepared).toBe(false);
  expect(bridge.streams.size).toBe(0);
  expect(doc.ready).toBe(true);
});

it('finds a prepared tree whose integrity digest disagrees at its first check after publication, and repairs it', async () => {
  // Preparation no longer digests the shadow before publication (OPENWRITE-1
  // round: a third whole-document pass); the canonical hash round trip still
  // gates ingestion, and the first check compares the integrity digests.
  const setup = await editor(), { PM, doc, bridge } = setup;
  await publishLarge(setup);
  bridge.corruptIntegrity = true;
  const next = project(PM, 'unverified-integrity', 77, true);
  await doc.prepare(next);
  PM.proj = next;
  bridge.corruptIntegrity = false;
  const first = await doc.checkAsync('after-open');
  expect(first?.ok).toBe(false);
  const second = await doc.checkAsync('after-repair');
  expect(second?.ok).toBe(true);
  expect(doc.api!.ev(next.layers[0], 'position.x', 0)).toBe(77);
});

it('releases a superseded preparation without canceling a newer in-flight tree', async () => {
  const setup = await editor(), { PM, doc, bridge, old } = setup;
  const first = project(PM, 'unused-project', 10, true);
  await doc.prepare(first);
  doc.discardPrepared!(first);
  expect(bridge.hasPrepared).toBe(false);
  expect(PM.proj).toBe(old);
  await doc.prepare(first);
  const second = project(PM, 'replacement-project', 20, true);
  const gate = bridge.holdCommit(), preparing = doc.prepare(second);
  await gate.entered;
  const cancels = bridge.calls.filter(call => call.op === 'cancel').length;
  doc.discardPrepared!(first);
  expect(bridge.calls.filter(call => call.op === 'cancel')).toHaveLength(cancels);
  gate.release(); await preparing;
  expect(bridge.hasPrepared).toBe(true);
  PM.proj = second;
  expect(PM.proj).toBe(second);
  expect(doc.api!.ev(second.layers[0], 'position.x', 0)).toBe(20);
});

it('keeps small synchronous assignments on the native route and replaces their document identity', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  await publishLarge(setup);
  for (const [id, value, prepare] of [['small-tab-one', 17, true], ['small-tab-two', 29, false]] as const) {
    const raw = project(PM, id, value);
    if (prepare) await doc.prepare(raw);
    PM.proj = raw;
    expect(PM.proj).toBe(raw);
    expect(doc.api!.ev(raw.layers[0], 'position.x', 0)).toBe(value);
    expect(bridge.currentDocument).toBe(id);
    expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
    expect(JSON.parse(doc.wasm!.toJson()).id).toBe(id);
  }
});

it('registers extension parameter labels again after replacing the evaluator', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  PM.layerDefinition = () => ({ label: 'Custom generator', params: [{ k: 'amount', label: 'Amount' }] });
  for (const id of ['first-label-project', 'second-label-project']) {
    const raw = project(PM, id, 42, true);
    const layer = PM.mkLayer('extension', { d: { definition: 'custom-generator', params: { amount: PM.P(1) } } }, raw);
    raw.layers.push(layer);
    await doc.prepare(raw); PM.proj = raw;
    doc.api!.allProps(layer);
  }
  expect(bridge.calls.filter(call => call.op === 'define').map(call => call.args)).toEqual([
    { id: 'custom-generator', label: 'Custom generator', params: { amount: 'Amount' } },
    { id: 'custom-generator', label: 'Custom generator', params: { amount: 'Amount' } },
  ]);
});

it('rebuilds a lost native kernel before a new engine generation becomes ready', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  await doc.reconnect(1);
  const raw = await publishLarge(setup), layer = raw.layers[0], revision = doc.barrier!.revision;
  const before = bridge.calls.length;
  await doc.reconnect(1);
  expect(bridge.calls).toHaveLength(before);
  bridge.call('{"op":"close"}');
  const gate = bridge.holdCommit(), recovery = doc.reconnect(2);
  await gate.entered;
  expect(doc.ready).toBe(false);
  let settled = false;
  const waiting = doc.settled().then(ok => { settled = ok; });
  let saved = false;
  const saving = Promise.resolve(doc.beforeSave('save during recovery')).then(() => { saved = true; });
  await Promise.resolve();
  expect(settled).toBe(false);
  expect(saved).toBe(true);
  gate.release(); await recovery; await waiting; await saving;
  expect(saved).toBe(true);
  expect(settled).toBe(true);
  expect(doc.ready).toBe(true);
  expect(PM.proj).toBe(raw);
  expect(PM.proj.layers[0]).toBe(layer);
  expect(doc.barrier!.revision).toBeGreaterThan(revision);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(42);
  expect(bridge.calls.slice(before).some(call => call.op === 'toJson')).toBe(false);
  const restored = bridge.calls.length;
  await doc.reconnect(2);
  expect(bridge.calls).toHaveLength(restored);
});

it('does not deadlock an open queued before a later recovery request', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  await doc.reconnect(1);
  const live = await publishLarge(setup);
  const first = project(PM, 'first-pending', 10, true);
  const second = project(PM, 'second-pending', 20, true);
  const gate = bridge.holdCommit(), preparing = doc.prepare(first);
  await gate.entered;
  const next = doc.prepare(second);
  // The revive timer can mark readiness false while the earlier ingest is
  // in flight. Its connection recovery must run before the queued open retries.
  doc.ready = false;
  const recovery = doc.reconnect(2);
  gate.release();
  await preparing; await recovery; await next;
  expect(doc.ready).toBe(true);
  expect(PM.proj).toBe(live);
  PM.proj = second;
  expect(doc.api!.ev(second.layers[0], 'position.x', 0)).toBe(20);
});

it('leaves the whole-document check out of autosaves outside the test modes, and keeps it for saves and exports', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const raw = await publishLarge(setup), before = doc.barrier!;
  raw.layers[0].p['position.x'].v = 777;
  const calls = bridge.calls.length, checks = before.stats.checks;
  expect(doc.beforeSave('autosave')).toBeUndefined();
  await Promise.resolve();
  expect(before.stats.checks).toBe(checks);
  expect(bridge.calls.slice(calls).length).toBe(0);
  await doc.beforeExport('export');
  expect(before.stats.checks).toBe(checks + 1);
  expect(before.stats.checkMismatches).toBe(1);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
});

it('persists live raw-alias writes immediately while their diagnostic repair runs in the background', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const raw = await publishLarge(setup), layer = raw.layers[0], before = doc.barrier!;
  const mismatches: unknown[] = []; PM.bus.on('engine:consistency-mismatch', (event: unknown) => mismatches.push(event));
  layer.p['position.x'].v = 777;
  const gate = bridge.holdCommit();
  expect(doc.beforeSave('raw alias save')).toBeUndefined();
  const saved = JSON.stringify(PM.proj);
  await gate.entered;
  expect(JSON.parse(saved).layers[0].p['position.x'].v).toBe(777);
  expect(doc.beforeSave('another save while repairing')).toBeUndefined();
  gate.release();
  await doc.beforeExport('wait for repaired mirror');
  expect(mismatches).toHaveLength(1);
  expect(before.stats.checkMismatches).toBe(1);
  expect(PM.proj).toBe(raw);
  expect(PM.proj.layers[0]).toBe(layer);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(777);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
  expect((await doc.checkAsync('repaired save'))!.ok).toBe(true);
  expect(doc.failures()).toEqual([]);
});

it('discards a save repair snapshot when an announced edit crosses its asynchronous ingest', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const raw = await publishLarge(setup), layer = raw.layers[0];
  layer.p['position.x'].v = 777;
  const gate = bridge.holdCommit();
  expect(doc.beforeSave('edit across repair')).toBeUndefined();
  await gate.entered;
  const staleHash = JSON.parse(bridge.call('{"op":"poll"}')).value.hash;
  try {
    PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { layer.p['position.x'].v = 888; });
    PM.doc.flush();
    expect(doc.api!.ev(layer, 'position.x', 0)).toBe(888);
  } finally { gate.release(); }
  await doc.beforeExport('wait for edit across repair');
  expect(PM.proj).toBe(raw);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(888);
  expect(bridge.calls.filter(call => call.op === 'activate').map(call => call.args.hash)).not.toContain(staleHash);
  expect(bridge.calls.map(call => call.op)).toContain('cancel');
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
});

it('discards an in-flight repair for a project replaced before the asynchronous save check settles', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const old = await publishLarge(setup);
  old.layers[0].p['position.x'].v = 777;
  const gate = bridge.holdCommit();
  expect(doc.beforeSave('project switch across repair')).toBeUndefined();
  await gate.entered;
  const staleHash = JSON.parse(bridge.call('{"op":"poll"}')).value.hash;
  const next = project(PM, 'replacement-tab', 123);
  try {
    PM.proj = next;
    expect(doc.api!.ev(next.layers[0], 'position.x', 0)).toBe(123);
  } finally { gate.release(); }
  await doc.beforeExport('wait for project switch across repair');
  expect(PM.proj).toBe(next);
  expect(bridge.currentDocument).toBe(next.id);
  expect(doc.api!.ev(next.layers[0], 'position.x', 0)).toBe(123);
  expect(bridge.calls.filter(call => call.op === 'activate').map(call => call.args.hash)).not.toContain(staleHash);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(next)));
  expect(doc.failures()).toEqual([]);
});

it('prepares an opened project from its canonical text, streamed while the text is still being made', async () => {
  const setup = await editor(), { PM, doc, bridge } = setup;
  const raw = project(PM, 'texts-project', 63, true), layer = raw.layers[0];
  const texts = projectTexts('1.0.0', raw)!;
  const writes: Array<Uint8Array | string> = [], write = bridge.write.bind(bridge);
  bridge.write = async (token, chunk) => { writes.push(chunk); return write(token, chunk); };
  const serialize = vi.spyOn(jsonSlices, 'serializeInSlices');
  await doc.prepare(raw, texts.canonical); PM.proj = raw;
  // The engine read the open's own text: nothing serialized the shadow again.
  expect(serialize).not.toHaveBeenCalled();
  expect(JSON.parse(new TextDecoder().decode(bridge.lastStream!))).toEqual(JSON.parse(JSON.stringify(canonicalProject(raw))));
  expect(doc.barrier!.shadow).toEqual(JSON.parse(JSON.stringify(canonicalProject(raw))));
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(63);
  expect(await texts.slot).toBe(JSON.stringify({ v: '1.0.0', proj: raw }));
  // The shadow is exact: an edit afterwards is one small batch at the edited path.
  PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { layer.p['position.x'].v = 64; });
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(64);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
  expect(writes.length).toBeGreaterThan(0);
});
