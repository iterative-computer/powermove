import { describe, expect, it } from 'vitest';

import { dockArea, joinAreas, resolveAreaAction, splitArea, type Area } from './area';
import { findPanel, type Workspace } from './model';

const rect = (left: number, top: number, right: number, bottom: number) => ({ left, top, right, bottom });

// Right dock: three stacked panels. Center: the viewer alone.
const areas: Area[] = [
  { id: 'viewer', dockId: 'center', index: 0, rect: rect(0, 0, 600, 800) },
  { id: 'layers', dockId: 'right', index: 0, rect: rect(608, 0, 900, 300) },
  { id: 'inspector', dockId: 'right', index: 1, rect: rect(608, 308, 900, 600) },
  { id: 'library', dockId: 'right', index: 2, rect: rect(608, 608, 900, 770) }
];

const workspace = (): Workspace => ({
  hiddenPanels: [],
  layout: {
    docks: [
      { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
      { id: 'right', size: 300, panels: [{ id: 'layers', size: 300 }, { id: 'inspector', flex: true }, { id: 'library', size: 192 }] }
    ]
  }
});

describe('resolveAreaAction', () => {
  it('splits the source when the pointer stays inside it, measuring from the dragged corner', () => {
    expect(resolveAreaAction(areas, 'layers', 'top-right', 700, 120)).toEqual({ kind: 'split', source: 'layers', edge: 'top', height: 116 });
    expect(resolveAreaAction(areas, 'layers', 'bottom-left', 700, 180)).toEqual({ kind: 'split', source: 'layers', edge: 'bottom', height: 116 });
  });

  it('refuses a split that would leave either part under the minimum', () => {
    expect(resolveAreaAction(areas, 'layers', 'top-left', 700, 40)).toMatchObject({ kind: 'none', reason: 'Drag further to split' });
    expect(resolveAreaAction(areas, 'library', 'top-left', 700, 700)).toMatchObject({ kind: 'none', reason: 'Too small to split' });
  });

  it('joins with an adjacent panel in the same dock', () => {
    expect(resolveAreaAction(areas, 'layers', 'bottom-right', 700, 400)).toEqual({ kind: 'join', source: 'layers', target: 'inspector' });
    expect(resolveAreaAction(areas, 'library', 'top-right', 700, 400)).toEqual({ kind: 'join', source: 'library', target: 'inspector' });
  });

  it('docks into non-adjacent panels by vertical band', () => {
    expect(resolveAreaAction(areas, 'layers', 'bottom-left', 700, 620)).toMatchObject({ kind: 'dock', target: 'library', place: 'above' });
    expect(resolveAreaAction(areas, 'layers', 'bottom-left', 700, 704)).toMatchObject({ kind: 'dock', target: 'library', place: 'replace' });
    expect(resolveAreaAction(areas, 'layers', 'bottom-left', 700, 765)).toMatchObject({ kind: 'dock', target: 'library', place: 'below' });
  });

  it('never closes the viewer: joining is refused and replacing falls back to the nearer side', () => {
    expect(resolveAreaAction(areas, 'layers', 'top-left', 300, 300)).toMatchObject({ kind: 'dock', target: 'viewer', place: 'above' });
    expect(resolveAreaAction(areas, 'layers', 'top-left', 300, 500)).toMatchObject({ kind: 'dock', target: 'viewer', place: 'below' });
    const stacked: Area[] = [
      { id: 'viewer', dockId: 'center', index: 0, rect: rect(0, 0, 600, 500) },
      { id: 'timeline', dockId: 'center', index: 1, rect: rect(0, 508, 600, 800) }
    ];
    expect(resolveAreaAction(stacked, 'timeline', 'top-left', 300, 200)).toMatchObject({ kind: 'none', reason: 'The viewer cannot be closed' });
  });

  it('reports gutters as no action', () => {
    expect(resolveAreaAction(areas, 'layers', 'top-left', 604, 100)).toMatchObject({ kind: 'none', reason: 'Drag into a panel' });
  });
});

describe('area operations', () => {
  it('join hides the neighbour and hands its fill role to the survivor', () => {
    const ws = workspace();
    expect(joinAreas(ws, 'layers', 'inspector', { keep: 300, close: 292 })).toBe(true);
    expect(ws.layout.docks[1]!.panels.map((panel) => panel.id)).toEqual(['layers', 'library']);
    expect(findPanel(ws, 'layers')!.spec.flex).toBe(true);
    expect(findPanel(ws, 'library')!.spec.flex).toBeFalsy();
    expect(ws.hiddenPanels?.map((item) => item.id)).toEqual(['inspector']);
  });

  it('join between sized panels adds the closed height and the gap', () => {
    const ws = workspace();
    ws.layout.docks[1]!.panels = [{ id: 'layers', size: 300 }, { id: 'inspector', size: 292 }, { id: 'library', flex: true }];
    expect(joinAreas(ws, 'layers', 'inspector', { keep: 300, close: 292 })).toBe(true);
    expect(ws.layout.docks[1]!.panels).toEqual([{ id: 'layers', size: 600 }, { id: 'library', flex: true }]);
  });

  it('join refuses to close the viewer or a panel in another dock', () => {
    const ws = workspace();
    expect(joinAreas(ws, 'layers', 'viewer', { keep: 300, close: 800 })).toBe(false);
    expect(ws.layout.docks[0]!.panels).toHaveLength(1);
  });

  it('split inserts the new panel on the dragged side and shrinks a sized source', () => {
    const ws = workspace();
    ws.hiddenPanels = [{ id: 'audio' }];
    expect(splitArea(ws, 'layers', 'audio', 'top', { source: 300, part: 116 })).toBe(true);
    const panels = ws.layout.docks[1]!.panels;
    expect(panels.map((panel) => panel.id)).toEqual(['audio', 'layers', 'inspector', 'library']);
    expect(panels[0]).toMatchObject({ size: 116 });
    expect(panels[1]).toMatchObject({ size: 176 });
    expect(ws.hiddenPanels).toEqual([]);
  });

  it('split below a flex source leaves the source fluid', () => {
    const ws = workspace();
    expect(splitArea(ws, 'inspector', 'audio', 'bottom', { source: 292, part: 120 })).toBe(true);
    expect(ws.layout.docks[1]!.panels.map((panel) => panel.id)).toEqual(['layers', 'inspector', 'audio', 'library']);
    expect(findPanel(ws, 'inspector')!.spec.flex).toBe(true);
  });

  it('dock beside a panel takes half of its height', () => {
    const ws = workspace();
    expect(dockArea(ws, 'layers', 'viewer', 'below', 800)).toBe(true);
    expect(ws.layout.docks[0]!.panels).toEqual([{ id: 'viewer', flex: true }, { id: 'layers', size: 396 }]);
    expect(ws.layout.docks[1]!.panels.map((panel) => panel.id)).toEqual(['inspector', 'library']);
  });

  it('replace takes over the target slot and hides the target', () => {
    const ws = workspace();
    expect(dockArea(ws, 'library', 'layers', 'replace', 300)).toBe(true);
    expect(ws.layout.docks[1]!.panels).toEqual([{ id: 'library', size: 300 }, { id: 'inspector', flex: true }]);
    expect(ws.hiddenPanels?.[0]).toMatchObject({ id: 'layers', dockId: 'right', index: 0 });
  });

  it('refuses to replace the viewer', () => {
    const ws = workspace();
    expect(dockArea(ws, 'layers', 'viewer', 'replace', 800)).toBe(false);
    expect(findPanel(ws, 'layers')?.dock.id).toBe('right');
  });
});
