import { describe, expect, it } from 'vitest';
import { dividesFlick, nominalRate, resolveFrameRate, SUPPORTED_FRAME_RATES, validateFrameRate } from './frame-rate';

describe('the frame-rate validator', () => {
  it('snaps floats to the supported rates', () => {
    expect(resolveFrameRate(29.97)).toEqual({ num: 30000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(23.976)).toEqual({ num: 24000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(59.94)).toEqual({ num: 60000, den: 1001, source: 'ntsc' });
    expect(resolveFrameRate(25.004)).toEqual({ num: 25, den: 1, source: 'integer' });
    expect(resolveFrameRate(12.5)).toEqual({ num: 25, den: 2, source: 'approximated' });
    expect(resolveFrameRate(30, [24000, 1001])).toEqual({ num: 24000, den: 1001, source: 'exact' });
    expect(resolveFrameRate(30, [0, 1])).toEqual({ num: 30, den: 1, source: 'integer' });
    expect(resolveFrameRate(-1)).toBeNull();
    for (const [num, den] of SUPPORTED_FRAME_RATES) expect(dividesFlick({ num, den })).toBe(true);
  });

  it('stores new rates exactly and refuses rates that do not divide the flick', () => {
    expect(validateFrameRate(29.97)).toEqual({ ok: true, fps: 30000 / 1001, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate('48')).toEqual({ ok: true, fps: 48, fpsRational: [48, 1], source: 'integer' });
    expect(validateFrameRate(12)).toMatchObject({ ok: true, fps: 12, fpsRational: [12, 1] });
    expect(validateFrameRate(11)).toMatchObject({ ok: false });
    expect(validateFrameRate(0.5)).toMatchObject({ ok: false });
    expect(validateFrameRate(241)).toMatchObject({ ok: false });
    expect(validateFrameRate('abc')).toMatchObject({ ok: false });
  });

  it('keeps document rates working and matches media to nominal rates', () => {
    expect(validateFrameRate(29.97, 'document')).toEqual({ ok: true, fps: 29.97, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate(11, 'document')).toEqual({ ok: true, fps: 11, fpsRational: [11, 1], source: 'approximated' });
    expect(validateFrameRate(29.83, 'media')).toEqual({ ok: true, fps: 30000 / 1001, fpsRational: [30000, 1001], source: 'ntsc' });
    expect(validateFrameRate(24.000001, 'media')).toMatchObject({ fpsRational: [24, 1] });
    expect(nominalRate(14.99)).toEqual([24000, 1001]);
  });
});
