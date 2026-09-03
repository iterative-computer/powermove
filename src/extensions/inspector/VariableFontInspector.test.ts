// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
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
import { sel } from '../../renderer/src/state/selection.svelte';
import { perf, transport } from '../../renderer/src/state/transport.svelte';
import VariableFontHarness from './VariableFontHarness.svelte';

const controls: ControlsAPI = {
  NumField: NumField as ControlsAPI['NumField'], ColorField: ColorField as ControlsAPI['ColorField'],
  FillField: FillField as ControlsAPI['FillField'], FontField: FontField as ControlsAPI['FontField'],
  SelectField: SelectField as ControlsAPI['SelectField'], TextField: TextField as ControlsAPI['TextField'],
  ToggleField: ToggleField as ControlsAPI['ToggleField'], Row: Row as ControlsAPI['Row'],
  Section: Section as ControlsAPI['Section'],
  binding: {
    channelBinding,
    compositionBinding: (PM, field, options) => compositionBinding(PM, field as any, options),
    contentBinding,
    layerFieldBinding: (PM, layerId, field, options) => layerFieldBinding(PM, layerId, field as any, options),
  },
};

let target: HTMLDivElement;
let instance: Record<string, any> | undefined;

function setup(content: Record<string, any>) {
  const layer = {
    id: 'text-1', type: 'text', name: 'Title', from: 0, dur: 5,
    p: {}, fx: [], masks: [], d: {
      text: 'Variable', font: 'Test Variable', weight: 400, size: 64,
      tracking: 0, leading: 1.1, align: 'center', color: '#fff', italic: false,
      ...content,
    },
  };
  const project = { id: 'p', w: 1920, h: 1080, fps: 30, dur: 5, assets: {}, layers: [layer] };
  const apply = vi.fn(() => ({ ok: true }));
  const PM: Record<string, any> = {
    proj: project,
    P: (value: number) => ({ v: value, kf: [], expr: null }),
    Fonts: { options: (value: string) => [value], ensure: vi.fn() },
    Edit: { apply, begin: vi.fn(), dispatch: vi.fn(), commit: vi.fn(), cancel: vi.fn() },
    evP: vi.fn((_layer, property) => property.v),
    round: (value: number, precision: number) => Number(value.toFixed(precision)),
    drag: vi.fn(), invalidate: vi.fn(),
    Inspector: { refresh: vi.fn() },
  };
  doc.replace(project as any);
  transport.time = 2;
  const api = {
    id: 'inspector', apiVersion: 1,
    manifest: { id: 'inspector', name: 'Inspector', version: '1', apiVersion: 1 },
    ui: { controls, icon: () => '<svg></svg>' },
    host: { pm: PM, state: { doc, sel, transport, perf }, mount: vi.fn() },
  } as unknown as PowermoveAPI;
  instance = mount(VariableFontHarness, { target, props: { api, layer } });
  flushSync();
  return { PM, apply };
}

beforeEach(() => {
  target = document.createElement('div');
  document.body.append(target);
});

afterEach(async () => {
  if (instance) await unmount(instance);
  instance = undefined;
  target.remove();
});

describe('variable font inspector', () => {
  it('binds an enabled axis to a real keyframeable content property', () => {
    const property = { v: 75, kf: [], expr: null };
    const { apply } = setup({ 'fontAxis.wdth': property });
    const row = target.querySelector<HTMLElement>('[data-channel="c.fontAxis.wdth"]')!;
    expect(row).not.toBeNull();
    row.querySelector<HTMLInputElement>('[role="spinbutton"]')!.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    );
    expect(apply).toHaveBeenCalledWith({
      type: 'set_property', target: 'text-1', path: 'c.fontAxis.wdth', value: 75.875,
      time: 2, mode: 'auto', preserveHandEdits: false,
    }, { label: 'Width · wdth', origin: 'inspector' });
  });

  it('adds arbitrary four-character axes as persisted channel objects', () => {
    const { apply } = setup({});
    const input = target.querySelector<HTMLInputElement>('[aria-label="Custom OpenType axis tag"]')!;
    input.value = 'GRAD';
    input.dispatchEvent(new InputEvent('input', { bubbles: true }));
    flushSync();
    target.querySelector<HTMLButtonElement>('.custom-axis-add button')!.click();
    expect(apply).toHaveBeenCalledWith({
      type: 'set_content', target: 'text-1',
      patch: { 'fontAxis.GRAD': { v: 0, kf: [], expr: null } },
    }, { label: 'Add Grade axis', origin: 'inspector' });
  });

  it('uses the variable weight channel instead of presenting two weight controls', () => {
    setup({ 'fontAxis.wght': { v: 520, kf: [], expr: null } });
    expect(target.querySelector('[data-channel="c.fontAxis.wght"]')).not.toBeNull();
    expect([...target.querySelectorAll('.row .k')].filter((node) => node.textContent === 'Weight')).toHaveLength(0);
  });
});
