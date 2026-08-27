// @vitest-environment happy-dom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { registryView } from '../legacy/kernel-view';
import { createKernel } from '../kernel/registries';
import { installKernelSignals, resetKernelSignals } from '../kernel/signals.svelte';
import FxBrowserPanel from './FxBrowserPanel.svelte';
import { registerSimplePanels } from './register-simple';

afterEach(() => {
  document.body.replaceChildren();
  delete window.PM;
});

describe('FxBrowserPanel', () => {
  it('groups effects and applies one add_effect command through the legacy editing path', async () => {
    const apply = vi.fn();
    const refresh = vi.fn();
    const invalidate = vi.fn();
    window.PM = {
      FX: {
        blur: { group: 'Blur', label: 'Gaussian Blur' },
        glow: { group: 'Stylize', label: 'Glow' },
        sharpen: { group: 'Blur', label: 'Sharpen' }
      },
      firstSel: () => ({ id: 'layer-1' }),
      Edit: { apply },
      Inspector: { refresh },
      invalidate,
      toast: vi.fn()
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
