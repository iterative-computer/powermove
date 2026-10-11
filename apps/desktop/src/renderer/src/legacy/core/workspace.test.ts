// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install as installCapabilities } from './capabilities';
import { install } from './workspace';

function workspaceModel(saved: Record<string, any> = {}): PMRegistry {
  let nextId = 0;
  const PM: PMRegistry = {
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    round: (value: number, places = 3) => Number(value.toFixed(places)),
    uid: (prefix: string) => `${prefix}${++nextId}`,
    store: {
      get: (key: string, fallback: any) => key in saved ? saved[key] : fallback,
      set: (key: string, value: any) => { saved[key] = value; },
    },
    bus: { emit() {} },
    registerPanel() {},
    Layout: { apply() {} },
    perf: {},
    proj: { id: 'project' },
    invalidate() {},
  };
  installCapabilities(PM);
  install(PM);
  return PM;
}

describe('legacy workspace install', () => {
  it('persists accessible headless panels and preserves ordinary collapsed panels before mounting', () => {
    const saved: Record<string, any> = {};
    const PM = workspaceModel(saved);
    PM.PANELS = { timeline: { headless: true }, fxbrowser: {} };
    PM.store.separateHistory = true;
    PM.WS.init();
    PM.WS.mutate((workspace: any) => {
      for (const dock of workspace.layout.docks) {
        for (const panel of dock.panels) {
          if (['viewer', 'timeline', 'fxbrowser'].includes(panel.id)) panel.collapsed = true;
        }
      }
    });
    const panels = saved['projectWorkspace.project'].layout.docks.flatMap((dock: any) => dock.panels);
    expect(panels.find((panel: any) => panel.id === 'viewer').collapsed).toBeUndefined();
    expect(panels.find((panel: any) => panel.id === 'timeline').collapsed).toBeUndefined();
    expect(panels.find((panel: any) => panel.id === 'fxbrowser').collapsed).toBe(true);
    expect(saved['projectWorkspace.project']).toEqual(PM.WS.snapshot());
  });

  it('hands an older saved docked agent slot to Layer Effects once, at load', () => {
    const PM = workspaceModel({
      workspace: 'agent-docked',
      workspaces: [{
        id: 'agent-docked', name: 'Agent docked',
        hiddenPanels: [{ id: 'agent', dockId: 'right' }],
        layout: { docks: [
          { id: 'left', panels: [{ id: 'assets', size: 220 }, { id: 'agent', flex: true }] },
          { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
          { id: 'right', panels: [{ id: 'agent', size: 350 }] },
        ] },
      }],
    });
    PM.WS.init();
    const workspace = PM.WS.get('agent-docked');
    const ids = workspace.layout.docks.flatMap((dock: any) => dock.panels.map((panel: any) => panel.id));

    expect(ids).toEqual(['assets', 'layer-effects', 'viewer']);
    expect(workspace.hiddenPanels).toEqual([]);
    expect(workspace.layout.docks[0].panels[1]).toMatchObject({ id: 'layer-effects', flex: true });
  });

  it('adds Layer Effects below Media once to saved workspaces that lack it, respecting a hidden one', () => {
    const left = { id: 'left', panels: [{ id: 'assets', flex: true }, { id: 'takes', size: 200 }] };
    const center = { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] };
    const PM = workspaceModel({
      workspace: 'design',
      workspaces: [
        { id: 'design', name: 'Design', builtin: true, layout: { docks: [left, center] } },
        { id: 'hidden', name: 'Hidden', hiddenPanels: [{ id: 'layer-effects', dockId: 'left' }], layout: { docks: [left, center] } },
        { id: 'current', name: 'Current', agentDockMigration: 2, layout: { docks: [left, center] } },
      ],
    });
    PM.WS.init();
    const ids = (id: string) => PM.WS.get(id).layout.docks.find((dock: any) => dock.id === 'left').panels.map((panel: any) => panel.id);

    expect(ids('design')).toEqual(['assets', 'layer-effects', 'takes']);
    expect(PM.WS.get('design').layout.docks[0].panels[1]).toMatchObject({ id: 'layer-effects', size: 350 });
    expect(ids('hidden')).toEqual(['assets', 'takes']);
    expect(ids('current')).toEqual(['assets', 'takes']);
    // Closing it afterwards sticks: normalization marks the workspace as migrated.
    expect(PM.WS.normalize({ ...PM.WS.get('design'), layout: { docks: [left, center] } }).layout.docks[0].panels.map((panel: any) => panel.id)).toEqual(['assets', 'takes']);
  });

  it('moves a sized center Timeline into a bottom dock under the left column once', () => {
    const left = { id: 'left', panels: [{ id: 'assets', flex: true }, { id: 'layer-effects', size: 350 }] };
    const center = { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 320 }] };
    const right = { id: 'right', panels: [{ id: 'inspector', flex: true }] };
    const animate = { id: 'center', flex: true, panels: [{ id: 'viewer', size: 300 }, { id: 'timeline', flex: true }] };
    const PM = workspaceModel({
      workspace: 'saved',
      workspaces: [
        { id: 'saved', name: 'Saved', agentDockMigration: 2, layout: { docks: [left, center, right] } },
        { id: 'tall', name: 'Tall', agentDockMigration: 2, layout: { docks: [left, animate, right] } },
        { id: 'current', name: 'Current', agentDockMigration: 3, layout: { docks: [left, center, right] } },
      ],
    });
    PM.WS.init();
    const docks = (id: string) => PM.WS.get(id).layout.docks.map((dock: any) => [dock.id, dock.panels.map((panel: any) => panel.id)]);

    expect(docks('saved')).toEqual([
      ['left', ['assets', 'layer-effects']], ['center', ['viewer']], ['bottom', ['timeline']], ['right', ['inspector']],
    ]);
    expect(PM.WS.get('saved').layout.docks[2]).toMatchObject({ id: 'bottom', size: 320 });
    expect(docks('tall')[1]).toEqual(['center', ['viewer', 'timeline']]);
    expect(docks('current')[1]).toEqual(['center', ['viewer', 'timeline']]);
  });

  it('keeps a shared agent panel at its set height', () => {
    const PM = workspaceModel();
    const workspace = PM.WS.normalize({
      id: 'agent-height', name: 'Agent height',
      layout: { docks: [
        { id: 'left', panels: [{ id: 'assets', size: 220 }, { id: 'agent', flex: true }] },
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
      ] },
    });
    const again = PM.WS.normalize(workspace);
    const agent = again.layout.docks[0].panels.find((panel: any) => panel.id === 'agent');

    expect(agent).toMatchObject({ id: 'agent', size: 350 });
    expect(agent.flex).toBeUndefined();
    expect(again.layout.docks[0].panels.find((panel: any) => panel.id === 'assets').flex).toBe(true);
  });

  it('fills an agent-only dock across normalization without losing its saved height', () => {
    const PM = workspaceModel();
    const workspace = PM.WS.normalize({
      id: 'agent-alone', name: 'Agent alone',
      layout: { docks: [
        { id: 'left', panels: [{ id: 'agent', size: 431 }] },
        { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
      ] },
    });
    expect(PM.WS.normalize(workspace).layout.docks[0].panels).toEqual([
      { id: 'agent', size: 431, flex: true },
    ]);
  });

  it('restores saved built-in panel geometry instead of replacing it at boot', () => {
    const saved = {
      workspace: 'design',
      workspaces: [{
        id: 'design', name: 'Design', builtin: true,
        layout: { docks: [
          { id: 'left', size: 444, panels: [{ id: 'assets', flex: true }] },
          { id: 'center', flex: true, panels: [{ id: 'viewer', flex: true }] },
        ] },
      }],
    };
    const PM = workspaceModel(saved);

    PM.WS.init();

    expect(PM.WS.current.id).toBe('design');
    expect(PM.WS.current.layout.docks.find((dock: any) => dock.id === 'left')?.size).toBe(444);
  });

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
