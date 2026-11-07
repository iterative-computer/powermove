import { describe, expect, it } from 'vitest';

import { compTrail } from './viewer-chrome';

/* root ─┬─ A ── B
         └─ C ── B      (B is nested in both A and C) */
const users: Record<string, string[]> = { root: [], A: ['root'], C: ['root'], B: ['A', 'C'], lone: [] };
const parents = (id: string) => users[id] ?? [];

describe('compTrail', () => {
  it('builds the path up to a top-level composition', () => {
    expect(compTrail(null, 'root', parents)).toEqual({ ids: ['root'], index: 0 });
    expect(compTrail(null, 'A', parents)).toEqual({ ids: ['root', 'A'], index: 1 });
    expect(compTrail(null, 'lone', parents)).toEqual({ ids: ['lone'], index: 0 });
    expect(compTrail(null, null, parents)).toEqual({ ids: [], index: -1 });
  });

  it('follows the way you opened a composition nested in several places', () => {
    const viaC = compTrail(compTrail(null, 'C', parents), 'B', parents);
    expect(viaC).toEqual({ ids: ['root', 'C', 'B'], index: 2 });
    const viaA = compTrail(compTrail(null, 'A', parents), 'B', parents);
    expect(viaA).toEqual({ ids: ['root', 'A', 'B'], index: 2 });
  });

  it('keeps the compositions you came up from so you can step back down', () => {
    const deep = { ids: ['root', 'A', 'B'], index: 2 };
    const up = compTrail(deep, 'root', parents);
    expect(up).toEqual({ ids: ['root', 'A', 'B'], index: 0 });
    expect(compTrail(up, 'A', parents)).toEqual({ ids: ['root', 'A', 'B'], index: 1 });
    // Opening a different child of the current one replaces what was ahead.
    expect(compTrail(up, 'C', parents)).toEqual({ ids: ['root', 'C'], index: 1 });
  });

  it('cuts a trail whose nesting no longer holds and rebuilds from the open one', () => {
    const stale = { ids: ['root', 'A', 'B'], index: 0 };
    const unnested = (id: string) => (id === 'B' ? ['C'] : parents(id));
    expect(compTrail(stale, 'root', unnested)).toEqual({ ids: ['root', 'A'], index: 0 });
    const deleted = (id: string) => id !== 'A';
    expect(compTrail(stale, 'B', parents, deleted)).toEqual({ ids: ['root', 'C', 'B'], index: 2 });
  });

  it('stops on a nesting cycle', () => {
    const cyclic = (id: string) => (id === 'x' ? ['y'] : id === 'y' ? ['x'] : []);
    expect(compTrail(null, 'x', cyclic)).toEqual({ ids: ['y', 'x'], index: 1 });
  });
});
