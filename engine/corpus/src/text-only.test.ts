import { describe, expect, test } from 'bun:test';

import { allProjects } from './projects';
import { hasText, textOnly, textOnlyProjects } from './text-only';

describe('text in isolation', () => {
  test('keeps text, captions, groups, precomps and nulls; flat, sharp, without extension effects', () => {
    const doc = {
      layers: [
        { id: 'a', type: 'image', fx: [] },
        { id: 'n', type: 'null' },
        { id: 't', type: 'text', parent: 'n', threeD: true, mblur: true, matteSource: 'a', fx: [{ type: 'typewriter' }, { type: 'glow' }] },
        { id: 'u', type: 'text', parent: 'a' },
      ],
      comps: { c: { layers: [{ id: 's', type: 'shape' }, { id: 'k', type: 'captions' }] } },
    };
    const out = textOnly(doc);
    expect(out.layers.map((l: any) => l.id)).toEqual(['n', 't', 'u']);
    expect(out.layers[1]).toMatchObject({ parent: 'n', threeD: false, mblur: false, matteSource: null, fx: [{ type: 'glow' }] });
    expect(out.layers[2].parent).toBeUndefined();
    expect(out.comps.c.layers.map((l: any) => l.id)).toEqual(['k']);
    expect(doc.layers).toHaveLength(4);
  });

  test('covers every corpus project with text', () => {
    const projects = allProjects();
    const variants = textOnlyProjects(projects);
    expect(variants.map(p => p.id)).toEqual(projects.filter(p => hasText(p.doc)).map(p => p.id));
    for (const v of variants) {
      expect(hasText(v.doc)).toBe(true);
      expect(v.extensions).toEqual([]);
    }
  });
});
