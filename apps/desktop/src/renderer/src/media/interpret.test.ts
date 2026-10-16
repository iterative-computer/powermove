import { describe, expect, it } from 'vitest';

import type { MediaFacts } from '../../../shared/engine-protocol';
import { applyChange, assetChange, automaticLabel, choicesFor, draftOf, originText, settingOf, storedOf, valueLabel } from './interpret';

const facts = (over: Partial<MediaFacts> = {}): MediaFacts => ({
  kind: 'video',
  facts: [{ label: 'Codec', value: 'prores (HQ)' }],
  settings: [
    { member: 'primaries', value: 'bt709', automatic: 'bt709', origin: { kind: 'default', rule: 'Untagged HD and larger video reads as BT.709' } },
    { member: 'frameRate', value: '30000/1001', automatic: '30000/1001', origin: { kind: 'file', source: 'timestamps' } },
    { member: 'pixelAspect', value: '8:9', automatic: '8:9', origin: { kind: 'file', source: 'pasp' } }
  ],
  issues: [],
  displaySize: [640, 480],
  duration: 2,
  ...over
});

describe('Interpret Footage model', () => {
  it('round-trips the stored member through a draft', () => {
    const stored = {
      primaries: 'displayP3', transfer: 'bt1886', range: 'full', alpha: 'premultipliedWithColor', alphaMatte: '#ff8000',
      frameRate: 23.976023976023978, frameRateRational: [24000, 1001] as [number, number], fieldOrder: 'bottomFirst',
      pixelAspect: [10, 11] as [number, number], cleanAperture: true
    };
    const draft = draftOf(stored);
    expect(draft).toMatchObject({ primaries: 'displayP3', frameRate: '24000/1001', pixelAspect: '10:11', cleanAperture: 'true' });
    expect(storedOf(draft)).toEqual({ ...stored, alphaMatte: '#FF8000' });
  });

  it('reads a float-only rate as the rate the engine snaps it to', () => {
    expect(draftOf({ frameRate: 29.97 }).frameRate).toBe('30000/1001');
    expect(draftOf({ frameRate: 25 }).frameRate).toBe('25/1');
  });

  it('drops the member when everything is automatic', () => {
    expect(storedOf({ primaries: null, frameRate: '' })).toBeNull();
    expect(draftOf({ primaries: 'auto', junk: 1 })).toEqual({});
    expect(draftOf(null)).toEqual({});
  });

  it('labels values, automatic values, and origins', () => {
    expect(valueLabel('frameRate', '24000/1001')).toBe('23.976 fps');
    expect(valueLabel('frameRate', '1000/1')).toBe('1000 fps');
    expect(valueLabel('pixelAspect', '8:9')).toBe('8:9 (0.89)');
    const f = facts();
    expect(automaticLabel('primaries', settingOf(f, 'primaries'))).toBe('Automatic (Rec. 709)');
    expect(originText(settingOf(f, 'pixelAspect')?.origin)).toBe('From the file (pasp)');
    expect(originText({ kind: 'default', rule: 'Untagged HD and larger video reads as BT.709' })).toBe('Untagged HD and larger video reads as BT.709');
    expect(originText({ kind: 'override' }, 'bt709', 'primaries')).toBe('Overridden · the file reads Rec. 709');
  });

  it('offers the file’s own unlisted value', () => {
    const list = choicesFor('pixelAspect', settingOf(facts(), 'pixelAspect'));
    expect(list.at(-1)).toEqual(['8:9', '8:9 (0.89)']);
    expect(choicesFor('pixelAspect', { member: 'pixelAspect', value: '1:1', automatic: '1:1', origin: { kind: 'override' } }).some(([v]) => v === '1:1')).toBe(true);
  });

  it('applies the member, display size, and duration as the reading gives them', () => {
    const asset: Record<string, unknown> = { id: 'a', w: 720, h: 480, dur: 1 };
    const change = assetChange({ pixelAspect: '1:1', frameRate: '24/1' }, facts({ displaySize: [720, 480], duration: 1.25 }), asset);
    expect(change).toEqual({ interpret: { pixelAspect: [1, 1], frameRate: 24, frameRateRational: [24, 1] }, dur: 1.25 });
    applyChange(asset, change);
    expect(asset).toEqual({ id: 'a', w: 720, h: 480, dur: 1.25, interpret: { pixelAspect: [1, 1], frameRate: 24, frameRateRational: [24, 1] } });
    applyChange(asset, assetChange({}, facts({ displaySize: [640, 480], duration: 1.25 }), asset));
    expect(asset).toEqual({ id: 'a', w: 640, h: 480, dur: 1.25 });
  });
});
