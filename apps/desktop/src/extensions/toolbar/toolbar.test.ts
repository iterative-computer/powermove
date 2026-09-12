// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate from './index';

describe('toolbar', () => {
  it('registers and builds the legacy toolbar through the extension API', () => {
    let panel: PanelDefinition | undefined;
    let toolListener: (() => void) | undefined;
    const run = vi.fn();
    const menu = vi.fn();
    const PM = {
      bus: {
        emit: vi.fn((event: string) => event === 'tool' && toolListener?.()),
        on: vi.fn((_event: string, listener: () => void) => {
          toolListener = listener;
          return vi.fn();
        })
      }
    };
    const api = {
      host: { pm: PM },
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
    const transformItems = menu.mock.lastCall![1];
    expect(transformItems.map((item: any) => item.label)).toEqual([
      'Selection Tool (V)', 'Rotation Tool (W)', 'Anchor Point Tool (Y)'
    ]);
    transformItems[1].run();
    expect(run).toHaveBeenLastCalledWith('toolRotate');
    (PM as { setTool?: (tool: string) => void }).setTool?.('rotate');
    expect(body.querySelector('[data-tool="rotate"]')?.getAttribute('aria-pressed')).toBe('true');

    // Shortcut changes expose the active hidden tool and remember it after switching groups.
    (PM as { setTool?: (tool: string) => void }).setTool?.('zoom');
    expect(body.querySelector('[data-tool="zoom"]')?.classList.contains('on')).toBe(true);
    body.querySelector<HTMLButtonElement>('[data-tool="rotate"]')?.click();
    expect(run).toHaveBeenLastCalledWith('toolRotate');
    body.querySelector('[data-tool="zoom"]')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    expect(menu.mock.lastCall![1].map((item: any) => item.label)).toEqual(['Hand Tool (H)', 'Zoom Tool (Z)']);

    body.querySelector<HTMLButtonElement>('[aria-label="Drawing tools"]')?.click();
    menu.mock.lastCall![1][1].run();
    expect(run).toHaveBeenLastCalledWith('toolPen');
    (PM as { setTool?: (tool: string, detail?: string) => void }).setTool?.('shape', 'ellipse');
    body.querySelector<HTMLButtonElement>('[data-tool="shape"]')?.click();
    expect((PM as { toolShape?: string }).toolShape).toBe('ellipse');

    body.querySelector<HTMLButtonElement>('[aria-label="Add layer or media"]')?.click();
    const createItems = menu.mock.lastCall![1];
    expect(createItems).toHaveLength(3);
    expect(createItems.map((item: any) => item.label)).toEqual([
      'Import media (Command+I)', 'New solid (Command+Y)', 'New null object (Command+Option+Shift+Y)'
    ]);
    for (const [index, command] of ['import', 'newSolid', 'newNull'].entries()) {
      createItems[index].run();
      expect(run).toHaveBeenLastCalledWith(command);
    }
  });
});
