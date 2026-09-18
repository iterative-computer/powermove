import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI } from 'powermove';

import activate from './index';
import { EFFECTS } from './effects';

const IDS = [
  'blur', 'motionblurDir', 'sharpen', 'glow', 'color',
  'levels', 'duotone', 'grain', 'vignette', 'chroma',
  'pixelate', 'posterize', 'displace', 'shadow', 'invert', 'gradient'
];

describe('effects-basic', () => {
  it('exports and registers all 16 raw shader effects', () => {
    const register = vi.fn();
    activate({ effects: { register } } as unknown as PowermoveAPI);

    expect(EFFECTS.map((definition) => definition.id)).toEqual(IDS);
    expect(EFFECTS.every((definition) => definition.rawShader === true)).toBe(true);
    expect(register.mock.calls.map(([definition]) => definition.id)).toEqual(IDS);
  });

  it.each(IDS)('%s ships a self-contained raw shader', (id) => {
    const definition = EFFECTS.find((effect) => effect.id === id);

    expect(definition?.rawShader).toBe(true);
    expect(definition?.frag).toMatch(/^#version 300 es/);
    expect(definition?.frag).toContain('void main()');
  });
});
