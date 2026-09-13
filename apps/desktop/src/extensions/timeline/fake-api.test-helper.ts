import type {
  Layer,
  PanelDefinition,
  PowermoveAPI,
  Selection,
  TimelineService,
} from 'powermove';

type EventHandler = (payload: unknown) => void;

export interface FakeAPIHarness {
  api: PowermoveAPI;
  state: {
    project: any;
    selection: Selection;
    time: number;
    playing: boolean;
    panel?: PanelDefinition;
    timeline?: TimelineService;
    disposers: Array<() => void>;
  };
  emit(event: string, payload?: unknown): void;
  dispose(): void;
}

export function fakePowermoveAPI(vi: { fn: (...args: any[]) => any }): FakeAPIHarness {
  const listeners = new Map<string, Set<EventHandler>>();
  const storage = new Map<string, unknown>();
  const services = new Map<string, unknown>();
  const state: FakeAPIHarness['state'] = {
    project: { id: 'project-1', w: 1920, h: 1080, fps: 30, dur: 10, work: [0, 10], layers: [], markers: [] },
    selection: { layers: [], keys: [], chan: null },
    time: 0,
    playing: false,
    disposers: [],
  };
  const uid = vi.fn((prefix = 'l') => `${prefix}-${Math.random()}`);
  const cloneLayer = vi.fn((layer: Layer) => {
    const clone = JSON.parse(JSON.stringify(layer)) as Layer;
    clone.id = uid('L');
    const baseName = layer.name.replace(/ (\d+)$/, '');
    clone.name = `${baseName} ${state.project.layers.filter((item: Layer) => item.name.startsWith(baseName)).length + 1}`;
    const renew = (property: unknown): void => {
      const channel = property as { kf?: Array<{ i: string }> } | null;
      for (const keyframe of channel?.kf ?? []) keyframe.i = uid('k');
    };
    Object.values(clone.p ?? {}).forEach(renew);
    return clone;
  });
  const emit = (event: string, payload?: unknown): void => {
    for (const handler of listeners.get(event) ?? []) handler(payload);
  };
  const api = {
    id: 'timeline', apiVersion: 1,
    manifest: { id: 'timeline', name: 'Timeline', version: '1.0.0', apiVersion: 1 },
    panels: {
      register: vi.fn((definition: PanelDefinition) => {
        state.panel = definition;
        return { dispose: vi.fn() };
      }),
      list: vi.fn(() => []), open: vi.fn(), close: vi.fn(), isOpen: vi.fn(() => false), refresh: vi.fn(),
    },
    commands: { register: vi.fn(() => ({ dispose: vi.fn() })), run: vi.fn(), has: vi.fn(() => false), list: vi.fn(() => []) },
    keybindings: { bind: vi.fn(() => ({ dispose: vi.fn() })), unbind: vi.fn(), list: vi.fn(() => []), chordOf: vi.fn(() => null) },
    effects: { register: vi.fn(() => ({ dispose: vi.fn() })), list: vi.fn(() => []), get: vi.fn(() => undefined) },
    transitions: { register: vi.fn(() => ({ dispose: vi.fn() })), list: vi.fn(() => []), get: vi.fn(() => undefined) },
    layers: { register: vi.fn(() => ({ dispose: vi.fn() })), list: vi.fn(() => []), get: vi.fn(() => undefined) },
    assets: { pick: vi.fn(async () => []), import: vi.fn(), get: vi.fn(), readText: vi.fn() },
    theme: { register: vi.fn(() => ({ dispose: vi.fn() })), activate: vi.fn(), active: vi.fn(() => ''), list: vi.fn(() => []), scheme: vi.fn(() => 'light'), setScheme: vi.fn() },
    palette: { registerProvider: vi.fn(() => ({ dispose: vi.fn() })), open: vi.fn() },
    menus: { contribute: vi.fn(() => ({ dispose: vi.fn() })), collect: vi.fn(() => []) },
    status: { register: vi.fn(() => ({ dispose: vi.fn() })), list: vi.fn(() => []) },
    project: {
      get: () => state.project,
      revision: () => Number(state.project.revision ?? 0),
      apply: vi.fn(() => ({ ok: true })),
      selection: () => ({ ...state.selection, layers: [...state.selection.layers], keys: [...state.selection.keys] }),
      select: vi.fn((ids: string[], add = false) => {
        const values = ([] as string[]).concat(ids).filter(Boolean);
        state.selection.layers = add ? [...new Set([...state.selection.layers, ...values])] : values;
        if (!add) state.selection.keys = [];
        emit('selection', state.selection);
      }),
      time: () => state.time,
      setTime: vi.fn((time: number) => { state.time = time; emit('time', time); }),
      play: vi.fn(), pause: vi.fn(), playing: () => state.playing,
      undo: vi.fn(), redo: vi.fn(), snapshot: vi.fn(async () => ''),
    },
    anim: {
      ev: vi.fn(), evP: vi.fn((_layer: unknown, property: any) => property?.v ?? null), active: vi.fn(() => true),
      findProp: vi.fn(), allProps: vi.fn((layer: any) => Object.entries(layer?.p ?? {}).map(([key, prop]) => ({ key, prop, label: key, group: '' }))),
      hasKeyAt: vi.fn(), setKey: vi.fn(), setKeyOn: vi.fn(), removeKey: vi.fn(), applyEaseTo: vi.fn(), wouldCycle: vi.fn(() => false),
      resolveContent: vi.fn((layer: any) => layer.d), expressionErrors: new WeakMap(), version: vi.fn(() => 0), touch: vi.fn(),
      worldMatrix: vi.fn(() => [1, 0, 0, 1, 0, 0]), localMatrix: vi.fn(() => [1, 0, 0, 1, 0, 0]),
      transformParentMatrix: vi.fn(() => [1, 0, 0, 1, 0, 0]), mul: vi.fn(() => [1, 0, 0, 1, 0, 0]),
    },
    model: {
      P: vi.fn((value: unknown) => ({ v: value, kf: [], expr: null })), CH: {}, KF: vi.fn(), BLENDS: [], TYPE_META: {}, MASK_SHAPES: [],
      mkLayer: vi.fn(), mkMask: vi.fn(), mkProject: vi.fn(), cloneLayer, normalizeFill: vi.fn(), layerDefinition: vi.fn(), curComp: () => state.project,
      layer: (id: string) => state.project.layers.find((layer: Layer) => layer.id === id) ?? null,
      byName: vi.fn(() => null),
    },
    selection: {
      get: () => state.selection,
      layers: () => state.selection.layers,
      first: () => state.project.layers.find((layer: Layer) => state.selection.layers.includes(layer.id)) ?? null,
      keys: () => state.selection.keys,
      chan: () => state.selection.chan,
      set: vi.fn((partial: Partial<Selection>) => { Object.assign(state.selection, partial); emit('selection', state.selection); }),
      select: vi.fn((ids: string[], add = false) => {
        const values = ([] as string[]).concat(ids).filter(Boolean);
        state.selection.layers = add ? [...new Set([...state.selection.layers, ...values])] : values;
        if (!add) state.selection.keys = [];
        emit('selection', state.selection);
      }),
      resolveSelectedKeys: vi.fn(() => []),
      keySelectionActive: false,
    },
    groups: { ancestors: vi.fn(() => []), transformRoots: vi.fn((ids: string[]) => ids), bounds: vi.fn(() => null), span: vi.fn((layer: any) => layer), expand: vi.fn((ids: string[]) => ids), normalizeStack: vi.fn(), moveToGroup: vi.fn((ids: string[], group: string | null) => ({ ids, group })) },
    transport: {
      time: () => state.time,
      setTime: vi.fn((time: number) => { state.time = time; emit('time', time); }),
      play: vi.fn(), pause: vi.fn(), toggle: vi.fn(), playing: () => state.playing, step: vi.fn(), quality: 1,
      perf: { fps: 0, ms: 0, drops: 0, budget: 0, auto: false }, invalidate: vi.fn(), previewResolution: null,
    },
    history: { do: vi.fn((_label: string, run: () => unknown) => run()), begin: vi.fn(), commit: vi.fn(() => true), cancel: vi.fn(), undo: vi.fn(() => true), redo: vi.fn(() => true), external: vi.fn(), selection: vi.fn() },
    edit: { apply: vi.fn(() => ({ ok: true })), begin: vi.fn(), commit: vi.fn(() => ({ ok: true })), cancel: vi.fn(() => true), dispatch: vi.fn(() => ({ ok: true })), mutate: vi.fn((_label: string, run: () => unknown) => run()) },
    media: { timing: { isTimed: vi.fn(() => false), rate: vi.fn(() => 1), earliestStart: vi.fn((layer: any) => layer.from) }, importFiles: vi.fn(), commandForAsset: vi.fn(), audio: { drawWaveform: vi.fn(() => false) }, assets: { get: vi.fn(), add: vi.fn(), kind: vi.fn() }, fonts: { bundled: [], system: [], families: [], setSystemFamilies: vi.fn(), options: vi.fn(() => []), ensure: vi.fn() } },
    render: { gl: { bounds: vi.fn(), pick: vi.fn(), init: vi.fn(), resize: vi.fn(), previewViewport: null, context: null }, raster: vi.fn(), renderFrameTo: vi.fn(), snapshot: vi.fn() },
    uiState: { getLayerCollapsed: vi.fn((layer: any) => !!layer.collapsed), setLayerCollapsed: vi.fn((layer: any, value: boolean) => (layer.collapsed = value)), getGroupCollapsed: vi.fn((layer: any) => !!layer.groupCollapsed), setGroupCollapsed: vi.fn((layer: any, value: boolean) => (layer.groupCollapsed = value)), getKeyHandles: vi.fn(), setKeyHandles: vi.fn(), getFxOpen: vi.fn(), setFxOpen: vi.fn(), getReveal: vi.fn((layer: any) => layer.reveal ?? null), setReveal: vi.fn((layer: any, value: string[]) => (layer.reveal = value)), setShaderMeta: vi.fn() },
    ui: { controls: {}, toast: vi.fn(), confirm: vi.fn(), menu: vi.fn(), modal: vi.fn(), icon: vi.fn((name: string) => `<svg data-icon="${name}"></svg>`), drag: vi.fn(() => ({ cancel: vi.fn() })), closeMenus: vi.fn(), showLayerMenu: vi.fn(), showParentMenu: vi.fn(), beginParentPick: vi.fn(), openShaderEditor: vi.fn(), gesture: class {} },
    dnd: { ASSET_MIME: '', FX_MIME: '', startAssetDrag: vi.fn(), mediaDrag: null, hasAssetDrag: vi.fn(), hasFileDrag: vi.fn(), hasMediaDrag: vi.fn(), readAssetDrag: vi.fn(), hasFxDrag: vi.fn(), readFxDrag: vi.fn(), applyFxDrop: vi.fn() },
    workspace: { current: vi.fn(() => null), mutate: vi.fn(), hasPanel: vi.fn(), addPanel: vi.fn(), movePanel: vi.fn(), removePanel: vi.fn(), hidePanel: vi.fn(), restorePanel: vi.fn(), refresh: vi.fn() },
    util: { round: vi.fn((value: number, places = 2) => Number(value.toFixed(places))), clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)), lerp: vi.fn(), snapF: vi.fn((time: number, fps: number) => Math.round(time * fps) / fps), tc: vi.fn(() => '00:00:00:00'), parseTc: vi.fn(), uid, hex2rgb: vi.fn(() => [0, 0, 0]), rgb2hex: vi.fn() },
    ease: { nameOf: vi.fn(() => 'linear'), PRESETS: {} },
    space3d: { CHANNELS_3D: {}, local3D: vi.fn(), parent3D: vi.fn(), world3D: vi.fn(), is3DLayer: vi.fn(), perspectiveAmount: vi.fn(), planeMatrix: vi.fn(), projectPoint: vi.fn(), inversePlane: vi.fn(), planeContains: vi.fn() },
    services: {
      register: vi.fn((name: string, implementation: unknown) => {
        services.set(name, implementation);
        if (name === 'timeline') state.timeline = implementation as TimelineService;
        const dispose = (): void => {
          if (services.get(name) === implementation) services.delete(name);
          if (name === 'timeline' && state.timeline === implementation) state.timeline = undefined;
        };
        state.disposers.push(dispose);
        return { dispose };
      }),
      get: <T,>(name: string): T | null => services.get(name) as T ?? null,
    },
    storage: { get: <T,>(key: string): T | undefined => storage.get(key) as T | undefined, set: (key: string, value: unknown) => storage.set(key, value), delete: (key: string) => storage.delete(key) },
    events: { on: (event: string, handler: EventHandler) => { const set = listeners.get(event) ?? new Set(); set.add(handler); listeners.set(event, set); return { dispose: () => set.delete(handler) }; }, emit },
    extensions: { list: vi.fn(() => []), setEnabled: vi.fn(), remove: vi.fn(), reload: vi.fn(), reveal: vi.fn(), requestFix: vi.fn() },
    host: { state: { doc: undefined, sel: undefined, transport: undefined, perf: undefined }, mount: vi.fn() },
    on: (event: string, handler: EventHandler) => { const set = listeners.get(event) ?? new Set(); set.add(handler); listeners.set(event, set); return { dispose: () => set.delete(handler) }; },
    log: vi.fn(),
    onDispose: (handler: () => void) => state.disposers.push(handler),
  } as unknown as PowermoveAPI;
  return {
    api,
    state,
    emit,
    dispose(): void { for (const disposer of state.disposers.splice(0).reverse()) disposer(); },
  };
}
