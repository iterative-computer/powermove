import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { makePM } from './make-pm';

function workspaceModel(): PMRegistry {
  const PM = makePM('core/workspace');
  PM.proj = { id: 'project-1' };
  return PM;
}

function manifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'custom',
    name: 'Custom',
    layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    ...overrides,
  };
}

describe('legacy workspace library behavior survivors', () => {
  it('versions manifests and repairs a missing composition panel', () => {
    const PM = workspaceModel();
    const repaired = PM.WS.normalize(manifest({
      scope: 'project',
      projectId: 'project-1',
      layout: { docks: [{ id: 'left', panels: [{ id: 'assets' }] }] },
    }));

    expect(repaired.schemaVersion).toBe(1);
    expect(repaired.scope).toBe('project');
    expect(repaired.projectId).toBe('project-1');
    expect(repaired.layout.docks.some(
      (dock: any) => dock.panels.some((panel: any) => panel.id === 'viewer'),
    )).toBe(true);
  });

  it('migrates the old Timeline surface order without overwriting later customization', () => {
    const PM = workspaceModel();
    expect(PM.WS.normalize(manifest()).chrome.timelineSurfaceOrder).toBe('normal');
    expect(PM.WS.normalize(manifest({
      chrome: { timelineSurfaceOrder: 'reversed' },
    })).chrome.timelineSurfaceOrder).toBe('normal');
    expect(PM.WS.normalize(manifest({
      chrome: { timelineSurfaceOrder: 'reversed', timelineSurfaceSchema: 3 },
    })).chrome.timelineSurfaceOrder).toBe('reversed');
  });

  it('migrates the stock Timeline to compact defaults while preserving custom dimensions', () => {
    const PM = workspaceModel();
    const compact = PM.WS.normalize(manifest()).chrome;
    expect({ ...compact.timeline }).toEqual({
      rowHeight: 32,
      gutterWidth: 224,
      rulerHeight: 28,
      clipRadius: 5,
      keyframeSize: 8,
      showLayerNumbers: true,
      showTypeBadges: true,
      toolbarDensity: 'compact',
    });
    expect(compact.timelineChromeSchema).toBe(3);

    const customized = PM.WS.normalize(manifest({
      chrome: { timeline: { rowHeight: 34, gutterWidth: 240 } },
    })).chrome.timeline;
    expect(customized.rowHeight).toBe(34);
    expect(customized.gutterWidth).toBe(240);
    expect(customized.toolbarDensity).toBe('compact');
  });

  it('recovers a deleted custom workspace from trash', () => {
    const PM = workspaceModel();
    const custom = PM.WS.normalize(manifest());
    const design = PM.WS.normalize(manifest({ id: 'design', name: 'Design', builtin: true }));
    PM.WS.all = [design, custom];
    PM.WS.current = custom;
    PM.WS.save = () => {};
    PM.WS.activate = (id: string) => { PM.WS.current = PM.WS.get(id); };

    PM.WS.remove('custom');
    expect(PM.WS.get('custom')).toBeUndefined();
    expect(PM.WS.trashList()[0].id).toBe('custom');

    const restored = PM.WS.restore('custom');
    expect(restored.id).toBe('custom');
    expect(PM.WS.get('custom').name).toBe('Custom');
  });
});
