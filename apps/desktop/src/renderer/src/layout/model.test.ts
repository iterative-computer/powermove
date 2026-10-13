// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import { addPanel, applyPanelSize, ensureDockFill, movePanel, type DockSpec, type Workspace } from './model';

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
