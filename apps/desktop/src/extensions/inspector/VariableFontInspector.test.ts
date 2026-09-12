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
import * as fontCatalog from '../../renderer/src/typography/font-catalog';
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
    proj: project, sel: { layers: [layer.id], keys: [], chan: null },
    selLayers: () => [layer], firstSel: () => layer, L: () => layer,
    findProp: (_layer: any, path: string) => (layer.d as any)[path.slice(2)],
    hasKeyAt: () => null,
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
  vi.restoreAllMocks();
});

describe('type settings', () => {
  it('shows real discovered axes without creating project values or duplicate weight fields', async () => {
    vi.spyOn(fontCatalog, 'inspectFont').mockResolvedValue({ family: 'Test Variable', status: 'variable', axes: [
      { tag: 'wght', label: 'Weight', min: 100, max: 900, default: 400 },
      { tag: 'wdth', label: 'Width', min: 75, max: 125, default: 100 }
    ] });
    const { PM, apply } = setup({});
    await vi.waitFor(() => { flushSync(); expect(target.querySelector('[data-font-axis="wdth"] [role="spinbutton"]')).not.toBeNull(); });
    expect(apply).not.toHaveBeenCalled();
    const field = target.querySelector<HTMLInputElement>('[data-font-axis="wdth"] [role="spinbutton"]')!;
    expect(field.getAttribute('aria-valuemin')).toBe('75'); expect(field.getAttribute('aria-valuemax')).toBe('125');
    expect(field.value).toBe('100');
    expect(target.querySelector('input[type="range"]')).toBeNull();
    expect([...target.querySelectorAll('.row .k')].filter(node => node.textContent === 'Weight')).toHaveLength(1);
    field.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }));
    expect(apply).toHaveBeenLastCalledWith([
      {type:'set_content',target:'text-1',patch:{'fontAxis.wdth':{v:100,kf:[],expr:null}}},
      {type:'set_property',target:'text-1',path:'c.fontAxis.wdth',value:100.01,time:2,mode:'auto',preserveHandEdits:false}
    ], { label: 'Width axis', origin: 'inspector' });
  });
  it('distinguishes static fonts and keeps saved custom axes without invented sliders', async () => {
    vi.spyOn(fontCatalog,'inspectFont').mockResolvedValue({family:'Static',status:'static',axes:[]});
    setup({'fontAxis.XTRA':{v:37,kf:[],expr:null}});
    await vi.waitFor(()=>{flushSync();expect(target.textContent).toContain('is a static font');});
    expect(target.querySelector('[data-channel="c.fontAxis.XTRA"]')).not.toBeNull();
    expect(target.querySelector('[aria-label="Custom OpenType axis tag"]')).toBeNull();
    expect(target.querySelector('input[type="range"]')).toBeNull();
  });
});


describe('text alignment', () => {
  it('shows the current alignment and writes changes through the animated content binding', () => {
    const { apply } = setup({ align: 'center' });
    const center = target.querySelector<HTMLButtonElement>('[aria-label="Align center"]')!;
    expect(center.getAttribute('aria-pressed')).toBe('true');
    center.click();
    expect(apply).not.toHaveBeenCalled();
    for (const alignment of ['left', 'right']) {
      target.querySelector<HTMLButtonElement>(`[aria-label="Align ${alignment}"]`)!.click();
      expect(apply).toHaveBeenLastCalledWith(
        { type: 'set_content', target: 'text-1', patch: { align: alignment } },
        { label: 'Text alignment', origin: 'inspector' }
      );
    }
  });
});
