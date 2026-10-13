import { describe, expect, it } from 'vitest';

import { precompCarriesAudio } from './audio';

describe('precompCarriesAudio', () => {
  const comps = {
    scene: { id: 'scene', layers: [{ type: 'precomp', d: { comp: 'deep' } }] },
    deep: { id: 'deep', layers: [{ type: 'video', d: { asset: 'v', embeddedAudio: true } }] },
    pictures: { id: 'pictures', layers: [{ type: 'image', d: { asset: 'i' } }, { type: 'video', d: { asset: 'v', embeddedAudio: false } }] },
    loop: { id: 'loop', layers: [{ type: 'precomp', d: { comp: 'loop' } }] },
    music: { id: 'music', layers: [{ type: 'audio', d: { asset: 'a' } }] },
  };

  it('finds sound at any depth', () => {
    expect(precompCarriesAudio('scene', comps)).toBe(true);
    expect(precompCarriesAudio('music', comps)).toBe(true);
  });

  it('is false for silent, missing and cyclic compositions', () => {
    expect(precompCarriesAudio('pictures', comps)).toBe(false);
    expect(precompCarriesAudio('missing', comps)).toBe(false);
    expect(precompCarriesAudio('loop', comps)).toBe(false);
    expect(precompCarriesAudio(null, comps)).toBe(false);
  });
});
