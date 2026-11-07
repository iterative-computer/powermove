// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { registryView } from '../legacy/kernel-view';
import { createKernel } from '../kernel/registries';
import { installKernelSignals, resetKernelSignals } from '../kernel/signals.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import { registerSimplePanels } from './register-simple';
import { fxBrowser } from './fx-browser.svelte';

function basePM(overrides: Record<string, any> = {}): Record<string, any> {
  return {
    FX: {
      blur: { group: 'Blur', label: 'Gaussian Blur' },
      glow: { group: 'Stylize', label: 'Glow' },
      sharpen: { group: 'Blur', label: 'Sharpen' }
    },
    TRANSITIONS: { fade: { label: 'Fade' } },
    proj: { layers: [{ id: 'audio-1', type: 'audio' }, { id: 'vis-1', type: 'shape' }] },
    firstSel: () => ({ id: 'layer-1' }),
    Edit: { apply: vi.fn() },
    Inspector: { refresh: vi.fn() },
    invalidate: vi.fn(),
    toast: vi.fn(),
    ...overrides
  };
}

function mountPanel() {
  const target = document.createElement('div');
  document.body.append(target);
  const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });
  return { target, component };
}

const labels = (target: HTMLElement) => [...target.querySelectorAll('.fxb-name')].map((node) => node.textContent);
const secs = (target: HTMLElement) => [...target.querySelectorAll('.fxb-sec')].map((node) => node.textContent?.trim());

afterEach(() => {
  fxBrowser.query = ''; fxBrowser.searchOpen = false;
  fxBrowser.category = 'all'; fxBrowser.folded = [];
  window.localStorage?.clear();
  document.body.replaceChildren();
  delete window.PM;
  vi.clearAllMocks();
});

describe('FxBrowserPanel', () => {
  it('groups by section and applies one add_effect through the legacy editing path', async () => {
    const PM = basePM();
    window.PM = PM as any;
    const { target, component } = mountPanel();

    expect(secs(target)).toEqual(['Blur', 'Stylize', 'Matte']);
    expect(labels(target)).toEqual(['Gaussian Blur', 'Sharpen', 'Glow', 'Track Matte']);

    const card = target.querySelector<HTMLElement>('.fxb-row');
    expect(card?.textContent).toContain('Gaussian Blur');
    const pick = card!.querySelector<HTMLButtonElement>('.fxb-pick')!;
    expect(pick.getAttribute('draggable')).toBe('true');
    flushSync(() => {
      pick.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
      pick.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 2 }));
    });
    expect(PM.Edit.apply).not.toHaveBeenCalled();
    const add = card!.querySelector<HTMLButtonElement>('button[aria-label="Add Gaussian Blur"]')!;
    flushSync(() => add.click());

    expect(PM.Edit.apply).toHaveBeenCalledOnce();
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      { type: 'add_effect', target: 'layer-1', effect: 'blur' },
      { label: 'Add Gaussian Blur', origin: 'fx-browser' }
    );
    expect(PM.Inspector.refresh).toHaveBeenCalledOnce();
    expect(PM.invalidate).toHaveBeenCalledOnce();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Added Gaussian Blur');

    await unmount(component);
  });

  it('filters by search into a flat list', async () => {
    window.PM = basePM() as any;
    const { target, component } = mountPanel();
    // The search field is always there; the header's search action focuses it.
    const search = target.querySelector<HTMLInputElement>('.fxb-search')!;
    expect(search).not.toBeNull();
    flushSync(() => { fxBrowser.searchOpen = true; });
    expect(document.activeElement).toBe(search);
    search.value = 'sharp';
    flushSync(() => search.dispatchEvent(new Event('input', { bubbles: true })));
    expect(labels(target)).toEqual(['Sharpen']);
    expect(secs(target)).toEqual([]);

    await unmount(component);
  });

  it('keeps the panel header to its title; search is always in the panel', () => {
    const PM = basePM({ PANELS: {}, registerPanel: vi.fn(), icon: () => document.createElement('span'), pickFiles: vi.fn() });
    PM.registerPanel = (id: string, def: any) => { PM.PANELS[id] = def; };
    window.PM = PM as any;
    registerSimplePanels(PM);
    const hdr = document.createElement('header');
    hdr.append(Object.assign(document.createElement('span'), { className: 'ptitle' }));
    document.body.append(hdr);
    PM.PANELS.fxbrowser.header?.(hdr, {});
    expect(hdr.querySelectorAll('[role="tab"]')).toHaveLength(0);
    expect(hdr.querySelector('.iconbtn')).toBeNull();
  });

  it('reports when no layer is selected and toasts', async () => {
    const PM = basePM({ firstSel: () => null });
    window.PM = PM as any;
    const { target, component } = mountPanel();
    const add = target.querySelector<HTMLButtonElement>('.fxb-add');
    flushSync(() => add?.click());
    expect(PM.toast).toHaveBeenCalledWith('Select a layer first');
    expect(PM.Edit.apply).not.toHaveBeenCalled();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Select a layer first');
    await unmount(component);
  });

  it('sets the drag payload on dragstart', async () => {
    window.PM = basePM({ firstSel: () => null }) as any;
    const { target, component } = mountPanel();
    const card = target.querySelector<HTMLElement>('.fxb-pick')!;

    const setData = vi.fn();
    const dataTransfer = { setData, effectAllowed: 'none' };
    const event = new Event('dragstart', { bubbles: true }) as any;
    Object.defineProperty(event, 'dataTransfer', { value: dataTransfer });
    card.dispatchEvent(event);
    expect(setData).toHaveBeenCalledWith(
      'application/x-powermove-fx',
      JSON.stringify({ kind: 'effect', id: 'blur', label: 'Gaussian Blur' })
    );
    expect(dataTransfer.effectAllowed).toBe('copy');
    expect(window.PM!.Edit.apply).not.toHaveBeenCalled();

    await unmount(component);
  });

  it('shows an effect registered by an extension without remounting, and drops it on dispose', async () => {
    const kernel = createKernel();
    const signals = installKernelSignals(kernel);
    window.PM = basePM({
      Kernel: kernel,
      FX: registryView(kernel.effects, { read: (item: any) => item })
    }) as any;
    kernel.registerEffect('legacy', {
      id: 'blur', label: 'Gaussian Blur', group: 'Blur', params: [], frag: 'o = texture(u_tex, v_st);'
    });
    const { target, component } = mountPanel();
    expect(labels(target)).toEqual(['Gaussian Blur', 'Track Matte']);

    let registration: { dispose(): void } | undefined;
    flushSync(() => {
      registration = kernel.registerEffect('ext:vhs', {
        id: 'vhs', label: 'VHS', group: 'Stylize', params: [], frag: 'o = texture(u_tex, v_st);'
      });
    });
    expect(secs(target)).toEqual(['Blur', 'Stylize', 'Matte']);
    expect(labels(target)).toEqual(['Gaussian Blur', 'VHS', 'Track Matte']);

    flushSync(() => registration?.dispose());
    expect(labels(target)).toEqual(['Gaussian Blur', 'Track Matte']);

    signals.dispose();
    resetKernelSignals();
    await unmount(component);
  });

  it('registers the simple panel ids, titles, and sizes', () => {
    const definitions = new Map<string, Record<string, any>>();
    registerSimplePanels({
      registerPanel(id: string, definition: Record<string, any>) {
        definitions.set(id, definition);
      }
    });

    expect([...definitions].map(([id, definition]) => [id, definition.title, definition.size])).toEqual([
      ['assets', 'Project', 200],
      ['fxbrowser', 'Effects & Presets', 240],
      ['mixer', 'Mixer', 300],
      ['workspaces', 'Workspaces', 200],
      ['takes', 'Takes', 180],
      ['notes', 'Notes', 180],
      ['easing', 'Easing', 320]
    ]);
    for (const definition of definitions.values()) {
      expect(definition.persist).toBe(true);
      expect(definition.build).toBeTypeOf('function');
    }
  });
});

it('browses registry metadata without translating unused shader definitions',async()=>{
 const PM=basePM({Kernel:{effects:{list:()=>[{id:'lazy',label:'Lazy effect',group:'Channel'}]}}});Object.defineProperty(PM,'FX',{get(){throw new Error('Unused shader translation must stay lazy');}});window.PM=PM as any;const{target,component}=mountPanel();expect(labels(target)).toEqual(['Lazy effect', 'Track Matte']);await unmount(component);
});

describe('FxBrowserPanel tree', () => {
  it('applies on double-click and folds a family shut', async () => {
    const PM = basePM();
    window.PM = PM as any;
    const { target, component } = mountPanel();
    const pick = target.querySelector<HTMLButtonElement>('.fxb-pick')!;
    flushSync(() => pick.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'add_effect', target: 'layer-1', effect: 'blur' }, { label: 'Add Gaussian Blur', origin: 'fx-browser' });
    // Applying it also lists it under Recently Used, ahead of the families.
    expect(secs(target)[0]).toBe('Recently Used');
    const blur = [...target.querySelectorAll<HTMLButtonElement>('.fxb-sec')].find((node) => node.textContent?.trim() === 'Blur')!;
    flushSync(() => blur.click());
    expect(labels(target)).not.toContain('Sharpen');
    expect(blur.getAttribute('aria-expanded')).toBe('false');
    await unmount(component);
  });

  it('applies Track Matte by matting the layer to the one above it', async () => {
    const above = { id: 'above', type: 'shape' }, layer = { id: 'layer-1', type: 'text' };
    const PM = basePM({ proj: { layers: [above, layer] }, firstSel: () => layer });
    window.PM = PM as any;
    const { target, component } = mountPanel();
    flushSync(() => target.querySelector<HTMLButtonElement>('button[aria-label="Add Track Matte"]')!.click());
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'set_layer', target: 'layer-1', patch: { matteSource: 'above' } }, { label: 'Add Track Matte', origin: 'fx-browser' });
    await unmount(component);
  });
});

describe('FxBrowserPanel My Presets', () => {
  function presetPM(initial: Array<{ id: string; name: string }>) {
    const listeners = new Map<string, Set<() => void>>();
    let stored = initial.map((preset) => ({ ...preset, effects: [{ type: 'blur', parameters: { amount: { kf: [{}] } }, enabled: true }], properties: [] }));
    const layer = { id: 'layer-1', name: 'Card', type: 'shape' };
    const PM = basePM({
      time: 2,
      firstSel: () => layer,
      L: (id: string) => (id === layer.id ? layer : null),
      menu: vi.fn(),
      confirm: vi.fn(async () => true),
      cmd: vi.fn(),
      bus: {
        on: (event: string, listener: () => void) => {
          if (!listeners.has(event)) listeners.set(event, new Set());
          listeners.get(event)!.add(listener);
          return () => listeners.get(event)!.delete(listener);
        },
        emit: (event: string) => listeners.get(event)?.forEach((listener) => listener())
      },
      Presets: {
        list: () => stored.map((preset) => ({ ...preset })),
        get: (id: string) => stored.find((preset) => preset.id === id) ?? null,
        rename: vi.fn((id: string, name: string) => { stored = stored.map((preset) => preset.id === id ? { ...preset, name: name.trim() } : preset); PM.bus.emit('presets'); return true; }),
        remove: vi.fn((id: string) => { stored = stored.filter((preset) => preset.id !== id); PM.bus.emit('presets'); return true; })
      },
      Edit: { apply: vi.fn(() => ({ ok: true, data: { results: [{ data: { skipped: [] } }] } })) }
    });
    const set = (next: Array<{ id: string; name: string }>) => { stored = next.map((preset) => ({ ...preset, effects: [], properties: [] })); PM.bus.emit('presets'); };
    return { PM, set };
  }

  it('teaches how to save one when empty, then lists presets live as another window saves', async () => {
    const { PM, set } = presetPM([]);
    window.PM = PM as any;
    const { target, component } = mountPanel();
    expect(secs(target)[0]).toBe('My Presets');
    expect(target.querySelector('[data-presets-empty]')?.textContent).toContain('Save Animation Preset…');
    flushSync(() => target.querySelector<HTMLButtonElement>('[data-presets-empty] .chip')!.click());
    expect(PM.cmd).toHaveBeenCalledWith('saveAnimationPreset');

    flushSync(() => set([{ id: 'p1', name: 'Soft entrance' }, { id: 'p2', name: 'Glow pulse' }]));
    expect(target.querySelector('[data-presets-empty]')).toBeNull();
    expect(labels(target).slice(0, 2)).toEqual(['Soft entrance', 'Glow pulse']);
    expect(target.querySelector('.fxb-count')?.textContent).toBe('2');
    expect(target.querySelector<HTMLInputElement>('.fxb-search')!.placeholder).toBe('Search effects and presets');
    await unmount(component);
  });

  it('applies by double-click or + at the playhead, and drags as a preset payload', async () => {
    const { PM } = presetPM([{ id: 'p1', name: 'Soft entrance' }]);
    window.PM = PM as any;
    const { target, component } = mountPanel();
    const row = target.querySelector<HTMLElement>('[data-kind="preset"]')!;
    flushSync(() => row.querySelector<HTMLButtonElement>('.fxb-pick')!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true })));
    expect(PM.Edit.apply).toHaveBeenCalledWith({ type: 'apply_preset', target: 'layer-1', preset: 'p1', time: 2 }, { label: 'Apply “Soft entrance”', origin: 'fx-browser' });
    expect(PM.toast).toHaveBeenLastCalledWith('Applied “Soft entrance” to Card', 2200, { error: false, key: 'preset-apply' });
    flushSync(() => row.querySelector<HTMLButtonElement>('button[aria-label="Apply Soft entrance"]')!.click());
    expect(PM.Edit.apply).toHaveBeenCalledTimes(2);

    const data = new Map<string, string>();
    const drag = new Event('dragstart', { bubbles: true }) as DragEvent;
    Object.defineProperty(drag, 'dataTransfer', { value: { setData: (type: string, value: string) => data.set(type, value), effectAllowed: '' } });
    row.querySelector('.fxb-pick')!.dispatchEvent(drag);
    expect(JSON.parse(data.get('application/x-powermove-fx')!)).toEqual({ kind: 'preset', id: 'p1', label: 'Soft entrance' });
    await unmount(component);
  });

  it('searches presets with effects, renames inline and deletes after confirming', async () => {
    const { PM } = presetPM([{ id: 'p1', name: 'Soft entrance' }, { id: 'p2', name: 'Glow pulse' }]);
    window.PM = PM as any;
    const { target, component } = mountPanel();
    flushSync(() => { fxBrowser.query = 'glow'; });
    expect(labels(target)).toEqual(['Glow pulse', 'Glow']);

    flushSync(() => { fxBrowser.query = ''; });
    const row = target.querySelector<HTMLElement>('[data-id="preset:p1"]')!;
    flushSync(() => row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 10, clientY: 10 })));
    const items = PM.menu.mock.calls[0][1];
    expect(items.filter((item: any) => item.label).map((item: any) => item.label)).toEqual(['Apply to Card', 'Rename…', 'Delete…']);

    items.find((item: any) => item.label === 'Rename…').run();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    const input = target.querySelector<HTMLInputElement>('.fxb-rename')!;
    input.value = 'Gentle entrance';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    expect(PM.Presets.rename).toHaveBeenCalledWith('p1', 'Gentle entrance');
    expect(labels(target)[0]).toBe('Gentle entrance');

    const pick = target.querySelector<HTMLButtonElement>('[data-id="preset:p2"] .fxb-pick')!;
    pick.dispatchEvent(new KeyboardEvent('keydown', { key: 'Backspace', bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    expect(PM.confirm).toHaveBeenCalledWith(expect.objectContaining({ confirmLabel: 'Delete Preset' }));
    expect(PM.Presets.remove).toHaveBeenCalledWith('p2');
    expect(labels(target)).not.toContain('Glow pulse');
    await unmount(component);
  });
});
