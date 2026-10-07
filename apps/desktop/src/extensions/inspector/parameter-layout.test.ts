import { describe, expect, it } from 'vitest';
import { parameterLayout } from './parameter-layout';
import type { EffectParamDefinition } from 'powermove';
import { EFFECTS } from '../effects-basic/effects';
const number=(k:string,extra={}):EffectParamDefinition=>({k,label:k,def:0,min:-100,max:100,...extra});

describe('visual parameter layout',()=>{
  it('keeps old gradient data editable while combining colors, mode, midpoint and center',()=>{
    const params=EFFECTS.find(effect=>effect.id==='gradient')!.params.map(({advanced,...p})=>p);
    const layout=parameterLayout(params);
    expect(layout.widgets).toContainEqual({kind:'ramp',start:'startColor',end:'endColor',midpoint:'midpoint',mode:'radial'});
    expect(layout.widgets.some(w=>w.kind==='point'&&w.x==='centerX'&&w.y==='centerY')).toBe(true);
    const represented=new Set([...layout.primary,...layout.more].map(p=>p.k));
    layout.widgets.forEach(w=>{if(w.kind==='ramp'&&w.mode)represented.add(w.mode);else if(w.kind==='point'){represented.add(w.x);represented.add(w.y);}});
    expect([...represented].sort()).toEqual(params.map(p=>p.k).sort());
  });
  it('never hides parameters referenced by invalid or overlapping widgets',()=>{
    const params=[number('x'),number('y'),number('z')];
    const layout=parameterLayout(params,[{kind:'point',x:'missing',y:'y'},{kind:'point',x:'x',y:'y'},{kind:'point',x:'y',y:'z'}]);
    expect(layout.widgets).toEqual([{kind:'point',x:'x',y:'y'}]);
    expect(layout.primary.map(p=>p.k)).toEqual(['z']);
    expect(layout.widgets[0]).toMatchObject({x:'x',y:'y'});
  });
  it('bounds legacy primary controls while retaining Amount and explicit visibility',()=>{
    const layout=parameterLayout([...Array.from({length:8},(_,i)=>number('p'+i)),number('amount'),number('important',{advanced:false}),number('seed',{advanced:true})]);
    expect(layout.primary.map(p=>p.k)).toEqual(['p0','p1','p2','p3','p4','important','amount']);
    expect(layout.more.map(p=>p.k)).toEqual(['p5','p6','p7','seed']);
  });
});
