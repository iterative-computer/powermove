import { describe, expect, it } from 'vitest';

import {
  CLIP_LEVEL, FADER_MARKS, MAX_GAIN, PEAK_FALL_DB_PER_S, PEAK_HOLD_S, SILENCE_DB,
  createChannelMeter, dbToGain, faderDb, faderPosition, formatDb, gainToDb, measureBlock,
  meterPosition, meterRmsDb, meterSettled, parseDb, resetChannelMeter, stepChannelMeter
} from './levels';

describe('gain and decibels', () => {
  it('converts unity, doubling and silence', () => {
    expect(gainToDb(1)).toBe(0);
    expect(gainToDb(2)).toBeCloseTo(6.0206, 4);
    expect(gainToDb(0.5)).toBeCloseTo(-6.0206, 4);
    expect(gainToDb(0)).toBe(-Infinity);
    expect(dbToGain(0)).toBe(1);
    expect(dbToGain(-6.0206)).toBeCloseTo(0.5, 4);
    expect(dbToGain(-Infinity)).toBe(0);
  });

  it('round-trips through dB within the model range and clamps the ceiling', () => {
    for (const gain of [0.001, 0.1, 0.25, 0.7, 1, 1.5, 3.2, MAX_GAIN]) expect(dbToGain(gainToDb(gain))).toBeCloseTo(gain, 9);
    expect(dbToGain(40)).toBe(MAX_GAIN);
    expect(dbToGain(-200)).toBe(0);
    expect(dbToGain(Number.NaN)).toBe(0);
  });

  it('formats and parses readouts the way a mixer shows them', () => {
    expect(formatDb(0)).toBe('0.0');
    expect(formatDb(-0.04)).toBe('0.0');
    expect(formatDb(2.46)).toBe('+2.5');
    expect(formatDb(-3.21)).toBe('−3.2');
    expect(formatDb(-Infinity)).toBe('−∞');
    expect(parseDb('−3.5')).toBe(-3.5);
    expect(parseDb('6 dB')).toBe(6);
    expect(parseDb('-inf')).toBe(-Infinity);
    expect(parseDb('−∞')).toBe(-Infinity);
    expect(parseDb('loud')).toBeNull();
    expect(parseDb('  ')).toBeNull();
  });
});

describe('fader taper', () => {
  it('places every mark exactly and inverts', () => {
    for (const [db, position] of FADER_MARKS) {
      expect(faderPosition(db)).toBeCloseTo(position, 12);
      expect(faderDb(position)).toBeCloseTo(db, 9);
    }
    expect(faderPosition(-Infinity)).toBe(0);
    expect(faderDb(0)).toBe(-Infinity);
    expect(faderPosition(20)).toBe(1);
  });

  it('is monotonic and invertible across the whole travel', () => {
    let previous = -Infinity;
    for (let step = 1; step <= 1000; step++) {
      const position = step / 1000;
      const db = faderDb(position);
      expect(db).toBeGreaterThan(previous);
      expect(faderPosition(db)).toBeCloseTo(position, 9);
      previous = db;
    }
  });

  it('gives the range around unity most of the travel', () => {
    const nearUnity = faderPosition(6) - faderPosition(-12);
    const quiet = faderPosition(-40) - faderPosition(-100);
    expect(nearUnity).toBeGreaterThan(quiet * 2.5);
    expect(faderPosition(0)).toBeGreaterThan(0.66);
    expect(faderPosition(0)).toBeLessThan(0.8);
  });
});

describe('meter scale', () => {
  it('follows the IEC 60268-18 deflection', () => {
    expect(meterPosition(0)).toBe(1);
    expect(meterPosition(3)).toBe(1);
    expect(meterPosition(-20)).toBeCloseTo(0.5, 12);
    expect(meterPosition(-30)).toBeCloseTo(0.3, 12);
    expect(meterPosition(-40)).toBeCloseTo(0.15, 12);
    expect(meterPosition(-60)).toBeCloseTo(0.025, 12);
    expect(meterPosition(-80)).toBe(0);
    expect(meterPosition(-Infinity)).toBe(0);
    let previous = -1;
    for (let db = -70; db <= 0; db += 0.5) {
      const position = meterPosition(db);
      expect(position).toBeGreaterThanOrEqual(previous);
      previous = position;
    }
  });
});

describe('meter ballistics', () => {
  it('attacks instantly and falls at the fixed rate', () => {
    const meter = createChannelMeter();
    stepChannelMeter(meter, 0.5, 0.3, 1 / 60);
    expect(meter.peak).toBeCloseTo(gainToDb(0.5), 9);
    stepChannelMeter(meter, 0, 0, 0.1);
    expect(meter.peak).toBeCloseTo(gainToDb(0.5) - PEAK_FALL_DB_PER_S * 0.1, 9);
  });

  it('integrates RMS rather than following it', () => {
    const meter = createChannelMeter();
    stepChannelMeter(meter, 0.5, 0.5, 1 / 60);
    const first = meterRmsDb(meter);
    expect(first).toBeLessThan(gainToDb(0.5));
    for (let frame = 0; frame < 120; frame++) stepChannelMeter(meter, 0.5, 0.5, 1 / 60);
    expect(meterRmsDb(meter)).toBeCloseTo(gainToDb(0.5), 2);
    stepChannelMeter(meter, 0, 0, 1 / 60);
    expect(meterRmsDb(meter)).toBeLessThan(gainToDb(0.5));
    expect(meterRmsDb(meter)).toBeGreaterThan(gainToDb(0.5) - 1);
  });

  it('holds the peak line before letting it fall', () => {
    const meter = createChannelMeter();
    stepChannelMeter(meter, 1 / 2, 0.1, 1 / 60);
    const held = meter.hold;
    for (let frame = 0; frame < Math.floor(PEAK_HOLD_S * 60) - 2; frame++) stepChannelMeter(meter, 0.01, 0.01, 1 / 60);
    expect(meter.hold).toBe(held);
    for (let frame = 0; frame < 30; frame++) stepChannelMeter(meter, 0.01, 0.01, 1 / 60);
    expect(meter.hold).toBeLessThan(held);
    expect(meter.hold).toBeGreaterThanOrEqual(meter.peak);
  });

  it('latches the clip indicator until reset', () => {
    const meter = createChannelMeter();
    stepChannelMeter(meter, 0.9, 0.5, 1 / 60);
    expect(meter.clipped).toBe(false);
    stepChannelMeter(meter, CLIP_LEVEL, 0.5, 1 / 60);
    for (let frame = 0; frame < 600; frame++) stepChannelMeter(meter, 0, 0, 1 / 60);
    expect(meter.clipped).toBe(true);
    resetChannelMeter(meter);
    expect(meter.clipped).toBe(false);
    expect(meter.peak).toBe(SILENCE_DB);
  });

  it('settles to silence so the draw loop can stop', () => {
    const meter = createChannelMeter();
    expect(meterSettled(meter)).toBe(true);
    stepChannelMeter(meter, 1, 0.7, 1 / 60);
    expect(meterSettled(meter)).toBe(false);
    for (let frame = 0; frame < 60 * 8; frame++) stepChannelMeter(meter, 0, 0, 1 / 60);
    expect(meterSettled(meter)).toBe(true);
  });

  it('ignores a stalled frame instead of collapsing the meter', () => {
    const meter = createChannelMeter();
    stepChannelMeter(meter, 0.5, 0.4, 1 / 60);
    const before = meter.peak;
    stepChannelMeter(meter, 0, 0, 30);
    expect(meter.peak).toBeCloseTo(before - PEAK_FALL_DB_PER_S * 0.25, 9);
  });

  it('measures peak and RMS of a block', () => {
    const block = Float32Array.from({ length: 480 }, (_, index) => Math.sin(index / 480 * Math.PI * 20) * 0.5);
    const { peak, rms } = measureBlock(block);
    expect(peak).toBeCloseTo(0.5, 3);
    expect(rms).toBeCloseTo(0.5 / Math.SQRT2, 2);
    expect(measureBlock(new Float32Array(0))).toEqual({ peak: 0, rms: 0 });
  });
});
