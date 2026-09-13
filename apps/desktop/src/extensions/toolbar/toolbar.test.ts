// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { MenuContribution, PanelDefinition, PowermoveAPI, ToolService } from 'powermove';

import activate from './index';

type ToolbarMenuItem = Extract<MenuContribution, { label: string }>;

describe('toolbar', () => {
  it('registers and builds the legacy toolbar through the extension API', () => {
    let panel: PanelDefinition | undefined;
    let toolService: ToolService | undefined;
    const run = vi.fn();
    const menu = vi.fn();
    const api = {
      services: {
        register: vi.fn((_name: string, implementation: ToolService) => {
          toolService = implementation;
          return { dispose() {} };
        })
      },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) },
      commands: { run },
      ui: { menu, icon: (name: string) => `<svg data-icon="${name}"></svg>` }
    } as unknown as PowermoveAPI;

    activate(api);
    const body = document.createElement('div');
    panel?.build?.(body, { spec: {} });

    expect(panel).toMatchObject({ id: 'toolbar', title: 'Tools', headless: true, flush: true, size: 40, noscroll: true });
    expect(body.querySelectorAll('button')).toHaveLength(8);
    expect(body.querySelectorAll('.tl-sep')).toHaveLength(1);
    expect(body.querySelector('[data-tool="select"]')?.classList.contains('on')).toBe(true);
    expect([...body.querySelectorAll<HTMLButtonElement>('button[data-tool]')].map((button) => button.dataset.tool))
      .toEqual(['select', 'hand', 'shape', 'text']);

    body.querySelector<HTMLButtonElement>('[aria-label="Selection and transform tools"]')?.click();
    const transformItems = menu.mock.lastCall![1] as ToolbarMenuItem[];
    expect(transformItems.map((item) => item.label)).toEqual([
      'Selection Tool (V)', 'Rotation Tool (W)', 'Anchor Point Tool (Y)'
    ]);
    transformItems[1]!.run?.();
    expect(run).toHaveBeenLastCalledWith('toolRotate');
    toolService?.setTool('rotate');
    expect(body.querySelector('[data-tool="rotate"]')?.getAttribute('aria-pressed')).toBe('true');

    // Shortcut changes expose the active hidden tool and remember it after switching groups.
    toolService?.setTool('zoom');
    expect(body.querySelector('[data-tool="zoom"]')?.classList.contains('on')).toBe(true);
    body.querySelector<HTMLButtonElement>('[data-tool="rotate"]')?.click();
    expect(run).toHaveBeenLastCalledWith('toolRotate');
    body.querySelector('[data-tool="zoom"]')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    expect((menu.mock.lastCall![1] as ToolbarMenuItem[]).map((item) => item.label)).toEqual(['Hand Tool (H)', 'Zoom Tool (Z)']);

    body.querySelector<HTMLButtonElement>('[aria-label="Drawing tools"]')?.click();
    (menu.mock.lastCall![1] as ToolbarMenuItem[])[1]!.run?.();
    expect(run).toHaveBeenLastCalledWith('toolPen');
    toolService?.setTool('shape', 'ellipse');
    body.querySelector<HTMLButtonElement>('[data-tool="shape"]')?.click();
    expect(toolService?.toolShape).toBe('ellipse');

    body.querySelector<HTMLButtonElement>('[aria-label="Add layer or media"]')?.click();
    const createItems = menu.mock.lastCall![1] as ToolbarMenuItem[];
    expect(createItems).toHaveLength(4);
    for (const [index, command] of ['import', 'newSolid', 'newShader', 'newNull'].entries()) {
      createItems[index]?.run?.();
      expect(run).toHaveBeenLastCalledWith(command);
    }
  });
});
