import { describe, expect, it } from 'vitest';

import {
  axisMeta, fontAxisChannelPath, fontAxisContentKey, normalizeAxisTag,
  parseOpenTypeVariationAxes, persistedFontAxes
} from './variable-font';

function variableFontBuffer(): ArrayBuffer {
  const bytes = new Uint8Array(84);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, 0x00010000, false);
  view.setUint16(4, 1, false);
  bytes.set([0x66, 0x76, 0x61, 0x72], 12); // fvar
  view.setUint32(20, 32, false);
  view.setUint32(24, 36, false);
  view.setUint16(32, 1, false);
  view.setUint16(34, 0, false);
  view.setUint16(36, 16, false);
  view.setUint16(38, 2, false);
  view.setUint16(40, 1, false);
  view.setUint16(42, 20, false);
  bytes.set([0x77, 0x67, 0x68, 0x74], 48); // wght
  view.setInt32(52, 100 * 65536, false);
  view.setInt32(56, 400 * 65536, false);
  view.setInt32(60, 900 * 65536, false);
  view.setUint16(64, 0, false);
  view.setUint16(66, 256, false);
  return bytes.buffer;
}

describe('variable font model helpers', () => {
  it('reads OpenType fvar axes and keeps their authored range', () => {
    expect(parseOpenTypeVariationAxes(variableFontBuffer())).toEqual([{
      tag: 'wght', label: 'Weight', min: 100, default: 400, max: 900
    }]);
    expect(parseOpenTypeVariationAxes(new ArrayBuffer(4))).toEqual([]);
  });

  it('normalizes common tags without changing arbitrary case-sensitive tags', () => {
    expect(normalizeAxisTag('WGHT')).toBe('wght');
    expect(normalizeAxisTag('GRAD')).toBe('GRAD');
    expect(normalizeAxisTag('longer')).toBeNull();
    expect(fontAxisContentKey('GRAD')).toBe('fontAxis.GRAD');
    expect(fontAxisChannelPath('GRAD')).toBe('c.fontAxis.GRAD');
  });

  it('restores persisted arbitrary channels even when font discovery is unavailable', () => {
    const content = { 'fontAxis.XTRA': { v: 37, kf: [], expr: null } };
    expect(persistedFontAxes(content)).toEqual([{ ...axisMeta('XTRA', 37), tag: 'XTRA' }]);
  });
});
