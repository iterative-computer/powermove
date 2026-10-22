import { expect, it, vi } from 'vitest';
import { canonicalStringify } from '../../../shared/canonical-json';
import { ingestDocumentText, RemotePmWasm } from './remote';
import { RealKernelBridge, realWasm } from './test-helpers/real-kernel';

const project = {
  id: 'remote', compId: 'c', comps: { c: { id: 'c', fps: 30, layers: [{ id: 'a', from: 0, p: {
    a: { v: 1 }, b: { v: 2 }, c: { v: 3 }, d: { v: 4 },
  } }] } },
};
const address = (key: string) => ({ comp: 'c', layer: 'a', key, via: 'channel' as const });

async function remote() {
  const bridge = await RealKernelBridge.create();
  const receipt = await ingestDocumentText(bridge, project, project.id, 1);
  const wasm = new RemotePmWasm(bridge, receipt);
  return { bridge, receipt, wasm };
}

it('constructs a prepared facade without activating or changing the published document', async () => {
  const { bridge, receipt, wasm } = await remote();
  // Streamed as JSON text; the receipt is the canonical hash of the parsed tree.
  expect(new TextDecoder().decode(bridge.lastStream!)).toBe(JSON.stringify(project));
  expect(receipt.hash).toBe((await realWasm().then(fresh => { fresh.resync(canonicalStringify(project)!, 1); return fresh.hash(); })));
  expect(bridge.live.revision()).toBe(0);
  expect(bridge.hasPrepared).toBe(true);
  expect(bridge.calls.filter(call => call.op === 'activate')).toHaveLength(0);
  expect(wasm.hash()).toBe(receipt.hash);
  wasm.activate();
  expect(bridge.hasPrepared).toBe(false);
  expect(bridge.currentDocument).toBe('remote');
  expect(wasm.toJson()).toBe(canonicalStringify(project));
  expect(wasm.ev(wasm.intern(address('b')), 0).value).toBe(2);
});

it('keeps prepared batch views scoped to their group and updates only the requested range', async () => {
  const { bridge, wasm } = await remote(); wasm.activate();
  const ids = ['a', 'b', 'c', 'd'].map(key => wasm.intern(address(key)));
  const input = ids.slice(0, 2), first = wasm.prepareBatch(input), second = wasm.prepareBatch(ids.slice(2));
  input[0] = ids[3]!;
  const a = first(0, 1, 1);
  expect(Array.from(a.values)).toEqual([0, 2]);
  const b = second(0);
  expect(Array.from(b.values)).toEqual([3, 4]);
  expect(Array.from(a.values)).toEqual([0, 2]);
  const updated = first(0, 0, 1);
  expect(updated.values).toBe(a.values);
  expect(updated.kinds).toBe(a.kinds);
  expect(Array.from(updated.values)).toEqual([1, 2]);
  expect(Array.from(b.values)).toEqual([3, 4]);
  expect(bridge.calls.filter(call => call.op === 'evBatch').map(call => call.args.ids)).toEqual([[ids[1]], ids.slice(2), [ids[0]]]);
  const calls = bridge.calls.length;
  expect(() => first(0, -1, 1)).toThrow(RangeError);
  expect(() => first(0, 0, 3)).toThrow(RangeError);
  expect(bridge.calls).toHaveLength(calls);
  expect(wasm.dead).toBe(false);
});

it('preserves exact scalar and batch times, special numbers, expression errors and marker-shaped user data', async () => {
  const { bridge, wasm } = await remote(); wasm.activate();
  const json = '{"id":"remote","compId":"c","comps":{"c":{"id":"c","fps":30,"layers":[{"id":"a","from":0,"p":{"time":{"v":5,"kf":[],"expr":"time"},"bad":{"v":7,"kf":[],"expr":"1 +"},"zero":{"v":-0},"plus":{"v":1e999},"minus":{"v":-1e999},"object":{"v":{"\\u0000":"NaN"}},"text":{"v":"\\ud800"}}}]}}}';
  wasm.resync(json, 2);
  const reference = await realWasm(); reference.resync(json, 2);
  const ids = new Map<string, [number, number]>();
  for (const key of ['time', 'bad', 'zero', 'plus', 'minus', 'object', 'text']) {
    ids.set(key, [wasm.intern(address(key)), reference.intern(address(key))]);
  }
  for (const t of [0, -0, NaN, Infinity, -Infinity, 0.5]) {
    for (const [id, ref] of ids.values()) {
      expect(wasm.ev(id, t)).toEqual(reference.ev(ref, t));
      expect(wasm.evaluationStatus()).toBe(reference.evaluationStatus());
      expect(wasm.expressionError(id)).toBe(reference.expressionError(ref));
    }
    const batch = wasm.evBatch([...ids.values()].map(pair => pair[0]), t);
    const expected = reference.evBatch([...ids.values()].map(pair => pair[1]), t);
    expect(Array.from(batch.values)).toEqual(Array.from(expected.values));
    expect(Array.from(batch.kinds)).toEqual(Array.from(expected.kinds));
  }
  expect(Object.is(wasm.ev(ids.get('zero')![0], 0).value, -0)).toBe(true);
  expect(wasm.ev(ids.get('plus')![0], 0).value).toBe(Infinity);
  expect(wasm.ev(ids.get('minus')![0], 0).value).toBe(-Infinity);
  expect(wasm.ev(ids.get('object')![0], 0).value).toEqual({ '\u0000': 'NaN' });
  expect(wasm.ev(ids.get('text')![0], 0).value).toBe('\ud800');
  expect(Number.isNaN(wasm.x.pm_ev(ids.get('object')![0], 0))).toBe(true);
  expect(wasm.x.pm_kind()).toBe(1);
  const query = { op: 'projectPoint', m: [1, 0, 0, 0, 1, 0, 1, 0, 0], x: 0, y: 0 };
  expect(wasm.query(query)).toEqual(reference.query(query));
  expect(wasm.query({ op: 'ev', comp: 'c', layer: 0, key: 'object', t: 0 })).toEqual({ ok: { '\u0000': 'NaN' } });
  const times = bridge.calls.filter(call => call.op === 'ev').map(call => call.args.t);
  expect(times).toContainEqual({ '\u0000': 'NaN' });
  expect(times).toContainEqual({ '\u0000': 'Infinity' });
  expect(times).toContainEqual({ '\u0000': '-Infinity' });
  expect(times).toContainEqual({ '\u0000': '-0' });
});

it('keeps definition and font context methods on the native route', async () => {
  const { bridge, wasm } = await remote(); wasm.activate();
  wasm.define({ id: 'extension', label: 'Extension', params: { x: 'Position' } });
  expect(wasm.setFontSeeds('{"seeds":[]}')).toBe(false);
  wasm.addFontAnswers('{"faces":[],"answers":[]}');
  wasm.clearFonts(); wasm.fontOutlines(true);
  expect(wasm.fontReport()).toEqual({ misses: [], pending: [] });
  expect(wasm.contextRevision).toBe(3);
  expect(bridge.calls.map(call => call.op)).toEqual(expect.arrayContaining(['define', 'setFontSeeds', 'addFontAnswers', 'clearFonts', 'fontOutlines', 'fontReport']));
});


it('keeps the published evaluator alive after refused requests and marks only transport loss dead', async () => {
  const { bridge, wasm } = await remote(); wasm.activate();
  const call = bridge.call.bind(bridge);
  const mocked = vi.spyOn(bridge, 'call');
  for (const code of ['busy', 'stale', undefined]) {
    mocked.mockReturnValueOnce(JSON.stringify({ ok: false, code, error: 'refused' }));
    expect(() => wasm.intern(address('a'))).toThrow('refused');
    expect(wasm.dead).toBe(false);
    mocked.mockImplementation(call);
    expect(wasm.ev(wasm.intern(address('a')), 0).value).toBe(1);
  }
  mocked.mockReturnValueOnce(JSON.stringify({ ok: false, code: 'unavailable', error: 'restarted' }));
  expect(() => wasm.intern(address('a'))).toThrow('restarted');
  expect(wasm.dead).toBe(true);
  expect(wasm.resumeAutomaticRecovery()).toBe(false);
});

it('retains native handles, context and prepared batch views after main-owned automatic recovery', async () => {
  const { bridge, receipt } = await remote();
  Object.defineProperty(bridge, 'automaticRecovery', { value: true });
  const wasm = new RemotePmWasm(bridge, receipt); wasm.activate();
  const ids = ['a', 'b'].map(key => wasm.intern(address(key)));
  const prepared = wasm.prepareBatch(ids), before = prepared(0);
  wasm.define({ id: 'extension', label: 'Preserved context', params: { amount: 'Amount' } });
  const context = wasm.contextRevision, revision = wasm.revision(), hash = wasm.hash();
  const call = bridge.call, mocked = vi.spyOn(bridge, 'call');
  mocked.mockReturnValueOnce(JSON.stringify({ ok: false, code: 'unavailable', error: 'Final recovery timeout' }));
  expect(() => wasm.fontReport()).toThrow('Final recovery timeout');
  expect(wasm.dead).toBe(true); expect(wasm.mirrorDiverged).toBe(false);
  mocked.mockImplementation(call);
  const calls = mocked.mock.calls.length;
  expect(wasm.resumeAutomaticRecovery()).toBe(true);
  expect(mocked.mock.calls).toHaveLength(calls);
  expect(wasm.dead).toBe(false);
  expect(wasm.revision()).toBe(revision); expect(wasm.hash()).toBe(hash);
  expect(wasm.contextRevision).toBe(context);
  const after = prepared(0.25);
  expect(after.values).toBe(before.values); expect(after.kinds).toBe(before.kinds);
  expect(Array.from(after.values)).toEqual([1, 2]);
  expect(wasm.ev(ids[0]!, 0).value).toBe(1);
  expect(bridge.calls.filter(call => call.op === 'intern')).toHaveLength(2);
});

it('requires a checked rebuild after explicit mirror divergence even with automatic recovery', async () => {
  const { bridge, receipt } = await remote();
  Object.defineProperty(bridge, 'automaticRecovery', { value: true });
  const wasm = new RemotePmWasm(bridge, receipt); wasm.activate();
  wasm.invalidateMirror();
  expect(wasm.dead).toBe(true); expect(wasm.mirrorDiverged).toBe(true);
  expect(wasm.resumeAutomaticRecovery()).toBe(false);
  expect(() => wasm.intern(address('a'))).toThrow('stopped');
});
