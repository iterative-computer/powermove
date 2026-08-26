import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install as installCapabilities } from './capabilities';
import { install } from './workspace';

function workspaceModel(): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places = 3) => Number(value.toFixed(places)),
    uid: (prefix: string) => `${prefix}${++nextId}`,
    store: { get: (_key: string, fallback: any) => fallback, set() {} },
    bus: { emit() {} },
    registerPanel() {},
  };
  installCapabilities(PM);
  install(PM);
  return PM;
}

describe('legacy workspace install', () => {
  it('retains a fluid main dock in agent-authored workspaces', () => {
    const PM = workspaceModel();
    const workspace = PM.WS.normalize({
      id: 'gradient', name: 'Gradient', layout: { docks: [
        { id: 'sidebar', size: 600, panels: [{ id: 'gradient-editor', flex: true }] },
        { id: 'main', size: 680, panels: [{ id: 'viewer' }, { id: 'timeline', size: 300 }] },
      ] },
    });
    const main = workspace.layout.docks.find((item: any) => item.id === 'main');

    expect(main.flex).toBe(true);
    expect(main.size).toBe(680);
  });

  it('repairs common generated-control aliases', () => {
    const PM = workspaceModel();
    const workspace = PM.WS.normalize({
      id: 'gradient', name: 'Gradient',
      layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
      custom: [{ name: 'Gradient Editor', controls: [
        { type: 'color', parameter: 'Gradient Start', label: 'Control 1', param: 'Control 1', default: '#112233' },
        { name: 'Angle', value: 45, min: 0, max: 360 },
      ] }],
    });
    const [color, angle] = workspace.custom[0].controls;

    expect({ label: color.label, param: color.param, def: color.def }).toEqual({
      label: 'Gradient Start', param: 'Gradient Start', def: '#112233',
    });
    expect({ type: angle.type, label: angle.label, def: angle.def }).toEqual({
      type: 'slider', label: 'Angle', def: 45,
    });
  });

  it('falls back from invalid docks and preserves recoverable hidden-panel metadata', () => {
    const PM = workspaceModel();
    const fallback = { id: 'safe', name: 'Safe', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }] } };
    const repaired = PM.WS.normalize({ id: 'safe', name: 'Broken', layout: { docks: [] } }, fallback);
    const hidden = PM.WS.normalize({
      id: 'custom', name: 'Custom',
      hiddenPanels: [
        { id: 'assets', dockId: 'left', dockIndex: 0, index: 1, spec: { id: 'assets', size: 180 } },
        { id: 'library', dockId: 'left', index: 0, spec: { id: 'library' } },
      ],
      layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    });

    expect(repaired.layout.docks[0].panels[0].id).toBe('viewer');
    expect(repaired.layout.docks[0].flex).toBe(true);
    expect(hidden.hiddenPanels).toHaveLength(1);
    expect(hidden.hiddenPanels[0].spec.size).toBe(180);
  });
});
