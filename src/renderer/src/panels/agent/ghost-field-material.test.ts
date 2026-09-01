import { describe, expect, it } from 'vitest';
import { resolveMaterial } from '@motion-core/motion-gpu';
import {
  GHOST_BREATH_SECONDS,
  GHOST_COLOR_PIPELINE,
  GHOST_CORNER_RADIUS,
  GHOST_ENTRANCE_SECONDS,
  ghostBreath,
  ghostCornerRadius,
  ghostEntrance,
  ghostGain,
  ghostMaterial,
} from './ghost-field-material';

describe('agent generation ghost field material', () => {
  it('declares only the frame inputs the field animates, with no texture sampling', () => {
    const resolved = resolveMaterial(ghostMaterial);
    expect(resolved.textureKeys).toEqual([]);
    expect(resolved.uniformLayout.entries.map(entry => entry.name)).toEqual([
      'uBreath', 'uElapsed', 'uGain', 'uIntensity', 'uRadius',
    ]);
    expect(resolved.fragmentSource).toContain('fn frag(uv: vec2f) -> vec4f');
    expect(resolved.fragmentSource).toContain('fn ghostFlow(point: vec2f, elapsed: f32) -> f32');
    expect(resolved.fragmentSource).toContain('fn ghostRoundedBox(');
  });

  it('presents synthesized light as straight alpha on an encoded sRGB canvas', () => {
    expect(GHOST_COLOR_PIPELINE).toMatchObject({
      dynamicRange: 'sdr',
      canvasColorSpace: 'srgb',
      outputEncoding: 'srgb',
    });
    expect(resolveMaterial(ghostMaterial).fragmentSource).toContain('return vec4f(color, alpha);');
  });

  it('breathes on one shared rhythm: a quicker inhale, a longer exhale', () => {
    expect(ghostBreath(0)).toBe(0);
    expect(ghostBreath(GHOST_BREATH_SECONDS * 0.38)).toBeCloseTo(1);
    expect(ghostBreath(GHOST_BREATH_SECONDS * 0.19)).toBeCloseTo(0.5);
    expect(ghostBreath(GHOST_BREATH_SECONDS)).toBe(0);
    // The exhale is the longer half, so it is still falling past the midpoint.
    expect(ghostBreath(GHOST_BREATH_SECONDS * 0.6)).toBeGreaterThan(0.4);
    expect(ghostBreath(GHOST_BREATH_SECONDS * 0.6)).toBeLessThan(ghostBreath(GHOST_BREATH_SECONDS * 0.45));
    // Cycles repeat rather than drifting over a long generation.
    expect(ghostBreath(GHOST_BREATH_SECONDS * 7.38)).toBeCloseTo(1);
  });

  it('gives the light theme the alpha it needs to be seen at all', () => {
    expect(ghostGain(true)).toBe(1);
    expect(ghostGain(false)).toBeGreaterThanOrEqual(ghostGain(true) * 2);
    expect(resolveMaterial(ghostMaterial).fragmentSource).toContain('* motiongpuUniforms.uGain');
  });

  it('fades the field up instead of snapping it on', () => {
    expect(ghostEntrance(-1)).toBe(0);
    expect(ghostEntrance(0)).toBe(0);
    expect(ghostEntrance(GHOST_ENTRANCE_SECONDS / 2)).toBeCloseTo(0.5);
    expect(ghostEntrance(GHOST_ENTRANCE_SECONDS)).toBe(1);
    expect(ghostEntrance(30)).toBe(1);
  });

  it('scales the shader corner to device pixels without inverting a short ghost', () => {
    expect(ghostCornerRadius(600, 400, 2)).toBe(GHOST_CORNER_RADIUS * 2);
    expect(ghostCornerRadius(600, 10, 2)).toBe(5);
    expect(ghostCornerRadius(0, 0, 1.5)).toBe(0);
  });
});
