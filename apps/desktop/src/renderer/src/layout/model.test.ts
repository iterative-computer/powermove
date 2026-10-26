// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { addPanel, applyPanelSize, ensureDockFill, movePanel, panelFlex, panelFloor, type DockSpec, type Workspace } from './model';

describe('set-height panels', () => {
  it('pins the agent and gives spare dock height to an eligible neighbour', () => {
    const dock: DockSpec = {
      id: 'left',
      panels: [{ id: 'assets', size: 220 }, { id: 'agent', flex: true }],
    };

    ensureDockFill(dock);

    expect(dock.panels).toEqual([
      { id: 'assets', size: 220, flex: true },
      { id: 'agent', size: 350 },
    ]);
  });

  it('fills an empty dock with the agent and restores its saved height when moved beside a neighbour', () => {
    const workspace: Workspace = {
      layout: { docks: [
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
        { id: 'right', size: 300, panels: [] },
      ] },
    };

    addPanel(workspace, 'agent', 'right');
    expect(workspace.layout.docks[1]!.panels).toEqual([{ id: 'agent', size: 350, flex: true }]);

    expect(movePanel(workspace, 'agent', 'center')).toBe(true);
    expect(workspace.layout.docks[0]!.panels.at(-1)).toEqual({ id: 'agent', size: 350 });
  });

  it('renders the agent without clearing the dock fill decision or its saved height', () => {
    const element = document.createElement('div');
    const spec = { id: 'agent', size: 420, flex: true };

    applyPanelSize(element, spec, { size: 350 });

    expect(spec).toEqual({ id: 'agent', size: 420, flex: true });
    expect(element.style.flex).toBe('1 1 auto');

    ensureDockFill({ id: 'left', panels: [{ id: 'assets' }, spec] });
    applyPanelSize(element, spec, { size: 350 });
    expect(element.style.flex).toBe('0 0 420px');
  });
});

describe('content-height panels', () => {
  it('show every row in a side column and let the column scroll', () => {
    const element = document.createElement('div');
    const fluid = { id: 'inspector', flex: true };

    applyPanelSize(element, fluid, { fitContent: true }, 'right');
    expect(element.style.flex).toBe('1 0 auto');
    // No usable-height floor: the rows are the height.
    expect(element.style.minHeight).toBe('88px');

    const sized = { id: 'inspector', size: 640 };
    applyPanelSize(element, sized, { fitContent: true }, 'right');
    expect(element.style.flex).toBe('0 0 auto');
    // A dragged height adds room below the rows; it never caps them.
    expect(element.style.minHeight).toBe('640px');
    expect(panelFloor(sized, { fitContent: true }, 'right')).toBe(640);
  });

  it('keep their ordinary sizing outside side columns', () => {
    const element = document.createElement('div');
    applyPanelSize(element, { id: 'inspector', flex: true }, { fitContent: true }, 'center');
    expect(element.style.flex).toBe('1 1 auto');
    expect(panelFlex({ id: 'inspector', size: 300 }, { fitContent: true }, 'bottom')).toBe('0 0 300px');
  });
});

describe('tab stacks', () => {
  it('stacks consecutive panels that share a group and shows the active one', async () => {
    const { dockSlots } = await import('./model');
    const slots = dockSlots([
      { id: 'assets', flex: true, group: 'project' },
      { id: 'layer-effects', group: 'project', active: true },
      { id: 'agent', size: 300 },
      { id: 'notes', group: 'project' },
    ]);
    expect(slots.map((slot) => [slot.specs.map((spec) => spec.id), slot.active.id])).toEqual([
      [['assets', 'layer-effects'], 'layer-effects'],
      [['agent'], 'agent'],
      [['notes'], 'notes'],
    ]);
  });

  it('switching tabs hands the slot size to the newly shown panel', async () => {
    const { activateTab } = await import('./model');
    const ws: any = { layout: { docks: [{ id: 'left', panels: [
      { id: 'assets', group: 'project' },
      { id: 'layer-effects', group: 'project', active: true, size: 420 },
    ] }] } };
    expect(activateTab(ws, 'assets')).toBe(true);
    expect(ws.layout.docks[0].panels).toEqual([
      { id: 'assets', group: 'project', active: true, size: 420 },
      { id: 'layer-effects', group: 'project', size: 420 },
    ]);
    expect(activateTab(ws, 'assets')).toBe(false);
  });
});
