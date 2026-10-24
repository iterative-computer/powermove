import { afterEach, expect, it, vi } from 'vitest';
import { canonicalProject, canonicalStringify } from '../../../shared/canonical-json';
import { documentId } from '../../../shared/engine-document-id';
import { DocumentBarrier, type PatchBatch } from './barrier';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';
import { RemotePmWasm } from './remote';
import { APPLY } from './wasm';
import { decodePatchOps, encodePatchOps } from '../../../shared/patch-binary';
import { loadTestModule, realWasm, RealKernelBridge } from './test-helpers/real-kernel';

const host = vi.hoisted(() => ({ native: null as RealKernelBridge | null }));
vi.mock('../kernel/bridge', async original => ({ ...await original<typeof import('../kernel/bridge')>(), bridge: () => ({ nativeEngine: { documentKernel: host.native } }) }));
afterEach(() => { host.native = null; vi.restoreAllMocks(); });

async function editor(automaticRecovery = false) {
  const bridge = await RealKernelBridge.create(); host.native = bridge;
  if (automaticRecovery) Object.defineProperty(bridge, 'automaticRecovery', { value: true });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  const raw = PM.mkProject({ name: 'Native apply recovery' }); raw.id = 'native-apply-recovery';
  const layer = PM.mkLayer('solid', {}, raw); layer.p['position.x'].v = 11;
  raw.layers.push(layer);
  raw.metadata = { samples: Array.from({ length: 4100 }, (_, i) => i) };
  PM.proj = raw; PM.time = 0;
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, fonts: null, loadModule: loadTestModule, sink: () => {} });
  expect(await doc.whenReady).toBe(true);
  expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(11);
  return { PM, raw, layer, doc, bridge };
}

function changed(setup: Awaited<ReturnType<typeof editor>>, value: number) {
  setup.PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { setup.layer.p['position.x'].v = value; });
}

for (const kind of ['throw', 'gap', 'mismatch'] as const) {
  it(`keeps the shadow and revision unchanged after a native apply ${kind} and recovers from raw`, async () => {
    const setup = await editor(kind === 'throw'), { PM, raw, layer, doc, bridge } = setup;
    const barrier = doc.barrier!, instance = doc.wasm!, revision = barrier.revision;
    const previous = canonicalStringify(barrier.shadow);
    const batches: PatchBatch[] = []; barrier.subscribe(batch => batches.push(batch));
    const call = bridge.call; let fail = true;
    const calls = vi.spyOn(bridge, 'call').mockImplementation((request, body) => {
      if (JSON.parse(request).op === 'apply' && fail) {
        fail = false;
        if (kind === 'gap') {
          // Simulate a real diverged mirror ahead by several revisions.
          bridge.live.resync(previous!, revision + 5);
          return call(request, body);
        }
        if (kind === 'mismatch') {
          const prefix = barrier.shadow.compId ? ['comps', barrier.shadow.compId] : [];
          // The real engine applies the valid prefix and stamps the new
          // revision before the missing-id operation returns code 4.
          return call(request, encodePatchOps([...decodePatchOps(body!), { op: 'delete', path: [...prefix, 'layers'], id: 'absent-layer' }]));
        }
        return JSON.stringify({ ok: false, code: 'invalid', error: 'One-shot refused apply' });
      }
      return call(request, body);
    });
    const resync = vi.spyOn(instance, 'resync'), snapshot = vi.spyOn(instance, 'toJsonBytes');
    changed(setup, 87);
    expect(() => PM.doc.flush()).toThrow('asynchronous recovery');
    expect(instance.dead).toBe(true);
    expect((instance as RemotePmWasm).mirrorDiverged).toBe(true);
    expect(barrier.pending).toBe(true);
    expect(barrier.revision).toBe(revision);
    expect(barrier.recoveryRevision).toBe(revision + (kind === 'gap' ? 5 : 1));
    if (kind === 'mismatch') {
      expect(bridge.live.revision()).toBe(revision + 1);
      expect(instance.revision()).toBe(revision);
    }
    expect(canonicalStringify(barrier.shadow)).toBe(previous);
    expect(batches).toEqual([]);
    expect(PM.proj).toBe(raw); expect(PM.proj.layers[0]).toBe(layer);
    expect(layer.p['position.x'].v).toBe(87);
    const after = calls.mock.calls.length;
    // A warmed cached answer must not hide the failed mirror update.
    expect(() => doc.api!.ev(layer, 'position.x', 0)).toThrow('asynchronous recovery');
    expect(() => barrier.resume()).toThrow('asynchronous recovery');
    expect(() => barrier.snapshot()).toThrow('asynchronous recovery');
    expect(() => barrier.resync()).toThrow('asynchronous recovery');
    expect(calls.mock.calls).toHaveLength(after);
    await doc.reconnect(2);
    expect(doc.ready).toBe(true); expect(doc.wasm).not.toBe(instance);
    expect(doc.api!.ev(layer, 'position.x', 0)).toBe(87);
    expect(doc.barrier!.revision).toBeGreaterThan(revision);
    expect(doc.barrier!.revision).toBeGreaterThan(barrier.recoveryRevision);
    expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
    expect(resync).not.toHaveBeenCalled(); expect(snapshot).not.toHaveBeenCalled();
    expect(calls.mock.calls.some(([request]) => ['resync', 'toJson'].includes(JSON.parse(request).op))).toBe(false);
  });
}

it('recovers after native success followed by a shadow exception without advancing the barrier revision', async () => {
  const setup = await editor(), { raw, layer, doc, bridge } = setup;
  const barrier = doc.barrier!, instance = doc.wasm!, revision = barrier.revision;
  const canonical = barrier.shadow.compId ? barrier.shadow.comps[barrier.shadow.compId] : barrier.shadow;
  Object.defineProperty(canonical.layers[0].p['position.x'], 'v', {
    enumerable: true, configurable: true, get: () => 11,
    set: () => { throw new Error('Synthetic shadow write failure'); },
  });
  const batches: PatchBatch[] = []; barrier.subscribe(batch => batches.push(batch));
  const resync = vi.spyOn(instance, 'resync');
  changed(setup, 93);
  expect(() => barrier.flush()).toThrow('asynchronous recovery');
  expect(instance.dead).toBe(true); expect(barrier.pending).toBe(true);
  expect(barrier.revision).toBe(revision);
  expect(instance.revision()).toBe(revision + 1);
  expect(bridge.live.revision()).toBe(revision + 1);
  expect(batches).toEqual([]);
  await doc.reconnect(2);
  expect(doc.ready).toBe(true);
  expect(doc.barrier!.revision).toBeGreaterThan(revision + 1);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(93);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
  expect(resync).not.toHaveBeenCalled();
});

it('preserves the existing synchronous resync after a legacy WASM refusal', async () => {
  const wasm = await realWasm(), raw = { id: 'legacy-refusal', counter: 1 };
  const barrier = new DocumentBarrier(wasm, () => raw); barrier.resync();
  vi.spyOn(wasm, 'apply').mockReturnValueOnce({ code: APPLY.gap, message: 'Legacy gap' });
  const resync = vi.spyOn(wasm, 'resync');
  raw.counter = 2; barrier.touchedCanonical([['counter']]); barrier.flush();
  expect(resync).toHaveBeenCalledOnce();
  expect(wasm.dead).toBe(false); expect(barrier.pending).toBe(false);
  expect(barrier.stats.refused).toBe(1); expect(barrier.stats.resyncs).toBe(2);
  expect(JSON.parse(wasm.toJson()).counter).toBe(2);
});

it('normalizes long Unicode wire document IDs while preserving raw project IDs', async () => {
  const { PM, doc, bridge } = await editor();
  const large = PM.mkProject({ name: 'Unicode prepared project' }); large.id = '😀'.repeat(200);
  const first = PM.mkLayer('solid', {}, large); first.p['position.x'].v = 44; large.layers.push(first);
  large.metadata = { samples: Array.from({ length: 4100 }, (_, i) => i) };
  await doc.prepare(large); PM.proj = large;
  expect(PM.proj.id).toBe(large.id);
  expect(bridge.currentDocument).toBe(documentId(large.id));
  expect(doc.api!.ev(first, 'position.x', 0)).toBe(44);
  const small = PM.mkProject({ name: 'Unicode small assignment' }); small.id = '長'.repeat(200);
  const second = PM.mkLayer('solid', {}, small); second.p['position.x'].v = 55; small.layers.push(second);
  PM.proj = small;
  expect(doc.api!.ev(second, 'position.x', 0)).toBe(55);
  expect(PM.proj).toBe(small); expect(PM.proj.id).toBe(small.id);
  expect(bridge.currentDocument).toBe(documentId(small.id));
  expect(bridge.calls.filter(call => call.op === 'resync').at(-1)!.args.document).toBe(documentId(small.id));
});
