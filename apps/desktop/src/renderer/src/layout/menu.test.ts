// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MenuContribution } from '../kernel/api';
import { createKernel } from '../kernel/registries';
import { openPanelMenu } from './menu';
import type { DockSpec, PanelSpec, Workspace } from './model';

function harness(panelId = 'notes', x = 10, y = 10) {
  const kernel = createKernel();
  const workspace = {
    layout: { docks: [{ id: 'right', panels: [{ id: panelId }] }] },
    hiddenPanels: []
  } as unknown as Workspace;
  const menu = vi.fn();
  const PM: any = {
    Kernel: kernel,
    Layout: { ws: workspace },
    PANELS: { [panelId]: { id: panelId, title: panelId === 'viewer' ? 'Viewer' : 'Notes' } },
    Popout: { open: vi.fn() },
    WS: { mutate: vi.fn((fn: any) => fn(workspace)) },
    menu
  };
  const dock = workspace.layout.docks[0] as DockSpec;
  const spec = dock.panels[0] as PanelSpec;
  const trigger = document.createElement('button');
  document.body.append(trigger);
  const open = (): MenuContribution[] => {
    openPanelMenu(PM, spec, dock, trigger, { x, y });
    return menu.mock.calls.at(-1)?.[1] as MenuContribution[];
  };
  return { PM, kernel, menu, open, trigger };
}

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('panel context menu contributions', () => {
  it('opens from the trigger at the requested pointer position', () => {
    const { menu, open, trigger } = harness('notes', 70, 24);

    open();

    expect(menu).toHaveBeenCalledWith(trigger, expect.any(Array), { x: 70, y: 24 });
  });

  it('only shows the essential built-in panel actions with native icons', () => {
    const { open } = harness();

    expect(open().map((item) => item === '-' ? item : 'header' in item ? item.header : item.label))
      .toEqual(['Notes', 'Pop out to window', 'Close panel']);
    expect(open()).toMatchObject([
      { header: 'Notes' },
      { label: 'Pop out to window', icon: 'export' },
      { label: 'Close panel', icon: 'x' }
    ]);
  });

  it('appends contributed items after the built-in rows, behind a separator', () => {
    const { open, kernel } = harness();
    const run = vi.fn();
    kernel.contributeMenu('ext', 'panel:context', () => [{ label: 'Duplicate panel', run }]);

    const items = open();

    expect(items.slice(-2)).toEqual(['-', { label: 'Duplicate panel', run }]);
    const duplicate = items.at(-1);
    if (duplicate !== '-' && duplicate && 'label' in duplicate) duplicate.run?.();
    expect(run).toHaveBeenCalledOnce();
  });

  it('passes the panel and dock ids as context', () => {
    const { open, kernel } = harness();
    const items = vi.fn(() => []);
    kernel.contributeMenu('ext', 'panel:context', items);

    open();

    expect(items).toHaveBeenCalledWith({ panelId: 'notes', dockId: 'right' });
  });

  it('preserves contributed headers, separators, icons, shortcuts and states', () => {
    const { open, kernel } = harness();
    const run = vi.fn();
    kernel.contributeMenu('ext', 'panel:context', () => [
      { header: 'Mods' },
      '-',
      { label: 'Not yet', icon: 'gear', kb: '⌘K', on: true, disabled: true, run }
    ]);

    expect(open().slice(-4)).toEqual([
      '-',
      { header: 'Mods' },
      '-',
      { label: 'Not yet', icon: 'gear', kb: '⌘K', on: true, disabled: true, run }
    ]);
  });

  it('does not offer Close panel for the viewer', () => {
    const { open } = harness('viewer');
    const items = open();

    expect(items).toEqual([{ header: 'Viewer' }]);
    expect(items).not.toContainEqual(expect.objectContaining({ label: 'Close panel' }));
  });
});
