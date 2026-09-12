import { describe, expect, it } from 'vitest';
import { panelFocusOptions, panelFocusContext, panelScope, scopePanelIds, intersectingPanels, NATIVE_PANEL_DESIGN } from './panel-focus';

const workspace: any = { layout: { docks: [{ id: 'center', panels: [{ id: 'viewer', title: 'My canvas' }] }] }, hiddenPanels: [{ id: 'timeline', dockId: 'bottom' }] };
const panels = { viewer: { title: 'Composition' }, timeline: { title: 'Timeline' }, custom: { title: 'Custom panel' } };
describe('panel focus', () => {
  it('includes visible, hidden, and registered but undocked panels', () => {
    expect(panelFocusOptions(workspace, panels).map(p => [p.id, p.hidden])).toEqual([['viewer', false], ['timeline', true], ['custom', true]]);
  });
  it('round-trips multiple selections, removes only deleted definitions, and preserves titles', () => {
    expect(panelScope(['viewer', 'timeline', 'viewer'])).toBe('panels:viewer,timeline');
    expect(scopePanelIds('panel:viewer')).toEqual(['viewer']);
    expect(panelFocusContext('panels:viewer,missing,custom', workspace, panels)).toMatchObject({ scope: 'panels:viewer,custom', label: 'My canvas, Custom panel' });
    expect(panelFocusContext('panel:missing', workspace, panels).scope).toBe('workspace');
  });
  it('finds even narrow panels inside a box without selecting panels that only touch its edge', () => {
    const rect = { x: 0, y: 0, width: 100, height: 100 };
    expect(intersectingPanels(rect, [
      { id: 'small', rect: { x: 15, y: 0, width: 1, height: 100 } },
      { id: 'large', rect: { x: 30, y: 0, width: 60, height: 100 } },
      { id: 'edge', rect: { x: 100, y: 0, width: 50, height: 100 } }
    ])).toEqual(['large', 'small']);
  });
  it('uses native controls by default but honors explicit style requests', () => {
    expect(NATIVE_PANEL_DESIGN).toContain('api.ui.controls');
    expect(NATIVE_PANEL_DESIGN).toContain('explicit user style request overrides');
  });
});
