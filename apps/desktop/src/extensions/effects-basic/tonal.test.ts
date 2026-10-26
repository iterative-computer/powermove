import { expect, it } from 'vitest';
import { TONAL_EFFECTS } from './tonal';
import { parameterLayout } from '../inspector/parameter-layout';
import { validateEffect } from '../../renderer/src/kernel/glsl';
it('registers distinct tonal operators with independent keyframe channels and source alpha',()=>{
  expect(TONAL_EFFECTS.map(effect=>effect.id)).toEqual(['tritone','colorBalanceHLS','brightnessContrast','blackWhite']);
  for(const effect of TONAL_EFFECTS){expect(validateEffect(effect)).toBe(effect);expect(effect.passes).toBe(1);expect(effect.frag).toContain('c.a');}
});
it('preserves controls and neutral defaults instead of changing the original combined Color effect',()=>{
  const hls=TONAL_EFFECTS.find(e=>e.id==='colorBalanceHLS')!,brightness=TONAL_EFFECTS.find(e=>e.id==='brightnessContrast')!;
  expect(hls.params.map(p=>[p.k,p.def])).toEqual([['hue',0],['lightness',0],['saturation',0]]);
  expect(brightness.params.map(p=>[p.k,p.def])).toEqual([['brightness',0],['contrast',0],['useLegacy',false]]);
  expect(parameterLayout(brightness.params).primary.map(p=>p.k)).toEqual(['brightness','contrast','useLegacy']);
});
it('exposes three Tritone color maps and six grouped Black & White weights',()=>{
  expect(TONAL_EFFECTS[0]!.params.map(p=>p.k)).toEqual(['highlights','midtones','shadows','blendWithOriginal']);
  const bw=TONAL_EFFECTS.find(e=>e.id==='blackWhite')!;
  expect(bw.params.slice(0,6).map(p=>p.def)).toEqual([40,60,40,60,20,80]);
  expect(parameterLayout(bw.params).more).toEqual([]);
  expect(bw.params.slice(0,6).every(p=>p.group==='Color Weights')).toBe(true);
});
