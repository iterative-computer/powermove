import { EFFECTS } from '../../extensions/effects-basic/effects';
// @vitest-environment happy-dom
import { afterEach, expect, it } from 'vitest';
import { createRpc } from '../../shared/sandbox-rpc';
import { createSandboxAPI, ProjectWritePermissionError, sandboxControl, type SandboxInit, type SandboxSnapshot } from './shim-api';

const close: Array<() => void> = [];
afterEach(() => { for (const fn of close.splice(0)) fn(); });
const settle = (ms = 5) => new Promise(resolve => setTimeout(resolve, ms));

function harness(permissions: string[] = [], apiVersion = 3, kernel: Record<string, (...args: any[]) => unknown> = {}) {
  const channel = new MessageChannel();
  const calls: Array<[string, ...unknown[]]> = [];
  const record = (name: string, fn: (...args: any[]) => unknown = () => undefined) => (...args: unknown[]) => { calls.push([name, ...args]); return fn(...args); };
  const host = createRpc(channel.port2, {
    register: record('register'), 'dispose-registration': record('dispose-registration'), 'runtime-error': record('runtime-error'),
    'sandbox-report': record('sandbox-report'), ...Object.fromEntries(Object.entries(kernel).map(([name, fn]) => [name, record(name, fn)]))
  });
  const rpc = createRpc(channel.port1, {}, 1000, { trusted: true, maxHandles: 1 });
  close.push(() => { rpc.close(); host.close(); });
  const init = { id: 'shim-ext', apiVersion, manifest: { id: 'shim-ext', name: 'Shim', version: '1.0.0', apiVersion, permissions }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, bundleUrl: '', state: { time: 1, playing: false, revision: 5, generation: 1, selection: { layers: ['a'], keys: [], chan: null } } } as unknown as SandboxInit;
  const api = createSandboxAPI(rpc, init) as any;
  return { api, control: sandboxControl(api), calls, count: (name: string) => calls.filter(call => call[0] === name).length };
}

it('reads state synchronously and applies tick deltas before dispatching events', async () => {
  const { api, control } = harness();
  expect([api.project.time(), api.project.playing(), api.project.revision(), api.transport.time()]).toEqual([1, false, 5, 1]);
  expect(Object.isFrozen(api.project.selection().layers)).toBe(true);
  const seen: unknown[] = [];
  api.events.on('project:changed', (payload: unknown) => seen.push([payload, api.project.revision()]));
  control.tick({ time: 2, playing: true, revision: 6, selection: { layers: [], keys: [], chan: null } }, [['project:changed', { kind: 'values' }], ['other', 1]]);
  expect([api.project.time(), api.transport.playing(), api.project.selection().layers]).toEqual([2, true, []]);
  expect(seen).toEqual([[{ kind: 'values' }, 6]]);
});

it('keeps listeners local, registering interest once per name and withdrawing it with the last listener', async () => {
  const { api, control, calls, count } = harness();
  const order: string[] = [];
  const first = api.events.on('project:changed', () => order.push('first'));
  const second = api.events.on('project:changed', () => { order.push('second'); throw new Error('listener boom'); });
  const third = api.events.on('time', () => order.push('time'));
  api.events.on('custom', () => {}); // no callback handle crosses, so the one-handle quota is untouched
  await settle();
  expect(calls.filter(call => call[0] === 'register').map(call => [call[1], call[3]]))
    .toEqual([['events', { event: 'project:changed' }], ['events', { event: 'time' }], ['events', { event: 'custom' }]]);
  control.tick({}, [['time', 3], ['project:changed', { kind: 'structure' }]]);
  expect(order).toEqual(['time', 'first', 'second']);
  await settle();
  expect(calls.find(call => call[0] === 'runtime-error')?.[1]).toMatchObject({ message: 'listener boom' });
  first.dispose(); third.dispose();
  await settle();
  expect(count('dispose-registration')).toBe(1);
  second.dispose(); second.dispose();
  await settle();
  expect(count('dispose-registration')).toBe(2);
  control.tick({}, [['project:changed', { kind: 'values' }]]);
  expect(order).toHaveLength(3);
});

it('pulls the project once per generation, shares one call in flight, and freezes it', async () => {
  let generation = 1;
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const { api, control, count } = harness(['project:write'], 3, {
    'project-snapshot': async (): Promise<SandboxSnapshot> => { await gate; return { generation, json: JSON.stringify({ revision: generation, layers: [{ id: 'a' }] }) }; }
  });
  const reads = [api.project.get(), api.project.get(), api.project.get()];
  await settle();
  release();
  const [one, two, three] = await Promise.all(reads);
  expect(one).toBe(two); expect(two).toBe(three);
  expect(count('project-snapshot')).toBe(1);
  expect(Object.isFrozen(one.layers[0])).toBe(true);
  expect(await api.project.get()).toBe(one);
  expect(count('project-snapshot')).toBe(1);
  generation = 2;
  control.tick({ generation: 2 }, []);
  const next = await api.project.get();
  expect(next.revision).toBe(2);
  expect(count('project-snapshot')).toBe(2);
});

it('asks again when the reply predates the generation the document already heard about', async () => {
  let replies = 0;
  const { api, control, count } = harness([], 3, {
    'project-snapshot': (): SandboxSnapshot => ({ generation: ++replies < 2 ? 1 : 3, json: JSON.stringify({ replies }) })
  });
  control.tick({ generation: 3 }, []);
  expect(await api.project.get()).toEqual({ replies: 2 });
  expect(count('project-snapshot')).toBe(2);
});

it('keeps its copy when the kernel answers that the generation is unchanged', async () => {
  let replies = 0;
  const { api, control, count } = harness([], 3, {
    'project-snapshot': (): SandboxSnapshot => ++replies === 1 ? { generation: 1, json: '{"layers":[1]}' } : { generation: 1, unchanged: true }
  });
  const first = await api.project.get();
  // The document heard of generation 2 before the kernel built it: the kernel still holds generation 1.
  control.tick({ generation: 2 }, []);
  await expect(api.project.get()).resolves.toBe(first);
  expect(count('project-snapshot')).toBe(4);
  // With nothing cached, `unchanged` is no project.
  const empty = harness([], 3, { 'project-snapshot': (): SandboxSnapshot => ({ generation: 1, unchanged: true }) });
  await expect(empty.api.project.get()).rejects.toThrow('could not read the project');
});

it('rejects project.get clearly when the snapshot is too large', async () => {
  const { api } = harness([], 3, { 'project-snapshot': (): SandboxSnapshot => ({ generation: 1, tooLarge: true }) });
  await expect(api.project.get()).rejects.toThrow('8 Mi character');
});

it('reads the project without any permission, while edits still need project:write', async () => {
  const { api, control, calls } = harness([], 3, {
    'project-snapshot': (): SandboxSnapshot => ({ generation: 1, json: '{"layers":[]}' }),
    invoke: () => 'data:image/jpeg;base64,'
  });
  await expect(api.project.get()).resolves.toEqual({ layers: [] });
  expect(api.project.selection().layers).toEqual(['a']);
  const heard: unknown[] = [];
  api.events.on('project:changed', (payload: unknown) => heard.push(payload));
  api.events.on('selection', (payload: unknown) => heard.push(payload));
  await expect(api.project.snapshot(0, 320)).resolves.toBe('data:image/jpeg;base64,');
  await expect(api.project.apply([])).rejects.toBeInstanceOf(ProjectWritePermissionError);
  control.tick({ selection: { layers: ['b'], keys: [], chan: null } }, [['project:changed', { kind: 'values' }], ['selection', { layers: ['b'] }]]);
  expect(api.project.selection().layers).toEqual(['b']);
  expect(heard).toEqual([{ kind: 'values' }, { layers: ['b'] }]);
  await settle();
  expect(calls.filter(call => call[0] === 'register').map(call => call[3])).toEqual([{ event: 'project:changed' }, { event: 'selection' }]);
  expect(calls.filter(call => call[0] === 'invoke')).toEqual([['invoke', 'project', 'snapshot', [0, 320]]]);
  expect(calls.filter(call => call[0] === 'sandbox-report')).toEqual([]);
});

it('reports a synchronous read of an async result from apiVersion 2 code', async () => {
  const { api, calls } = harness([], 2, { invoke: () => ({ layers: [] }) });
  const result = api.storage.get('layout');
  expect(result.layers).toBeUndefined();
  await expect(result).resolves.toEqual({ layers: [] });
  await settle();
  expect(calls.filter(call => call[0] === 'sandbox-report').map(call => call[1])).toEqual([{ kind: 'async', member: 'storage.get' }]);
});

it('lets apiVersion 2 code read the project, and names project.get when it reads the Promise synchronously', async () => {
  const { api, calls } = harness([], 2, { 'project-snapshot': (): SandboxSnapshot => ({ generation: 1, json: '{"layers":[]}' }) });
  const project = api.project.get();
  expect(project.layers).toBeUndefined();
  await expect(project).resolves.toEqual({ layers: [] });
  expect(api.project.selection().layers).toEqual(['a']);
  expect(() => api.events.on('selection', () => {})).not.toThrow();
  await settle();
  expect(calls.filter(call => call[0] === 'sandbox-report').map(call => call[1])).toEqual([{ kind: 'async', member: 'project.get' }]);
});

it('asks panels.isOpen of the host even while a view replays activate', async () => {
  const channel = new MessageChannel();
  const asked: unknown[] = [];
  const host = createRpc(channel.port2, { invoke: (namespace: string, method: string, args: unknown[]) => { asked.push([namespace, method, args]); return true; } });
  const rpc = createRpc(channel.port1, {}, 1000, { trusted: true });
  close.push(() => { rpc.close(); host.close(); });
  const init = { id: 'shim-ext', apiVersion: 3, manifest: { id: 'shim-ext', name: 'Shim', version: '1.0.0', apiVersion: 3, permissions: [] }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, bundleUrl: '', state: { time: 0, playing: false, revision: 1, generation: 1, selection: null } } as unknown as SandboxInit;
  const api = createSandboxAPI(rpc, init, 'view') as any;
  sandboxControl(api).setQuiet(true);
  await expect(api.panels.isOpen('shim-ext.panel')).resolves.toBe(true);
  api.panels.open('shim-ext.panel'); // a side effect the runtime already performed: dropped
  await settle();
  expect(asked).toEqual([['panels', 'isOpen', ['shim-ext.panel']]]);
});

it('collects this extension’s own menu contributions for a location, in order', async () => {
  const channel = new MessageChannel();
  const errors: string[] = [];
  const host = createRpc(channel.port2, { register: () => undefined, 'dispose-registration': () => undefined, 'runtime-error': (error: { message: string }) => { errors.push(error.message); } });
  const rpc = createRpc(channel.port1, {}, 1000, { trusted: true });
  close.push(() => { rpc.close(); host.close(); });
  const init = { id: 'shim-ext', apiVersion: 3, manifest: { id: 'shim-ext', name: 'Shim', version: '1.0.0', apiVersion: 3, permissions: [] }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, bundleUrl: '', state: { time: 0, playing: false, revision: 1, generation: 1, selection: null } } as unknown as SandboxInit;
  const api = createSandboxAPI(rpc, init) as any;
  const first = api.menus.contribute('layer:context', (ctx: { layer?: string }) => [{ header: 'Stub' }, { label: `Rename ${ctx.layer}`, run: () => 1 }]);
  api.menus.contribute('viewer:context', () => [{ label: 'Elsewhere' }]);
  api.menus.contribute('layer:context', () => { throw new Error('contributor broke'); });
  api.menus.contribute('layer:context', () => ['-']);
  const items = api.menus.collect('layer:context', { layer: 'Title' });
  expect(items.map((item: any) => typeof item === 'string' ? item : item.header ?? item.label)).toEqual(['Stub', 'Rename Title', '-']);
  expect(typeof items[1].run).toBe('function'); // local callbacks, nothing crossed
  expect(api.menus.collect('timeline:context')).toEqual([]);
  first.dispose();
  expect(api.menus.collect('layer:context')).toEqual(['-']);
  await settle();
  expect(errors).toEqual(['contributor broke', 'contributor broke']);
});

it('gathers this extension’s own async menu contributions for one open', async () => {
  const channel = new MessageChannel();
  const errors: string[] = [];
  const host = createRpc(channel.port2, { register: () => undefined, 'dispose-registration': () => undefined, 'runtime-error': (error: { message: string }) => { errors.push(error.message); } });
  const rpc = createRpc(channel.port1, {}, 1000, { trusted: true });
  close.push(() => { rpc.close(); host.close(); });
  const init = { id: 'shim-ext', apiVersion: 3, manifest: { id: 'shim-ext', name: 'Shim', version: '1.0.0', apiVersion: 3, permissions: [] }, vars: {},
    theme: { scheme: 'dark', tokens: {} }, bundleUrl: '', state: { time: 0, playing: false, revision: 1, generation: 1, selection: null } } as unknown as SandboxInit;
  const api = createSandboxAPI(rpc, init) as any;
  api.menus.contribute('layer:context', async (ctx: { layer?: string }) => [{ label: `Rename ${ctx.layer}`, run: () => 1 }]);
  api.menus.contribute('viewer:context', () => [{ label: 'Elsewhere' }]);
  api.menus.contribute('layer:context', async () => { throw new Error('contributor broke'); });
  api.menus.contribute('layer:context', () => ['-']);
  const items = await api.menus.gather('layer:context', { layer: 'B' });
  expect(items.map((item: any) => typeof item === 'string' ? item : item.label)).toEqual(['Rename B', '-']);
  expect(await api.menus.gather('timeline:context')).toEqual([]);
  await settle();
  expect(errors).toEqual(['contributor broke']);
});

it('sends ui.copy to the kernel as plain invoke, which decides permission and focus', async () => {
  const { api, calls } = harness(['clipboard'], 3, { invoke: () => undefined });
  expect('copy' in api.ui).toBe(true);
  await api.ui.copy('#ff6600');
  expect(calls.filter(call => call[0] === 'invoke')).toEqual([['invoke', 'ui', 'copy', ['#ff6600']]]);
  expect(() => api.ui.read).toThrow(); // there is no clipboard read
});

it('resolves current and historical effect controls in the sandbox catalog', () => {
  const { api, control } = harness();
  const invert = EFFECTS.find(effect => effect.id === 'invert')!;
  control.catalog({ effects: [invert] } as SandboxInit['catalog']);
  expect(api.effects.supportsVersions).toBe(true);
  expect(api.effects.get('invert').version).toBe(2);
  expect(api.effects.get('invert', 1).params.map((param: any) => param.k)).toEqual(['amount']);
  expect(api.effects.get('invert', 99)).toBeUndefined();
});
