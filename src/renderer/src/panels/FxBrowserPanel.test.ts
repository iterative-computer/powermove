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

const labels = (target: HTMLElement) => [...target.querySelectorAll('.fxb-label')].map((node) => node.textContent);
const secs = (target: HTMLElement) => [...target.querySelectorAll('.fxb-sec')].map((node) => node.textContent);

afterEach(() => {
  fxBrowser.kind = 'effect'; fxBrowser.query = ''; fxBrowser.searchOpen = false;
  document.body.replaceChildren();
  delete window.PM;
  vi.clearAllMocks();
});

describe('FxBrowserPanel', () => {
  it('groups by section and applies one add_effect through the legacy editing path', async () => {
    const PM = basePM();
    window.PM = PM as any;
    const { target, component } = mountPanel();

    expect(secs(target)).toEqual(['Blur', 'Stylize']);
    expect(labels(target)).toEqual(['Gaussian Blur', 'Sharpen', 'Glow']);

    const card = target.querySelector<HTMLButtonElement>('.fxb-row');
    expect(card?.textContent).toContain('Gaussian Blur');
    expect(card?.getAttribute('draggable')).toBe('true');
    flushSync(() => {
      card?.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
      card?.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 2 }));
    });

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
    expect(target.querySelector('.fxb-search')).toBeNull();
    flushSync(() => { fxBrowser.searchOpen = true; });
    const search = target.querySelector<HTMLInputElement>('.fxb-search')!;
    expect(document.activeElement).toBe(search);
    search.value = 'sharp';
    flushSync(() => search.dispatchEvent(new Event('input', { bubbles: true })));
    expect(labels(target)).toEqual(['Sharpen']);
    expect(secs(target)).toEqual([]);

    await unmount(component);
  });

  it('switches to transitions merged from the kernel and PM.TRANSITIONS and applies set_transition', async () => {
    const kernel = createKernel();
    const signals = installKernelSignals(kernel);
    kernel.registerTransition?.('ext:wipe', { id: 'wipe', label: 'Wipe', group: 'Wipes', params: [], frag: 'void main(){gl_FragColor=mix(texture2D(u_from,v_uv),texture2D(u_to,v_uv),u_prog);}' } as any);
    const PM = basePM({ Kernel: kernel });
    window.PM = PM as any;
    const { target, component } = mountPanel();

    flushSync(() => { fxBrowser.kind = 'transition'; });
    const shown = labels(target);
    expect(shown).toContain('Fade');

    const fade = [...target.querySelectorAll<HTMLButtonElement>('.fxb-row')].find((b) => b.textContent?.includes('Fade'))!;
    flushSync(() => fade.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 })));
    expect(PM.Edit.apply).toHaveBeenCalledWith(
      { type: 'set_transition', layer: 'layer-1', edge: 'in', transition: { type: 'fade' } },
      { label: 'Add Fade', origin: 'fx-browser' }
    );

    signals.dispose();
    resetKernelSignals();
    await unmount(component);
  });

  it('mounts the segmented control into the panel header and drives the list from it', async () => {
    const PM = basePM({ PANELS: {}, registerPanel: vi.fn(), icon: () => document.createElement('span'), pickFiles: vi.fn() });
    PM.registerPanel = (id: string, def: any) => { PM.PANELS[id] = def; };
    window.PM = PM as any;
    registerSimplePanels(PM);
    const hdr = document.createElement('header');
    hdr.append(Object.assign(document.createElement('span'), { className: 'ptitle' }));
    document.body.append(hdr);
    PM.PANELS.fxbrowser.header(hdr, {});
    const tabs = hdr.querySelectorAll<HTMLButtonElement>('[role="tab"]');
    expect(tabs).toHaveLength(2);
    expect(hdr.querySelector('.iconbtn[aria-label="Search"]')).not.toBeNull();

    const { target, component } = mountPanel();
    flushSync(() => tabs[0]!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true })));
    expect(fxBrowser.kind).toBe('transition');
    expect(tabs[1]!.classList.contains('on')).toBe(true);
    expect(labels(target)).toEqual(['Fade']);
    await unmount(component);
  });

  it('reports when no layer is selected and toasts', async () => {
    const PM = basePM({ firstSel: () => null });
    window.PM = PM as any;
    const { target, component } = mountPanel();
    const card = target.querySelector<HTMLButtonElement>('.fxb-row');
    flushSync(() => card?.click());
    expect(PM.toast).toHaveBeenCalledWith('Select a layer first');
    expect(PM.Edit.apply).not.toHaveBeenCalled();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Select a layer first');
    await unmount(component);
  });

  it('sets the drag payload on dragstart', async () => {
    window.PM = basePM({ firstSel: () => null }) as any;
    const { target, component } = mountPanel();
    const card = target.querySelector<HTMLButtonElement>('.fxb-row')!;

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
    expect(labels(target)).toEqual(['Gaussian Blur']);

    let registration: { dispose(): void } | undefined;
    flushSync(() => {
      registration = kernel.registerEffect('ext:vhs', {
        id: 'vhs', label: 'VHS', group: 'Stylize', params: [], frag: 'o = texture(u_tex, v_st);'
      });
    });
    expect(secs(target)).toEqual(['Blur', 'Stylize']);
    expect(labels(target)).toEqual(['Gaussian Blur', 'VHS']);

    flushSync(() => registration?.dispose());
    expect(labels(target)).toEqual(['Gaussian Blur']);

    signals.dispose();
    resetKernelSignals();
    await unmount(component);
  });

  it('registers the five legacy panel ids, titles, and sizes', () => {
    const definitions = new Map<string, Record<string, any>>();
    registerSimplePanels({
      registerPanel(id: string, definition: Record<string, any>) {
        definitions.set(id, definition);
      }
    });

    expect([...definitions].map(([id, definition]) => [id, definition.title, definition.size])).toEqual([
      ['assets', 'Media', 200],
      ['fxbrowser', 'Effects & Transitions', 240],
      ['workspaces', 'Workspaces', 200],
      ['takes', 'Takes', 180],
      ['notes', 'Notes', 180]
    ]);
    for (const definition of definitions.values()) {
      expect(definition.persist).toBe(true);
      expect(definition.build).toBeTypeOf('function');
    }
  });
});
