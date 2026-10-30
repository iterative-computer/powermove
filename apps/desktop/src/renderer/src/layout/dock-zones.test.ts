import { describe, expect, it } from 'vitest';

import { resolvePanelZone, type ZonePanel } from './dock-zones';
import { placePanels, stackOf, type Workspace } from './model';

const panel = (id: string, top: number, stack = [id], extra: Partial<ZonePanel> = {}): ZonePanel => ({
  id,
  stack,
  rect: { left: 0, right: 300, top, bottom: top + 400 },
  header: { left: 0, right: 300, top, bottom: top + 40 },
  tabs: stack.map((member, index) => ({ id: member, left: 10 + index * 100, right: 100 + index * 100 })),
  groupable: true,
  ...extra
});

describe('After Effects drop zones', () => {
  const panels = [panel('assets', 0, ['assets', 'layer-effects']), panel('inspector', 420)];

  it('groups over the header at the pointer and over the middle at the end', () => {
    expect(resolvePanelZone(panels, 40, 20, ['inspector'], true)).toMatchObject({
      kind: 'group', placement: { kind: 'group', target: 'assets', before: 'assets' }, tab: { left: 10, top: 0, height: 40 }
    });
    expect(resolvePanelZone(panels, 150, 200, ['inspector'], true)).toMatchObject({
      kind: 'group', placement: { kind: 'group', target: 'assets', before: null }, zone: { top: 40, bottom: 400 }
    });
  });

  it('docks above or below from the bands at the top and bottom of the body', () => {
    expect(resolvePanelZone(panels, 150, 60, ['inspector'], true)).toMatchObject({
      kind: 'beside', placement: { kind: 'beside', target: 'assets', place: 'above' }, zone: { top: 0, bottom: 200 }
    });
    expect(resolvePanelZone(panels, 150, 390, ['inspector'], true)).toMatchObject({
      kind: 'beside', placement: { place: 'below' }, zone: { top: 200, bottom: 400 }
    });
  });

  it('never groups with a panel that cannot show tabs, or when grouping is off', () => {
    const viewer = [panel('viewer', 0, ['viewer'], { header: null, groupable: false })];
    expect(resolvePanelZone(viewer, 150, 180, ['inspector'], true)).toMatchObject({ kind: 'beside', placement: { place: 'above' } });
    expect(resolvePanelZone(panels, 150, 300, ['viewer'], false)).toMatchObject({ kind: 'beside', placement: { place: 'below' } });
  });

  it('reports a drop onto the moved stack itself', () => {
    expect(resolvePanelZone(panels, 150, 200, ['assets', 'layer-effects'], true)).toEqual({ kind: 'self', target: 'assets' });
    expect(resolvePanelZone(panels, 150, 200, ['layer-effects'], true)?.kind).toBe('group');
    expect(resolvePanelZone(panels, 400, 200, ['inspector'], true)).toBeNull();
  });
});

const workspace = (): Workspace => ({
  layout: {
    docks: [
      { id: 'left', panels: [{ id: 'assets', flex: true, group: 'project' }, { id: 'layer-effects', flex: true, group: 'project', active: true }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }] },
      { id: 'right', panels: [{ id: 'inspector', flex: true }, { id: 'fxbrowser', size: 268 }] }
    ]
  }
});
const ids = (ws: Workspace, dock: string) => ws.layout.docks.find((item) => item.id === dock)!.panels;

describe('placing panels', () => {
  it('a dropped panel joins the stack in front and takes over its size', () => {
    const ws = workspace();
    expect(placePanels(ws, [{ id: 'fxbrowser' }], { kind: 'group', target: 'assets' })).toBe(true);
    expect(ids(ws, 'left')).toEqual([
      { id: 'assets', flex: true, group: 'project' },
      { id: 'layer-effects', flex: true, group: 'project' },
      { id: 'fxbrowser', flex: true, group: 'project', active: true }
    ]);
    expect(ids(ws, 'right')).toEqual([{ id: 'inspector', flex: true }]);
  });

  it('two lone panels form a new stack at the tab the pointer chose', () => {
    const ws = workspace();
    placePanels(ws, [{ id: 'fxbrowser' }], { kind: 'group', target: 'inspector', before: 'inspector' });
    expect(ids(ws, 'right')).toEqual([
      { id: 'fxbrowser', flex: true, group: 'stack-inspector', active: true },
      { id: 'inspector', flex: true, group: 'stack-inspector' }
    ]);
  });

  it('dragging the shown tab out leaves the next one shown, and a stack of one stops being a stack', () => {
    const ws = workspace();
    placePanels(ws, [{ id: 'layer-effects' }], { kind: 'beside', target: 'inspector', place: 'below' });
    expect(ids(ws, 'left')).toEqual([{ id: 'assets', flex: true }]);
    expect(ids(ws, 'right').map((spec) => spec.id)).toEqual(['inspector', 'layer-effects', 'fxbrowser']);
    expect(ids(ws, 'right')[1]).toEqual({ id: 'layer-effects', flex: true });
  });

  it('moves a whole stack together, docked above another panel', () => {
    const ws = workspace();
    placePanels(ws, stackOf(ws, 'assets'), { kind: 'beside', target: 'inspector', place: 'above' });
    expect(ws.layout.docks.find((dock) => dock.id === 'left')?.panels).toEqual([]);
    expect(ids(ws, 'right').map((spec) => [spec.id, spec.group ?? null])).toEqual([
      ['assets', 'project'], ['layer-effects', 'project'], ['inspector', null], ['fxbrowser', null]
    ]);
  });

  it('reorders tabs inside a stack and refuses to drop a stack into itself', () => {
    const ws = workspace();
    placePanels(ws, [{ id: 'layer-effects' }], { kind: 'group', target: 'assets', before: 'assets' });
    expect(ids(ws, 'left').map((spec) => [spec.id, !!spec.active])).toEqual([['layer-effects', true], ['assets', false]]);
    expect(placePanels(ws, stackOf(ws, 'assets'), { kind: 'group', target: 'assets' })).toBe(false);
  });

  it('docks before a stack, never between its tabs', () => {
    const ws = workspace();
    placePanels(ws, [{ id: 'fxbrowser' }], { kind: 'dock', dockId: 'left', before: 'layer-effects' });
    expect(ids(ws, 'left').map((spec) => spec.id)).toEqual(['fxbrowser', 'assets', 'layer-effects']);
  });
});
