import { describe, expect, it } from 'vitest';

import {
  dropFrameRate,
  dropFrameTimecode,
  formatDropFrameTimecode,
  formatTimecode,
  parseDropFrameTimecode,
  parseTimecode,
  timecodeBase,
} from './timecode';

/* PM.tc and PM.parseTc exactly as they shipped before the fractional-rate fix
   (util.ts at d5af28a1). Integer rates must keep these results. */
const legacyTc = (sec: any, fps: any = 30, showFrames = true) => {
  const neg = sec < 0; sec = Math.abs(sec);
  let f = Math.round(sec * fps);
  const ff = f % fps; f = Math.floor(f / fps);
  const ss = f % 60; const mm = Math.floor(f / 60) % 60; const hh = Math.floor(f / 3600);
  const p = (n: any, w = 2) => String(n).padStart(w, '0');
  const base = (hh ? p(hh) + ':' : '') + p(mm) + ':' + p(ss);
  return (neg ? '-' : '') + base + (showFrames ? ':' + p(ff) : '');
};
const legacyParseTc = (str: any, fps: any = 30) => {
  const parts: any = String(str).trim().split(':').map(Number);
  if (parts.some(isNaN)) return null;
  let s = 0;
  if (parts.length === 4) s = parts[0] * 3600 + parts[1] * 60 + parts[2] + parts[3] / fps;
  else if (parts.length === 3) s = parts[0] * 60 + parts[1] + parts[2] / fps;
  else if (parts.length === 2) s = parts[0] + parts[1] / fps;
  else s = parts[0];
  return s;
};

/* Deterministic pseudo-random sequence so failures reproduce. */
function sequence(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (state * 1664525 + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

const INTEGER_RATES = [1, 8, 10, 12, 15, 24, 25, 30, 48, 50, 60, 90, 100, 120, 240];

describe('formatTimecode at integer rates', () => {
  it('keeps the exact historical format', () => {
    expect(formatTimecode(0, 30)).toBe('00:00:00');
    expect(formatTimecode(0, 30, false)).toBe('00:00');
    expect(formatTimecode(1.5, 30)).toBe('00:01:15');
    expect(formatTimecode(59 + 29 / 30, 30)).toBe('00:59:29');
    expect(formatTimecode(61, 24)).toBe('01:01:00');
    expect(formatTimecode(3600, 25)).toBe('01:00:00:00');
    expect(formatTimecode(3723 + 12 / 25, 25)).toBe('01:02:03:12');
    expect(formatTimecode(-2.5, 24)).toBe('-00:02:12');
    expect(formatTimecode(90, 60, false)).toBe('01:30');
    expect(formatTimecode(36000 * 3, 30)).toBe('30:00:00:00');
  });

  it('matches the legacy implementation for every integer rate', () => {
    const random = sequence(0x5eed);
    for (const fps of INTEGER_RATES) {
      for (let i = 0; i < 4000; i++) {
        const frame = Math.floor((random() - 0.25) * 4 * 10 ** (1 + (i % 7)));
        const jitter = (random() - 0.5) * 0.999;
        const seconds = (frame + (i % 3 === 0 ? jitter : 0)) / fps;
        for (const showFrames of [true, false]) {
          expect(formatTimecode(seconds, fps, showFrames)).toBe(legacyTc(seconds, fps, showFrames));
        }
      }
    }
  });

  it('defaults to 30 fps like before', () => {
    expect(formatTimecode(10.5)).toBe(legacyTc(10.5));
    expect(formatTimecode(10.5, undefined)).toBe(legacyTc(10.5, undefined));
  });
});

describe('formatTimecode at fractional rates', () => {
  it('counts 29.97 non-drop frames with a base of 30', () => {
    expect(timecodeBase(29.97)).toBe(30);
    expect(formatTimecode(29 / 29.97, 29.97)).toBe('00:00:29');
    expect(formatTimecode(30 / 29.97, 29.97)).toBe('00:01:00');
    expect(formatTimecode(1799 / 29.97, 29.97)).toBe('00:59:29');
    expect(formatTimecode(1800 / 29.97, 29.97)).toBe('01:00:00');
    expect(formatTimecode(17982 / 29.97, 29.97)).toBe('09:59:12');
    expect(formatTimecode(108000 / 29.97, 29.97)).toBe('01:00:00:00');
    // One wall-clock hour is 107,892 frames at 29.97: the non-drop label lags.
    expect(formatTimecode(3600, 29.97)).toBe('59:56:12');
    expect(formatTimecode(3600, 30000 / 1001)).toBe('59:56:12');
  });

  it('counts 23.976 non-drop frames with a base of 24', () => {
    expect(timecodeBase(23.976)).toBe(24);
    expect(formatTimecode(23 / 23.976, 23.976)).toBe('00:00:23');
    expect(formatTimecode(24 / 23.976, 23.976)).toBe('00:01:00');
    expect(formatTimecode(86400 / 23.976, 23.976)).toBe('01:00:00:00');
    expect(formatTimecode(86400 / (24000 / 1001), 24000 / 1001)).toBe('01:00:00:00');
    expect(formatTimecode(3600, 23.976)).toBe('59:56:10');
    expect(formatTimecode(-1 / 23.976, 23.976, true)).toBe('-00:00:01');
  });

  it('never shows a fractional or out-of-range frame field', () => {
    const random = sequence(42);
    for (const fps of [23.976, 29.97, 47.952, 59.94, 119.88, 239.76, 24000 / 1001, 30000 / 1001, 12.5]) {
      const base = timecodeBase(fps);
      for (let i = 0; i < 2000; i++) {
        const seconds = random() * 86400;
        const label = formatTimecode(seconds, fps);
        const ff = Number(label.split(':').at(-1));
        expect(Number.isInteger(ff)).toBe(true);
        expect(ff).toBeGreaterThanOrEqual(0);
        expect(ff).toBeLessThan(base);
      }
    }
  });

  it('labels every frame once, in order, across ten minutes at 29.97', () => {
    let previous = '';
    for (let frame = 0; frame < 17982; frame++) {
      const label = formatTimecode(frame / 29.97, 29.97);
      expect(label > previous).toBe(true);
      expect(parseTimecode(label, 29.97)! * 29.97).toBeCloseTo(frame, 6);
      previous = label;
    }
  });

  it('falls back to 30 fps for unusable rates instead of printing NaN', () => {
    expect(formatTimecode(1, 0)).toBe('00:01:00');
    expect(formatTimecode(1, Number.NaN)).toBe('00:01:00');
    expect(formatTimecode(1, -24)).toBe('00:01:00');
  });
});

describe('parseTimecode', () => {
  it('matches the legacy parser at integer rates', () => {
    const inputs = ['', '12', '1.5', '01:15', '00:01:15', '01:02:03:12', '1:2:3:4:5', 'x', '10:aa', ' 00:00:10:05 '];
    for (const fps of [24, 25, 30, 60]) {
      for (const input of inputs) expect(parseTimecode(input, fps)).toBe(legacyParseTc(input, fps));
    }
    expect(parseTimecode('00:01:15')).toBe(legacyParseTc('00:01:15'));
  });

  it('inverts formatTimecode at fractional rates', () => {
    expect(parseTimecode('00:10:00:00', 29.97)).toBeCloseTo(18000 / 29.97, 12);
    expect(parseTimecode('01:00:00:00', 23.976)).toBeCloseTo(86400 / 23.976, 12);
    expect(parseTimecode('00:01:00', 23.976)).toBeCloseTo(24 / 23.976, 12);
    expect(parseTimecode('5', 29.97)).toBe(5);
    expect(parseTimecode('aa:00', 29.97)).toBeNull();
  });
});

describe('drop-frame timecode', () => {
  it('recognises only 29.97 and 59.94', () => {
    expect(dropFrameRate(29.97)).toEqual({ base: 30, drop: 2 });
    expect(dropFrameRate(30000 / 1001)).toEqual({ base: 30, drop: 2 });
    expect(dropFrameRate(59.94)).toEqual({ base: 60, drop: 4 });
    for (const fps of [23.976, 24, 25, 30, 47.952, 60, 119.88, 0, Number.NaN]) expect(dropFrameRate(fps)).toBeNull();
    expect(() => formatDropFrameTimecode(0, 30)).toThrow(RangeError);
    expect(() => dropFrameTimecode(0, 23.976)).toThrow(RangeError);
    expect(() => formatDropFrameTimecode(1.5, 29.97)).toThrow(RangeError);
  });

  it('matches the plan vectors at 29.97', () => {
    expect(formatDropFrameTimecode(0, 29.97)).toBe('00:00:00;00');
    expect(formatDropFrameTimecode(1799, 29.97)).toBe('00:00:59;29');
    expect(formatDropFrameTimecode(1800, 29.97)).toBe('00:01:00;02');
    expect(formatDropFrameTimecode(17981, 29.97)).toBe('00:09:59;29');
    expect(formatDropFrameTimecode(17982, 29.97)).toBe('00:10:00;00');
    expect(formatDropFrameTimecode(17983, 29.97)).toBe('00:10:00;01');
    expect(formatDropFrameTimecode(107892, 29.97)).toBe('01:00:00;00');
    expect(formatDropFrameTimecode(2589407, 29.97)).toBe('23:59:59;29');
    expect(formatDropFrameTimecode(2589408, 29.97)).toBe('00:00:00;00');
    expect(formatDropFrameTimecode(-1, 29.97)).toBe('23:59:59;29');
  });

  it('matches the plan vectors at 59.94', () => {
    expect(formatDropFrameTimecode(3599, 59.94)).toBe('00:00:59;59');
    expect(formatDropFrameTimecode(3600, 59.94)).toBe('00:01:00;04');
    expect(formatDropFrameTimecode(35964, 59.94)).toBe('00:10:00;00');
    expect(formatDropFrameTimecode(35964 * 6 * 24, 59.94)).toBe('00:00:00;00');
  });

  it('formats seconds through the nearest frame', () => {
    expect(dropFrameTimecode(1800 / 29.97, 29.97)).toBe('00:01:00;02');
    expect(dropFrameTimecode(3600, 30000 / 1001)).toBe('01:00:00;00');
    expect(dropFrameTimecode(600, 59.94)).toBe('00:10:00;00');
  });

  it('round-trips every frame of a day at 29.97 and two hours at 59.94', () => {
    for (const [fps, frames] of [[29.97, 2589408], [59.94, 35964 * 12]] as const) {
      let previous = -1;
      for (let frame = 0; frame < frames; frame++) {
        const label = formatDropFrameTimecode(frame, fps);
        const parsed = parseDropFrameTimecode(label, fps);
        if (parsed !== frame) expect({ frame, label, parsed }).toEqual({ frame, label, parsed: frame });
        if (parsed! <= previous) throw new Error(`labels out of order at ${frame}`);
        previous = parsed!;
      }
    }
  });

  it('rejects invalid drop-frame strings', () => {
    for (const invalid of ['00:01:00;00', '00:01:00;01', '00:09:00;01', '24:00:00;00', '00:60:00;00', '00:00:60;00', '00:00:00;30', '0:00:00;00', '00:00:00:00x', 'nope']) {
      expect(parseDropFrameTimecode(invalid, 29.97)).toBeNull();
    }
    expect(parseDropFrameTimecode('00:01:00;03', 59.94)).toBeNull();
    expect(parseDropFrameTimecode('00:00:00;59', 59.94)).toBe(59);
    expect(parseDropFrameTimecode('00:10:00;00', 29.97)).toBe(17982);
    expect(parseDropFrameTimecode('00:10:00.01', 29.97)).toBe(17983);
    expect(parseDropFrameTimecode('00:01:00;02', 30)).toBeNull();
  });
});
