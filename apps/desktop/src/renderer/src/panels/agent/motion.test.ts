import { describe, expect, it, vi } from 'vitest';

import { nextInBatch, scrittoEase, sweepDelay, wordRollIn } from './motion';

describe('scrittoEase', () => {
  it('is pinned at both ends', () => {
    expect(scrittoEase(0)).toBe(0);
    expect(scrittoEase(1)).toBe(1);
    expect(scrittoEase(-1)).toBe(0);
    expect(scrittoEase(2)).toBe(1);
  });

  /* The stops above 1 are Scritto's spring overshoot; losing them would flatten
     the roll into a plain ease-out, which is the whole difference. */
  it('overshoots past 1 before settling', () => {
    const peak = Math.max(...Array.from({ length: 101 }, (_, i) => scrittoEase(i / 100)));
    expect(peak).toBeGreaterThan(1);
    expect(peak).toBeLessThan(1.02);
  });

  it('rises monotonically through the first half', () => {
    for (let i = 1; i <= 50; i++) {
      expect(scrittoEase(i / 100)).toBeGreaterThan(scrittoEase((i - 1) / 100));
    }
  });
});

describe('sweepDelay', () => {
  it('starts the first word immediately', () => {
    expect(sweepDelay(0)).toBe(0);
  });

  it('offsets every word from the one before it', () => {
    for (let i = 1; i < 40; i++) {
      expect(sweepDelay(i)).toBeGreaterThan(sweepDelay(i - 1));
    }
  });

  /* A replayed 400-word message must still finish its wave, not trickle. */
  it('stays under the sweep window however long the batch', () => {
    expect(sweepDelay(400)).toBeLessThanOrEqual(0.3 * 550);
    expect(sweepDelay(400)).toBeGreaterThan(0.3 * 550 - 1);
  });
});

describe('nextInBatch', () => {
  it('numbers words within one flush and resets on the next', async () => {
    expect(nextInBatch()).toBe(0);
    expect(nextInBatch()).toBe(1);
    expect(nextInBatch()).toBe(2);
    await Promise.resolve();
    await new Promise((r) => queueMicrotask(() => r(null)));
    expect(nextInBatch()).toBe(0);
  });
});

describe('wordRollIn', () => {
  const node = { } as unknown as Element;

  it('carries Scritto\'s duration and enter transform', () => {
    const config = wordRollIn(node);
    expect(config.duration).toBe(550);
    const start = config.css(0, 1);
    expect(start).toContain('opacity:0');
    expect(start).toContain('blur(0.1em)');
    expect(start).toContain('translateY(0.35em)');
    expect(start).toContain('scale(0.6)');
    expect(start).toContain('rotateZ(2deg)');
  });

  /* An inline box takes no transform, so the roll needs inline-block — but only
     while it runs, or it changes how the settled line breaks. */
  it('blockifies for the duration and lands clean', () => {
    const config = wordRollIn(node);
    expect(config.css(0, 1)).toContain('display:inline-block');
    const end = config.css(1, 0);
    expect(end).toContain('opacity:1');
    expect(end).toContain('blur(0em)');
    expect(end).toContain('translateY(0em) scale(1) rotateZ(0deg)');
  });

  it('never drives opacity past 1 on the overshoot', () => {
    const config = wordRollIn(node);
    expect(config.css(1.015, -0.015)).toContain('opacity:1;');
  });

  it('is a hard opt-out under reduced motion', () => {
    const matchMedia = vi.fn().mockReturnValue({ matches: true });
    vi.stubGlobal('matchMedia', matchMedia);
    (globalThis as any).window = { matchMedia };
    const config = wordRollIn(node);
    expect(config.duration).toBe(0);
    expect(config.delay).toBe(0);
    expect(config.css(0, 1)).toBe('');
    vi.unstubAllGlobals();
  });
});
