import { afterEach, expect, it, vi } from 'vitest';
import { canonicalProject, canonicalStringify } from '../../../shared/canonical-json';
import { makePM } from '../legacy/__tests__/make-pm';
import { installEngineDoc } from './install';
import { parseMode } from './flags';
import { RemotePmWasm } from './remote';
import { loadTestModule, RealKernelBridge } from './test-helpers/real-kernel';

const host = vi.hoisted(() => ({ native: null as RealKernelBridge | null, status: undefined as undefined | (() => Promise<any>), requests: 0 }));
vi.mock('../kernel/bridge', async original => ({
  ...await original<typeof import('../kernel/bridge')>(),
  bridge: () => ({ nativeEngine: {
    documentKernel: host.native, requestPorts: () => { host.requests++; },
    ...(host.status ? { status: host.status } : {}),
  } }),
}));
afterEach(() => { host.native = null; host.status = undefined; host.requests = 0; });

function project(PM: ReturnType<typeof makePM>, id: string, value: number, large = true): any {
  const raw = PM.mkProject({ name: id }); raw.id = id;
  const layer = PM.mkLayer('solid', {}, raw); layer.p['position.x'].v = value;
  raw.layers.push(layer);
  if (large) raw.metadata = { samples: Array.from({ length: 4100 }, (_, i) => i) };
  return raw;
}

async function bootSetup() {
  const bridge = await RealKernelBridge.create(); host.native = bridge;
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.time = 0;
  const reports: string[] = [];
  const loadModule = vi.fn(loadTestModule);
  const install = () => installEngineDoc(PM, { mode: parseMode('on'), strict: true, fonts: null, loadModule, sink: line => reports.push(line) });
  return { bridge, PM, reports, loadModule, install };
}

it('prepares a large restored boot document before readiness without loading renderer WASM', async () => {
  const { bridge, PM, install, loadModule } = await bootSetup();
  const raw = project(PM, 'restored-large', 31), layer = raw.layers[0]; PM.proj = raw;
  const gate = bridge.holdCommit(), doc = install();
  let ready = false; void doc.whenReady.then(ok => { ready = ok; });
  await gate.entered;
  expect(doc.ready).toBe(false); expect(ready).toBe(false);
  expect(doc.snapshot()).toBeNull(); expect(PM.proj).toBe(raw);
  expect(bridge.calls.some(call => call.op === 'activate')).toBe(false);
  expect(loadModule).not.toHaveBeenCalled();
  gate.release(); expect(await doc.whenReady).toBe(true);
  expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
  expect(doc.wasm!.x.memory.buffer.byteLength).toBe(0);
  expect(PM.proj).toBe(raw); expect(PM.proj.layers[0]).toBe(layer);
  expect(doc.api!.ev(layer, 'position.x', 0)).toBe(31);
  expect(bridge.live.toJson()).toBe(canonicalStringify(canonicalProject(raw)));
  expect(doc.failures()).toEqual([]);
  expect(doc.stats().startup).toMatchObject({ moduleMs: 0, instantiateMs: 0, readyMs: expect.any(Number) });
});

it('cancels a replaced boot snapshot and publishes only the current raw project', async () => {
  const { bridge, PM, install, loadModule } = await bootSetup();
  const first = project(PM, 'superseded-boot', 4), replacement = project(PM, 'current-boot', 89);
  PM.proj = first;
  const gate = bridge.holdCommit(), doc = install(); await gate.entered;
  PM.proj = replacement;
  gate.release(); expect(await doc.whenReady).toBe(true);
  expect(PM.proj).toBe(replacement);
  expect(bridge.currentDocument).toBe(replacement.id);
  expect(doc.api!.ev(replacement.layers[0], 'position.x', 0)).toBe(89);
  expect(bridge.calls.filter(call => call.op === 'cancel')).toHaveLength(1);
  expect(bridge.calls.filter(call => call.op === 'activate')).toHaveLength(1);
  expect(loadModule).not.toHaveBeenCalled();
});

it('retries an announced boot edit before a barrier exists', async () => {
  const { bridge, PM, install } = await bootSetup();
  const raw = project(PM, 'edited-boot', 4); PM.proj = raw;
  const gate = bridge.holdCommit(), doc = install(); await gate.entered;
  PM.doc.write([['layers', 0, 'p', 'position.x', 'v']], () => { raw.layers[0].p['position.x'].v = 72; });
  gate.release(); expect(await doc.whenReady).toBe(true);
  expect(PM.proj).toBe(raw);
  expect(doc.api!.ev(raw.layers[0], 'position.x', 0)).toBe(72);
  expect(bridge.calls.filter(call => call.op === 'cancel')).toHaveLength(1);
});

it('waits for main startup independently of EngineConnection and retains its generation', async () => {
  const { bridge, PM, install, loadModule } = await bootSetup();
  PM.proj = project(PM, 'early-boot', 61);
  const ready = (state: string) => ({ enabled: true, processes: { interactive: { state, generation: 3, attempts: 0 }, job: { state: 'starting', generation: 1, attempts: 0 } } });
  host.status = vi.fn().mockResolvedValueOnce(ready('starting')).mockResolvedValue(ready('ready'));
  const begin = vi.spyOn(bridge, 'begin'), doc = install();
  await vi.waitFor(() => expect(host.status).toHaveBeenCalledOnce());
  expect(begin).not.toHaveBeenCalled(); expect(doc.ready).toBe(false);
  expect(await doc.whenReady).toBe(true);
  expect(host.requests).toBe(1); expect(loadModule).not.toHaveBeenCalled();
  const calls = bridge.calls.length; await doc.reconnect(3);
  expect(bridge.calls).toHaveLength(calls);
});

it('rechecks a restored project assigned while the small boot module is loading', async () => {
  const { PM, reports } = await bootSetup();
  PM.proj = project(PM, 'initial-small', 1, false);
  let release!: (module: WebAssembly.Module) => void;
  const loaded = new Promise<WebAssembly.Module>(resolve => { release = resolve; });
  const doc = installEngineDoc(PM, { mode: parseMode('on'), strict: true, fonts: null, loadModule: () => loaded, sink: line => reports.push(line) });
  const restored = project(PM, 'late-large', 53); PM.proj = restored;
  release(await loadTestModule()); expect(await doc.whenReady).toBe(true);
  expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
  expect(PM.proj).toBe(restored);
  expect(doc.api!.ev(restored.layers[0], 'position.x', 0)).toBe(53);
});

it('reports a failed native boot and retries natively without a large WASM fallback', async () => {
  const { bridge, PM, install, loadModule } = await bootSetup();
  const raw = project(PM, 'retry-native-boot', 37); PM.proj = raw;
  bridge.corruptHash = true;
  const doc = install();
  expect(await doc.whenReady).toBe(false);
  expect(doc.ready).toBe(false); expect(doc.wasm).toBeNull();
  expect(PM.proj).toBe(raw); expect(bridge.hasPrepared).toBe(false);
  expect(doc.failures()).toEqual([expect.objectContaining({ code: 'pm-wasm-unavailable', retry: true })]);
  expect(loadModule).not.toHaveBeenCalled();
  bridge.corruptHash = false;
  expect(await doc.retry()).toBe(true);
  expect(doc.wasm).toBeInstanceOf(RemotePmWasm);
  expect(doc.api!.ev(raw.layers[0], 'position.x', 0)).toBe(37);
  expect(loadModule).not.toHaveBeenCalled();
});
