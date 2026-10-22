import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  ENGINE_PROXIES_CHANGED,
  proxyIntentOf,
  storedProxyIntent,
  validGeneratedMediaLocation,
  type EngineProxiesBridge,
  type ProxySnapshot
} from './proxies-types';
import { assetDescriptor, createProxyController, installMediaProxies, proxyStatus, type ProxyController } from './proxies';
import { folderName, locationOptions } from './proxies-location';

const HASH = 'a'.repeat(64);
const OTHER = 'b'.repeat(64);

function fakeBridge() {
  let listener: ((snapshot: ProxySnapshot) => void) | null = null;
  const bridge = {
    snapshot: vi.fn(async (project: string): Promise<ProxySnapshot> => ({ project, assets: {}, activity: { creating: null, paused: null } })),
    subscribe: vi.fn((fn: (snapshot: ProxySnapshot) => void) => { listener = fn; return () => { listener = null; }; }),
    setIntent: vi.fn(async (..._args: Parameters<EngineProxiesBridge['setIntent']>) => {}),
    create: vi.fn(async () => {}),
    rebuild: vi.fn(async () => {}),
    remove: vi.fn(async () => {}),
    location: vi.fn(async () => ({ kind: 'cache' as const })),
    setLocation: vi.fn(async (location) => location),
    chooseFolder: vi.fn(async (): Promise<string | null> => '/Volumes/Media/Proxies'),
    setPreviewMemory: vi.fn(async () => {})
  } satisfies EngineProxiesBridge;
  return { bridge, push: (snapshot: ProxySnapshot) => listener?.(snapshot), subscribed: () => listener !== null };
}

function fakePM(project: any = {
  id: 'p1',
  assets: {
    v1: { id: 'v1', name: 'A001_C003.mov', kind: 'video', contentHash: HASH, sourcePath: '/Users/me/A001_C003.mov' },
    v2: { id: 'v2', name: 'linked.mov', kind: 'video', link: { path: '/Volumes/Shoot/linked.mov', hash: OTHER } },
    a1: { id: 'a1', name: 'music.wav', kind: 'audio', storageKey: 'k-a1' },
    s1: { id: 's1', name: 'frames', kind: 'image', imageSequence: { fps: 24 } }
  }
}) {
  const handlers = new Map<string, Array<() => void>>();
  const history: string[] = [];
  const PM: Record<string, any> = {
    proj: project,
    playing: false,
    bus: {
      on: (event: string, fn: () => void) => {
        handlers.set(event, [...(handlers.get(event) ?? []), fn]);
        return () => handlers.set(event, (handlers.get(event) ?? []).filter((f) => f !== fn));
      },
      emit: (event: string) => { for (const fn of handlers.get(event) ?? []) fn(); }
    },
    hist: { do: (label: string, fn: () => void) => { history.push(label); fn(); } },
    touch: vi.fn(),
    autosave: vi.fn(),
    toast: vi.fn(),
    dismissToast: vi.fn(),
    log: vi.fn(),
    projectFileState: vi.fn(() => ({ path: '/Users/me/Film.pmv', dirty: false })),
    HostMedia: { store: { recordFor: (key: string) => (key === 'k-a1' ? { hash: 'c'.repeat(64) } : null) } },
    Memory: { budget: () => 512 * 1024 * 1024 }
  };
  return { PM, history, handlers };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const controllers: ProxyController[] = [];
const make = (PM: Record<string, any>, bridge: EngineProxiesBridge) => {
  const controller = createProxyController(PM, bridge);
  controllers.push(controller);
  return controller;
};

it('prioritizes pending clips by distance from a paused playhead', async () => {
  const { bridge } = fakeBridge();
  const prioritize = vi.fn(async () => {});
  const { PM } = fakePM();
  PM.time = 12;
  PM.proj.layers = [
    { from: 0, dur: 2, d: { asset: 'v1' } },
    { from: 10, dur: 4, d: { asset: 'v2' } }
  ];
  make(PM, { ...bridge, prioritize });
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v2', 'v1']);
  PM.time = 1; PM.bus.emit('time');
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v1', 'v2']);
});

it('prioritizes footage in a nearby precomp instance ahead of a distant top-level clip', async () => {
  const { bridge } = fakeBridge();
  const prioritize = vi.fn(async () => {});
  const { PM } = fakePM();
  PM.time = 12;
  PM.proj.comps = { nested: { layers: [{ id: 'inner', type: 'video', from: 5, dur: 2, d: { asset: 'v2' } }] } };
  PM.proj.layers = [
    { id: 'far', type: 'video', from: 20, dur: 2, d: { asset: 'v1' } },
    { id: 'first', type: 'precomp', from: 0, dur: 4, d: { comp: 'nested', trim: 0, speed: 1 } },
    { id: 'near', type: 'precomp', from: 10, dur: 4, d: { comp: 'nested', trim: 2, speed: 2 } }
  ];
  make(PM, { ...bridge, prioritize });
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v2', 'v1']);
  PM.proj.comps.nested.layers.push({ id: 'cycle', type: 'precomp', from: 0, dur: 10, d: { comp: 'nested' } });
  PM.time = 11; PM.bus.emit('time');
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v2', 'v1']);
});

it('maps reverse speed and time remap into the nested composition before ranking assets', async () => {
  const { bridge } = fakeBridge();
  const prioritize = vi.fn(async () => {});
  const { PM } = fakePM();
  PM.time = 15.5;
  PM.proj.comps = { nested: { layers: [
    { id: 'late', type: 'video', from: 5, dur: 1, d: { asset: 'v2' } },
    { id: 'early', type: 'video', from: 0, dur: 1, d: { asset: 'v1' } }
  ] } };
  const instance = { id: 'reverse', type: 'precomp', from: 10, dur: 6,
    d: { comp: 'nested', trim: 6, speed: -1 } as Record<string, unknown> };
  PM.proj.layers = [instance];
  make(PM, { ...bridge, prioritize });
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v1', 'v2']);
  instance.d = { comp: 'nested', timeRemap: true,
    sourceTime: { v: 6, kf: [] } };
  PM.evP = (layer: any, _property: any, time: number) => 6 - (time - layer.from);
  PM.time = 10.6; PM.bus.emit('time');
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v2', 'v1']);
  PM.evP = () => 0.5; // a frozen remap only visits the early clip
  PM.time = 10.7; PM.bus.emit('time');
  await settle();
  expect(prioritize).toHaveBeenLastCalledWith('p1', ['v1', 'v2']);
});

it('offers Create Proxies after actual drops and measured slow decoding, once playback stops', async () => {
  const { bridge } = fakeBridge();
  const { PM } = fakePM();
  const controller = make(PM, bridge);
  await settle();
  PM.toast.mockClear();
  PM.playing = true;
  controller.playbackStats(1);
  expect(PM.toast).not.toHaveBeenCalled();
  controller.slowDecodes(['v1', 'a1']);
  controller.playbackStats(2);
  expect(PM.toast).not.toHaveBeenCalled();
  PM.playing = false; PM.bus.emit('transport');
  const notice = PM.toast.mock.calls.find(([text]: any[]) => text === 'Dropped frames · clips decode slowly');
  expect(notice?.[2].action.label).toBe('Create Proxies');
  notice?.[2].action.run();
  await settle();
  expect(bridge.create).toHaveBeenCalledWith('p1', ['v1']);
  controller.playbackStats(3);
  expect(PM.toast.mock.calls.filter(([text]: any[]) => text === 'Dropped frames · clips decode slowly')).toHaveLength(1);
});

afterEach(() => {
  for (const controller of controllers.splice(0)) controller.dispose();
  vi.useRealTimers();
});

describe('shared proxy intent', () => {
  it('reads defaults for absent fields and stores only what differs', () => {
    expect(proxyIntentOf({})).toEqual({ create: 'whenNeeded', size: 'automatic', format: 'proResProxy', alwaysPlayOriginal: [] });
    expect(proxyIntentOf({ media: { proxies: { create: 'allVideo', size: 'bogus', format: 'hevc10' } }, assets: { b: { alwaysPlayOriginal: true }, a: { alwaysPlayOriginal: true }, c: {} } }))
      .toEqual({ create: 'allVideo', size: 'automatic', format: 'hevc10', alwaysPlayOriginal: ['a', 'b'] });
    expect(storedProxyIntent({ create: 'whenNeeded', size: 'automatic', format: 'proResProxy' })).toBeNull();
    expect(storedProxyIntent({ create: 'onlyWhenAsked', size: 'p720', format: 'proResProxy' })).toEqual({ create: 'onlyWhenAsked', size: 'p720' });
  });
  it('validates generated media locations', () => {
    expect(validGeneratedMediaLocation({ kind: 'cache' })).toBe(true);
    expect(validGeneratedMediaLocation({ kind: 'project' })).toBe(true);
    expect(validGeneratedMediaLocation({ kind: 'folder', path: '/Volumes/Media' })).toBe(true);
    expect(validGeneratedMediaLocation({ kind: 'folder', path: 'relative' })).toBe(false);
    expect(validGeneratedMediaLocation({ kind: 'folder', path: '/a\0b' })).toBe(false);
    expect(validGeneratedMediaLocation({ kind: 'cache', extra: 1 })).toBe(false);
    expect(validGeneratedMediaLocation(null)).toBe(false);
  });
  it('lists the location choices, with the chosen folder by name', () => {
    expect(locationOptions({ kind: 'cache' }).map((o) => o.label)).toEqual(['Powermove cache', 'Next to the project', 'Choose Folder…']);
    expect(locationOptions({ kind: 'folder', path: '/Volumes/Media/Proxies/' }).map((o) => o.v)).toEqual(['cache', 'project', 'folder', 'choose']);
    expect(folderName('/Volumes/Media/Proxies/')).toBe('Proxies');
  });
});

describe('asset descriptors', () => {
  it('sends the hash, kind, name and source main keys proxies by', () => {
    expect(assetDescriptor({ name: 'A.mov', kind: 'video', contentHash: HASH, sourcePath: '/x/A.mov', alwaysPlayOriginal: true }))
      .toEqual({ contentHash: HASH, kind: 'video', name: 'A.mov', sourcePath: '/x/A.mov', alwaysPlayOriginal: true });
    // A linked file: its link's hash and path.
    expect(assetDescriptor({ kind: 'video', link: { path: '/v/B.mov', hash: OTHER }, sourcePath: '/old/B.mov' }))
      .toEqual({ kind: 'video', contentHash: OTHER, sourcePath: '/v/B.mov' });
    // A stored file: the media store's record; malformed hashes are left out.
    expect(assetDescriptor({ kind: 'video', storageKey: 'k', contentHash: 'nope' }, () => HASH)).toEqual({ kind: 'video', contentHash: HASH });
    expect(assetDescriptor({ kind: 'image', imageSequence: { fps: 24 } })).toEqual({ kind: 'image', imageSequence: true });
  });
});

describe('proxy controller', () => {
  it('sends the intent, descriptors and project path on open, then only on change', async () => {
    vi.useFakeTimers();
    const { bridge } = fakeBridge();
    const { PM } = fakePM();
    make(PM, bridge);
    await vi.advanceTimersByTimeAsync(0);
    expect(bridge.setIntent).toHaveBeenCalledOnce();
    const [project, intent, assets, path] = bridge.setIntent.mock.calls[0] as unknown as [string, any, Record<string, any>, string];
    expect(project).toBe('p1');
    expect(intent).toEqual({ create: 'whenNeeded', size: 'automatic', format: 'proResProxy', alwaysPlayOriginal: [] });
    expect(assets['v1']).toEqual({ contentHash: HASH, kind: 'video', name: 'A001_C003.mov', sourcePath: '/Users/me/A001_C003.mov' });
    expect(assets['a1']?.contentHash).toBe('c'.repeat(64));
    expect(path).toBe('/Users/me/Film.pmv');
    expect(bridge.setPreviewMemory).toHaveBeenCalledWith(512 * 1024 * 1024);
    // Nothing changed: a project event sends nothing.
    PM.bus.emit('project');
    await vi.advanceTimersByTimeAsync(300);
    expect(bridge.setIntent).toHaveBeenCalledOnce();
    // A new asset is sent after the debounce.
    PM.proj.assets.v3 = { id: 'v3', name: 'new.mov', kind: 'video', contentHash: OTHER };
    PM.bus.emit('assets');
    await vi.advanceTimersByTimeAsync(300);
    expect(bridge.setIntent).toHaveBeenCalledTimes(2);
    expect((bridge.setIntent.mock.calls[1] as unknown as [string, any, Record<string, any>])[2]['v3']).toEqual({ contentHash: OTHER, kind: 'video', name: 'new.mov' });
  });

  it('edits the project intent as one undo step each, storing only non-defaults', async () => {
    const { bridge } = fakeBridge();
    const { PM, history } = fakePM();
    const controller = make(PM, bridge);
    await settle();
    controller.setIntent({ size: 'p1080' });
    controller.setIntent({ size: 'p1080' });
    controller.setIntent({ create: 'onlyWhenAsked' });
    expect(history).toEqual(['Proxy size', 'Create proxies']);
    expect(PM.proj.media).toEqual({ proxies: { size: 'p1080', create: 'onlyWhenAsked' } });
    expect(controller.intent()).toMatchObject({ create: 'onlyWhenAsked', size: 'p1080', format: 'proResProxy' });
    await settle();
    expect(bridge.setIntent.mock.calls.at(-1)?.[1]).toMatchObject({ create: 'onlyWhenAsked', size: 'p1080' });
    controller.setIntent({ size: 'automatic', create: 'whenNeeded' });
    expect(PM.proj.media).toBeUndefined();
  });

  it('keeps other media fields when the proxy intent returns to defaults', async () => {
    const { bridge } = fakeBridge();
    const { PM } = fakePM({ id: 'p2', assets: {}, media: { whenImporting: 'link', proxies: { format: 'hevc10' } } });
    const controller = make(PM, bridge);
    controller.setIntent({ format: 'proResProxy' });
    expect(PM.proj.media).toEqual({ whenImporting: 'link' });
  });

  it('marks Always Play Original on the asset and sends it', async () => {
    const { bridge } = fakeBridge();
    const { PM, history } = fakePM();
    const controller = make(PM, bridge);
    await settle();
    controller.setAlwaysPlayOriginal('v1', true);
    expect(PM.proj.assets.v1.alwaysPlayOriginal).toBe(true);
    expect(controller.alwaysPlayOriginal('v1')).toBe(true);
    await settle();
    expect(bridge.setIntent.mock.calls.at(-1)?.[1]).toMatchObject({ alwaysPlayOriginal: ['v1'] });
    controller.setAlwaysPlayOriginal('v1', false);
    expect('alwaysPlayOriginal' in PM.proj.assets.v1).toBe(false);
    expect(history).toEqual(['Always Play Original', 'Always Play Original']);
  });

  it('reports eligibility and the state main last sent', async () => {
    const { bridge, push } = fakeBridge();
    const { PM } = fakePM();
    const controller = make(PM, bridge);
    const changed = vi.fn();
    controller.onChange(changed);
    await settle();
    expect(controller.eligible('v1')).toBe(true);
    expect(controller.eligible('s1')).toBe(true);
    expect(controller.eligible('a1')).toBe(false);
    expect(controller.assetState('a1')).toEqual({ state: 'unsupported' });
    expect(controller.assetState('v1')).toEqual({ state: 'none' });
    push({ project: 'p1', assets: { v1: { state: 'creating', progress: 0.4 } }, activity: { creating: { done: 0, total: 2 }, paused: null } });
    expect(controller.assetState('v1')).toEqual({ state: 'creating', progress: 0.4 });
    expect(changed).toHaveBeenCalled();
    // Another project's snapshot is not this one's.
    push({ project: 'other', assets: { v1: { state: 'failed', error: 'x' } }, activity: { creating: null, paused: null } });
    expect(controller.assetState('v1')).toEqual({ state: 'creating', progress: 0.4 });
  });

  it('runs Create, Rebuild and Delete Proxy after sending the descriptors, and says what failed', async () => {
    const { bridge } = fakeBridge();
    const { PM } = fakePM();
    const controller = make(PM, bridge);
    await settle();
    PM.proj.assets.v1.contentHash = OTHER;
    await controller.create('v1');
    // The changed hash reached main before the create.
    const order = [bridge.setIntent.mock.invocationCallOrder.at(-1)!, bridge.create.mock.invocationCallOrder[0]!];
    expect(order[0]!).toBeLessThan(order[1]!);
    expect(bridge.create).toHaveBeenCalledWith('p1', ['v1']);
    await controller.rebuild('v1');
    expect(bridge.rebuild).toHaveBeenCalledWith('p1', ['v1']);
    bridge.remove.mockRejectedValueOnce(new Error("Error invoking remote method 'proxies:remove': Error: The proxy is in use"));
    await controller.remove('v1');
    expect(PM.toast).toHaveBeenCalledWith("Couldn't delete the proxy for A001_C003.mov · The proxy is in use", 6000, { error: true, kind: 'error' });
  });

  it('shows one status notice, and leaves it alone during playback', async () => {
    const { bridge, push } = fakeBridge();
    const { PM } = fakePM();
    make(PM, bridge);
    await settle();
    PM.toast.mockClear();
    push({ project: 'p1', assets: {}, activity: { creating: { done: 2, total: 12 }, paused: null } });
    expect(PM.toast).toHaveBeenLastCalledWith('Creating proxies · 3 of 12', 0, expect.objectContaining({ key: 'media-proxies', sticky: true, progress: 2 / 12 }));
    PM.playing = true;
    push({ project: 'p1', assets: {}, activity: { creating: { done: 3, total: 12 }, paused: 'lowDisk' } });
    expect(PM.toast).toHaveBeenCalledTimes(1);
    PM.playing = false;
    PM.bus.emit('transport');
    expect(PM.toast).toHaveBeenLastCalledWith('Proxies paused · Low disk space', 0, expect.not.objectContaining({ progress: expect.anything() }));
    push({ project: 'p1', assets: {}, activity: { creating: null, paused: null } });
    expect(PM.dismissToast).toHaveBeenCalledWith('media-proxies');
  });

  it('passes the generated media location and folder choice through', async () => {
    const { bridge } = fakeBridge();
    const { PM } = fakePM();
    const controller = make(PM, bridge);
    expect(await controller.location()).toEqual({ kind: 'cache' });
    expect(await controller.chooseFolder()).toEqual({ kind: 'folder', path: '/Volumes/Media/Proxies' });
    expect(bridge.setLocation).toHaveBeenCalledWith({ kind: 'folder', path: '/Volumes/Media/Proxies' });
    bridge.chooseFolder.mockResolvedValueOnce(null);
    expect(await controller.chooseFolder()).toBeNull();
    expect(bridge.setLocation).toHaveBeenCalledOnce();
  });

  it('starts over when another project opens, and unsubscribes on dispose', async () => {
    const { bridge, subscribed } = fakeBridge();
    const { PM } = fakePM();
    const controller = make(PM, bridge);
    await settle();
    PM.proj = { id: 'p9', assets: {} };
    PM.bus.emit('projects:open');
    await settle();
    expect(bridge.setIntent.mock.calls.at(-1)?.[0]).toBe('p9');
    expect(bridge.snapshot).toHaveBeenLastCalledWith('p9');
    controller.dispose();
    expect(subscribed()).toBe(false);
  });
});

describe('statuses', () => {
  it('names the clip being made, and the disk guard first', () => {
    expect(proxyStatus({ creating: { done: 0, total: 1 }, paused: null })).toEqual({ text: 'Creating proxies · 1 of 1', progress: 0 });
    expect(proxyStatus({ creating: { done: 12, total: 12 }, paused: null })?.text).toBe('Creating proxies · 12 of 12');
    expect(proxyStatus({ creating: { done: 1, total: 4 }, paused: 'lowDisk' })?.text).toBe('Proxies paused · Low disk space');
    expect(proxyStatus({ creating: null, paused: 'playback' })).toBeNull();
  });
});

describe('installMediaProxies', () => {
  it('follows PM.EngineProxies as the engine connection sets and clears it', async () => {
    const target = new EventTarget();
    const { PM } = fakePM();
    const dispose = installMediaProxies(PM, target);
    try {
      expect(PM.Proxies).toBeNull();
      const { bridge } = fakeBridge();
      PM.EngineProxies = bridge;
      target.dispatchEvent(new Event(ENGINE_PROXIES_CHANGED));
      expect(PM.Proxies?.bridge).toBe(bridge);
      const first = PM.Proxies;
      target.dispatchEvent(new Event(ENGINE_PROXIES_CHANGED));
      expect(PM.Proxies).toBe(first);
      delete PM.EngineProxies;
      target.dispatchEvent(new Event(ENGINE_PROXIES_CHANGED));
      expect(PM.Proxies).toBeNull();
    } finally {
      dispose();
    }
    await settle();
  });
});
