// @vitest-environment happy-dom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { doc } from '../state/document.svelte';
import ColorField from './ColorField.svelte';
import FillField from './FillField.svelte';
import FontField from './FontField.svelte';
import NumField from './NumField.svelte';
import RowNumFieldHarness from './__fixtures__/RowNumFieldHarness.svelte';
import SelectField from './SelectField.svelte';
import TextField from './TextField.svelte';
import ToggleField from './ToggleField.svelte';
import type { EditBinding } from './gesture';

const mounted: object[] = [];

afterEach(async () => {
  for (const component of mounted.splice(0)) await unmount(component);
  document.body.replaceChildren();
  delete (window as Window & { EyeDropper?: unknown }).EyeDropper;
});

function fakeAPI() {
  interface DragOptions {
    move(dx: number, dy: number, event: PointerEvent): void;
    up(): void;
    cancel(): void;
  }
  let dragOptions: DragOptions | undefined;
  const Edit = {
    begin: vi.fn(),
    dispatch: vi.fn(),
    commit: vi.fn(),
    cancel: vi.fn(),
    apply: vi.fn((_command: unknown, _meta: unknown) => ({ ok: true }))
  };
  const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value));
  const normalizeFill = vi.fn((value: any, fallback = '#000000') => {
    const color = (candidate: any) => typeof candidate === 'string' && /^#[0-9a-f]{6}$/i.test(candidate) ? candidate.toUpperCase() : fallback;
    const raw = value && typeof value === 'object' ? value : {};
    const type = ['solid', 'linear', 'radial', 'none'].includes(raw.type) ? raw.type : 'solid';
    let stops = (Array.isArray(raw.stops) ? raw.stops : []).slice(0, 8).map((stop: any, index: number) => ({
      id: typeof stop?.id === 'string' && stop.id ? stop.id : `stop-${index + 1}`,
      color: color(stop?.color),
      position: clamp(Number(stop?.position) || 0, 0, 100)
    })).sort((a: any, b: any) => a.position - b.position);
    if (!stops.length) stops = [{ id: 'stop-1', color: color(raw.color || fallback), position: 0 }];
    if (!['solid', 'none'].includes(type) && stops.length < 2) stops.push({ id: 'stop-2', color: stops[0].color, position: 100 });
    if (['solid', 'none'].includes(type)) stops = [{ ...stops[0], position: 0 }];
    return { type, angle: clamp(Number(raw.angle) || 0, -180, 180), stops };
  });
  const hist = { begin: vi.fn(), commit: vi.fn(), cancel: vi.fn(), do: vi.fn((_label: string, fn: () => void) => fn()) };
  const drag = vi.fn((_event: PointerEvent, options: DragOptions) => { dragOptions = options; });
  const invalidate = vi.fn();
  const closeMenus = vi.fn();
  const toast = vi.fn();
  const fonts = { options: vi.fn(() => ['Inter', 'Avenir Next']), ensure: vi.fn() };
  const api = {
    edit: Edit,
    history: hist,
    transport: { time: () => 2, invalidate },
    ui: { drag, closeMenus, toast },
    util: {
      round: (value: number, precision: number) => Number(value.toFixed(precision)),
      clamp,
      uid: (prefix: string) => `${prefix}-new`
    },
    model: { normalizeFill },
    media: { fonts }
  };
  return { api, Edit, normalizeFill, invalidate, closeMenus, fonts, drag: () => dragOptions! };
}

const commandEdit = (label = 'Value'): EditBinding => ({
  mode: 'command',
  label,
  origin: 'inspector',
  command: (value) => ({ type: 'set_property', target: 'L1', path: 'opacity', value: value as number })
});

function render(component: any, props: Record<string, unknown>): HTMLElement {
  const target = document.createElement('div');
  document.body.append(target);
  mounted.push(mount(component, { target, props }));
  flushSync();
  return target;
}

const pointer = (type: string, init: PointerEventInit = {}) => new PointerEvent(type, { bubbles: true, button: 0, ...init });

describe('NumField', () => {
  it('adjusts from horizontal trackpad movement and commits one gesture', () => {
    vi.useFakeTimers();
    const { api, Edit } = fakeAPI();
    let current = 1000;
    Edit.dispatch.mockImplementation((command: any) => { current = command.value; });
    const target = render(NumField, { api, get: () => current, edit: commandEdit(), step: 1 });
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.dispatchEvent(new WheelEvent('wheel', { deltaX: 100, cancelable: true }));
    input.dispatchEvent(new WheelEvent('wheel', { deltaX: -20, cancelable: true }));
    expect(current).toBe(1010);
    expect(Edit.begin).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(350);
    expect(Edit.commit).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it('keeps a negative value when the editor is opened and left, and treats a bare minus as a number', async () => {
    const { api, Edit, drag } = fakeAPI();
    let current = -2;
    Edit.dispatch.mockImplementation((command: any) => { current = command.value; });
    Edit.apply.mockImplementation((command: any) => { current = command.value; return { ok: true }; });
    const target = render(NumField, { api, get: () => current, edit: commandEdit('Tracking'), step: 1, label: 'Tracking' });
    const input = target.querySelector<HTMLInputElement>('input.num')!;
    input.dispatchEvent(pointer('pointerdown'));
    drag().up();
    await tick();
    expect(input.readOnly).toBe(false);
    input.dispatchEvent(new FocusEvent('blur'));
    expect(Edit.apply).not.toHaveBeenCalled();
    expect(current).toBe(-2);

    input.dispatchEvent(pointer('pointerdown'));
    drag().up();
    await tick();
    input.value = '-7';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(Edit.apply).toHaveBeenLastCalledWith(expect.objectContaining({ value: -7 }), expect.anything());

    input.dispatchEvent(pointer('pointerdown'));
    drag().up();
    await tick();
    input.value = '-=3';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(Edit.apply).toHaveBeenLastCalledWith(expect.objectContaining({ value: -10 }), expect.anything());
  });

  it('aborts a scrub cleanly when another edit transaction is already live', () => {
    const { api, Edit, drag } = fakeAPI();
    Edit.begin.mockImplementationOnce(() => { throw new Error('A source edit is already active'); });
    const target = render(NumField, { api, get: () => 10, edit: commandEdit('Size'), label: 'Size' });
    const input = target.querySelector<HTMLInputElement>('input.num')!;
    expect(() => input.dispatchEvent(pointer('pointerdown'))).not.toThrow();
    expect(api.ui.drag).not.toHaveBeenCalled();
    expect(Edit.cancel).not.toHaveBeenCalled();
    // The next gesture works normally.
    input.dispatchEvent(pointer('pointerdown'));
    expect(api.ui.drag).toHaveBeenCalledTimes(1);
    drag().up();
  });

  it('scrubs on both axes: right or up raises, left or down lowers', () => {
    const { api, Edit, drag } = fakeAPI();
    let current = 10;
    Edit.dispatch.mockImplementation((command: any) => { current = command.value; });
    const target = render(NumField, { api, get: () => current, edit: commandEdit('Size'), step: 1, speed: 1, label: 'Size' });
    const input = target.querySelector<HTMLInputElement>('input.num')!;
    input.dispatchEvent(pointer('pointerdown'));
    drag().move(0, -2, pointer('pointermove'));
    expect(Edit.dispatch).not.toHaveBeenCalled();
    drag().move(0, -8, pointer('pointermove'));
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: 18 }));
    drag().move(-4, 6, pointer('pointermove'));
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: 0 }));
    drag().up();
    expect(Edit.commit).toHaveBeenCalledWith('Size');
  });

  it('scrubs through begin → writes → commit and opens editing after a click/cancel', async () => {
    const { api, Edit, drag } = fakeAPI();
    let current = 10;
    Edit.dispatch.mockImplementation((command: any) => { current = command.value; });
    const target = render(NumField, { api, get: () => current, edit: commandEdit('Opacity'), step: 2, speed: 0.5, label: 'Opacity' });
    const input = target.querySelector<HTMLInputElement>('input.num')!;

    input.dispatchEvent(pointer('pointerdown'));
    expect(Edit.begin).toHaveBeenCalledWith('Opacity', { origin: 'inspector' });
    drag().move(2, 0, pointer('pointermove'));
    drag().move(4, 0, pointer('pointermove'));
    drag().move(8, 0, pointer('pointermove', { shiftKey: true }));
    drag().up();
    expect(Edit.dispatch).toHaveBeenCalledTimes(2);
    expect(Edit.dispatch).toHaveBeenNthCalledWith(1, expect.objectContaining({ value: 14 }));
    expect(Edit.dispatch).toHaveBeenNthCalledWith(2, expect.objectContaining({ value: 90 }));
    expect(Edit.commit).toHaveBeenCalledWith('Opacity');

    input.dispatchEvent(pointer('pointerdown'));
    drag().up();
    await tick();
    expect(Edit.cancel).toHaveBeenCalledTimes(1);
    expect(input.classList.contains('editing')).toBe(true);
    expect(input.readOnly).toBe(false);

    input.value = '+5';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(Edit.apply).toHaveBeenLastCalledWith(
      expect.objectContaining({ value: 95 }),
      { label: 'Opacity', origin: 'inspector' }
    );

    input.dispatchEvent(pointer('pointerdown'));
    drag().up();
    await tick();
    const applies = Edit.apply.mock.calls.length;
    input.value = '999';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    flushSync();
    expect(Edit.apply).toHaveBeenCalledTimes(applies);
    expect(input.classList.contains('editing')).toBe(false);
  });

  it('supports Tab-focus keyboard editing and commits arrows through once', async () => {
    const { api, Edit } = fakeAPI();
    const target = render(NumField, { api, get: () => 4, edit: commandEdit(), step: 0.5, label: 'Value' });
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.focus();
    await tick();
    expect(input.readOnly).toBe(false);
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', shiftKey: true, bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(Edit.apply).toHaveBeenCalledWith(expect.objectContaining({ value: 9 }), expect.anything());
  });

  it('formats precision/unit, clamps scrubs, and preserves legacy zero-option defaults', () => {
    const { api, Edit, drag } = fakeAPI();
    let current = 9.876;
    Edit.dispatch.mockImplementation((command: any) => { current = command.value; });
    const target = render(NumField, {
      api, get: () => current, edit: commandEdit(), min: 0, max: 10,
      step: 0.01, speed: 1, precision: 2, unit: 'px', label: 'Value'
    });
    const input = target.querySelector<HTMLInputElement>('input')!;
    expect(input.value).toBe('9.88px');
    input.dispatchEvent(pointer('pointerdown'));
    drag().move(100, 0, pointer('pointermove'));
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: 10 }));

    const second = render(NumField, { api, get: () => 2, edit: commandEdit(), step: 0, speed: 0, label: 'Legacy defaults' });
    second.querySelector<HTMLInputElement>('input')!.dispatchEvent(pointer('pointerdown'));
    drag().move(4, 0, pointer('pointermove'));
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: 4 }));
  });

  it('associates a visible Row label with the spinbutton', () => {
    const { api } = fakeAPI();
    const target = render(RowNumFieldHarness, { api, get: () => 50, edit: commandEdit('Opacity') });
    const label = target.querySelector<HTMLElement>('.row .k')!;
    const input = target.querySelector<HTMLInputElement>('input.num')!;
    expect(label.textContent).toBe('Opacity');
    expect(label.id).not.toBe('');
    expect(input.getAttribute('aria-labelledby')).toBe(label.id);
  });

  it('re-derives its getter after a document value tick', () => {
    const { api } = fakeAPI();
    let current = 12;
    const target = render(NumField, { api, get: () => current, edit: commandEdit(), step: 1, label: 'Value' });
    const input = target.querySelector<HTMLInputElement>('input')!;
    expect(input.value).toBe('12');
    current = 27;
    doc.bump('values');
    flushSync();
    expect(input.value).toBe('27');
  });
});

describe('one-shot fields', () => {
  it('ToggleField applies the inverted getter value once', () => {
    const { api, Edit, invalidate } = fakeAPI();
    const target = render(ToggleField, { api, get: () => false, edit: commandEdit('Enabled'), label: 'Enabled' });
    const [off, on] = target.querySelectorAll<HTMLButtonElement>('.onoff button');
    expect(off!.getAttribute('aria-checked')).toBe('true');
    off!.click();
    expect(Edit.apply).not.toHaveBeenCalled();
    on!.click();
    expect(Edit.apply).toHaveBeenCalledWith(expect.objectContaining({ value: true }), { label: 'Enabled', origin: 'inspector' });
    expect(invalidate).toHaveBeenCalledWith();
  });

  it('SelectField applies the selected typed option once', () => {
    const { api, Edit, invalidate } = fakeAPI();
    const target = render(SelectField, {
      api, get: () => 'a', edit: commandEdit('Mode'), label: 'Mode',
      options: [{ v: 'a', label: 'Alpha' }, { v: 2, label: 'Two' }]
    });
    const select = target.querySelector<HTMLSelectElement>('select')!;
    expect(select.selectedIndex).toBe(0);
    expect(select.selectedOptions[0]?.textContent).toBe('Alpha');
    select.value = '1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    expect(Edit.apply).toHaveBeenCalledWith(expect.objectContaining({ value: 2 }), { label: 'Mode', origin: 'inspector' });
    expect(invalidate).toHaveBeenCalledWith();
  });

  it('SelectField displays an unmatched raw value instead of the first option', () => {
    const { api } = fakeAPI();
    const target = render(SelectField, {
      api, get: () => 'overlay', edit: commandEdit('Blend'), label: 'Blend',
      options: [{ v: 'normal', label: 'Normal' }, { v: 'multiply', label: 'Multiply' }]
    });
    const select = target.querySelector<HTMLSelectElement>('select')!;
    expect(select.value).toBe('-1');
    expect(select.selectedIndex).toBe(0);
    expect(select.selectedOptions[0]?.textContent).toBe('overlay');
  });

  it('FontField toggles closed and can open again', async () => {
    const { api } = fakeAPI();
    const target = render(FontField, { api, get: () => 'Inter', edit: commandEdit('Font') });
    const trigger = target.querySelector<HTMLButtonElement>('button.font-select')!;
    trigger.click();
    await tick();
    expect(target.querySelector('.font-menu')).not.toBeNull();
    trigger.click();
    await tick();
    // The sheet stays mounted while its close animation plays.
    expect(target.querySelector<HTMLElement>('.font-menu')?.dataset.state ?? 'closed').toBe('closed');
    await new Promise((resolve) => setTimeout(resolve, 230));
    expect(target.querySelector('.font-menu')).toBeNull();
    trigger.click();
    await tick();
    expect(target.querySelector('.font-menu')).not.toBeNull();
  });

  it('FontField uses the font list, invalidates all views, and exposes a dialog/listbox', async () => {
    const { api, Edit, closeMenus, invalidate, fonts } = fakeAPI();
    const target = render(FontField, { api, get: () => 'Inter', edit: commandEdit('Font'), label: 'Font' });
    target.querySelector<HTMLButtonElement>('button.font-select')!.click();
    await tick();
    expect(closeMenus).toHaveBeenCalledTimes(1);
    expect(target.querySelector('[role="dialog"] [role="listbox"]')).not.toBeNull();
    target.querySelectorAll<HTMLElement>('.font-menu-row')[1]!.click();
    expect(Edit.apply).toHaveBeenCalledWith(expect.objectContaining({ value: 'Avenir Next' }), { label: 'Font', origin: 'inspector' });
    expect(invalidate).toHaveBeenCalledWith();
    expect(fonts.ensure).toHaveBeenCalledWith('Avenir Next', 400);
  });
});

describe('TextField', () => {
  it('uses begin/write/commit and cancels an Escape edit', () => {
    const { api, Edit } = fakeAPI();
    const target = render(TextField, { api, get: () => 'Hello', edit: commandEdit('Text'), label: 'Text' });
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.focus();
    input.value = 'World';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    expect(Edit.begin).toHaveBeenCalledWith('Text', { origin: 'inspector' });
    expect(Edit.dispatch).toHaveBeenCalledWith(expect.objectContaining({ value: 'World' }));
    input.blur();
    expect(Edit.commit).toHaveBeenCalledWith('Text');

    input.focus();
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(Edit.cancel).toHaveBeenCalledTimes(1);
  });
});

describe('picker drafts', () => {
  it('ColorField previews saturation and brightness continuously, then cancels the preview', async () => {
    const { api, Edit, drag, invalidate } = fakeAPI();
    const target = render(ColorField, { api, get: () => '#FF0000', edit: commandEdit('Color'), label: 'Color' });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();

    const sv = document.body.querySelector<HTMLElement>('.cp-plane')!;
    vi.spyOn(sv, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100,
      toJSON: () => ({})
    });
    sv.dispatchEvent(pointer('pointerdown', { pointerId: 1, clientX: 50, clientY: 25 }));

    expect(Edit.begin).toHaveBeenCalledWith('Color', { origin: 'inspector' });
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: '#BF6060' }));

    drag().move(25, 25, pointer('pointermove', { pointerId: 1, clientX: 75, clientY: 50 }));
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: '#802020' }));
    expect(invalidate).toHaveBeenLastCalledWith('render');

    document.body.querySelector<HTMLElement>('.color-picker')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(Edit.cancel).toHaveBeenCalledOnce();
    expect(Edit.commit).not.toHaveBeenCalled();
  });

  it('ColorField removes confirmation buttons and commits the normalized choice when clicking outside', async () => {
    const { api, Edit } = fakeAPI();
    const target = render(ColorField, { api, get: () => '#ff6b1a', edit: commandEdit('Color'), label: 'Color' });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    expect(document.body.querySelector<HTMLElement>('.color-picker')!.style.top).not.toBe('0px');
    const hex = document.body.querySelector<HTMLInputElement>('.cp-value')!;
    hex.value = '#34c759';
    hex.dispatchEvent(new InputEvent('input', { bubbles: true }));
    flushSync();
    expect(document.body.querySelector('.color-picker footer')).toBeNull();
    document.body.querySelector<HTMLElement>('.color-picker-layer')!.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    flushSync();
    // The popover plays the menu's exit before it unmounts.
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(Edit.begin).toHaveBeenCalledWith('Color', { origin: 'inspector' });
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: '#34C759' }));
    expect(Edit.commit).toHaveBeenCalledWith('Color');
    expect(document.body.querySelector('.color-picker')).toBeNull();
  });

  it('ColorField samples a screen color through the system eyedropper', async () => {
    const open = vi.fn().mockResolvedValue({ sRGBHex: '#0a84ff' });
    (window as Window & { EyeDropper?: new () => { open: () => ReturnType<typeof open> } }).EyeDropper = class {
      open() { return open(); }
    };
    const { api, Edit } = fakeAPI();
    const target = render(ColorField, { api, get: () => '#ff6b1a', edit: commandEdit('Color'), label: 'Color' });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();

    document.body.querySelector<HTMLButtonElement>('[aria-label="Sample screen color"]')!.click();
    await vi.waitFor(() => expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: '#0A84FF' })));
    expect(open).toHaveBeenCalledOnce();
    expect(document.body.querySelector<HTMLInputElement>('.cp-value')!.value).toBe('#0A84FF / 100%');
  });

  it('mounts color and fill picker overlays above clipped panel contents', async () => {
    const { api } = fakeAPI();
    const colorTarget = render(ColorField, { api, get: () => '#ff6b1a', edit: commandEdit('Color'), label: 'Color' });
    colorTarget.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    expect(document.body.querySelector('.color-picker-layer')?.parentElement).toBe(document.body);

    document.body.querySelector<HTMLElement>('.color-picker')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(document.body.querySelector('.color-picker')).toBeNull();

    const fillTarget = render(FillField, {
      api,
      get: () => ({ type: 'solid', angle: 0, stops: [{ id: 'orange', color: '#FF6B1A', position: 0 }] }),
      edit: commandEdit('Fill'),
      label: 'Fill'
    });
    fillTarget.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    expect(document.body.querySelector('.fill-picker-layer')?.parentElement).toBe(document.body);
  });

  // Picker preferences are shared across pickers, so this runs after the other ColorField tests.
  it('ColorField picks in Display P3 and writes the clipped sRGB hex', async () => {
    const { api, Edit } = fakeAPI();
    const target = render(ColorField, { api, get: () => '#808080', edit: commandEdit('Color'), label: 'Color' });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    document.body.querySelector<HTMLButtonElement>('[role="tab"]:nth-child(2)')!.click();
    flushSync();

    const plane = document.body.querySelector<HTMLElement>('.cp-plane')!;
    vi.spyOn(plane, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100,
      toJSON: () => ({})
    });
    const hue = document.body.querySelector<HTMLElement>('.cp-hue')!;
    vi.spyOn(hue, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 14, bottom: 100, width: 14, height: 100,
      toJSON: () => ({})
    });
    hue.dispatchEvent(pointer('pointerdown', { pointerId: 1, clientX: 7, clientY: 0 }));
    plane.dispatchEvent(pointer('pointerdown', { pointerId: 1, clientX: 100, clientY: 0 }));
    flushSync();

    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({ value: '#FF0000' }));
    expect(document.body.querySelector('.cp-clipped')?.textContent).toBe('Clipped');
    expect(document.body.querySelector<HTMLInputElement>('.cp-value')!.getAttribute('aria-label')).toBe('Color OKLCH value');
  });

  it('FillField previews color continuously across pointer moves', async () => {
    const { api, Edit, drag, invalidate } = fakeAPI();
    const target = render(FillField, {
      api,
      get: () => ({ type: 'solid', angle: 0, stops: [{ id: 'red', color: '#FF0000', position: 0 }] }),
      edit: commandEdit('Fill'),
      label: 'Fill'
    });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();

    const sv = document.body.querySelector<HTMLElement>('.fill-sv')!;
    vi.spyOn(sv, 'getBoundingClientRect').mockReturnValue({
      x: 0, y: 0, left: 0, top: 0, right: 100, bottom: 100, width: 100, height: 100,
      toJSON: () => ({})
    });
    sv.dispatchEvent(pointer('pointerdown', { pointerId: 1, clientX: 50, clientY: 25 }));
    drag().move(25, 25, pointer('pointermove', { pointerId: 1, clientX: 75, clientY: 50 }));

    expect(Edit.begin).toHaveBeenCalledWith('Fill', { origin: 'inspector' });
    expect(Edit.dispatch).toHaveBeenLastCalledWith(expect.objectContaining({
      value: expect.objectContaining({ stops: [expect.objectContaining({ color: '#802020' })] })
    }));
    expect(invalidate).toHaveBeenLastCalledWith('render');

    document.body.querySelector<HTMLButtonElement>('.fill-picker footer .pri')!.click();
    expect(Edit.commit).toHaveBeenCalledWith('Fill');
    expect(Edit.apply).not.toHaveBeenCalled();
  });

  it('FillField applies a normalized fill rather than its mutable draft', async () => {
    const { api, Edit, normalizeFill } = fakeAPI();
    const target = render(FillField, {
      api,
      get: () => ({ type: 'linear', angle: 240, stops: [{ color: '#f00', position: -20 }] }),
      edit: commandEdit('Fill'),
      label: 'Fill',
      fallback: '#123456'
    });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    expect(document.body.querySelector<HTMLElement>('.fill-picker')!.style.top).not.toBe('0px');
    document.body.querySelector<HTMLButtonElement>('.fill-picker footer .pri')!.click();
    expect(normalizeFill).toHaveBeenCalled();
    const command = Edit.apply.mock.calls[0]![0] as any;
    expect(command.value).toMatchObject({ type: 'linear', angle: 180 });
    expect(command.value.stops).toEqual([
      { id: 'stop-1', color: '#123456', position: 0 },
      { id: 'stop-2', color: '#123456', position: 100 }
    ]);
  });

  it('keeps the last valid gradient-stop color when the text buffer is invalid', async () => {
    const { api, Edit } = fakeAPI();
    const target = render(FillField, {
      api,
      get: () => ({
        type: 'linear', angle: 0,
        stops: [{ id: 'red', color: '#FF0000', position: 0 }, { id: 'blue', color: '#0000FF', position: 100 }]
      }),
      edit: commandEdit('Fill'),
      label: 'Fill'
    });
    target.querySelector<HTMLButtonElement>('button.color-field')!.click();
    await tick();
    const stop = document.body.querySelector<HTMLInputElement>('.fill-stop-color')!;
    stop.value = '#12';
    stop.dispatchEvent(new InputEvent('input', { bubbles: true }));
    document.body.querySelector<HTMLButtonElement>('.fill-picker footer .pri')!.click();
    const command = Edit.apply.mock.calls[0]![0] as any;
    expect(command.value.stops[0].color).toBe('#FF0000');
  });
});
