// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate from './index';

describe('toolbar', () => {
  it('registers and builds the legacy toolbar through the extension API', () => {
    let panel: PanelDefinition | undefined;
    let toolListener: (() => void) | undefined;
    const run = vi.fn();
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
      ui: { icon: (name: string) => `<svg data-icon="${name}"></svg>` }
    } as unknown as PowermoveAPI;

    activate(api);
    const body = document.createElement('div');
    panel?.build?.(body, { spec: {} });

    expect(panel).toMatchObject({ id: 'toolbar', title: 'Tools', headless: true, flush: true, size: 40, noscroll: true });
    expect(body.querySelectorAll('button')).toHaveLength(11);
    expect(body.querySelectorAll('.tl-sep')).toHaveLength(2);
    expect(body.querySelector('[data-tool="select"]')?.classList.contains('on')).toBe(true);

    expect([...body.querySelectorAll<HTMLButtonElement>('button[data-tool]')].map((button) => button.dataset.tool))
      .toEqual(['select', 'hand', 'zoom', 'rotate', 'anchor', 'shape', 'text']);
    expect([...body.querySelectorAll<HTMLButtonElement>('button:not([data-tool])')].map((button) => button.title))
      .toEqual([
        'New solid (Command+Y)',
        'New shader layer (Command+Shift+G)',
        'New null object (Command+Option+Shift+Y)',
        'Import media (Command+I)'
      ]);

    body.querySelector<HTMLButtonElement>('[data-tool="hand"]')?.click();
    expect(run).toHaveBeenCalledWith('toolHand');

    (PM as { setTool?: (tool: string) => void }).setTool?.('hand');
    expect(body.querySelector('[data-tool="hand"]')?.classList.contains('on')).toBe(true);

    body.querySelector<HTMLButtonElement>('[data-tool="rotate"]')?.click();
    expect(run).toHaveBeenLastCalledWith('toolRotate');
  });
});
