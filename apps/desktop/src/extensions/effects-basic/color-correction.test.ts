import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { LEGACY_EFFECTS } from './effects';
import { COLOR_CORRECTION_EFFECTS } from './color-correction';
import { parameterLayout } from '../inspector/parameter-layout';

const tint = COLOR_CORRECTION_EFFECTS.find(effect => effect.id === 'tint')!;
const mixer = COLOR_CORRECTION_EFFECTS.find(effect => effect.id === 'channelMixer')!;

describe('additive color correction compatibility', () => {
  it('preserves original saved defaults, parameter order and effect identities', () => {
    // Baseline metadata from 1.0.3. Shader pixel-unit repairs and additional
    // neighborhood padding are allowed; saved channels retain their meaning.
    const metadata = LEGACY_EFFECTS.map(({ frag, viewportPaddingPixels, ...definition }) => definition);
    expect(createHash('sha256').update(JSON.stringify(metadata)).digest('hex'))
      .toBe('0a2f9372bb3c2d05792651cf3c71483377fe2cbd7b4f63628bda163d090ce1d4');
  });

  it('exposes independent black/white maps and tint amount in documented control order', () => {
    expect(tint.params.map(param => [param.k, param.label, param.def, param.type || 'number'])).toEqual([
      ['mapBlackTo', 'Map Black To', '#000000', 'color'],
      ['mapWhiteTo', 'Map White To', '#FFFFFF', 'color'],
      ['amountToTint', 'Amount to Tint', 100, 'number'],
    ]);
    expect(tint.params[2]).toMatchObject({ min: 0, max: 100, step: 1, unit: '%' });
  });

  it('starts the channel matrix at identity, with separate constant terms and a monochrome switch', () => {
    expect(mixer.params.map(param => [param.k, param.def])).toEqual([
      ['redRed', 100], ['redGreen', 0], ['redBlue', 0], ['redConst', 0],
      ['greenRed', 0], ['greenGreen', 100], ['greenBlue', 0], ['greenConst', 0],
      ['blueRed', 0], ['blueGreen', 0], ['blueBlue', 100], ['blueConst', 0],
      ['monochrome', false],
    ]);
    expect(mixer.params.slice(0, 12).every(param => 'min' in param && param.min === -200
      && param.max === 200 && param.step === 1 && param.unit === '%')).toBe(true);
    const layout = parameterLayout(mixer.params);
    expect(layout.primary.map(param => param.k)).toEqual(mixer.params.map(param => param.k));
    expect(layout.more).toEqual([]);
  });
});
