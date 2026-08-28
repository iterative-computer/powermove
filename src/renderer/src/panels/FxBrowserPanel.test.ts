// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { registryView } from '../legacy/kernel-view';
import { createKernel } from '../kernel/registries';
import { installKernelSignals, resetKernelSignals } from '../kernel/signals.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import { registerSimplePanels } from './register-simple';

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
  delete window.PM;
});

describe('FxBrowserPanel', () => {
  async function feedbackPanel(apply = vi.fn(() => ({ ok: true } as { ok: boolean; message?: string }))) {
    const listeners = new Map<string, () => void>();
    const toast = vi.fn();
    window.PM = {
      FX: { blur: { group: 'Blur', label: 'Gaussian Blur' }, glow: { group: 'Stylize', label: 'Glow' } },
      firstSel: () => ({ id: 'layer-1', name: 'Title' }),
      Edit: { apply }, Inspector: { refresh: vi.fn() }, invalidate: vi.fn(), toast,
      bus: { on: (event: string, callback: () => void) => { listeners.set(event, callback); return () => listeners.delete(event); } }
    } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });
    flushSync();
    const buttons = [...target.querySelectorAll<HTMLButtonElement>('.simple-effect-row')];
    return { component, target, buttons, apply, toast, listeners };
  }

  it('clears the confirmation after a short delay and restarts it on another addition', async () => {
    vi.useFakeTimers();
    const panel = await feedbackPanel();
    flushSync(() => panel.buttons[0]?.click());
    flushSync(() => vi.advanceTimersByTime(1600));
    flushSync(() => panel.buttons[1]?.click());
    expect(panel.buttons[0]?.classList.contains('effect-added')).toBe(false);
    expect(panel.buttons[1]?.classList.contains('effect-added')).toBe(true);
    flushSync(() => vi.advanceTimersByTime(1600));
    expect(panel.buttons[1]?.classList.contains('effect-added')).toBe(true);
    flushSync(() => vi.advanceTimersByTime(600));
    expect(panel.target.querySelectorAll('.effect-added')).toHaveLength(0);
    expect(panel.buttons[1]?.querySelector('.idx')?.textContent).toBe('+');
    await unmount(panel.component);
    expect(vi.getTimerCount()).toBe(0);
    expect(panel.listeners.size).toBe(0);
  });

  it.each(['sel', 'project', 'history'])('clears stale feedback on %s changes', async event => {
    const panel = await feedbackPanel();
    flushSync(() => panel.buttons[0]?.click());
    flushSync(() => panel.listeners.get(event)?.());
    expect(panel.target.querySelectorAll('.effect-added')).toHaveLength(0);
    await unmount(panel.component);
  });

  it('reports a rejected edit without claiming the effect was added', async () => {
    const panel = await feedbackPanel(vi.fn(() => ({ ok: false, message: 'Layer is locked' })));
    flushSync(() => panel.buttons[0]?.click());
    expect(panel.target.querySelectorAll('.effect-added')).toHaveLength(0);
    expect(panel.toast).toHaveBeenCalledWith('Could not add Gaussian Blur: Layer is locked');
    expect(window.PM?.Inspector.refresh).not.toHaveBeenCalled();
    await unmount(panel.component);
  });

  it('groups effects and applies one add_effect command through the legacy editing path', async () => {
    const apply = vi.fn(() => ({ ok: true }));
    const toast = vi.fn();
    const refresh = vi.fn();
    const invalidate = vi.fn();
    window.PM = {
      FX: {
        blur: { group: 'Blur', label: 'Gaussian Blur' },
        glow: { group: 'Stylize', label: 'Glow' },
        sharpen: { group: 'Blur', label: 'Sharpen' }
      },
      firstSel: () => ({ id: 'layer-1', name: 'Title' }),
      Edit: { apply },
      Inspector: { refresh },
      invalidate,
      toast
    } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });

    expect([...target.querySelectorAll('.sec')].map((section) => section.textContent)).toEqual(['Blur', 'Stylize']);
    expect([...target.querySelectorAll('.nm')].map((effect) => effect.textContent)).toEqual([
      'Gaussian Blur',
      'Sharpen',
      'Glow'
    ]);
    const button = target.querySelector<HTMLButtonElement>('.simple-effect-row');
    expect(button?.textContent).toContain('Gaussian Blur');
    flushSync(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 1 }));
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true, detail: 2 }));
    });

    expect(apply).toHaveBeenCalledOnce();
    expect(apply).toHaveBeenCalledWith(
      { type: 'add_effect', target: 'layer-1', effect: 'blur' },
      { label: 'Add Gaussian Blur', origin: 'effects-panel' }
    );
    expect(refresh).toHaveBeenCalledOnce();
    expect(invalidate).toHaveBeenCalledOnce();
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Added Gaussian Blur');
    expect(button?.classList.contains('effect-added')).toBe(true);
    expect(button?.querySelector('.effect-added-label')?.textContent).toBe('Added');
    expect(toast).toHaveBeenCalledWith('Gaussian Blur added to Title');

    await unmount(component);
  });

  it('keeps the named button operable and reports when no layer is selected', async () => {
    const toast = vi.fn();
    window.PM = {
      FX: { blur: { group: 'Blur', label: 'Gaussian Blur' } },
      firstSel: () => null,
      Edit: { apply: vi.fn() },
      Inspector: { refresh: vi.fn() },
      invalidate: vi.fn(),
      toast
    } as any;
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });
    const button = target.querySelector<HTMLButtonElement>('button');

    expect(button?.textContent).toContain('Gaussian Blur');
    flushSync(() => button?.click());
    expect(toast).toHaveBeenCalledWith('Select a layer first');
    expect(target.querySelector('[role="status"]')?.textContent).toBe('Select a layer first');

    await unmount(component);
  });

  it('shows an effect registered by an extension without remounting', async () => {
    const kernel = createKernel();
    const signals = installKernelSignals(kernel);
    window.PM = {
      Kernel: kernel,
      FX: registryView(kernel.effects, { read: (item: any) => item }),
      firstSel: () => null,
      Edit: { apply: vi.fn() },
      Inspector: { refresh: vi.fn() },
      invalidate: vi.fn(),
      toast: vi.fn()
    } as any;
    kernel.registerEffect('legacy', {
      id: 'blur', label: 'Gaussian Blur', group: 'Blur', params: [], frag: 'o = texture(u_tex, v_st);'
    });
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });
    expect([...target.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Gaussian Blur']);

    flushSync(() => void kernel.registerEffect('ext:vhs', {
      id: 'vhs', label: 'VHS', group: 'Stylize', params: [], frag: 'o = texture(u_tex, v_st);'
    }));

    expect([...target.querySelectorAll('.sec')].map((node) => node.textContent)).toEqual(['Blur', 'Stylize']);
    expect([...target.querySelectorAll('.nm')].map((node) => node.textContent)).toEqual(['Gaussian Blur', 'VHS']);

    signals.dispose();
    resetKernelSignals();
    await unmount(component);
  });

  it('drops an effect from the browser when its extension unregisters it', async () => {
    const kernel = createKernel();
    const signals = installKernelSignals(kernel);
    window.PM = {
      Kernel: kernel,
      FX: registryView(kernel.effects, { read: (item: any) => item }),
      firstSel: () => null,
      Edit: { apply: vi.fn() },
      Inspector: { refresh: vi.fn() },
      invalidate: vi.fn(),
      toast: vi.fn()
    } as any;
    const registration = kernel.registerEffect('ext:vhs', {
      id: 'vhs', label: 'VHS', group: 'Stylize', params: [], frag: 'o = texture(u_tex, v_st);'
    });
    const target = document.createElement('div');
    document.body.append(target);
    const component = mount(FxBrowserPanel, { target, props: { panelId: 'fxbrowser', spec: {} } });
    expect(target.querySelectorAll('.nm')).toHaveLength(1);

    flushSync(() => registration.dispose());

    expect(target.querySelectorAll('.nm')).toHaveLength(0);

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
      ['fxbrowser', 'Effects', 220],
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
