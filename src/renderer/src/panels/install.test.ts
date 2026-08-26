import { describe, expect, it, vi } from 'vitest';

import { installSveltePanels } from './install';

function registry(): Record<string, any> {
  const PM: Record<string, any> = {
    PANELS: {},
    panelInst: {},
    registerPanel(id: string, definition: Record<string, any>) {
      PM.PANELS[id] = { id, title: id, ...definition };
    },
    parseUniforms: vi.fn(() => [{ name: 'uSpeed', def: 1 }]),
    UIState: { setShaderMeta: vi.fn() },
    P: vi.fn((value: unknown) => ({ v: value })),
    selectLayers: vi.fn(),
    Layout: {
      hasPanel: vi.fn(() => true),
      addPanel: vi.fn(),
      refresh: vi.fn()
    },
    WS: {
      current: { layout: { docks: [] } },
      mutate: vi.fn(),
      registerCustom: vi.fn()
    }
  };
  installSveltePanels(PM);
  return PM;
}

describe('unconditional Svelte panel install', () => {
  it('registers every built-in panel formerly supplied by the legacy chrome engines', () => {
    const PM = registry();

    expect(Object.keys(PM.PANELS)).toEqual([
      'perf', 'assets', 'fxbrowser', 'workspaces', 'takes', 'notes',
      'inspector', 'shader', 'viewer', 'timeline'
    ]);
    expect(PM.PANELS.assets).toMatchObject({ title: 'Media', size: 200, persist: true });
    expect(PM.PANELS.inspector).toMatchObject({ title: 'Properties', persist: true });
    expect(PM.PANELS.shader).toMatchObject({ title: 'Shader', size: 320, persist: true });
  });

  it('owns the inspector and shader runtime hooks needed by the application engines', () => {
    const PM = registry();
    const layer = { id: 'shader-1', d: { code: 'uniform float uSpeed;', uniforms: { stale: { v: 2 } } } };

    PM.syncShaderUniforms(layer);

    expect(PM.Inspector.refresh).toBeTypeOf('function');
    expect(PM.fxMenu).toBeTypeOf('function');
    expect(PM.openShaderEditor).toBeTypeOf('function');
    expect(layer.d.uniforms).toEqual({ uSpeed: { v: 1 } });
    expect(PM.UIState.setShaderMeta).toHaveBeenCalledWith(layer, {
      udefs: [{ name: 'uSpeed', def: 1 }]
    });
  });
});
