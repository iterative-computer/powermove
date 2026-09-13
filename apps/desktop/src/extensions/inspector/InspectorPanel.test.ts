// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ControlsAPI, InspectorService, PowermoveAPI, ShaderHooks } from 'powermove';
import type { InspectorRuntimeService } from './context';

import ColorField from '../../renderer/src/controls/ColorField.svelte';
import FillField from '../../renderer/src/controls/FillField.svelte';
import FontField from '../../renderer/src/controls/FontField.svelte';
import NumField from '../../renderer/src/controls/NumField.svelte';
import Row from '../../renderer/src/controls/Row.svelte';
import Section from '../../renderer/src/controls/Section.svelte';
import SelectField from '../../renderer/src/controls/SelectField.svelte';
import TextField from '../../renderer/src/controls/TextField.svelte';
import ToggleField from '../../renderer/src/controls/ToggleField.svelte';
import { channelBinding, compositionBinding, contentBinding, layerFieldBinding } from '../../renderer/src/controls/binding';
import { doc } from '../../renderer/src/state/document.svelte';
import { setSelection, sel } from '../../renderer/src/state/selection.svelte';
import { perf, transport } from '../../renderer/src/state/transport.svelte';
import { clearEffectClipboard } from './effect-clipboard';
import InspectorPanel from './InspectorPanel.svelte';
import activate from './index';

type Channel = { v: number; kf: Array<{ t: number; v: number }>; expr: string | null };
type TestLayer = Record<string, any> & { p: Record<string, Channel> };

interface InspectorTestBackend {
  proj?: any;
  TL?: any;
  setTool?: any;
  FX?: any;
  cmd?: any;
  ev?: any;
  evP?: any;
  findProp?: any;
  hasKeyAt?: any;
  setKeyOn?: any;
  removeKey?: any;
  applyEaseTo?: any;
  touch?: any;
  wouldCycle?: any;
  CH?: any;
  BLENDS?: any;
  MASK_SHAPES?: any;
  TYPE_META?: any;
  P?: any;
  L?: any;
  curComp?: any;
  mkMask?: any;
  firstSel?: any;
  sel?: any;
  groupAncestors?: any;
  groupBounds?: any;
  expandGroups?: any;
  invalidate?: any;
  hist?: any;
  Edit?: any;
  UIState?: any;
  menu?: any;
  modal?: any;
  toast?: any;
  drag?: any;
  beginParentPick?: any;
  openShaderEditor?: any;
  closeMenus?: any;
  Fonts?: any;
  round?: any;
  clamp?: any;
  uid?: any;
  toggleStopwatch?: any;
  selLayers?: any;
  time?: number;
  ICONS?: any;
}

const CHANNELS = [
  'anchor.x', 'anchor.y', 'position.x', 'position.y', 'scale.x',
  'scale.y', 'rotation', 'opacity', 'skew'
] as const;

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;
let activeApi: PowermoveAPI | undefined;

function bump(kind: 'values' | 'structure') {
  doc.bump(kind);
  activeApi?.events.emit('project:changed', { kind });
}

const controlsFor = (getAPI: () => PowermoveAPI): ControlsAPI => ({
  NumField: NumField as ControlsAPI['NumField'],
  ColorField: ColorField as ControlsAPI['ColorField'],
  FillField: FillField as ControlsAPI['FillField'],
  FontField: FontField as ControlsAPI['FontField'],
  SelectField: SelectField as ControlsAPI['SelectField'],
  TextField: TextField as ControlsAPI['TextField'],
  ToggleField: ToggleField as ControlsAPI['ToggleField'],
  Row: Row as ControlsAPI['Row'],
  Section: Section as ControlsAPI['Section'],
  binding: {
    channelBinding: (layerId, channel, options) => channelBinding(getAPI(), layerId, channel, options),
    compositionBinding: (field, options) => compositionBinding(getAPI(), field as any, options),
    contentBinding: (layerId, field, options) => contentBinding(getAPI(), layerId, field, options),
    layerFieldBinding: (layerId, field, options) => layerFieldBinding(getAPI(), layerId, field as any, options)
  }
});

function apiFor(runtime: InspectorTestBackend, register = vi.fn()): PowermoveAPI {
  const implementations = new Map<string, unknown>();
  const listeners = new Map<string, Set<(payload: unknown) => void>>();
  const events: PowermoveAPI['events'] = {
    on(event, listener) {
      const key = String(event);
      const eventListeners = listeners.get(key) ?? new Set<(payload: unknown) => void>();
      eventListeners.add(listener as (payload: unknown) => void);
      listeners.set(key, eventListeners);
      return { dispose: () => eventListeners.delete(listener as (payload: unknown) => void) };
    },
    emit(event, payload) {
      for (const listener of listeners.get(String(event)) ?? []) listener(payload);
    }
  };
  if (runtime.TL) implementations.set('timeline', runtime.TL);
  if (runtime.setTool) implementations.set('tool', { tool: 'select', toolShape: 'rect', setTool: runtime.setTool });
  const definitions = Object.entries(runtime.FX ?? {}).map(([id, definition]) => ({ id, ...(definition as object) }));
  let api: PowermoveAPI;
  api = {
    id: 'inspector',
    apiVersion: 1,
    manifest: { id: 'inspector', name: 'Inspector', version: '1.0.0', apiVersion: 1 },
    panels: { register },
    commands: { run: runtime.cmd ?? vi.fn() },
    project: { get: () => runtime.proj ?? project([]) },
    anim: {
      ev: runtime.ev ?? (() => null), evP: runtime.evP ?? ((_layer: any, property: any) => property?.v),
      findProp: (layer: any, path: string) => runtime.findProp?.(layer, path) ?? layer.p?.[path],
      hasKeyAt: runtime.hasKeyAt ?? (() => null), setKeyOn: runtime.setKeyOn ?? vi.fn(),
      removeKey: runtime.removeKey ?? vi.fn(), applyEaseTo: runtime.applyEaseTo ?? vi.fn(),
      touch: runtime.touch ?? vi.fn(), wouldCycle: runtime.wouldCycle ?? (() => false),
      resolveContent: (layer: any) => layer.d, expressionErrors: new WeakMap(), version: () => 0
    },
    model: {
      CH: runtime.CH ?? {}, BLENDS: runtime.BLENDS ?? [], MASK_SHAPES: runtime.MASK_SHAPES ?? [],
      TYPE_META: runtime.TYPE_META ?? {}, P: runtime.P ?? ((value: unknown) => ({ v: value, kf: [], expr: null })),
      layer: runtime.L ?? (() => null), curComp: runtime.curComp ?? (() => runtime.proj),
      mkMask: runtime.mkMask ?? vi.fn(), layerDefinition: vi.fn(),
      normalizeFill: (value: any, fallback = '#000000') => typeof value === 'string'
        ? { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: value, position: 0 }] }
        : value ?? { type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: fallback, position: 0 }] }
    },
    selection: {
      get: () => runtime.sel ?? { layers: [], keys: [], chan: null },
      layers: () => runtime.sel?.layers ?? [], keys: () => runtime.sel?.keys ?? [], chan: () => runtime.sel?.chan ?? null,
      first: runtime.firstSel ?? (() => null),
      set: (partial: object) => Object.assign(runtime.sel, partial),
      select: (ids: string[]) => { runtime.sel.layers = [...ids]; setSelection({ ...runtime.sel }); },
      resolveSelectedKeys: () => []
    },
    groups: {
      ancestors: runtime.groupAncestors ?? (() => []),
      bounds: (...args: any[]) => runtime.groupBounds?.(...args) ?? null,
      expand: runtime.expandGroups ?? ((ids: string[]) => ids)
    },
    transport: { time: () => transport.time, invalidate: runtime.invalidate ?? vi.fn() },
    history: runtime.hist ?? { do: (_label: string, operation: () => unknown) => operation(), begin: vi.fn(), commit: vi.fn(), cancel: vi.fn() },
    edit: runtime.Edit ?? { apply: vi.fn(), begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn(), mutate: (_label: string, operation: () => unknown) => operation() },
    effects: { list: () => definitions, get: (id: string) => definitions.find((definition) => definition.id === id) },
    layers: { get: () => undefined },
    assets: { get: () => undefined },
    render: { gl: { compileError: () => null } },
    uiState: {
      ...(runtime.UIState ?? {}),
      getShaderMeta: runtime.UIState?.getShaderMeta ?? (() => null),
      setShaderMeta: runtime.UIState?.setShaderMeta ?? vi.fn()
    },
    ui: {
      controls: controlsFor(() => api),
      icon: (name: string) => `<svg data-icon="${name}" aria-hidden="true"><path/></svg>`,
      menu: runtime.menu ?? vi.fn(), modal: runtime.modal ?? vi.fn(), toast: runtime.toast ?? vi.fn(),
      drag: runtime.drag ?? vi.fn(), beginParentPick: runtime.beginParentPick ?? vi.fn(),
      openShaderEditor: runtime.openShaderEditor ?? vi.fn(), closeMenus: runtime.closeMenus ?? vi.fn()
    },
    media: { fonts: runtime.Fonts ?? { options: (value: string) => [value], ensure: vi.fn() } },
    util: { round: runtime.round ?? ((value: number) => value), clamp: runtime.clamp ?? ((value: number, min: number, max: number) => Math.max(min, Math.min(max, value))), uid: runtime.uid ?? (() => 'test-id') },
    services: {
      register: (name: string, implementation: unknown) => { implementations.set(name, implementation); return { dispose: () => implementations.delete(name) }; },
      get: <T,>(name: string) => (implementations.get(name) as T | undefined) ?? null
    },
    events,
    log: vi.fn()
  } as unknown as PowermoveAPI;
  return api;
}

function layer(id: string, opacity = 100): TestLayer {
  const p = Object.fromEntries(CHANNELS.map((channel) => [channel, {
    v: channel === 'opacity' ? opacity : channel.startsWith('scale.') ? 100 : 0,
    kf: [],
    expr: null
  }])) as Record<string, Channel>;
  return {
    id,
    type: 'solid',
    name: `Layer ${id}`,
    from: 0,
    dur: 5,
    on: true,
    color: '#3366CC',
    blend: 'normal',
    mblur: false,
    parent: null,
    p,
    fx: [],
    masks: [],
    d: { color: '#3366CC', w: 640, h: 360, radius: 0 }
  };
}

function shapeLayer(id: string): TestLayer {
  const candidate = layer(id);
  const property = (value: unknown) => ({ v: value, kf: [], expr: null });
  candidate.type = 'shape';
  candidate.name = 'Imported SVG';
  candidate.d = {
    paths: [{
      id: 'path-1',
      name: 'Powermove mark',
      parent: null,
      vertices: [],
      p: Object.fromEntries(Object.entries({
        x: 0, y: 0, rotation: 0, scaleX: 100, scaleY: 100,
        closed: true, fill: '#FFFFFF', fillEnabled: true, fillOpacity: 100,
        stroke: '#000000', strokeWidth: 0, strokeOpacity: 100,
        trimStart: 0, trimEnd: 100, trimOffset: 0,
        copies: 1, repeatX: 30, repeatY: 0, repeatRotation: 0
      }).map(([key, value]) => [key, property(value)]))
    }]
  };
  return candidate;
}

function project(layers: TestLayer[]) {
  return {
    id: 'project-1',
    name: 'Test composition',
    w: 1920,
    h: 1080,
    dur: 10,
    fps: 30,
    work: [0, 10],
    bg: '#000000',
    backgroundFill: {
      type: 'solid', angle: 0, stops: [{ id: 'stop-1', color: '#000000', position: 0 }]
    },
    layers,
    assets: {},
    params: {}
  };
}

function setup(
  testLayers: TestLayer[],
  selected = testLayers.map(({ id }) => id),
  options: { fxOpen?: boolean } = {}
) {
  let dragOptions: { up(): void; move?(dx: number, dy: number, event: PointerEvent): void; cancel?(): void } | undefined;
  const currentProject = project(testLayers);
  const apply = vi.fn((_input?: unknown) => ({ ok: true }));
  const menu = vi.fn();
  const runtime: InspectorTestBackend = {
    proj: currentProject,
    CH: Object.fromEntries(CHANNELS.map((channel) => [channel, {
      step: 1,
      unit: channel === 'opacity' || channel.startsWith('scale.') ? '%' : undefined
    }])),
    BLENDS: ['normal', 'screen'],
    MASK_SHAPES: ['rect', 'ellipse'],
    TYPE_META: { solid: { label: 'Solid' }, text: { label: 'Text' }, shape: { label: 'Shape' }, null: { label: 'Null' }, group: { label: 'Group', transform: true } },
    FX: {
      blur: { label: 'Gaussian Blur', group: 'Blur', params: [{ k: 'amount', label: 'Amount', step: 1, min: 0, max: 100 }] },
      duotone: { label: 'Duotone', group: 'Color', params: [{ k: 'shadow', label: 'Shadow', type: 'color' }] }
    },
    ICONS: Object.fromEntries(['layers', 'plus', 'clock', 'diamond', 'chev', 'eye', 'x'].map((name) => [name, '<path/>'])),
    sel: { layers: [...selected], keys: [], chan: null },
    TL: { graph: false, reveal: vi.fn() },
    Edit: {
      apply,
      begin: vi.fn(),
      dispatch: vi.fn(),
      commit: vi.fn(),
      cancel: vi.fn(),
      mutate: vi.fn((_label: string, operation: () => void) => operation())
    },
    hist: {
      do: vi.fn((_label: string, operation: () => void) => operation()),
      begin: vi.fn(),
      commit: vi.fn(),
      cancel: vi.fn()
    },
    UIState: {
      getFxOpen: vi.fn(() => !!options.fxOpen),
      setFxOpen: vi.fn()
    },
    menu,
    modal: vi.fn(),
    cmd: vi.fn(),
    toast: vi.fn(),
    setTool: vi.fn(),
    touch: vi.fn(),
    invalidate: vi.fn(),
    uid: vi.fn((prefix: string) => `${prefix}-new`),
    P: vi.fn((value: unknown) => ({ v: value, kf: [], expr: null })),
    wouldCycle: vi.fn(() => false),
    curComp: () => currentProject,
    L: (id: string) => currentProject.layers.find((candidate) => candidate.id === id),
    mkMask: vi.fn(() => ({
      id: 'mask-new', shape: 'rect', mode: 'add', on: true,
      p: Object.fromEntries(['x', 'y', 'w', 'h', 'rotation', 'feather'].map((key) => [key, { v: 0, kf: [], expr: null }]))
    })),
    applyEaseTo: vi.fn(),
    round: (value: number, precision: number) => Number(value.toFixed(precision)),
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    drag: vi.fn((_event: PointerEvent, options: typeof dragOptions) => { dragOptions = options; }),
    ev: (candidate: TestLayer, channel: string) => candidate.p[channel]?.v,
    evP: vi.fn((_candidate: TestLayer, property: Channel) => property.v),
    hasKeyAt: (candidate: TestLayer, property: Channel, time: number) =>
      property.kf.find((key) => key.t === time - candidate.from),
    setKeyOn: (property: Channel, time: number, value: number) => property.kf.push({ t: time, v: value }),
    removeKey: (property: Channel, key: { t: number; v: number }) => {
      property.kf.splice(property.kf.indexOf(key), 1);
    },
    toggleStopwatch: (candidate: TestLayer, channel: string, time: number) => {
      const property = candidate.p[channel]!;
      property.kf = property.kf.length ? [] : [{ t: time - candidate.from, v: property.v }];
    },
    firstSel: () => currentProject.layers.find(({ id }) => id === selected[0]) ?? null,
    selLayers: () => currentProject.layers.filter(({ id }) => runtime.sel.layers.includes(id)),
    time: 0
  };

  doc.replace(currentProject as any);
  setSelection({ layers: selected, keys: [], chan: null });
  transport.time = 0;
  const api = apiFor(runtime);
  activeApi = api;
  activate(api);
  instance = mount(InspectorPanel, { target, props: { panelId: 'inspector', spec: {}, api } });
  flushSync();

  return { runtime, api, apply, menu, drag: () => dragOptions! };
}

function channelRow(layerId: string, channel: string): HTMLElement {
  const row = target.querySelector<HTMLElement>(
    `[data-channel-instance][data-layer-id="${layerId}"][data-channel="${channel}"]`
  );
  if (!row) throw new Error(`Missing ${layerId} ${channel} channel row`);
  return row;
}

function labelledSpinbutton(label: string): HTMLInputElement {
  const result = [...target.querySelectorAll<HTMLInputElement>('[role="spinbutton"]')].find((input) => {
    if (input.getAttribute('aria-label') === label) return true;
    const id = input.getAttribute('aria-labelledby');
    return !!id && document.getElementById(id)?.textContent?.trim() === label;
  });
  if (!result) throw new Error(`Missing ${label} spinbutton`);
  return result;
}

function labelledSelect(label: string): HTMLSelectElement {
  const result = [...target.querySelectorAll<HTMLSelectElement>('select')].find((select) => {
    const id = select.getAttribute('aria-labelledby');
    return !!id && document.getElementById(id)?.textContent?.trim() === label;
  });
  if (!result) throw new Error(`Missing ${label} select`);
  return result;
}

beforeEach(() => {
  clearEffectClipboard();
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  activeApi = undefined;
  target.remove();
  setSelection({ layers: [], keys: [], chan: null });
  transport.time = 0;
  vi.restoreAllMocks();
});

describe('InspectorPanel', () => {
  it('exposes the compact footprint of a transparent null object', () => {
    const candidate = layer('N'); candidate.type = 'null'; candidate.name = 'Null';
    candidate.d = { color: '#6A6A70', w: 100, h: 100, radius: 0 };
    candidate.p.opacity!.v = 0;
    setup([candidate], ['N']);

    expect(labelledSpinbutton('Width').value).toBe('100px');
    expect(labelledSpinbutton('Height').value).toBe('100px');
    expect(labelledSpinbutton('Opacity').value).toBe('0%');
  });

  it('offers parent selection when a group layer is selected', () => {
    const group = layer('G'); group.type = 'group'; group.name = 'Group'; group.d = {};
    const rig = layer('R'); rig.type = 'solid'; rig.name = 'Rig';
    setup([group, rig], ['G']);

    expect([...labelledSelect('Parent').options].map(option => option.textContent)).toEqual(['None', 'Rig']);
    expect(target.querySelector('[aria-label="Pick parent layer"]')).not.toBeNull();
  });

  it('exposes effects, masks, blend, motion blur and track mattes for groups', () => {
    const group = layer('G'); group.type = 'group'; group.name = 'Group'; group.d = {};
    const matte = layer('M'); matte.name = 'Matte';
    const { runtime } = setup([group, matte], ['G']);
    runtime.groupBounds = vi.fn(() => ({ x0: 20, y0: 30, x1: 220, y1: 130, w: 200, h: 100, ax: 0, ay: 0 }));

    const headings = [...target.querySelectorAll('.sec')].map((item) => item.textContent?.trim());
    expect(headings).toContain('Effects');
    expect(headings).toContain('Masks');
    expect(target.querySelector('[aria-label="Add effect"]')).not.toBeNull();
    expect(labelledSelect('Blend mode')).not.toBeNull();
    expect([...target.querySelectorAll('button[aria-labelledby]')].some((button) =>
      document.getElementById(button.getAttribute('aria-labelledby')!)?.textContent?.trim() === 'Motion blur'
    )).toBe(true);
    expect(labelledSelect('Track matte')).not.toBeNull();

    target.querySelector<HTMLButtonElement>('[aria-label="Add mask"]')!.click();
    expect(runtime.mkMask).toHaveBeenCalledWith('rect', runtime.proj);
    expect(group.masks).toHaveLength(1);
    expect(group.masks[0].p).toMatchObject({
      x: { v: 120 }, y: { v: 80 }, w: { v: 200 }, h: { v: 100 },
    });
  });

  it('offers a group as a transform parent without changing membership', () => {
    const child = layer('C'); child.name = 'Child';
    const group = layer('G'); group.type = 'group'; group.name = 'Rig Group'; group.d = {};
    const { apply } = setup([child, group], ['C']);

    expect([...labelledSelect('Parent').options].map(option => option.textContent)).toEqual(['None', 'Rig Group']);
    labelledSelect('Parent').value = '1';
    labelledSelect('Parent').dispatchEvent(new Event('change', { bubbles: true }));

    expect(apply).toHaveBeenCalledWith(
      [{ type: 'set_layer', target: child.id, patch: { parent: group.id } }],
      { label: 'Parent layers', origin: 'inspector' },
    );
  });

  it('activates by registering the inspector panel contribution', () => {
    const register = vi.fn(() => ({ dispose: vi.fn() }));

    activate(apiFor({}, register));

    expect(register).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      id: 'inspector',
      title: 'Properties',
      component: InspectorPanel
    }));
  });

  it('registers the persistent Properties panel and typed inspector services', () => {
    let definition: Record<string, any> | undefined;
    const runtime: InspectorTestBackend = {
      P: vi.fn((value: unknown) => ({ v: value, kf: [], expr: null })),
      UIState: { setShaderMeta: vi.fn() },
    };

    const register = vi.fn((next: Record<string, any>) => {
      definition = next;
      return { dispose: vi.fn() };
    });
    const api = apiFor(runtime, register);
    activate(api);

    expect(register).toHaveBeenCalledOnce();
    expect(definition).toMatchObject({ id: 'inspector', title: 'Properties' });
    expect(definition?.component).toBe(InspectorPanel);
    expect(definition?.header).toBeTypeOf('function');
    const service = api.services.get<InspectorRuntimeService>('inspector');
    expect(service?.body).toBeNull();
    expect(service?.syncs).toEqual([]);
    expect(service?.refresh).toBeTypeOf('function');
    expect(service?.focusText).toBeTypeOf('function');
    expect(() => service?.refresh()).not.toThrow();

    const shader = { id: 'shader', type: 'shader', d: { code: 'uniform float amount; // @param 1', uniforms: { stale: { v: 2 } } } };
    api.services.get<ShaderHooks>('shaderHooks')?.syncShaderUniforms(shader as any);
    expect(runtime.UIState.setShaderMeta).toHaveBeenCalledWith(shader, {
      udefs: [expect.objectContaining({ name: 'amount', def: 1 })]
    });
    expect(shader.d.uniforms).toEqual({ amount: { v: 1, kf: [], expr: null } });
  });

  it('caps multi-selection rendering to the first layer and keeps the header consistent', () => {
    const a = layer('A', 10), b = layer('B', 20);
    setup([a, b], ['A', 'B']);

    expect(target.querySelector('[data-inspector-layer="A"]')).not.toBeNull();
    expect(target.querySelector('[data-inspector-layer="B"]')).toBeNull();
    expect(target.querySelector('[data-inspector-header]')?.textContent).toContain('Layer A');
    expect(target.querySelector('[role="status"]')?.textContent).toContain('2 layers selected · shared edits');
  });

  it('does not repeat the Properties heading above composition controls', () => {
    setup([], []);

    expect(target.querySelector('[data-inspector-header]')).toBeNull();
    expect(target.textContent).toContain('Composition');
  });

  it('presents imported SVG paths as Motioner-style property sections', () => {
    const candidate = shapeLayer('SVG');
    const { apply, runtime } = setup([candidate]);
    const headings = [...target.querySelectorAll('.sec')].map((section) => section.textContent?.trim());

    expect(headings.slice(0, 5)).toEqual(['Transform', 'Path', 'Fill', 'Stroke', 'Effects']);
    expect(target.querySelector('[aria-label="Edit Powermove mark vertices"]')).not.toBeNull();
    expect(target.querySelector('details.advanced')?.hasAttribute('open')).toBe(false);
    expect(target.querySelector('[aria-label="Remove Powermove mark"]')).toBeNull();

    target.querySelector<HTMLButtonElement>('[aria-label="Add stroke"]')?.click();
    expect(apply).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'set_property', target: 'SVG', path: 'g.path-1.strokeWidth', value: 1 }),
      { label: 'Add stroke', origin: 'inspector' }
    );
    expect(runtime.setTool).not.toHaveBeenCalled();
  });

  it('updates a channel value on the values tick without remounting its row', () => {
    const candidate = layer('A', 40);
    setup([candidate], ['A']);
    const row = channelRow('A', 'opacity');
    const marker = row.dataset.channelInstance;
    const input = row.querySelector<HTMLInputElement>('[role="spinbutton"]')!;
    expect(input.value).toBe('40%');

    candidate.p.opacity!.v = 55;
    bump('values');
    flushSync();

    const updated = channelRow('A', 'opacity');
    expect(updated.dataset.channelInstance).toBe(marker);
    expect(updated).toBe(row);
    expect(updated.querySelector<HTMLInputElement>('[role="spinbutton"]')?.value).toBe('55%');
    expect(updated.querySelector('[role="spinbutton"]')?.getAttribute('aria-valuenow')).toBe('55');
  });

  it('keeps both axes in the same Scale row when linking or unlinking', () => {
    const candidate = layer('A');
    candidate.scaleLinked = true;
    candidate.p['scale.x']!.v = 180;
    candidate.p['scale.y']!.v = 90;
    const { apply } = setup([candidate]);
    const row = channelRow('A', 'scale.x');
    const x = labelledSpinbutton('Scale X');
    const y = labelledSpinbutton('Scale Y');
    expect(x.value).toBe('180%');
    expect(y.value).toBe('90%');
    expect(row.getAttribute('aria-label')).toBe('Scale property');
    expect(row.querySelectorAll('[role="spinbutton"]')).toHaveLength(2);
    expect(target.querySelector('[data-channel="scale.y"]')).toBeNull();
    const link = row.querySelector<HTMLButtonElement>('[aria-label="Link Scale X and Y"]')!;
    link.click();
    expect(apply).toHaveBeenCalledExactlyOnceWith(
      { type: 'set_layer', target: 'A', patch: { scaleLinked: false } },
      { label: 'Link scale axes', origin: 'inspector' }
    );
    candidate.scaleLinked = false;
    bump('values');
    flushSync();
    expect(channelRow('A', 'scale.x')).toBe(row);
    expect(labelledSpinbutton('Scale X')).toBe(x);
    expect(labelledSpinbutton('Scale Y')).toBe(y);
    expect(row.getAttribute('aria-label')).toBe('Scale property');
    expect(target.querySelector('[data-channel="scale.y"]')).toBeNull();
    expect(labelledSpinbutton('Scale X').value).toBe('180%');
    expect(labelledSpinbutton('Scale Y').value).toBe('90%');
    expect(link.getAttribute('aria-pressed')).toBe('false');
    candidate.scaleLinked = true;
    bump('values');
    flushSync();
    expect(labelledSpinbutton('Scale X')).toBe(x);
    expect(labelledSpinbutton('Scale Y')).toBe(y);
    expect(target.querySelector('[data-channel="scale.y"]')).toBeNull();
  });

  it('edits both linked axes proportionally and only the chosen axis when unlinked', () => {
    const candidate = layer('A');
    candidate.scaleLinked = true;
    candidate.p['scale.y']!.v = 50;
    const { apply } = setup([candidate]);
    const increment = (label: string) => labelledSpinbutton(label).dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    );
    increment('Scale X');
    expect(apply).toHaveBeenCalledExactlyOnceWith([
      expect.objectContaining({ type: 'set_property', path: 'scale.x', value: 101 }),
      expect.objectContaining({ type: 'set_property', path: 'scale.y', value: 50.5 })
    ], { label: 'Scale', origin: 'inspector' });
    apply.mockClear();
    increment('Scale Y');
    expect(apply).toHaveBeenCalledExactlyOnceWith([
      expect.objectContaining({ type: 'set_property', path: 'scale.y', value: 51 }),
      expect.objectContaining({ type: 'set_property', path: 'scale.x', value: 102 })
    ], { label: 'Scale', origin: 'inspector' });
    candidate.scaleLinked = false;
    bump('values');
    flushSync();
    apply.mockClear();
    increment('Scale Y');
    expect(apply).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: 'set_property', path: 'scale.y', value: 51 }),
      { label: 'Scale Y', origin: 'inspector' }
    );
  });

  it.each([true, false])('animates both axes and handles partial keys when scaleLinked=%s', (linked) => {
    const candidate = layer('A');
    candidate.scaleLinked = linked;
    const { runtime, menu, apply } = setup([candidate]);
    apply.mockImplementation((commands?: any)=>{for(const c of [].concat(commands) as any[]){const p=candidate.p[c.path];if(!p)continue;if(c.type==='replace_keyframes')p.kf=[];else if(c.type==='set_property'&&c.mode==='keyframe')p.kf.push({t:c.time,v:c.value});}return {ok:true};});
    const row = channelRow('A', 'scale.x');
    const diamond = row.querySelector<HTMLButtonElement>('.property-stopwatch')!;
    expect(row.querySelector('.well .kf')).toBeNull();
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    diamond.click();
    bump('values'); flushSync();
    expect(apply).toHaveBeenLastCalledWith(expect.any(Array), { label: 'Add keyframe for Scale', origin: 'inspector' });
    expect(runtime.TL.reveal).not.toHaveBeenCalled();
    expect(candidate.p['scale.x']!.kf).toHaveLength(1);
    expect(candidate.p['scale.y']!.kf).toHaveLength(1);
    // An existing project may have a key on just one of the linked axes.
    candidate.p['scale.y']!.kf = [];
    bump('values');
    flushSync();
    expect(diamond.classList.contains('at-key')).toBe(false);
    diamond.click();
    bump('values');
    flushSync();
    expect(candidate.p['scale.x']!.kf).toHaveLength(1);
    expect(candidate.p['scale.y']!.kf).toHaveLength(1);
    expect(diamond.classList.contains('at-key')).toBe(true);
    diamond.click();
    bump('values');
    flushSync();
    expect(candidate.p['scale.x']!.kf).toHaveLength(0);
    expect(candidate.p['scale.y']!.kf).toHaveLength(0);
    /* Removing the animation lives in the context menu. */
    candidate.p['scale.y']!.kf = [{ t: 0, v: 100 }];
    bump('values');
    flushSync();
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
    const items = menu.mock.calls.at(-1)![1] as Array<{ label?: string; run?: (e?: any) => void }>;
    items.find((item) => item.label === 'Remove animation')!.run!(new MouseEvent('click'));
    expect(candidate.p['scale.x']!.kf).toHaveLength(0);
    expect(candidate.p['scale.y']!.kf).toHaveLength(0);
  });

  it('toggles only playhead keys with the property diamond', () => {
    const candidate = layer('A');
    candidate.p['scale.x']!.kf = [{ t: 0, v: 50 }, { t: 2, v: 100 }];
    candidate.p['scale.y']!.kf = [{ t: 0, v: 50 }, { t: 2, v: 100 }];
    setup([candidate]);
    transport.time = 1; bump('values'); flushSync();
    const button = channelRow('A', 'scale.x').querySelector<HTMLButtonElement>('.property-stopwatch')!;
    button.click(); bump('values'); flushSync();
    expect(candidate.p['scale.x']!.kf.map((k: any) => k.t).sort()).toEqual([0, 1, 2]);
    button.click(); bump('values'); flushSync();
    expect(candidate.p['scale.x']!.kf.map((k: any) => k.t)).toEqual([0, 2]);
    expect(candidate.p['scale.y']!.kf.map((k: any) => k.t)).toEqual([0, 2]);
  });

  it('selects a property without opening its timeline rows', () => {
    const candidate = layer('A');
    const { runtime } = setup([candidate]);
    for (const channel of ['scale.x', 'opacity']) {
      channelRow('A', channel).dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
      expect(runtime.sel.chan).toBe(channel);
      expect(runtime.TL.reveal).not.toHaveBeenCalled();
    }
  });

  it('applies the unified Scale context menu to both axes', () => {
    const candidate = layer('A');
    candidate.scaleLinked = true;
    candidate.p['scale.y']!.expr = 'value * 2';
    const { runtime, apply, menu } = setup([candidate]);
    channelRow('A', 'scale.x').dispatchEvent(new MouseEvent('contextmenu', { bubbles: true }));
    const items = menu.mock.calls[0]![1] as Array<{ label?: string; run?: () => void }>;
    expect(labelledSpinbutton('Scale Y').classList.contains('link')).toBe(true);
    items.find((item) => item.label === 'Remove expression')!.run!();
    expect(apply).toHaveBeenCalledWith([
      { type: 'set_expression', target: 'A', path: 'scale.x', expression: null },
      { type: 'set_expression', target: 'A', path: 'scale.y', expression: null }
    ], { label: 'Remove expression', origin: 'inspector' });
    items.find((item) => item.label === 'Add keyframe at playhead')!.run!();
    expect(candidate.p['scale.x']!.kf).toHaveLength(1);
    expect(candidate.p['scale.y']!.kf).toHaveLength(1);
    items.find((item) => item.label === 'Show in graph editor')!.run!();
    expect(runtime.TL.reveal).toHaveBeenLastCalledWith(candidate, ['scale.x', 'scale.y']);
    items.find((item) => item.label === 'Reset')!.run!();
    expect(apply).toHaveBeenCalledWith([
      expect.objectContaining({ type: 'replace_keyframes', path: 'scale.x', keyframes: [] }),
      expect.objectContaining({ type: 'replace_keyframes', path: 'scale.y', keyframes: [] })
    ], { label: 'Reset', origin: 'inspector' });
  });

  it('derives keyframe-diamond state from keyframes and playhead time', () => {
    const candidate = layer('A', 75);
    candidate.p.opacity!.kf = [{ t: 0, v: 75 }];
    setup([candidate], ['A']);
    const row = channelRow('A', 'opacity');
    const diamond = row.querySelector<HTMLButtonElement>('.property-stopwatch')!;

    /* animated + key under the playhead: filled */
    expect(diamond.classList.contains('on')).toBe(true);
    expect(diamond.classList.contains('at-key')).toBe(true);
    expect(diamond.getAttribute('aria-pressed')).toBe('true');

    /* animated, playhead between keys: outlined */
    transport.time = 1;
    flushSync();
    expect(diamond.classList.contains('on')).toBe(true);
    expect(diamond.classList.contains('at-key')).toBe(false);
    expect(diamond.getAttribute('aria-pressed')).toBe('false');

    /* Static: muted, still rendered so it can add the first keyframe. */
    candidate.p.opacity!.kf = [];
    bump('values');
    flushSync();
    expect(diamond.classList.contains('on')).toBe(false);
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    expect(row.querySelector('.well .kf')).toBeNull();
  });

  it('sends the exact legacy add-effect and remove-effect commands', () => {
    const candidate = layer('A');
    const { apply, menu } = setup([candidate], ['A']);
    target.querySelector<HTMLButtonElement>('[aria-label="Add effect"]')!.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, button: 0 })
    );
    const items = menu.mock.calls[0]![1] as Array<{ label?: string; run?: () => void }>;
    items.find((item) => item.label === 'Gaussian Blur')?.run?.();
    expect(apply).toHaveBeenCalledWith(
      { type: 'add_effect', target: 'A', effect: 'blur' },
      { label: 'Add Gaussian Blur', origin: 'inspector' }
    );

    apply.mockClear();
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: {} });
    bump('structure');
    flushSync();
    target.querySelector<HTMLButtonElement>('button[aria-label="Remove Gaussian Blur"]')!.click();
    expect(apply).toHaveBeenCalledExactlyOnceWith(
      { type: 'remove_effect', target: 'A', effect: 'fx-1' },
      { label: 'Remove effect', origin: 'inspector' }
    );
  });

  it('selects multiple effects and routes copy, paste, and delete before layer shortcuts', () => {
    const candidate = layer('A');
    const channel = (value: number, id: string) => ({
      v: value,
      expr: value === 12 ? 'value * 2' : null,
      kf: [{ t: 1, v: value + 1, eo: [.25, .1], ei: [.25, 1], hold: false, i: id }]
    });
    candidate.fx.push(
      { id: 'fx-1', type: 'blur', on: false, open: false, p: { amount: channel(12, 'key-1') } },
      { id: 'fx-2', type: 'blur', on: true, open: true, p: { amount: channel(24, 'key-2') } }
    );
    const { apply } = setup([candidate], ['A']);
    const first = target.querySelector<HTMLElement>('[data-effect-id="fx-1"]')!;
    const second = target.querySelector<HTMLElement>('[data-effect-id="fx-2"]')!;

    first.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    second.dispatchEvent(new MouseEvent('click', { bubbles: true, metaKey: true }));
    flushSync();
    expect(first.dataset.selected).toBe('true');
    expect(second.dataset.selected).toBe('true');

    second.dispatchEvent(new KeyboardEvent('keydown', {
      key: 'c', metaKey: true, bubbles: true, cancelable: true
    }));
    apply.mockClear();
    const paste = new KeyboardEvent('keydown', {
      key: 'v', metaKey: true, bubbles: true, cancelable: true
    });
    second.dispatchEvent(paste);
    expect(paste.defaultPrevented).toBe(true);
    expect(apply).toHaveBeenCalledExactlyOnceWith([
      {
        type: 'add_effect', target: 'A', effect: 'blur', parameters: candidate.fx[0].p,
        open: false, enabled: false
      },
      {
        type: 'add_effect', target: 'A', effect: 'blur', parameters: candidate.fx[1].p,
        open: true, enabled: true
      }
    ], { label: 'Paste effects', origin: 'inspector' });

    apply.mockClear();
    const remove = new KeyboardEvent('keydown', { key: 'Delete', bubbles: true, cancelable: true });
    second.dispatchEvent(remove);
    expect(remove.defaultPrevented).toBe(true);
    expect(apply).toHaveBeenCalledExactlyOnceWith([
      { type: 'remove_effect', target: 'A', effect: 'fx-1' },
      { type: 'remove_effect', target: 'A', effect: 'fx-2' }
    ], { label: 'Remove effects', origin: 'inspector' });
  });

  it('keeps a copied effect active while a different destination layer is selected', () => {
    const source = layer('A');
    const destination = layer('B');
    source.fx.push({
      id: 'fx-1', type: 'blur', on: true, open: true,
      p: { amount: { v: 18, expr: null, kf: [] } }
    });
    const { runtime, api, apply } = setup([source, destination], ['A']);
    const effect = target.querySelector<HTMLElement>('[data-effect-id="fx-1"]')!;
    effect.click();
    expect(api.services.get<InspectorService>('inspector')?.copySelectedEffects()).toBe(true);
    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0 }));
    runtime.sel.layers = ['B'];
    apply.mockClear();

    expect(api.services.get<InspectorService>('inspector')?.pasteCopiedEffects()).toBe(true);
    expect(apply).toHaveBeenCalledExactlyOnceWith({
      type: 'add_effect', target: 'B', effect: 'blur', parameters: source.fx[0].p,
      open: true, enabled: true
    }, { label: 'Paste effect', origin: 'inspector' });
  });

  it('routes composition fields through an exact set_composition command', async () => {
    const { apply, drag } = setup([], []);
    const width = labelledSpinbutton('Width');

    width.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0 }));
    drag().up();
    await tick();
    width.value = '2048';
    width.dispatchEvent(new InputEvent('input', { bubbles: true }));
    width.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));

    expect(apply).toHaveBeenCalledWith(
      { type: 'set_composition', patch: { width: 2048 } },
      { label: 'Width', origin: 'inspector' }
    );
  });

  it('routes transform, effect, layer, and content fields through exact typed commands', () => {
    const candidate = layer('A', 40);
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } });
    const { runtime, apply } = setup([candidate], ['A'], { fxOpen: true });
    transport.time = 2;
    flushSync();

    channelRow('A', 'opacity').querySelector<HTMLInputElement>('[role="spinbutton"]')!
      .dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }));
    expect(apply).toHaveBeenCalledWith(
      {
        type: 'set_property', target: 'A', path: 'opacity', value: 41, time: 2,
        mode: 'auto', preserveHandEdits: false, markIntent: 'human'
      },
      { label: 'Opacity', origin: 'inspector' }
    );

    apply.mockClear();
    expect(runtime.evP).toHaveBeenCalledWith(candidate, candidate.fx[0].p.amount, 2, 'amount');
    labelledSpinbutton('Amount').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    );
    expect(apply).toHaveBeenCalledWith(
      {
        type: 'set_property', target: 'A', path: 'fx-1.amount', value: 6, time: 2,
        mode: 'auto', preserveHandEdits: false
      },
      { label: 'Amount', origin: 'inspector' }
    );

    apply.mockClear();
    labelledSelect('Blend mode').value = '1';
    labelledSelect('Blend mode').dispatchEvent(new Event('change', { bubbles: true }));
    expect(apply).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'set_property', target: 'A', path: 'l.blend', value: 'screen', mode: 'auto' }),
      { label: 'Blend', origin: 'inspector' }
    );

    apply.mockClear();
    labelledSpinbutton('Width').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    );
    expect(apply).toHaveBeenCalledWith(
      { type: 'set_content', target: 'A', patch: { w: 641 } },
      { label: 'Width', origin: 'inspector' }
    );
  });

  it('exposes direct keyframe controls for numeric and color effect parameters', () => {
    const candidate = layer('A');
    candidate.fx.push(
      { id: 'fx-blur', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } },
      { id: 'fx-duotone', type: 'duotone', on: true, p: { shadow: { v: '#1B2A4A', kf: [], expr: null } } }
    );
    const { apply, runtime } = setup([candidate], ['A'], { fxOpen: true });
    runtime.findProp = (_layer: any, path: string) => { const [id, key] = path.split('.'); return candidate.fx.find((fx: any) => fx.id === id)?.p[key!]; };
    bump('values');
    transport.time = 2;
    flushSync();

    const amount = target.querySelector<HTMLButtonElement>('[aria-label="Add keyframe for Amount"]');
    const shadow = target.querySelector<HTMLButtonElement>('[aria-label="Add keyframe for Shadow"]');
    expect(amount).not.toBeNull();
    expect(shadow).not.toBeNull();

    amount!.click();
    shadow!.click();
    expect(apply).toHaveBeenCalledWith([expect.objectContaining({type:'set_property',path:'fx-blur.amount',value:5,mode:'keyframe',time:2})],expect.objectContaining({origin:'inspector'}));
    expect(apply).toHaveBeenCalledWith([expect.objectContaining({ type: 'set_property', path: 'fx-duotone.shadow', value: '#1B2A4A', mode: 'keyframe', time: 2 })], expect.objectContaining({ origin: 'inspector' }));
  });

  it('uses set_effect for toggles and exposes one controlled expansion button', () => {
    const candidate = layer('A');
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } });
    const { runtime, apply } = setup([candidate], ['A'], { fxOpen: true });
    const head = target.querySelector<HTMLElement>('[data-effect-id="fx-1"]')!;
    const expanders = head.querySelectorAll<HTMLButtonElement>('button[aria-expanded]');
    expect(expanders).toHaveLength(1);
    expect(expanders[0]!.getAttribute('aria-expanded')).toBe('true');
    const controlled = expanders[0]!.getAttribute('aria-controls')!;
    expect(document.getElementById(controlled)).not.toBeNull();

    target.querySelector<HTMLButtonElement>('[aria-label="Disable Gaussian Blur"]')!.click();
    expect(apply).toHaveBeenCalledWith(
      { type: 'set_effect', target: 'A', effect: 'fx-1', patch: { enabled: false } },
      { label: 'Toggle effect', origin: 'inspector' }
    );

    expanders[0]!.click();
    expect(runtime.UIState.setFxOpen).toHaveBeenCalledWith(candidate.fx[0], false);
  });

  it('selects effect rows without changing disclosure and reserves disclosure for the chevron', () => {
    const candidate = layer('A');
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } });
    const { runtime } = setup([candidate], ['A']);
    const head = target.querySelector<HTMLElement>('[data-effect-id="fx-1"]')!;
    const label = head.querySelector<HTMLElement>('.k')!;
    const chevron = head.querySelector<HTMLButtonElement>('button[aria-expanded]')!;

    head.click();
    flushSync();
    expect(head.dataset.selected).toBe('true');
    expect(runtime.UIState.setFxOpen).not.toHaveBeenCalled();

    label.click();
    expect(runtime.UIState.setFxOpen).not.toHaveBeenCalled();

    chevron.click();
    expect(runtime.UIState.setFxOpen).toHaveBeenCalledExactlyOnceWith(candidate.fx[0], true);
  });

  it('clears effect selection when clicking anywhere outside it but not from nested controls', () => {
    const candidate = layer('A');
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } });
    setup([candidate], ['A'], { fxOpen: true });
    const head = target.querySelector<HTMLElement>('[data-effect-id="fx-1"]')!;

    head.click();
    flushSync();
    expect(head.dataset.selected).toBe('true');

    labelledSpinbutton('Amount').click();
    flushSync();
    expect(head.dataset.selected).toBe('true');

    document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0 }));
    flushSync();
    expect(head.dataset.selected).toBeUndefined();
  });

  it('uses a typed mask property path while add mask stays history-backed', () => {
    const candidate = layer('A');
    const { runtime, apply } = setup([candidate], ['A']);
    target.querySelector<HTMLButtonElement>('[aria-label="Add mask"]')!.click();
    expect(runtime.hist.do).toHaveBeenCalledWith('Add mask', expect.any(Function));
    expect(runtime.mkMask).toHaveBeenCalledWith('rect', runtime.proj);
    expect(candidate.masks).toHaveLength(1);
    bump('structure');
    flushSync();

    transport.time = 3;
    flushSync();
    labelledSpinbutton('X').dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    );
    expect(apply).toHaveBeenCalledWith(
      {
        type: 'set_property', target: 'A', path: 'm.mask-new.x', value: 1, time: 3,
        mode: 'auto', preserveHandEdits: false
      },
      { label: 'X', origin: 'inspector' }
    );
  });

  it('keeps the legacy Edit begin/dispatch/commit sequence for text', () => {
    const candidate = layer('A');
    candidate.type = 'text';
    candidate.d = { text: 'Hello', font: 'SF Pro Display', weight: 400, size: 64, tracking: 0, leading: 1.2, align: 'center', color: '#FFFFFF' };
    const { runtime } = setup([candidate], ['A']);
    const textarea = target.querySelector<HTMLTextAreaElement>('[data-inspector-text-layer="A"]')!;

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = 'Hello world';
    textarea.dispatchEvent(new InputEvent('input', { bubbles: true }));
    textarea.dispatchEvent(new FocusEvent('blur'));

    expect(runtime.Edit.begin).toHaveBeenCalledWith('Edit text', { origin: 'inspector' });
    expect(runtime.Edit.dispatch).toHaveBeenCalledWith({
      type: 'set_content', target: 'A', patch: { text: 'Hello world' }
    });
    expect(runtime.Edit.commit).toHaveBeenCalledWith('Edit text');
  });

  it('unmounts cleanly without a host-state subscription', async () => {
    setup([layer('A')], ['A']);
    await unmount(instance!);
    instance = undefined;
    expect(target.querySelector('[data-svelte-panel="inspector"]')).toBeNull();
  });

  it('labels every numeric spinbutton and gives every button an accessible name', () => {
    setup([layer('A')], ['A']);
    const spinbuttons = [...target.querySelectorAll<HTMLInputElement>('[role="spinbutton"]')];
    expect(spinbuttons.length).toBeGreaterThan(0);
    for (const input of spinbuttons) {
      const labelledBy = input.getAttribute('aria-labelledby');
      const name = labelledBy
        ? document.getElementById(labelledBy)?.textContent?.trim()
        : input.getAttribute('aria-label')?.trim();
      expect(name, input.outerHTML).toBeTruthy();
    }

    const buttons = [...target.querySelectorAll<HTMLButtonElement>('button')];
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) {
      const labelledBy = button.getAttribute('aria-labelledby');
      const name = button.getAttribute('aria-label')?.trim()
        || (labelledBy ? document.getElementById(labelledBy)?.textContent?.trim() : '')
        || button.textContent?.trim();
      expect(name, button.outerHTML).toBeTruthy();
    }
  });
});
