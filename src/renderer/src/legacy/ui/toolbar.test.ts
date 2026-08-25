import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './toolbar';

describe('legacy toolbar install', () => {
  it('preserves the default tool, setter event, and toolbar registration', () => {
    const emit = vi.fn();
    const panels = new Map<string, any>();
    const PM: PMRegistry = {
      bus: { emit, on() {} },
      h() {},
      registerPanel(id: string, definition: any) { panels.set(id, definition); },
    };

    install(PM);
    PM.setTool('hand');

    expect(PM.tool).toBe('hand');
    expect(emit).toHaveBeenCalledWith('tool');
    expect(panels.get('toolbar')).toMatchObject({ title: 'Tools', headless: true, flush: true, size: 40, noscroll: true });
  });
});
