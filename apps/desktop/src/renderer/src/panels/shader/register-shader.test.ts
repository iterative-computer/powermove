// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { registerShaderPanel } from '../register-shader';

afterEach(() => {
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe('registerShaderPanel', () => {
  it('registers the legacy panel contract and installs the focus/reveal compatibility function', () => {
    const definitions = new Map<string, Record<string, any>>();
    const body = document.createElement('div');
    const textarea = document.createElement('textarea');
    textarea.dataset.shaderEditor = '';
    body.append(textarea);
    document.body.append(body);
    const workspace = { id: 'workspace-1' };
    const PM = {
      registerPanel: vi.fn((id: string, definition: Record<string, any>) => definitions.set(id, definition)),
      selectLayers: vi.fn(),
      panelInst: { shader: { body } },
      Layout: {
        hasPanel: vi.fn(() => false),
        addPanel: vi.fn(),
        refresh: vi.fn()
      },
      WS: {
        current: workspace,
        mutate: vi.fn((operation: (draft: Record<string, any>) => void) => operation(workspace))
      }
    } as Record<string, any>;

    registerShaderPanel(PM);

    expect(definitions.get('shader')).toMatchObject({
      title: 'Shader',
      noscroll: true,
      size: 320,
      persist: true
    });

    const layer = { id: 'shader-1', type: 'shader' };
    PM.openShaderEditor(layer);

    expect(PM.selectLayers).toHaveBeenCalledWith('shader-1');
    expect(PM.WS.mutate).toHaveBeenCalledOnce();
    expect(PM.Layout.addPanel).toHaveBeenCalledWith(workspace, 'shader', 'center');
    expect(PM.Layout.refresh).toHaveBeenCalledWith('shader');
    expect(document.activeElement).toBe(textarea);
  });

  it('openShaderEditor without a layer keeps selection and skips mutation when the panel exists', () => {
    const body = document.createElement('div');
    const textarea = document.createElement('textarea');
    textarea.dataset.shaderEditor = '';
    body.append(textarea);
    document.body.append(body);
    const PM = {
      registerPanel: vi.fn(),
      selectLayers: vi.fn(),
      panelInst: { shader: { body } },
      Layout: { hasPanel: vi.fn(() => true), addPanel: vi.fn(), refresh: vi.fn() },
      WS: { current: { id: 'workspace-1' }, mutate: vi.fn() }
    } as Record<string, any>;

    registerShaderPanel(PM);
    PM.openShaderEditor();

    expect(PM.selectLayers).not.toHaveBeenCalled();
    expect(PM.WS.mutate).not.toHaveBeenCalled();
    expect(PM.Layout.addPanel).not.toHaveBeenCalled();
    expect(PM.Layout.refresh).toHaveBeenCalledWith('shader');
    expect(document.activeElement).toBe(textarea);
  });
});
