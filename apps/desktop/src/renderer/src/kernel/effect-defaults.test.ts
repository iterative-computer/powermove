import { expect, it } from 'vitest';
import { effectDefaultDimensions,effectParameterDefault,validEffectParameterDefault } from './effect-defaults';
import { EFFECTS } from '../../../extensions/effects-basic/effects';
import { makePM } from '../legacy/__tests__/make-pm';
import { validateEffect } from './glsl';
it('resolves numeric point defaults from source dimensions while retaining literal fallbacks',()=>{
  const p=EFFECTS.find(e=>e.id==='gradient')!.params;
  expect(p.map(param=>effectParameterDefault(param,{width:240,height:80}))).toEqual([120,0,'#000000',120,80,'#FFFFFF',0,0,0]);
  expect(effectParameterDefault(p[0]!,{width:NaN,height:80})).toBe(0);
  expect(effectDefaultDimensions({d:{w:{v:80},h:40}},{w:1920,h:1080})).toEqual({width:80,height:40});
  expect(effectDefaultDimensions({d:{w:0,h:NaN}},{w:1920,h:1080})).toEqual({width:1920,height:1080});
  expect(effectDefaultDimensions({}, {w:1920,h:1080},{w:300,h:200})).toEqual({width:300,height:200});
});
it('uses layer-aware creation defaults without changing saved literal revision-one channels',()=>{
  const PM=makePM('core/easing','core/model','core/anim','gl/shaders');
  for(const d of EFFECTS)PM.Kernel.registerEffect('effects-basic',d);
  PM.proj={w:1920,h:1080,layers:[]};const layer={d:{w:240,h:80}};PM.proj.layers=[layer];
  const fx=PM.mkEffect('gradient',undefined,layer),old=PM.mkEffect('gradient',1);
  expect([fx.p.startX.v,fx.p.endX.v,fx.p.endY.v]).toEqual([120,120,80]);
  expect([PM.mkEffect('gradient').p.endX.v,PM.mkEffect('gradient').p.endY.v]).toEqual([960,1080]);
  expect(old.p.angle.v).toBe(90);expect(old.p.dither.v).toBe(1);expect(old.p.startX).toBeUndefined();
});
it('rejects contextual defaults on nonnumeric channels or invalid dimensions/factors',()=>{
  const gradient=EFFECTS.find(e=>e.id==='gradient')!;
  for(const value of [{dimension:'depth',factor:1},{dimension:'height',factor:NaN},{dimension:'height',factor:Infinity},{dimension:'width',factor:1e7}]){
    const params=gradient.params.map((p,i)=>i===0?{...p,defaultFrom:value}:p);
    expect(validEffectParameterDefault(params[0] as any)).toBe(false);
    expect(()=>validateEffect({...gradient,params} as any)).toThrow('contextual default');
  }
  expect(validEffectParameterDefault({k:'c',label:'Color',type:'color',def:'#000000',defaultFrom:{dimension:'width',factor:.5}})).toBe(false);
});
