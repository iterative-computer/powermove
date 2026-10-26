// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { MenuContribution, PanelDefinition, PowermoveAPI, ToolService } from 'powermove';

import activate from './index';

type ToolbarMenuItem = Extract<MenuContribution, { label: string }>;

describe('toolbar', () => {
  it('shows every tool as its own button, in After Effects order, with no add button', () => {
    let panel: PanelDefinition | undefined;
    let toolService: ToolService | undefined;
    const run = vi.fn();
    const menu = vi.fn();
    const invalidate = vi.fn();
    const api = {
      services: {
        register: vi.fn((_name: string, implementation: ToolService) => {
          toolService = implementation;
          return { dispose() {} };
        })
      },
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) },
      commands: { run },
      transport: { invalidate },
      ui: { menu, icon: (name: string) => `<svg data-icon="${name}"></svg>` }
    } as unknown as PowermoveAPI;

    activate(api);
    const body = document.createElement('div');
    panel?.build?.(body, { spec: {} });

    expect(panel).toMatchObject({ id: 'toolbar', title: 'Tools', headless: true, flush: true, size: 40, noscroll: true });
    expect([...body.querySelectorAll<HTMLButtonElement>('button[data-tool]')].map((button) => button.dataset.tool))
      .toEqual(['select', 'hand', 'zoom', 'rotate', 'anchor', 'shape', 'pen', 'text']);
    expect(body.querySelectorAll('.tl-sep')).toHaveLength(3);
    // Making layers is Layer ▸ New, not a toolbar button.
    expect(body.querySelector('[aria-label="Add layer or media"]')).toBeNull();
    expect(body.querySelector('[data-tool="select"]')?.classList.contains('on')).toBe(true);

    for (const [tool, command] of [['rotate', 'toolRotate'], ['anchor', 'toolAnchor'], ['pen', 'toolPen'], ['zoom', 'toolZoom']]) {
      body.querySelector<HTMLButtonElement>(`[data-tool="${tool}"]`)?.click();
      expect(run).toHaveBeenLastCalledWith(command);
    }
    toolService?.setTool('anchor');
    expect(body.querySelector('[data-tool="anchor"]')?.getAttribute('aria-pressed')).toBe('true');
    expect(body.querySelector('[data-tool="select"]')?.getAttribute('aria-pressed')).toBe('false');
    // The viewer redraws its tool affordances on a tool change.
    expect(invalidate).toHaveBeenCalled();

    // Shape keeps its variant and lists the variants beside it.
    toolService?.setTool('shape', 'ellipse');
    body.querySelector<HTMLButtonElement>('[data-tool="shape"]')?.click();
    expect(toolService?.toolShape).toBe('ellipse');
    body.querySelector<HTMLButtonElement>('[aria-label="Shape tools"]')?.click();
    const shapes = menu.mock.lastCall![1] as ToolbarMenuItem[];
    expect(shapes.map((item) => item.label)).toEqual([
      'Rectangle Tool', 'Rounded Rectangle Tool', 'Ellipse Tool', 'Polygon Tool', 'Star Tool'
    ]);
    expect(shapes.find((item) => item.on)?.label).toBe('Ellipse Tool');
    shapes[4]!.run?.();
    expect(toolService).toMatchObject({ tool: 'shape', toolShape: 'star' });
    body.querySelector('[data-tool="shape"]')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    expect(menu).toHaveBeenCalledTimes(2);
  });
});
