import { describe, expect, it, vi } from 'vitest';

import { deletePanel, deletedPanelIds } from './panel-deletion';

function harness() {
  const active = {
    id: 'active',
    layout: { docks: [{ id: 'right', panels: [{ id: 'shader' }, { id: 'notes' }] }] },
    hiddenPanels: [{ id: 'shader' }],
    custom: [{ id: 'shader' }, { id: 'notes' }]
  };
  const other = {
    id: 'other',
    layout: { docks: [{ id: 'left', panels: [{ id: 'shader' }] }] },
    hiddenPanels: [],
    custom: []
  };
  const values = new Map<string, unknown>();
  const removePanel = vi.fn((workspace: any, id: string) => {
    workspace.layout.docks.forEach((dock: any) => {
      dock.panels = dock.panels.filter((panel: any) => panel.id !== id);
    });
  });
  const PM: any = {
    PANELS: { shader: { title: 'Shader' }, viewer: { title: 'Viewer' } },
    WS: { all: [active, other], current: active, save: vi.fn() },
    Layout: { removePanel, apply: vi.fn() },
    store: {
      get: vi.fn((key: string, fallback: unknown) => values.get(key) ?? fallback),
      set: vi.fn((key: string, value: unknown) => values.set(key, value))
    }
  };
  return { PM, active, other, removePanel };
}

describe('Library panel deletion', () => {
  it('removes a confirmed panel from the catalog and every workspace', () => {
    const { PM, active, other, removePanel } = harness();

    expect(deletePanel(PM, 'shader')).toBe(true);

    expect(PM.PANELS.shader).toBeUndefined();
    expect(active.layout.docks[0]!.panels.map((panel: any) => panel.id)).toEqual(['notes']);
    expect(other.layout.docks[0]!.panels).toEqual([]);
    expect(active.hiddenPanels).toEqual([]);
    expect(active.custom.map((panel: any) => panel.id)).toEqual(['notes']);
    expect(removePanel).toHaveBeenCalledTimes(2);
    expect(PM.WS.save).toHaveBeenCalledOnce();
    expect(PM.Layout.apply).toHaveBeenCalledWith(active);
    expect(deletedPanelIds(PM)).toEqual(new Set(['shader']));
  });

  it('protects structural panels from deletion', () => {
    const { PM } = harness();

    expect(deletePanel(PM, 'viewer')).toBe(false);
    expect(deletePanel(PM, 'toolbar')).toBe(false);
    expect(PM.WS.save).not.toHaveBeenCalled();
  });
});
