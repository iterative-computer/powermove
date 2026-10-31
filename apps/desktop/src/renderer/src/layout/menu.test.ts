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
    Layout: { ws: workspace, closePanel: vi.fn() },
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
  it('adds any other panel to this one as a tab', () => {
    const { open, PM } = harness();
    PM.PANELS.extra = { id: 'extra', title: 'Extra' };
    PM.PANELS.timeline = { id: 'timeline', title: 'Timeline', headless: true };
    const add = open().find((item) => item !== '-' && 'label' in item && item.label === 'Add Panel as Tab') as { submenu: Array<{ label: string; run(): void }> };
    expect(add.submenu.map((item) => item.label)).toEqual(['Extra']);
    add.submenu[0]!.run();
    expect(PM.Layout.ws.layout.docks[0].panels.map((panel: any) => [panel.id, panel.group, !!panel.active]))
      .toEqual([['notes', 'stack-notes', false], ['extra', 'stack-notes', true]]);
  });

  it('maximizes the panel, and offers to close the rest of its tab stack', () => {
    const { open, PM } = harness();
    PM.Layout.ws.layout.docks[0].panels = [
      { id: 'notes', group: 'g', active: true }, { id: 'other', group: 'g' }
    ];
    PM.Layout.toggleMaximize = vi.fn();
    const items = open().filter((item) => item !== '-' && 'label' in item) as Array<{ label: string; run?: () => unknown }>;
    items.find((item) => item.label === 'Maximize Panel')!.run!();
    expect(PM.Layout.toggleMaximize).toHaveBeenCalledWith('notes');
    items.find((item) => item.label === 'Close Other Panels in Group')!.run!();
    expect(PM.Layout.closePanel).toHaveBeenCalledWith(PM.Layout.ws, 'other');
    expect(PM.Layout.closePanel).not.toHaveBeenCalledWith(PM.Layout.ws, 'notes');
  });

  it('opens from the trigger at the requested pointer position', () => {
    const { menu, open, trigger } = harness('notes', 70, 24);

    open();

    expect(menu).toHaveBeenCalledWith(trigger, expect.any(Array), { x: 70, y: 24 });
  });

  it('only shows the essential built-in panel actions, with icons on the first group', () => {
    const { open, PM } = harness();

    expect(open().map((item) => item === '-' ? item : 'header' in item ? item.header : item.label))
      .toEqual(['Maximize Panel', 'Open in New Window', '-', 'Close Panel']);
    // The tab already names the panel, so no title; closing is a group of its own, without icons.
    expect(open()).toMatchObject([
      { label: 'Maximize Panel', icon: 'frame', kb: '`' },
      { label: 'Open in New Window', icon: 'export' },
      '-',
      { label: 'Close Panel' }
    ]);
    const close = open().find((item) => item !== '-' && 'label' in item && item.label === 'Close Panel');
    if (close && close !== '-' && 'run' in close) close.run?.();
    expect(PM.Layout.closePanel).toHaveBeenCalledWith(PM.Layout.ws, 'notes');
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

  it('waits for an asynchronous contribution asked with this panel', async () => {
    const { menu, kernel, PM, trigger } = harness();
    const run = vi.fn();
    kernel.contributeMenu('sandboxed', 'panel:context', async (ctx) => [{ label: `Tidy ${String(ctx.panelId)}`, run }]);

    openPanelMenu(PM, PM.Layout.ws.layout.docks[0].panels[0], PM.Layout.ws.layout.docks[0], trigger, { x: 1, y: 2 });
    const pending = menu.mock.calls.at(-1)?.[1] as Promise<MenuContribution[]>;

    expect(pending).toBeInstanceOf(Promise);
    expect((await pending).slice(-2)).toEqual(['-', { label: 'Tidy notes', run }]);
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

  it('does not offer Close Panel for the viewer', () => {
    const { open } = harness('viewer');
    const items = open();

    expect(items).toMatchObject([{ label: 'Maximize Panel' }]);
    expect(items).not.toContainEqual(expect.objectContaining({ label: 'Close Panel' }));
  });
});
