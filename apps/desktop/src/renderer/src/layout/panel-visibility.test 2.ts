import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../legacy/registry';
import { applyPanelVisibility, preferredPanelVisibility, rememberPanelVisibility } from './panel-visibility';
import { restorePanel, type Workspace } from './model';

function harness(initial: unknown = {}): { PM: PMRegistry; saved: Map<string, unknown> } {
  const saved = new Map<string, unknown>([['panelVisibility', initial]]);
  const PM = {
    store: {
      get: (key: string, fallback: unknown) => saved.has(key) ? saved.get(key) : fallback,
      set: vi.fn((key: string, value: unknown) => { saved.set(key, value); return true; })
    }
  } as PMRegistry;
  return { PM, saved };
}

function workspace(size: number): Workspace {
  return {
    layout: { docks: [{ id: 'right', size: 340, panels: [{ id: 'pexels', size }] }] },
    hiddenPanels: []
  };
}

describe('panel visibility preferences', () => {
  it('keeps an explicitly closed panel hidden when another workspace is applied', () => {
    const { PM } = harness();
    const first = workspace(210);
    const second = workspace(480);

    rememberPanelVisibility(PM, 'pexels', false);
    expect(applyPanelVisibility(PM, first)).toBe(true);
    expect(applyPanelVisibility(PM, second)).toBe(true);

    expect(first.layout.docks[0]!.panels).toEqual([]);
    expect(second.layout.docks[0]!.panels).toEqual([]);
    expect(first.hiddenPanels?.[0]?.spec).toMatchObject({ id: 'pexels', size: 210 });
    expect(second.hiddenPanels?.[0]?.spec).toMatchObject({ id: 'pexels', size: 480 });
  });

  it('clears the global dismissal on explicit open without changing other workspace geometry', () => {
    const { PM } = harness({ pexels: false });
    const current = workspace(260);
    applyPanelVisibility(PM, current);

    rememberPanelVisibility(PM, 'pexels', true);
    expect(preferredPanelVisibility(PM, 'pexels')).toBeUndefined();
    expect(restorePanel(current, 'pexels')).toBe(true);
    expect(current.layout.docks[0]!.panels[0]).toMatchObject({ id: 'pexels', size: 260 });

    const untouched = workspace(510);
    expect(applyPanelVisibility(PM, untouched)).toBe(false);
    expect(untouched.layout.docks[0]!.panels[0]).toMatchObject({ id: 'pexels', size: 510 });
  });

  it('ignores invalid saved data and never hides the viewer', () => {
    const { PM, saved } = harness({ '../escape': false, viewer: false, pexels: 'no' });
    const current: Workspace = {
      layout: { docks: [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }] },
      hiddenPanels: []
    };

    expect(applyPanelVisibility(PM, current)).toBe(false);
    rememberPanelVisibility(PM, 'viewer', false);
    expect(saved.get('panelVisibility')).toEqual({ '../escape': false, viewer: false, pexels: 'no' });
    expect(current.layout.docks[0]!.panels[0]?.id).toBe('viewer');
  });
});
