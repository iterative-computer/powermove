// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import { showNewLayerMenu } from './actions';

describe('inspector actions', () => {
  it('does not offer the removed shader-layer creation action', () => {
    const menu = vi.fn();
    const PM = { menu, cmd: vi.fn() };

    showNewLayerMenu(PM, document.createElement('button'));

    const items = menu.mock.calls[0]?.[1] ?? [];
    expect(items.filter((item: any) => typeof item?.label === 'string').map((item: any) => item.label))
      .toEqual(['Text', 'Shape', 'Solid', 'Null', 'Import media…']);
  });
});
