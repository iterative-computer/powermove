import { describe, expect, it } from 'vitest';
import { resolveMaterial } from '@motion-core/motion-gpu';
import {
  RIPPLE_FADE_START_SECONDS,
  RIPPLE_SETTLE_SECONDS,
  rippleFollow,
  rippleIntensity,
  rippleMaterial,
} from '../assistant/ripple-material';

describe('spatial ripple material', () => {
  it('declares the captured scene and mutable frame inputs through Motion GPU', () => {
    const resolved = resolveMaterial(rippleMaterial);
    expect(resolved.textureKeys).toEqual(['uScene']);
    expect(resolved.uniformLayout.entries.map((entry) => entry.name)).toEqual([
      'uElapsed', 'uHasScene', 'uIntensity', 'uOrigin',
    ]);
    expect(resolved.fragmentSource).toContain('fn frag(uv: vec2f) -> vec4f');
    expect(resolved.fragmentSource).toContain('textureSample(uScene, uSceneSampler');
    expect(resolved.fragmentSource).toContain('mix(ringColor, composited');
  });

  it('preserves the original settle curve', () => {
    expect(rippleIntensity(0)).toBe(1);
    expect(rippleIntensity(RIPPLE_FADE_START_SECONDS)).toBe(1);
    expect(rippleIntensity((RIPPLE_FADE_START_SECONDS + RIPPLE_SETTLE_SECONDS) / 2)).toBeCloseTo(0.5);
    expect(rippleIntensity(RIPPLE_SETTLE_SECONDS)).toBe(0);
  });

  it('tracks the live pointer without overshooting it', () => {
    const next = rippleFollow(0, 100, 1 / 60);
    expect(next).toBeGreaterThan(0);
    expect(next).toBeLessThan(100);
    expect(rippleFollow(next, 100, 0)).toBe(next);
  });
});
