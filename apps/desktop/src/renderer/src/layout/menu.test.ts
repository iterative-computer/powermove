// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createKernel } from '../kernel/registries';
import { openPanelMenu } from './menu';
import type { DockSpec, PanelSpec, Workspace } from './model';

function harness(x = 10, y = 10) {
  const kernel = createKernel();
  const workspace = { layout: { docks: [{ id: 'right', panels: [{ id: 'notes' }] }] }, hiddenPanels: [] } as unknown as Workspace;
  const PM: any = {
    Kernel: kernel,
    Layout: { ws: workspace },
    PANELS: { notes: { id: 'notes', title: 'Notes' } },
    WS: { mutate: vi.fn((fn: any) => fn(workspace)) },
    closeMenus: vi.fn(),
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value))
  };
  const dock = workspace.layout.docks[0] as DockSpec;
  const spec = dock.panels[0] as PanelSpec;
  const trigger = document.createElement('button');
  document.body.append(trigger);
  const open = (): HTMLElement | null =>
    openPanelMenu(PM, new MouseEvent('contextmenu', { clientX: x, clientY: y }), spec, dock, trigger);
  return { PM, kernel, open };
}

afterEach(() => {
  document.body.replaceChildren();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('panel context menu contributions', () => {
  it('places the whole menu below or above the cursor based on available space', () => {
    vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(180);
    vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(400);
    vi.stubGlobal('innerWidth', 800);
    vi.stubGlobal('innerHeight', 600);

    const below = harness(70, 24).open()!;
    expect({ top: below.style.top, side: below.dataset.side }).toEqual({ top: '30px', side: 'below' });
    below.remove();
    const above = harness(70, 520).open()!;
    expect({ top: above.style.top, side: above.dataset.side }).toEqual({ top: '114px', side: 'above' });
  });

  it('only shows the essential built-in panel actions', () => {
    const { open } = harness();
    const menu = open()!;
    const labels = [...menu.querySelectorAll('button')].map((button) => button.textContent);

    expect(menu.querySelector('.hd')?.textContent).toBe('Notes');
    expect(labels).toEqual(['Pop out to window', 'Close panel']);
    expect(menu.textContent).not.toContain('Move to');
    expect(menu.textContent).not.toContain('Add panel');
    expect(menu.textContent).not.toContain('Restore ');
  });

  it('appends contributed items after the built-in rows, behind a separator', () => {
    const { open, kernel } = harness();
    const run = vi.fn();
    kernel.contributeMenu('ext', 'panel:context', () => [{ label: 'Duplicate panel', run }]);

    const menu = open()!;
    const buttons = [...menu.querySelectorAll('button')];

    expect(buttons.at(-1)!.textContent).toBe('Duplicate panel');
    expect(menu.lastElementChild).toBe(buttons.at(-1));
    expect(menu.children[menu.children.length - 2]!.className).toBe('sep');
    buttons.at(-1)!.click();
    expect(run).toHaveBeenCalledOnce();
  });

  it('passes the panel and dock ids as context', () => {
    const { open, kernel } = harness();
    const items = vi.fn(() => []);
    kernel.contributeMenu('ext', 'panel:context', items);

    open();

    expect(items).toHaveBeenCalledWith({ panelId: 'notes', dockId: 'right' });
  });

  it('renders contributed headers, separators and disabled rows', () => {
    const { open, kernel } = harness();
    kernel.contributeMenu('ext', 'panel:context', () => [
      { header: 'Mods' },
      '-',
      { label: 'Not yet', disabled: true, run: () => {} }
    ]);

    const menu = open()!;
    const headers = [...menu.querySelectorAll('.hd')].map((node) => node.textContent);

    expect(headers).toContain('Mods');
    const disabled = [...menu.querySelectorAll('button')].find((button) => button.textContent === 'Not yet');
    expect(disabled?.getAttribute('aria-disabled')).toBe('true');
  });
});
