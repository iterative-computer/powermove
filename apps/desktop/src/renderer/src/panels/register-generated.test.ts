// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { GeneratedSection } from '../core/types/workspace';
import {
  formatPreview,
  generatedReadout,
  resolveButtonAction,
  sourceBinding
} from '../core/generated-bindings';
import { doc } from '../state/document.svelte';
import { setSelection } from '../state/selection.svelte';
import GeneratedPanel from './GeneratedPanel.svelte';
import {
  installGeneratedPanels,
  registerGeneratedPanels,
  unmountGeneratedPanels
} from './register-generated';

let mounted: object[] = [];
let PMs: Record<string, any>[] = [];
let canvasContext: Record<string, any>;

beforeEach(() => {
  canvasContext = {
    clearRect: vi.fn(), beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(),
    stroke: vi.fn(), arc: vi.fn(), fill: vi.fn(),
    lineWidth: 0, strokeStyle: '', fillStyle: ''
  };
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(canvasContext as any);
});

afterEach(async () => {
  vi.useRealTimers();
  for (const instance of mounted.splice(0)) await unmount(instance);
  for (const PM of PMs.splice(0)) unmountGeneratedPanels(PM);
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

function normalizeFill(value: any, fallback = '#000000') {
  const type = ['solid', 'linear', 'radial', 'none'].includes(value?.type) ? value.type : 'solid';
  let stops = (value?.stops || [{ id: 'stop-1', color: value?.color || fallback, position: 0 }]).map((stop: any) => ({ ...stop }));
  if (!['solid', 'none'].includes(type) && stops.length < 2) stops.push({ id: 'stop-2', color: stops[0].color, position: 100 });
  if (['solid', 'none'].includes(type)) stops = [stops[0]];
  return { type, angle: value?.angle || 0, stops };
}

function fakePM() {
  const layers: Record<string, any> = {
    L1: { id: 'L1', name: 'One', type: 'text', lock: false, on: true, d: { text: 'First', color: '#112233', mode: 'a' }, p: { opacity: { v: 25 } } },
    L2: { id: 'L2', name: 'Two', type: 'text', lock: false, on: true, d: { text: 'Second', color: '#445566', mode: 'b' }, p: { opacity: { v: 75 } } }
  };
  let dragOptions: any;
  const panels = new Map<string, any>();
  const PM: Record<string, any> = {
    time: 1,
    sel: { layers: ['L1'], keys: [] },
    proj: {
      id: 'P1', name: 'Demo', w: 1920, h: 1080, fps: 20, dur: 10, shutter: .5, bg: '#000000',
      backgroundFill: normalizeFill({ type: 'solid', color: '#000000' }), work: [0, 10],
      params: {}, layers: Object.values(layers), comps: {}, assets: {}, markers: [], revision: 0, edits: [], created: 0
    },
    WS: { save: vi.fn(), current: null, registerCustom: vi.fn() },
    panelInst: {},
    PANELS: {},
    registerPanel: vi.fn((id: string, def: any) => { panels.set(id, def); PM.PANELS[id] = def; }),
    Edit: {
      begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn(),
      apply: vi.fn(() => ({ ok: true }))
    },
    hist: { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(), do: vi.fn((_label: string, fn: () => void) => fn()), undo: vi.fn(() => true) },
    drag: vi.fn((_event: PointerEvent, options: any) => { dragOptions = options; }),
    round: (value: number, places: number) => Number(value.toFixed(places)),
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    normalizeFill,
    uid: (prefix: string) => `${prefix}-new`,
    invalidate: vi.fn(),
    toast: vi.fn(),
    cmd: vi.fn(),
    firstSel: () => layers[PM.sel.layers[0]],
    L: (id: string) => layers[id],
    byName: (name: string) => Object.values(layers).find((layer) => layer.name === name),
    findProp: (layer: any, path: string) => layer.p[path],
    evP: (_layer: any, property: any) => property.v,
    selLayers: () => PM.sel.layers.map((id: string) => layers[id]),
    Ease: {
      PRESETS: { ease: [.25, .1, .25, 1] },
      nameOf: vi.fn(() => 'custom'),
      bezier: vi.fn(() => (value: number) => value)
    },
    Capabilities: {
      sanitizeCurve: (value: any, fallback: any) => Array.isArray(value) && value.length === 4 ? [...value] : [...fallback],
      resolveTargets: vi.fn(() => [layers.L1]),
      selectedKeyframes: vi.fn(() => []),
      selectionSummary: vi.fn(() => `${PM.sel.layers.length} layer selected`),
      keyframeSummary: vi.fn(() => 'No keyframes selected')
    },
    Script: { canRun: vi.fn(() => true) }
  };
  PMs.push(PM);
  return { PM, layers, panels, drag: () => dragOptions };
}

function allControls(): GeneratedSection {
  return {
    id: 'generated', title: 'Generated', size: 260,
    state: { text: 'Draft', amount: 3 },
    controls: [
      { type: 'text', label: 'Local text', stateKey: 'text', param: 'text', def: '' },
      { type: 'color', label: 'Layer color', stateKey: '', param: 'color', target: 'L1', path: 'content.color', def: '#112233' },
      { type: 'fill', label: 'Scene fill', stateKey: '', param: 'fill', def: { type: 'solid', angle: 0, stops: [{ id: 'one', color: '#123456', position: 0 }] } },
      { type: 'toggle', label: 'Scene toggle', stateKey: '', param: 'enabled', def: false },
      { type: 'select', label: 'Layer mode', stateKey: '', param: 'mode', target: 'L1', path: 'content.mode', options: ['a', 'b'], def: 'a' },
      { type: 'slider', label: 'Local amount', stateKey: 'amount', param: 'amount', min: 0, max: 10, def: 1, step: 1 },
      { type: 'curve', label: 'Curve', stateKey: 'curve', def: [.62, .05, 0, 1], minY: -1, maxY: 2, presets: ['ease'] },
      { type: 'readout', label: 'Selection', source: 'selection.count' },
      {
        type: 'button', label: 'Run commands', primary: true, action: null, cmd: '',
        commands: [{ type: 'set_layer', target: 'L1', patch: { visible: false }, overrideLock: true, preserveHandEdits: false, markIntent: 'human' }]
      },
      { type: 'button', label: 'Reset', primary: false, action: { type: 'reset' }, cmd: '' }
    ]
  } as GeneratedSection;
}

function renderPanel(PM: Record<string, any>, section: GeneratedSection): HTMLElement {
  PM.WS.current = { custom: [section] };
  doc.replace(PM.proj);
  setSelection({ layers: [...PM.sel.layers], keys: [], chan: null });
  const target = document.createElement('div');
  document.body.append(target);
  mounted.push(mount(GeneratedPanel, { target, props: { panelId: section.id, spec: {}, PM, section } }));
  flushSync();
  return target;
}

const buttonByText = (target: ParentNode, text: string): HTMLButtonElement => {
  const button = [...target.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.trim() === text);
  if (!button) throw new Error(`Button not found: ${text}`);
  return button;
};

describe('GeneratedPanel', () => {
  it('renders every manifest control kind and writes local, command, and scene-param modes', async () => {
    vi.useFakeTimers();
    const { PM, drag } = fakePM();
    const section = allControls();
    const target = renderPanel(PM, section);

    expect(target.querySelectorAll('.row.split')).toHaveLength(7);
    expect(target.querySelector('.generated-curve-canvas')).toBeTruthy();
    expect(target.querySelector('.generated-tool-readout')?.textContent).toBe('1');
    expect(buttonByText(target, 'Run commands')).toBeTruthy();

    const text = target.querySelector<HTMLInputElement>('[aria-labelledby*="pm-control-label"]')!;
    text.dispatchEvent(new FocusEvent('focus'));
    text.value = 'Changed';
    text.dispatchEvent(new InputEvent('input', { bubbles: true }));
    text.dispatchEvent(new FocusEvent('blur'));
    expect(section.state.text).toBe('Changed');
    await vi.advanceTimersByTimeAsync(140);
    expect(PM.WS.save).toHaveBeenCalledTimes(1);

    const toggle = target.querySelector<HTMLButtonElement>('button.toggle')!;
    toggle.click();
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      { type: 'set_scene_parameter', name: 'enabled', value: true },
      { label: 'Scene toggle', origin: 'generated-ui' }
    );

    const select = target.querySelector<HTMLSelectElement>('select')!;
    select.value = '1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      { type: 'set_content', target: 'L1', patch: { mode: 'b' } },
      { label: 'Layer mode', origin: 'generated-ui' }
    );

    const number = target.querySelector<HTMLInputElement>('input.num')!;
    number.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0 }));
    drag().up();
    await tick();
    number.value = '7';
    number.dispatchEvent(new InputEvent('input', { bubbles: true }));
    number.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(section.state.amount).toBe(7);

    buttonByText(target, 'ease').click();
    expect((section.state as any).curve).toEqual([.25, .1, .25, 1]);
    expect(structuredClone(section.state)).toEqual({ text: 'Changed', amount: 7, curve: [.25, .1, .25, 1] });

    buttonByText(target, 'Run commands').click();
    await tick();
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      [{ type: 'set_layer', target: 'L1', patch: { visible: false } }],
      { label: 'Run commands', origin: 'generated-ui' }
    );

    buttonByText(target, 'Reset').click();
    await tick();
    expect(section.state).toEqual({ text: 'Draft', amount: 3 });
  });

  it('keeps a $selection field mounted while its lazy getter follows selection changes', () => {
    const { PM } = fakePM();
    const section = {
      id: 'selection-tool', title: 'Selection', size: 180, state: {},
      controls: [{ type: 'text', label: 'Text', stateKey: '', param: 'text', target: '$selection', path: 'content.text', def: '' }]
    } as GeneratedSection;
    const target = renderPanel(PM, section);
    const input = target.querySelector<HTMLInputElement>('input')!;
    expect(input.value).toBe('First');

    PM.sel.layers = ['L2'];
    setSelection({ layers: ['L2'], keys: [], chan: null });
    flushSync();

    expect(target.querySelector('input')).toBe(input);
    expect(input.value).toBe('Second');
  });

  it('renders the DOM-free preview model with frame formatting', async () => {
    const { PM } = fakePM();
    PM.Capabilities.preview = vi.fn(() => ({
      ok: true, message: 'One change ready',
      changes: [{ name: 'One', path: 'layer.duration', before: 1, after: 2 }]
    }));
    const section = {
      id: 'preview-tool', title: 'Preview', size: 180, state: { nested: { amount: 1 } }, controls: [{
        type: 'button', label: 'Preview', primary: false, cmd: '',
        action: { type: 'transform', mode: 'preview', transform: { selector: { scope: 'selection', types: [] }, edits: [] } }
      }]
    } as unknown as GeneratedSection;
    const target = renderPanel(PM, section);
    buttonByText(target, 'Preview').click();
    await Promise.resolve();
    await tick();
    flushSync();
    expect(target.querySelector('[role="status"] strong')?.textContent).toBe('Preview');
    expect(target.querySelector('[role="status"] code')?.textContent).toBe('20fr → 40fr');
    expect(() => structuredClone(PM.Capabilities.preview.mock.calls[0][1])).not.toThrow();
    expect(PM.Capabilities.preview.mock.calls[0][1]).toEqual({ nested: { amount: 1 } });
  });

  it('draws the curve and supports pointer and keyboard handle edits', () => {
    const { PM } = fakePM();
    const section = allControls();
    const target = renderPanel(PM, section);
    const canvas = target.querySelector<HTMLCanvasElement>('.generated-curve-canvas')!;
    vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 600, bottom: 300,
      width: 600, height: 300, toJSON: () => ({})
    } as DOMRect);
    expect(canvasContext.clearRect).toHaveBeenCalled();

    canvas.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, button: 0, pointerId: 1, clientX: 180, clientY: 120 }));
    canvas.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, pointerId: 1, clientX: 240, clientY: 90 }));
    canvas.dispatchEvent(new PointerEvent('pointerup', { bubbles: true, pointerId: 1 }));
    const afterDrag = [...((section.state as any).curve as number[])];
    expect(afterDrag).not.toEqual([.62, .05, 0, 1]);
    expect(PM.invalidate).not.toHaveBeenCalled();

    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true }));
    expect((section.state as any).curve).not.toEqual(afterDrag);
    flushSync(() => canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })));
    const beforeSecondHandle = [...((section.state as any).curve as number[])];
    const activeY = canvas.getAttribute('aria-label')?.includes('Handle 2') ? 3 : 1;
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', shiftKey: true, bubbles: true, cancelable: true }));
    expect((section.state as any).curve[activeY]).toBeCloseTo(beforeSecondHandle[activeY]! + .1);
  });

  it('keeps the current curve when the sanitizer returns undefined', () => {
    const { PM } = fakePM();
    PM.Capabilities.sanitizeCurve = vi.fn(() => undefined);
    const target = renderPanel(PM, allControls());
    expect(target.querySelector('.generated-curve-canvas')).not.toBeNull();
    expect(canvasContext.clearRect).toHaveBeenCalled();
  });
});

describe('generated binding branches', () => {
  const button = (action: any = null, extra: Record<string, any> = {}) => ({
    type: 'button', label: 'Action', primary: false, action, cmd: '', ...extra
  }) as any;

  it('resolves easing, script, history, and command actions', async () => {
    const { PM } = fakePM();
    PM.Capabilities.applyEasing = vi.fn(() => ({ ok: true, message: 'eased', changes: [] }));
    const easing = await resolveButtonAction(PM, button({ type: 'easing', mode: 'apply' }), { curve: [.2, .3, .4, .5] });
    expect(easing).toMatchObject({ kind: 'preview', applied: true, toast: 'Action applied' });
    expect(PM.Capabilities.applyEasing).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'easing', mode: 'apply' }),
      { curve: [.2, .3, .4, .5] },
      { label: 'Action', origin: 'generated-tool' }
    );

    PM.Script.preview = vi.fn(async () => ({ ok: true, message: 'script preview', changes: [] }));
    const script = await resolveButtonAction(PM, button({ type: 'script', mode: 'preview', code: 'return []', label: 'Script label' }), { amount: 2 });
    expect(script).toMatchObject({ kind: 'preview', applied: false, toast: 'Preview ready' });
    expect(PM.Script.preview).toHaveBeenCalledWith('return []', { amount: 2 }, { label: 'Script label' });

    const history = await resolveButtonAction(PM, button({ type: 'history' }), {});
    expect(history).toMatchObject({ kind: 'preview', result: { ok: true, message: 'Restored the previous editable source.' } });

    const command = await resolveButtonAction(PM, button(null, { cmd: 'newShape' }), {});
    expect(command).toEqual({ kind: 'command' });
    expect(PM.cmd).toHaveBeenCalledWith('newShape');
  });

  it('turns a thrown script into a bounded failed result', async () => {
    const { PM } = fakePM();
    PM.Script.apply = vi.fn(async () => { throw new Error('sandbox failed'); });
    const result = await resolveButtonAction(PM, button({ type: 'script', mode: 'apply', code: 'throw 1' }), {});
    expect(result).toMatchObject({
      kind: 'preview', applied: false, toast: 'sandbox failed',
      result: { ok: false, message: 'sandbox failed', changes: [] }
    });
  });

  it('formats every generated readout kind and the error preview branch', () => {
    const { PM } = fakePM();
    PM.sel.layers = ['L1', 'L2'];
    PM.Capabilities.selectedKeyframes.mockReturnValue([{ id: 'k1' }, { id: 'k2' }]);
    PM.Capabilities.keyframeSummary.mockReturnValue('2 keys across 1 layer');
    PM.Capabilities.selectionSummary.mockReturnValue('2 text layers');
    expect(generatedReadout(PM, 'selection.count')).toBe('2');
    expect(generatedReadout(PM, 'keyframes.count')).toBe('2');
    expect(generatedReadout(PM, 'keyframes.summary')).toBe('2 keys across 1 layer');
    expect(generatedReadout(PM, 'selection.summary')).toBe('2 text layers');
    expect(formatPreview(PM, { ok: false, message: 'No valid changes', changes: [] })).toEqual({
      ok: false, title: 'Nothing changed', message: 'No valid changes', items: []
    });
  });

  it('uses work-area and background getter fallbacks and maps layer aliases', () => {
    const { PM } = fakePM();
    delete PM.proj.work;
    PM.proj.backgroundFill = undefined;
    const start = sourceBinding(PM, { target: '$composition', path: 'composition.workArea.start', def: 4 } as any)!;
    const end = sourceBinding(PM, { target: '$composition', path: 'composition.workArea.end', def: 4 } as any)!;
    const endColor = sourceBinding(PM, { target: '$composition', path: 'composition.background.endColor', def: '#ABCDEF' } as any)!;
    const visible = sourceBinding(PM, { target: 'L1', path: 'layer.visible', def: false } as any)!;
    expect(start.get()).toBe(0);
    expect(end.get()).toBe(PM.proj.dur);
    expect(endColor.get()).toBe('#000000');
    expect(visible.get()).toBe(true);
    expect(visible.command(false)).toEqual({ type: 'set_layer', target: 'L1', patch: { visible: false } });
  });
});

describe('generated panel registration', () => {
  it('patches WS.registerCustom and preserves ids, titles, and sizes', () => {
    const { PM, panels } = fakePM();
    const section = allControls();
    installGeneratedPanels(PM);
    PM.WS.registerCustom({ custom: [section] });
    expect(PM.registerPanel).toHaveBeenCalledWith('generated', expect.objectContaining({ title: 'Generated', size: 260 }));
    expect(panels.get('generated').persist).toBe(true);
  });

  it('unmounts a previous component and deletes the legacy instance before re-registration', async () => {
    const { PM, panels } = fakePM();
    const first = allControls();
    registerGeneratedPanels(PM, { custom: [first] });
    const firstBody = document.createElement('div');
    panels.get('generated').build(firstBody, { spec: { id: 'generated' } });
    flushSync();
    expect(firstBody.querySelector('[data-svelte-panel="generated"]')).toBeTruthy();

    PM.panelInst.generated = { built: true, body: firstBody };
    const second = { ...first, title: 'Updated', note: 'Fresh manifest' };
    registerGeneratedPanels(PM, { custom: [second] });
    expect(PM.panelInst.generated).toBeUndefined();
    expect(firstBody.childElementCount).toBe(0);

    const secondBody = document.createElement('div');
    panels.get('generated').build(secondBody, { spec: { id: 'generated' } });
    await tick();
    expect(secondBody.textContent).toContain('Fresh manifest');
  });

  it('disposes a mounted section removed from the next custom manifest', async () => {
    const { PM, panels } = fakePM();
    const keep = { ...allControls(), id: 'keep' };
    const removed = { ...allControls(), id: 'removed' };
    registerGeneratedPanels(PM, { custom: [keep, removed] });
    const body = document.createElement('div');
    panels.get('removed').build(body, { spec: { id: 'removed' } });
    flushSync();
    PM.panelInst.removed = { body };

    registerGeneratedPanels(PM, { custom: [keep] });
    await tick();
    expect(PM.panelInst.removed).toBeUndefined();
    expect(body.childElementCount).toBe(0);
  });
});
