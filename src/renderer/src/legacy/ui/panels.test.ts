import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './panels';

describe('legacy auxiliary panels install', () => {
  it('registers the representative panel contracts from the legacy asset and panel tests', () => {
    const panels = new Map<string, any>();
    const PM: PMRegistry = {
      h() {},
      registerPanel(id: string, definition: any) { panels.set(id, definition); },
    };

    install(PM);

    expect([...panels.keys()]).toEqual(['assets', 'fxbrowser', 'shader', 'workspaces', 'takes', 'perf', 'notes']);
    expect(panels.get('assets')).toMatchObject({ title: 'Media', size: 200 });
    expect(panels.get('shader')).toMatchObject({ title: 'Shader', noscroll: true, size: 320 });
    expect(panels.has('library')).toBe(false);
  });
});
