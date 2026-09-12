// @vitest-environment happy-dom
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
      'shader'
    ]);
    expect(PM.PANELS.assets).toMatchObject({ title: 'Media', size: 200, persist: true });
    expect(PM.PANELS.shader).toMatchObject({ title: 'Shader', size: 320, persist: true });
  });

  it('puts the Media import action in the panel header beside the options button', () => {
    const PM = registry();
    PM.pickFiles = vi.fn();
    PM.icon = vi.fn(() => document.createElement('svg'));
    const header = document.createElement('header');
    const options = document.createElement('button');
    options.className = 'panel-options';
    header.appendChild(options);

    PM.PANELS.assets.header(header, {});

    const action = header.querySelector<HTMLButtonElement>('button.panel-action')!;
    expect(action.getAttribute('aria-label')).toBe('Import media (⌘I)');
    expect(action.nextElementSibling).toBe(options);
    action.click();
    expect(PM.pickFiles).toHaveBeenCalledOnce();
  });

  it('owns the shader runtime hook still supplied by the legacy panel installer', () => {
    const PM = registry();
    expect(PM.openShaderEditor).toBeTypeOf('function');
    expect(PM.Inspector).toBeUndefined();
    expect(PM.syncShaderUniforms).toBeUndefined();
  });
});
