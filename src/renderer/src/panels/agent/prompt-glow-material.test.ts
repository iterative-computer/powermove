import { describe, expect, it } from 'vitest';
import {
  GLOW_PULSE_SECONDS,
  PROMPT_GLOW_PAD,
  PROMPT_GLOW_RADIUS,
  PROMPT_GLOW_TAIL_RADIUS,
  glowCornerRadius,
  glowEntrance,
  glowGain,
  glowPad,
  glowPulse,
  promptGlowMaterial,
} from './prompt-glow-material';

describe('prompt run glow', () => {
  it('keeps the halo the same CSS thickness at any canvas density', () => {
    expect(glowPad(1)).toBe(PROMPT_GLOW_PAD);
    expect(glowPad(1.5)).toBe(PROMPT_GLOW_PAD * 1.5);
    // A canvas below CSS density still gets the full inset it was laid out for.
    expect(glowPad(0.5)).toBe(PROMPT_GLOW_PAD);
    expect(promptGlowMaterial.uniforms?.uPad).toBe(PROMPT_GLOW_PAD);
  });

  it('tracks the bubble corner and never inverts it on a short prompt', () => {
    // A roomy bubble gets the bubble's own radius, scaled to device pixels.
    expect(glowCornerRadius(600, 200, 2)).toBe(PROMPT_GLOW_RADIUS * 2);
    // A one-line bubble is capped at half the height inside the padding.
    const short = glowCornerRadius(600, 2 * glowPad(1) + 10, 1);
    expect(short).toBe(5);
    // Nothing left inside the padding: a radius of zero, never a negative one.
    expect(glowCornerRadius(40, 2 * glowPad(1), 1)).toBe(0);
    expect(glowCornerRadius(10, 10, 1)).toBe(0);
  });

  it('carries the bubble tail corner, which is nearly square', () => {
    // Drawn at the body radius, the halo lifts off the one corner that is not.
    expect(PROMPT_GLOW_TAIL_RADIUS).toBeLessThan(PROMPT_GLOW_RADIUS);
    expect(glowCornerRadius(600, 200, 2, PROMPT_GLOW_TAIL_RADIUS)).toBe(PROMPT_GLOW_TAIL_RADIUS * 2);
    expect(promptGlowMaterial.uniforms?.uTailRadius).toBe(PROMPT_GLOW_TAIL_RADIUS);
  });

  it('fades the halo up from nothing and holds it at full strength', () => {
    expect(glowEntrance(0)).toBe(0);
    expect(glowEntrance(-1)).toBe(0);
    expect(glowEntrance(0.1)).toBeGreaterThan(0);
    expect(glowEntrance(0.1)).toBeLessThan(1);
    expect(glowEntrance(10)).toBe(1);
  });

  it('pulses on a repeating swell and settle', () => {
    expect(glowPulse(0)).toBe(0);
    expect(glowPulse(-5)).toBe(0);
    expect(glowPulse(GLOW_PULSE_SECONDS * 0.36)).toBeCloseTo(1, 6);
    // The settle is the longer half, so the same offset reads lower after it.
    expect(glowPulse(GLOW_PULSE_SECONDS * 0.7)).toBeLessThan(1);
    expect(glowPulse(GLOW_PULSE_SECONDS * 1.36)).toBeCloseTo(1, 6);
    expect(glowPulse(GLOW_PULSE_SECONDS * 2)).toBeCloseTo(0, 6);
  });

  it('holds the light theme back, where the halo multiplies instead of screening', () => {
    expect(glowGain(true)).toBe(1);
    expect(glowGain(false)).toBeLessThan(1);
    expect(glowGain(false)).toBeGreaterThan(0);
  });
});
