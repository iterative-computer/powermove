import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI } from 'powermove';

import activate from './index';
import { EFFECTS } from './effects';

const IDS = [
  'blur', 'motionblurDir', 'sharpen', 'glow', 'color',
  'levels', 'duotone', 'grain', 'vignette', 'chroma',
  'pixelate', 'posterize', 'displace', 'shadow', 'invert', 'gradient', 'tint', 'channelMixer', 'threshold', 'shiftChannels', 'exposure', 'levelsIndividual', 'unsharpMask', 'findEdges', 'tritone', 'colorBalanceHLS', 'brightnessContrast', 'blackWhite', 'setMatte', 'displacementMap', 'compoundBlur', 'setChannels', 'texturize', 'stroke', 'applyColorLut', 'solidComposite','arithmetic','blend','calculations','channelCombiner','colorProfileConverter','hdrCompander','echo'
];

describe('effects-basic', () => {
  it('exports and registers all 43 real shader effects', () => {
    const register = vi.fn();
    activate({ effects: { register, supportsTemporalInputs: true, supportsVersions: true, supportsSpatialSampling: true, supportsLayerInputs: true, supportsInputSubframes: true, supportsSourceRasters: true, supportsSourceOwners: true, supportsBrushGeometry: true, supportsOwnerValidity: true, supportsLutAssets: true, supportsTypedReferences: true, parameterDefault: (p: any) => p.def } } as unknown as PowermoveAPI);

    expect(EFFECTS.map((definition) => definition.id)).toEqual(IDS);
    expect(EFFECTS.slice(0, 28).every((definition) => definition.rawShader === true)).toBe(true);
    expect(EFFECTS.slice(28,35).every((definition) => definition.params.some(p => ['layer','path','asset'].includes(p.type??'')))).toBe(true);
    expect(register.mock.calls.map(([definition]) => definition.id)).toEqual(IDS);
  });

  it.each(IDS.slice(0,28))('%s ships a self-contained raw shader', (id) => {
    const definition = EFFECTS.find((effect) => effect.id === id);

    expect(definition?.rawShader).toBe(true);
    expect(definition?.frag).toMatch(/^#version 300 es/);
    expect(definition?.frag).toContain('void main()');
  });

  it('organizes effects into families by what they do', () => {
    const groups: Record<string, string[]> = {};
    for (const { id, group } of EFFECTS) (groups[group] ??= []).push(id);

    expect(groups).toEqual({
      'Blur & Sharpen': ['blur', 'motionblurDir', 'sharpen', 'unsharpMask', 'compoundBlur'],
      'Light & Shadow': ['glow', 'vignette', 'shadow'],
      Color: ['color', 'levels', 'duotone', 'posterize', 'invert', 'tint', 'channelMixer', 'shiftChannels', 'exposure', 'levelsIndividual', 'tritone', 'colorBalanceHLS', 'brightnessContrast', 'blackWhite'],
      Stylize: ['grain', 'pixelate', 'threshold', 'findEdges', 'texturize'],
      Distort: ['chroma', 'displace', 'displacementMap'],
      Channel: ['setMatte', 'setChannels','solidComposite','arithmetic','blend','calculations','channelCombiner'],
      Generate: ['gradient','stroke'],
      Utility:['applyColorLut','colorProfileConverter','hdrCompander'],Time:['echo']
    });
    expect(Object.keys(groups)).toEqual(['Blur & Sharpen', 'Light & Shadow', 'Color', 'Stylize', 'Distort', 'Generate', 'Channel','Utility','Time']);
  });
});

it('refuses to silently reinterpret saved effects on a host without revision support', () => {
  const register = vi.fn();
  expect(() => activate({ effects: { register } } as unknown as PowermoveAPI)).toThrow('updated Powermove renderer');
  expect(register).not.toHaveBeenCalled();
});

it('retains the original displacement mapping and gives new instances a precision-aware neutral revision',()=>{
 const current=EFFECTS.find(f=>f.id==='displacementMap')!;expect(current.version).toBe(2);
 expect(current.legacy?.[0]?.version).toBe(1);expect(current.legacy?.[0]?.frag).toContain('clamp(vec2(x,y),0.,1.)*2.-1.');
 expect(current.frag).toContain('u_mapMidpoint');expect(current.frag).toContain('amount.x=0.');
});

it('keeps Stroke revision 1 width-only bounds and gives revision 2 an explicit brush geometry contract',()=>{
 const current=EFFECTS.find(f=>f.id==='stroke')!;expect(current.version).toBe(2);expect(current.legacy?.[0]?.version).toBe(1);expect(current.legacy?.[0]?.sourceProcessing).toEqual({padding:['brushSize'],factor:.5,pixels:1});expect(current.sourceProcessing?.geometry).toEqual(['path']);expect(current.params.find(p=>p.k==='path')).toHaveProperty('pathBrush');
});

it.each(['blend','calculations','channelCombiner'])('%s declares one actual layer sampler and source-sized processing without new animation channels',id=>{
 const definition=EFFECTS.find(f=>f.id===id)!;expect(definition.params.filter(p=>p.type==='layer').map(p=>p.k)).toEqual(['map']);expect(definition.sourceProcessing).toEqual({});expect(definition.passes).toBe(1);expect(definition.rawShader).toBe(true);expect(definition.viewportSafe).toBe(false);
});

it('keeps Channel Combiner revision 1 intact and appends multichannel conversions in revision 2',()=>{
 const def=EFFECTS.find(f=>f.id==='channelCombiner')!;expect(def.version).toBe(2);expect(def.legacy?.[0]?.version).toBe(1);expect(def.legacy?.[0]?.params.find(p=>p.k==='from')?.options).toHaveLength(11);expect(def.params.find(p=>p.k==='from')?.options).toHaveLength(23);expect(def.legacy?.[0]?.frag).not.toContain('convertChannels');expect(def.frag).toContain('convertChannels');
});
