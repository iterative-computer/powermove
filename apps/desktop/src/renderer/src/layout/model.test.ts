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

  it('keeps the agent pinned when it is opened or moved into an empty dock', () => {
    const workspace: Workspace = {
      layout: { docks: [
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
        { id: 'right', size: 300, panels: [] },
      ] },
    };

    addPanel(workspace, 'agent', 'right');
    expect(workspace.layout.docks[1]!.panels).toEqual([{ id: 'agent', size: 350 }]);

    expect(movePanel(workspace, 'agent', 'center')).toBe(true);
    expect(workspace.layout.docks[0]!.panels.at(-1)).toEqual({ id: 'agent', size: 350 });
  });

  it('writes the set height used by the agent panel shell', () => {
    const element = document.createElement('div');
    const spec = { id: 'agent', flex: true };

    applyPanelSize(element, spec, { size: 350 });

    expect(spec).toEqual({ id: 'agent', size: 350 });
    expect(element.style.flex).toBe('0 0 350px');
    expect(element.style.getPropertyValue('--set-panel-height')).toBe('350px');
  });
});
