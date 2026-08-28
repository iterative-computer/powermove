// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ControlsAPI, PowermoveAPI } from 'powermove';

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
import InspectorPanel from './InspectorPanel.svelte';
import activate from './index';

type Channel = { v: number; kf: Array<{ t: number; v: number }>; expr: string | null };
type TestLayer = Record<string, any> & { p: Record<string, Channel> };

const CHANNELS = [
  'anchor.x', 'anchor.y', 'position.x', 'position.y', 'scale.x',
  'scale.y', 'rotation', 'opacity', 'skew'
] as const;

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

const controls: ControlsAPI = {
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
    channelBinding,
    compositionBinding: (PM, field, options) => compositionBinding(PM, field as any, options),
    contentBinding,
    layerFieldBinding: (PM, layerId, field, options) => layerFieldBinding(PM, layerId, field as any, options)
  }
};

function apiFor(PM: Record<string, any>, register = vi.fn()): PowermoveAPI {
  return {
    id: 'inspector',
    apiVersion: 1,
    manifest: { id: 'inspector', name: 'Inspector', version: '1.0.0', apiVersion: 1 },
    panels: { register },
    ui: {
      controls,
      icon: (name: string) => `<svg data-icon="${name}" aria-hidden="true"><path/></svg>`
    },
    host: {
      pm: PM,
      state: { doc, sel, transport, perf },
      mount: vi.fn()
    }
  } as unknown as PowermoveAPI;
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
  const offFonts = vi.fn();
  const currentProject = project(testLayers);
  const apply = vi.fn();
  const menu = vi.fn();
  const PM: Record<string, any> = {
    proj: currentProject,
    CH: Object.fromEntries(CHANNELS.map((channel) => [channel, {
      step: 1,
      unit: channel === 'opacity' || channel.startsWith('scale.') ? '%' : undefined
    }])),
    BLENDS: ['normal', 'screen'],
    MASK_SHAPES: ['rect', 'ellipse'],
    TYPE_META: { solid: { label: 'Solid' }, text: { label: 'Text' } },
    FX: { blur: { label: 'Gaussian Blur', group: 'Blur', params: [{ k: 'amount', label: 'Amount', step: 1, min: 0, max: 100 }] } },
    ICONS: Object.fromEntries(['layers', 'plus', 'clock', 'diamond', 'chev', 'eye', 'x'].map((name) => [name, '<path/>'])),
    sel: { layers: [...selected], keys: [], chan: null },
    TL: { graph: false, reveal: vi.fn() },
    Edit: {
      apply,
      begin: vi.fn(),
      dispatch: vi.fn(),
      commit: vi.fn(),
      cancel: vi.fn()
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
    bus: { on: vi.fn((_event: string, _listener: () => void) => offFonts) },
    menu,
    modal: vi.fn(),
    cmd: vi.fn(),
    toast: vi.fn(),
    touch: vi.fn(),
    invalidate: vi.fn(),
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
    time: 0
  };

  doc.replace(currentProject as any);
  setSelection({ layers: selected, keys: [], chan: null });
  transport.time = 0;
  const api = apiFor(PM);
  activate(api);
  instance = mount(InspectorPanel, { target, props: { panelId: 'inspector', spec: {}, api } });
  flushSync();

  return { PM, apply, menu, offFonts, drag: () => dragOptions! };
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
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
  setSelection({ layers: [], keys: [], chan: null });
  transport.time = 0;
  delete (window as any).PM;
  vi.restoreAllMocks();
});

describe('InspectorPanel', () => {
  it('activates by registering the inspector panel contribution', () => {
    const register = vi.fn(() => ({ dispose: vi.fn() }));

    activate(apiFor({}, register));

    expect(register).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      id: 'inspector',
      title: 'Properties',
      component: InspectorPanel
    }));
  });

  it('registers the persistent Properties panel and installs the inert legacy shim', () => {
    let definition: Record<string, any> | undefined;
    const PM: Record<string, any> = {
      parseUniforms: vi.fn(() => [{ name: 'amount', def: 1 }]),
      P: vi.fn((value: unknown) => ({ v: value, kf: [], expr: null })),
      UIState: { setShaderMeta: vi.fn() },
      registerPanel: vi.fn((id: string, next: Record<string, any>) => {
        expect(id).toBe('inspector');
        definition = next;
      })
    };

    const register = vi.fn((next: Record<string, any>) => {
      definition = next;
      return { dispose: vi.fn() };
    });
    activate(apiFor(PM, register));

    expect(register).toHaveBeenCalledOnce();
    expect(definition).toMatchObject({ id: 'inspector', title: 'Properties' });
    expect(definition?.component).toBe(InspectorPanel);
    expect(definition?.header).toBeTypeOf('function');
    expect(PM.Inspector.body).toBeNull();
    expect(PM.Inspector.syncs).toEqual([]);
    expect(PM.Inspector.refresh).toBeTypeOf('function');
    expect(PM.Inspector.focusText).toBeTypeOf('function');
    expect(() => PM.Inspector.refresh()).not.toThrow();
    expect(PM.fxMenu).toBeTypeOf('function');

    const shader = { d: { code: 'uniform float amount;', uniforms: { stale: { v: 2 } } } };
    PM.syncShaderUniforms(shader);
    expect(PM.UIState.setShaderMeta).toHaveBeenCalledWith(shader, {
      udefs: [{ name: 'amount', def: 1 }]
    });
    expect(shader.d.uniforms).toEqual({ amount: { v: 1, kf: [], expr: null } });
  });

  it('caps multi-selection rendering to the first layer and keeps the header consistent', () => {
    const a = layer('A', 10), b = layer('B', 20);
    setup([a, b], ['A', 'B']);

    expect(target.querySelector('[data-inspector-layer="A"]')).not.toBeNull();
    expect(target.querySelector('[data-inspector-layer="B"]')).toBeNull();
    expect(target.querySelector('[data-inspector-header]')?.textContent).toContain('Layer A');
    expect(target.querySelector('[role="status"]')?.textContent).toContain('2 layers selected · editing Layer A');
  });

  it('updates a channel value on the values tick without remounting its row', () => {
    const candidate = layer('A', 40);
    setup([candidate], ['A']);
    const row = channelRow('A', 'opacity');
    const marker = row.dataset.channelInstance;
    const input = row.querySelector<HTMLInputElement>('[role="spinbutton"]')!;
    expect(input.value).toBe('40%');

    candidate.p.opacity!.v = 55;
    doc.bump('values');
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
    doc.bump('values');
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
    doc.bump('values');
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
    doc.bump('values');
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
    const { PM } = setup([candidate]);
    const row = channelRow('A', 'scale.x');
    const stopwatch = row.querySelector<HTMLButtonElement>('[aria-label="Animate Scale"]')!;
    const diamond = row.querySelector<HTMLButtonElement>('.kd')!;
    stopwatch.click();
    expect(PM.hist.do).toHaveBeenLastCalledWith('Animate Scale', expect.any(Function));
    expect(candidate.p['scale.x']!.kf).toHaveLength(1);
    expect(candidate.p['scale.y']!.kf).toHaveLength(1);
    // An existing project may have a key on just one of the linked axes.
    candidate.p['scale.y']!.kf = [];
    doc.bump('values');
    flushSync();
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    diamond.click();
    doc.bump('values');
    flushSync();
    expect(candidate.p['scale.x']!.kf).toHaveLength(1);
    expect(candidate.p['scale.y']!.kf).toHaveLength(1);
    expect(diamond.getAttribute('aria-pressed')).toBe('true');
    diamond.click();
    expect(candidate.p['scale.x']!.kf).toHaveLength(0);
    expect(candidate.p['scale.y']!.kf).toHaveLength(0);
    candidate.p['scale.y']!.kf = [{ t: 0, v: 100 }];
    doc.bump('values');
    flushSync();
    expect(stopwatch.getAttribute('aria-pressed')).toBe('true');
    stopwatch.click();
    expect(candidate.p['scale.x']!.kf).toHaveLength(0);
    expect(candidate.p['scale.y']!.kf).toHaveLength(0);
  });

  it('applies the unified Scale context menu to both axes', () => {
    const candidate = layer('A');
    candidate.scaleLinked = true;
    candidate.p['scale.y']!.expr = 'value * 2';
    const { PM, apply, menu } = setup([candidate]);
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
    expect(PM.TL.reveal).toHaveBeenLastCalledWith(candidate, ['scale.x', 'scale.y']);
    items.find((item) => item.label === 'Reset')!.run!();
    expect(apply).toHaveBeenCalledWith([
      expect.objectContaining({ type: 'replace_keyframes', path: 'scale.x', keyframes: [] }),
      expect.objectContaining({ type: 'replace_keyframes', path: 'scale.y', keyframes: [] })
    ], { label: 'Reset', origin: 'inspector' });
  });

  it('derives stopwatch and keyframe-diamond state from keyframes and playhead time', () => {
    const candidate = layer('A', 75);
    candidate.p.opacity!.kf = [{ t: 0, v: 75 }];
    setup([candidate], ['A']);
    const row = channelRow('A', 'opacity');
    const stopwatch = row.querySelector<HTMLButtonElement>('button[aria-label="Animate Opacity"]')!;
    const diamond = row.querySelector<HTMLButtonElement>('button.kd')!;

    expect(stopwatch.classList.contains('on')).toBe(true);
    expect(stopwatch.getAttribute('aria-pressed')).toBe('true');
    expect(diamond.style.display).not.toBe('none');
    expect(diamond.classList.contains('on')).toBe(true);
    expect(diamond.getAttribute('aria-pressed')).toBe('true');

    transport.time = 1;
    flushSync();
    expect(diamond.classList.contains('on')).toBe(false);
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    expect(stopwatch.classList.contains('on')).toBe(true);

    candidate.p.opacity!.kf = [];
    doc.bump('values');
    flushSync();
    expect(stopwatch.classList.contains('on')).toBe(false);
    expect(stopwatch.getAttribute('aria-pressed')).toBe('false');
    expect(diamond.style.display).toBe('none');
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
    doc.bump('structure');
    flushSync();
    target.querySelector<HTMLButtonElement>('button[aria-label="Remove Gaussian Blur"]')!.click();
    expect(apply).toHaveBeenCalledExactlyOnceWith(
      { type: 'remove_effect', target: 'A', effect: 'fx-1' },
      { label: 'Remove effect', origin: 'inspector' }
    );
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
    const { PM, apply } = setup([candidate], ['A'], { fxOpen: true });
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
    expect(PM.evP).toHaveBeenCalledWith(candidate, candidate.fx[0].p.amount, 2, 'amount');
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
      { type: 'set_layer', target: 'A', patch: { blend: 'screen' } },
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

  it('uses set_effect for toggles and exposes one controlled expansion button', () => {
    const candidate = layer('A');
    candidate.fx.push({ id: 'fx-1', type: 'blur', on: true, p: { amount: { v: 5, kf: [], expr: null } } });
    const { PM, apply } = setup([candidate], ['A'], { fxOpen: true });
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
    expect(PM.UIState.setFxOpen).toHaveBeenCalledWith(candidate.fx[0], false);
  });

  it('uses a typed mask property path while add mask stays history-backed', () => {
    const candidate = layer('A');
    const { PM, apply } = setup([candidate], ['A']);
    target.querySelector<HTMLButtonElement>('[aria-label="Add mask"]')!.click();
    expect(PM.hist.do).toHaveBeenCalledWith('Add mask', expect.any(Function));
    expect(PM.mkMask).toHaveBeenCalledWith('rect', PM.proj);
    expect(candidate.masks).toHaveLength(1);
    doc.bump('structure');
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
    const { PM } = setup([candidate], ['A']);
    const textarea = target.querySelector<HTMLTextAreaElement>('[data-inspector-text-layer="A"]')!;

    textarea.dispatchEvent(new FocusEvent('focus'));
    textarea.value = 'Hello world';
    textarea.dispatchEvent(new InputEvent('input', { bubbles: true }));
    textarea.dispatchEvent(new FocusEvent('blur'));

    expect(PM.Edit.begin).toHaveBeenCalledWith('Edit text', { origin: 'inspector' });
    expect(PM.Edit.dispatch).toHaveBeenCalledWith({
      type: 'set_content', target: 'A', patch: { text: 'Hello world' }
    });
    expect(PM.Edit.commit).toHaveBeenCalledWith('Edit text');
  });

  it('subscribes only to fonts and removes that subscription on unmount', async () => {
    const { PM, offFonts } = setup([layer('A')], ['A']);
    expect(PM.bus.on).toHaveBeenCalledExactlyOnceWith('fonts', expect.any(Function));

    await unmount(instance!);
    instance = undefined;
    expect(offFonts).toHaveBeenCalledOnce();
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
