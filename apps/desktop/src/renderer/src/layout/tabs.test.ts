import { describe, expect, it } from 'vitest';

import { visibleDockPlan } from './geometry';
import {
  activateTab,
  addPanel,
  cycleTab,
  dockSlots,
  hidePanel,
  insertPanel,
  moveTabBy,
  restorePanel,
  stackPanel,
  tidyTabGroups,
  unstackPanel,
  type Workspace
} from './model';
import { resolveTabDrop, type TabDropTarget } from './tabs';

function workspace(): Workspace {
  return {
    hiddenPanels: [],
    layout: { docks: [
      { id: 'left', size: 300, panels: [{ id: 'assets', flex: true }, { id: 'layer-effects', size: 350 }] },
      { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
      { id: 'right', size: 320, panels: [{ id: 'inspector', flex: true }, { id: 'fxbrowser', size: 380 }] }
    ] }
  };
}

const ids = (ws: Workspace, dockId: string): string[][] =>
  dockSlots(ws.layout.docks.find((dock) => dock.id === dockId)!.panels).map((slot) => slot.map((spec) => spec.id));
const shown = (ws: Workspace, dockId: string): string[] =>
  visibleDockPlan(ws).find((entry) => entry.dock.id === dockId)!.specs.map((spec) => spec.id);

describe('tab groups', () => {
  it('stacks a panel from another dock into a slot, keeps the slot size and shows the new tab', () => {
    const ws = workspace();

    expect(stackPanel(ws, 'layer-effects', 'fxbrowser')).toBe(true);

    expect(ids(ws, 'left')).toEqual([['assets']]);
    expect(ids(ws, 'right')).toEqual([['inspector'], ['fxbrowser', 'layer-effects']]);
    expect(shown(ws, 'right')).toEqual(['inspector', 'layer-effects']);
    const [fx, effects] = ws.layout.docks[2]!.panels.slice(1);
    expect(fx).toMatchObject({ id: 'fxbrowser', size: 380, group: 'tabs-1' });
    expect(effects).toMatchObject({ id: 'layer-effects', size: 380, group: 'tabs-1', active: true });
    expect(fx!.active).toBeUndefined();
  });

  it('switches, cycles and reorders tabs, carrying the slot size to the shown tab', () => {
    const ws = workspace();
    stackPanel(ws, 'layer-effects', 'fxbrowser');
    stackPanel(ws, 'assets', 'fxbrowser', 'fxbrowser');
    expect(ids(ws, 'right')[1]).toEqual(['assets', 'fxbrowser', 'layer-effects']);

    ws.layout.docks[2]!.panels.find((spec) => spec.id === 'assets')!.size = 260;
    expect(activateTab(ws, 'fxbrowser')).toBe(true);
    expect(activateTab(ws, 'fxbrowser')).toBe(false);
    expect(shown(ws, 'right')).toEqual(['inspector', 'fxbrowser']);
    expect(ws.layout.docks[2]!.panels.slice(1).map((spec) => spec.size)).toEqual([260, 260, 260]);

    expect(cycleTab(ws, 'fxbrowser', 1)).toBe('layer-effects');
    expect(cycleTab(ws, 'fxbrowser', 1)).toBe('assets');

    expect(moveTabBy(ws, 'assets', 1)).toBe(true);
    expect(ids(ws, 'right')[1]).toEqual(['fxbrowser', 'assets', 'layer-effects']);
    expect(stackPanel(ws, 'layer-effects', 'fxbrowser', 'fxbrowser')).toBe(true);
    expect(ids(ws, 'right')[1]).toEqual(['layer-effects', 'fxbrowser', 'assets']);
    expect(stackPanel(ws, 'fxbrowser', 'layer-effects', 'assets')).toBe(false);
  });

  it('separates a tab below its group and dissolves a group of one', () => {
    const ws = workspace();
    stackPanel(ws, 'layer-effects', 'fxbrowser');

    expect(unstackPanel(ws, 'layer-effects')).toBe(true);
    expect(ids(ws, 'right')).toEqual([['inspector'], ['fxbrowser'], ['layer-effects']]);
    expect(shown(ws, 'right')).toEqual(['inspector', 'fxbrowser', 'layer-effects']);
    expect(unstackPanel(ws, 'fxbrowser')).toBe(false);
  });

  it('closes a tab to its neighbour and restores it into its group in front', () => {
    const ws = workspace();
    stackPanel(ws, 'layer-effects', 'fxbrowser');

    expect(hidePanel(ws, 'layer-effects')).toBe(true);
    expect(shown(ws, 'right')).toEqual(['inspector', 'fxbrowser']);
    expect(restorePanel(ws, 'layer-effects')).toBe(true);
    expect(ids(ws, 'right')[1]).toEqual(['fxbrowser', 'layer-effects']);
    expect(shown(ws, 'right')).toEqual(['inspector', 'layer-effects']);

    hidePanel(ws, 'layer-effects');
    hidePanel(ws, 'fxbrowser');
    expect(ids(ws, 'right')).toEqual([['inspector']]);
  });

  it('never splits a group when a panel is inserted at an index inside it', () => {
    const ws = workspace();
    stackPanel(ws, 'layer-effects', 'inspector');
    insertPanel(ws, { id: 'notes' }, 'right', 1);
    addPanel(ws, 'perf', 'right');

    expect(ids(ws, 'right')).toEqual([['inspector', 'layer-effects'], ['notes'], ['fxbrowser'], ['perf']]);
  });

  it('keeps the viewer and unstackable panels out of groups', () => {
    const ws = workspace();
    const headless = (id: string) => id !== 'toolbar';

    expect(stackPanel(ws, 'viewer', 'inspector')).toBe(false);
    expect(stackPanel(ws, 'inspector', 'viewer')).toBe(false);
    expect(stackPanel(ws, 'toolbar', 'inspector', null, headless)).toBe(false);
  });

  it('shows the first available tab while the shown one is popped out, without changing the saved choice', () => {
    const ws = workspace();
    stackPanel(ws, 'layer-effects', 'fxbrowser');

    const plan = visibleDockPlan(ws, (id) => id === 'layer-effects');
    expect(plan.find((entry) => entry.dock.id === 'right')!.slots.map((slot) => slot.map((spec) => spec.id)))
      .toEqual([['inspector'], ['fxbrowser']]);
    expect(ws.layout.docks[2]!.panels.find((spec) => spec.id === 'layer-effects')!.active).toBe(true);
  });

  it('repairs saved groups: reused keys, the viewer, and more than one shown tab', () => {
    const panels = [
      { id: 'a', group: 'g', active: true },
      { id: 'b', group: 'g', active: true },
      { id: 'viewer', group: 'g' },
      { id: 'c', group: 'g' },
      { id: 'd', group: 'g' },
      { id: 'e', active: true }
    ];

    tidyTabGroups(panels);

    expect(dockSlots(panels).map((slot) => slot.map((spec) => spec.id))).toEqual([['a', 'b'], ['viewer'], ['c', 'd'], ['e']]);
    expect(panels).toEqual([
      { id: 'a', group: 'g', active: true },
      { id: 'b', group: 'g' },
      { id: 'viewer' },
      { id: 'c', group: 'g-2', active: true },
      { id: 'd', group: 'g-2' },
      { id: 'e' }
    ]);
  });
});

describe('tab drop targets', () => {
  const rect = (left: number, top: number, right: number, bottom: number) => ({ left, top, right, bottom });
  const target = (id: string, tabs: string[]): TabDropTarget => ({
    id,
    header: rect(0, 0, 300, 30),
    body: rect(0, 30, 300, 330),
    tabs: tabs.map((tab, index) => ({ id: tab, rect: rect(20 + index * 80, 0, 90 + index * 80, 30) }))
  });

  it('places a dragged panel between tabs in a header, or last from the middle of the panel', () => {
    const targets = [target('fxbrowser', ['fxbrowser', 'layer-effects'])];

    expect(resolveTabDrop(targets, 'assets', 30, 10)).toMatchObject({ targetId: 'fxbrowser', beforeId: 'fxbrowser', zone: 'header' });
    expect(resolveTabDrop(targets, 'assets', 120, 10)).toMatchObject({ beforeId: 'layer-effects' });
    expect(resolveTabDrop(targets, 'assets', 280, 10)).toMatchObject({ beforeId: null, zone: 'header' });
    expect(resolveTabDrop(targets, 'assets', 150, 180)).toMatchObject({ beforeId: null, zone: 'center' });
    expect(resolveTabDrop(targets, 'assets', 150, 50)).toBeNull();
  });

  it('treats a tab dropped back in place as no move, and ignores a panel over its own title', () => {
    expect(resolveTabDrop([target('fxbrowser', ['fxbrowser', 'layer-effects'])], 'fxbrowser', 70, 10)).toBe('same');
    expect(resolveTabDrop([target('fxbrowser', ['fxbrowser', 'layer-effects'])], 'fxbrowser', 150, 180)).toBe('same');
    expect(resolveTabDrop([target('fxbrowser', ['fxbrowser', 'layer-effects'])], 'fxbrowser', 280, 10))
      .toMatchObject({ beforeId: null });
    expect(resolveTabDrop([target('assets', ['assets'])], 'assets', 40, 10)).toBeNull();
  });
});
