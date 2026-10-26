import { expect, it } from 'vitest';
import { EFFECTS } from '../../../extensions/effects-basic/effects';
import { effectProgramKey, effectVersion, resolveEffectDefinition } from './effect-versions';
import { validateEffect } from './glsl';
import { makePM } from '../legacy/__tests__/make-pm';

it('keeps historical and current effect definitions distinct with stable cache identities', () => {
  const invert = EFFECTS.find(effect => effect.id === 'invert')!;
  expect(resolveEffectDefinition(invert)).toBe(invert);
  expect(resolveEffectDefinition(invert, 1)).toBe(invert.legacy![0]);
  expect(resolveEffectDefinition(invert, 2)).toBe(invert);
  expect(resolveEffectDefinition(invert, 99)).toBeUndefined();
  expect(effectVersion(undefined)).toBe(1);
  for (const version of [0, -1, NaN, Infinity, 1.5, '2']) expect(effectVersion(version)).toBeUndefined();
  expect(effectProgramKey('invert', undefined, invert)).toBe('fx:invert@1');
  expect(effectProgramKey('invert', 2, invert)).toBe('fx:invert');
});

it('makes new instances current while restoring explicit revisions without changing saved parameters', () => {
  const PM = makePM('core/easing', 'core/model', 'core/anim', 'gl/shaders');
  for (const definition of EFFECTS) PM.Kernel.registerEffect('effects-basic', definition);
  const current = PM.mkEffect('invert'), old = PM.mkEffect('invert', 1);
  expect(current.version).toBe(2); expect(current.p.blendWithOriginal.v).toBe(0);
  expect(old.version).toBe(1); expect(old.p.amount.v).toBe(100);
  expect(PM.effectDef({ type: 'invert', p: { amount: { v: 37 } } }).params.map((p: any) => p.k)).toEqual(['amount']);
  expect(PM.effectDef({ type: 'invert', version: 99 })).toBeUndefined();
  expect(PM.mkEffect('invert', 99)).toBeNull();
});

it('rejects ambiguous, recursive and malformed revision metadata or unsafe color actions', () => {
  const definition = EFFECTS.find(effect => effect.id === 'invert')!;
  expect(validateEffect(definition)).toBe(definition);
  for (const candidate of [
    { ...definition, version: 0 },
    { ...definition, legacy: [{ ...definition.legacy![0], version: 2 }] },
    { ...definition, legacy: [definition.legacy![0]!, definition.legacy![0]!] },
    { ...definition, legacy: [{ ...definition.legacy![0], id: 'other' }] },
    { ...definition, actions: [{ kind: 'swap', label: 'Swap', keys: ['channel', 'channel'] }] },
  ]) expect(() => validateEffect(candidate as any)).toThrow();
});

it('refreshes a historical translation and retires its own program after an in-place provider edit', () => {
  const PM = makePM('core/easing', 'core/model', 'core/anim', 'gl/shaders');
  const definition = structuredClone(EFFECTS.find(effect => effect.id === 'invert')!);
  PM.Kernel.registerEffect('effects-basic', definition);
  const effect = PM.mkEffect('invert', 1), before = PM.effectDef(effect);
  const retired: string[] = [];
  PM.GL = { dropProgram: (key: string) => retired.push(key) };
  definition.legacy![0]!.frag += '\n// new historical body';
  const after = PM.effectDef(effect);
  expect(after).not.toBe(before); expect(after.frag).toContain('new historical body');
  expect(retired).toEqual(['fx:invert@1']);
});

it('validates spatial sampling, cell grids and fixed crop support', () => {
  const blur = EFFECTS.find(effect => effect.id === 'blur')!;
  const mosaic = EFFECTS.find(effect => effect.id === 'pixelate')!;
  for (const definition of EFFECTS) expect(validateEffect(definition)).toBe(definition);
  for (const candidate of [
    {...blur,sampling:{radius:'absent',support:3,maxRadius:32}},
    {...blur,sampling:{radius:'amount',support:Infinity,maxRadius:32}},
    {...blur,sampling:{radius:'amount',support:3,maxRadius:65}},
    {...mosaic,passes:1}, {...mosaic,grid:{columns:'horizontalBlocks',rows:'missing'}},
    {...mosaic,grid:{...mosaic.grid,pointSample:'horizontalBlocks'}},
    {...blur,sampling:{...blur.sampling,space:'unknown'}},
    {...blur,sampling:{...blur.sampling,firstPass:2}},
    {...blur,sampling:{...blur.sampling,firstPass:-1}},
    {...blur,viewportPaddingPixels:-1}, {...blur,viewportPaddingPixels:NaN},
  ]) expect(() => validateEffect(candidate as any)).toThrow();
});

it('validates geometry, typed LUT formats and coordinate uniform contracts before shader registration',()=>{
 const stroke=EFFECTS.find(f=>f.id==='stroke')!,lut=EFFECTS.find(f=>f.id==='applyColorLut')!,solid=EFFECTS.find(f=>f.id==='solidComposite')!;expect(()=>validateEffect(stroke)).not.toThrow();expect(()=>validateEffect(lut)).not.toThrow();expect(()=>validateEffect(solid)).not.toThrow();
 expect(()=>validateEffect({...stroke,sourceProcessing:{geometry:['color']}})).toThrow('geometry');expect(()=>validateEffect({...stroke,params:stroke.params.map(p=>p.type==='path'?{...p,pathWidth:'color'}:p)})).toThrow('path width');
 expect(()=>validateEffect({...lut,params:lut.params.map(p=>p.type==='asset'?{...p,assetKind:'video'}:p)} as any)).toThrow('asset kind');expect(()=>validateEffect({...solid,coordinates:'world'} as any)).toThrow('coordinates');expect(()=>validateEffect({...solid,params:[...solid.params,{k:'layerUV',label:'Collision',def:0,min:0,max:1}]})).toThrow('uniform name collision');
});


it('validates bounded raw temporal contracts without allowing missing keys, backdrop recursion or excessive samples',()=>{
 const base:any={id:'temporal',label:'Temporal',group:'Time',params:[{k:'step',def:-.1},{k:'count',def:2}],rawShader:true,frag:'void main(){}',temporal:{step:'step',count:'count',maxSamples:32}};
 expect(validateEffect(base).temporal).toEqual(base.temporal);
 for(const patch of [{temporal:{...base.temporal,maxSamples:33}},{temporal:{...base.temporal,step:'missing'}},{backdrop:true},{passes:2},{rawShader:false},{keepOrig:true},{coordinates:'layer'},{sourceProcessing:{pixels:1}},{params:[...base.params,{k:'map',type:'layer',def:''}]}])expect(()=>validateEffect({...base,...patch})).toThrow('temporal contract');
});
