// @vitest-environment happy-dom
/*
 * Kernel contributions rendered by the shell: status items in the status bar
 * and `titlebar:right` menu entries as extra buttons after the fixed three.
 */
import { flushSync, mount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createKernel } from '../kernel/registries';
import { installKernelSignals, resetKernelSignals } from '../kernel/signals.svelte';
import { frameBus } from '../runtime/frame-bus';
import StatusBar from './StatusBar.svelte';
import Titlebar from './Titlebar.svelte';

function fakePM(kernel: ReturnType<typeof createKernel>) {
  return {
    Kernel: kernel,
    ICONS: { home: '<path/>', x: '<path/>', plus: '<path/>', wand: '<path/>', grid: '<path/>', gear: '<path/>' },
    bus: { on: () => () => {}, emit() {} },
    proj: { id: 'p1', name: 'First', w: 1920, h: 1080, fps: 30, layers: [] },
    app: { dirty: false },
    Projects: { tabs: () => [], list: () => [], get: () => null },
    ProjectsScreen: { isOpen: false },
    PANELS: {},
    GL: { gl: {} },
    allProps: () => [],
    round: (value: number) => value,
    invalidate: vi.fn()
  } as any;
}

let disposeSignals: { dispose(): void } | null = null;

afterEach(() => {
  disposeSignals?.dispose();
  disposeSignals = null;
  resetKernelSignals();
  document.body.replaceChildren();
});

function render(component: any, PM: any) {
  const target = document.createElement('div');
  document.body.append(target);
  const instance = mount(component, { target, props: { PM } });
  return { target, instance };
}

describe('StatusBar kernel status items', () => {
  it('renders items on the tick and splits them by side', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    kernel.status.register('ext', { id: 'left', text: () => 'LEFT' });
    kernel.status.register('ext', { id: 'right', text: () => 'RIGHT', side: 'right' });
    const { target } = render(StatusBar, fakePM(kernel));

    const fields = [...target.querySelectorAll('.status-field')].map((node) => node.textContent);
    expect(fields).toContain('LEFT');
    expect(fields).toContain('RIGHT');
    expect(fields.indexOf('LEFT')).toBeLessThan(fields.indexOf('RIGHT'));
  });

  it('hides an item whose text() returns null', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    kernel.status.register('ext', { id: 'maybe', text: () => null });
    const { target } = render(StatusBar, fakePM(kernel));

    expect(target.querySelector('.status-contribution')).toBeNull();
  });

  it('re-evaluates text() on the status tick', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    let value = 'one';
    kernel.status.register('ext', { id: 'counter', text: () => value });
    const { target } = render(StatusBar, fakePM(kernel));
    expect(target.querySelector('.status-contribution')?.textContent).toBe('one');

    value = 'two';
    flushSync(() => frameBus.emit('status'));

    expect(target.querySelector('.status-contribution')?.textContent).toBe('two');
  });

  it('renders a clickable item as a button and calls onClick', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    const onClick = vi.fn();
    kernel.status.register('ext', { id: 'go', text: () => 'GO', title: 'Go somewhere', onClick });
    const { target } = render(StatusBar, fakePM(kernel));
    const button = target.querySelector<HTMLButtonElement>('button.status-contribution');

    expect(button?.title).toBe('Go somewhere');
    flushSync(() => button?.click());
    expect(onClick).toHaveBeenCalledOnce();
  });

  it('picks up an item registered after mount', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    const { target } = render(StatusBar, fakePM(kernel));
    expect(target.querySelector('.status-contribution')).toBeNull();

    flushSync(() => void kernel.status.register('ext', { id: 'late', text: () => 'LATE' }));

    expect(target.querySelector('.status-contribution')?.textContent).toBe('LATE');
  });
});

describe('Titlebar menu contributions', () => {
  it('renders contributed buttons after the three fixed ones', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    const run = vi.fn();
    kernel.contributeMenu('ext', 'titlebar:right', () => [{ label: 'Mods', kb: '⌘M', run }]);
    const { target } = render(Titlebar, fakePM(kernel));

    const buttons = [...target.querySelectorAll<HTMLButtonElement>('.tb-right .iconbtn')];
    expect(buttons).toHaveLength(4);
    expect(buttons[3]!.title).toBe('Mods (⌘M)');
    expect(buttons[3]!.getAttribute('aria-label')).toBe('Mods');

    flushSync(() => buttons[3]!.click());
    expect(run).toHaveBeenCalledOnce();
  });

  it('appears without a remount when a contribution arrives later', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    const { target } = render(Titlebar, fakePM(kernel));
    expect(target.querySelectorAll('.tb-right .iconbtn')).toHaveLength(3);

    flushSync(() => void kernel.contributeMenu('ext', 'titlebar:right', () => [{ label: 'Mods', run: () => {} }]));

    expect(target.querySelectorAll('.tb-right .iconbtn')).toHaveLength(4);
  });

  it('ignores separators and headers, which have no button form', () => {
    const kernel = createKernel();
    disposeSignals = installKernelSignals(kernel);
    kernel.contributeMenu('ext', 'titlebar:right', () => ['-', { header: 'Tools' }, { label: 'Mods', run: () => {} }]);
    const { target } = render(Titlebar, fakePM(kernel));

    expect(target.querySelectorAll('.tb-right .iconbtn')).toHaveLength(4);
  });
});
