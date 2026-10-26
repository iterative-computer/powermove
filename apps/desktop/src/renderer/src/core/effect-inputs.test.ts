import { describe, expect, it } from 'vitest';
import { hasLayerInputs, effectMightRender, assertInputBudget, inputKey, layerInputPlan, remapLayerInputs, validateLayerInputs,validateEffectInputs } from './effect-inputs';

const fx = (id: string | null, stage = 'source', on: any = true) => ({ type: 'map', on, inputs: { map: { layerId: id, stage } } });
const layer = (id: string, effects: any[] = []) => ({ id, name: id, type: 'shape', fx: effects });
const definition = (effect: any) => ({ params: [{ k: 'map', type: 'layer' }], backdrop: effect.backdrop });
const plan = (layers: any[], options: any = {}) => layerInputPlan({ layers }, { definition, ...options });

describe('stable layer effect inputs', () => {
  it('validates stages and static IDs without accepting channel objects or prototype keys', () => {
    expect(validateLayerInputs({ map: { layerId: null, stage: 'source' } })).toEqual({ map: { layerId: null, stage: 'source' } });
    for (const value of [{map:{layerId:'a',stage:'final'}},{map:{layerId:{v:'a'},stage:'source'}},JSON.parse('{"__proto__":{"layerId":null,"stage":"source"}}')])
      expect(() => validateLayerInputs(value)).toThrow();
  });
  it('allows source/mask self inputs and orders references before effected consumers', () => {
    const p = plan([layer('a',[fx('a'),fx('b','effects')]),layer('b',[fx('c','masks')]),layer('c')]);
    expect(p.requests.map(n=>n.key)).toEqual([inputKey('a','source'),inputKey('c','masks'),inputKey('b','effects')]);
    expect(() => plan([layer('a',[fx('a','masks')])])).not.toThrow();
  });
  it('detects self, cross-layer, group, matte and backdrop cycles with readable phase paths', () => {
    for (const layers of [
      [layer('a',[fx('a','effects')])],
      [layer('a',[fx('b','effects')]),layer('b',[fx('a','effects')])],
      [{...layer('g'),type:'group'},{...layer('a',[fx('g')]),group:'g'}],
      [{...layer('a',[fx('b','effects')])},{...layer('b'),matteSource:'a'}],
      [layer('a',[{...fx(null),backdrop:true}]),layer('b',[fx('a','effects')])],
    ]) expect(() => plan(layers,{topologyOnly:true})).toThrow(/cycle:.*\(effects\)/);
  });
  it('handles a deep DAG iteratively and rejects its final back edge', () => {
    const layers=Array.from({length:12000},(_,i)=>layer(String(i),i<11999?[fx(String(i+1),'effects')]:[]));
    expect(plan(layers).order).toHaveLength(36000);
    layers[11999]!.fx=[fx('0','effects')];expect(()=>plan(layers)).toThrow('cycle');
  });
  it('keeps deleted IDs recoverable and distinguishes absent from unsupported inputs', () => {
    const a=layer('a',[fx('deleted')]);expect(()=>plan([a])).toThrow('missing input layer');
    expect(()=>plan([a],{allowMissing:true})).not.toThrow();expect(a.fx[0].inputs.map.layerId).toBe('deleted');
    expect(()=>plan([layer('a',[fx('b')]),{...layer('b'),type:'adjustment'}])).toThrow('cannot be used');
    expect(()=>plan([layer('a',[fx('b','effects')]),layer('b',[{...fx(null),backdrop:true}])])).toThrow('backdrop');
  });
  it('ignores disabled effects but conservatively includes animated enable channels', () => {
    expect(()=>plan([layer('a',[fx('a','effects',false)])])).not.toThrow();
    expect(()=>plan([layer('a',[fx('a','effects',{v:false,kf:[{v:true}]})])])).toThrow('cycle');
  });
  it('remaps copied internal references and preserves external and missing references', () => {
    const a=layer('a',[fx('b'),fx('external'),fx(null)]);remapLayerInputs(a,new Map([['b','copy-b']]));
    expect(a.fx.map(f=>f.inputs.map.layerId)).toEqual(['copy-b','external',null]);
    expect(()=>assertInputBudget(3,1024,8192)).toThrow('memory budget');
    expect(()=>assertInputBudget(2,1024,8192)).not.toThrow();
  });
});


it('keeps composition and source-space phases distinct, validates spaces and detects mixed-space cycles',()=>{
 const first:any=layer('a',[fx('b','effects')]),second:any=layer('b');first.fx[0].inputs.map.space='source';
 expect(inputKey('b','effects')).not.toBe(inputKey('b','effects','source'));
 expect(plan([first,second]).requests.map(n=>[n.key,n.space])).toEqual([[inputKey('b','effects','source'),'source']]);
 second.fx=[fx('a','effects')];expect(()=>plan([first,second])).toThrow('cycle');
 expect(validateLayerInputs({map:{layerId:'b',stage:'source',space:'source'}}).map?.space).toBe('source');
 expect(()=>validateLayerInputs({map:{layerId:'b',stage:'source',space:'future'}})).toThrow('Invalid layer input');
 const raw:any=layer('a',[fx('b')]);raw.fx[0].inputs.map.space='source';
 expect(()=>plan([raw,{...layer('b'),type:'group'}])).toThrow('source-space group');
 raw.fx[0].inputs.map.stage='effects';expect(()=>plan([raw,{...layer('b'),matteSource:'c'},layer('c')])).toThrow('external track mattes');
});


it('validates typed geometry and image references without accepting animation channels or unknown fields',()=>{
 const refs={path:{kind:'path',layerId:'owner',pathId:'mask'},texture:{kind:'asset',assetId:'image'}};
 expect(validateEffectInputs(refs)).toEqual(refs);
 for(const invalid of [{path:{kind:'path',layerId:null,pathId:'mask'}},{texture:{kind:'asset',assetId:{v:'image'}}},{texture:{kind:'asset',assetId:'image',stage:'source'}},{path:{kind:'future',layerId:'owner'}}])expect(()=>validateEffectInputs(invalid)).toThrow();
 const a:any=layer('a');a.masks=[{id:'mask'}];a.fx=[{type:'stroke',inputs:{path:{kind:'path',layerId:'a',pathId:'mask'}}}];
 const definition=()=>({params:[{k:'path',type:'path'}]});expect(layerInputPlan({layers:[a]},{definition}).requests).toHaveLength(0);
 a.fx[0].inputs.path.pathId='deleted';expect(()=>layerInputPlan({layers:[a]},{definition})).toThrow('missing path');expect(()=>layerInputPlan({layers:[a]},{definition,allowMissing:true})).not.toThrow();
 a.fx=[{type:'texture',inputs:{texture:{kind:'asset',assetId:'image'}}}];const imageDefinition=()=>({params:[{k:'texture',type:'asset'}]});
 expect(()=>layerInputPlan({layers:[a],assets:{image:{kind:'image'}}},{definition:imageDefinition})).not.toThrow();
 expect(()=>layerInputPlan({layers:[a],assets:{}},{definition:imageDefinition})).toThrow('missing asset');
 remapLayerInputs(a,new Map([['image','not-an-asset-remap']]));expect(a.fx[0].inputs.texture.assetId).toBe('image');
});

it('distinguishes typed LUT assets from images and detects deletion even while a runtime is retained for Undo',()=>{
 const owner=layer('a',[{type:'lut',inputs:{lut:{kind:'asset',assetId:'grade'}}}]);const definition=()=>({params:[{k:'lut',type:'asset',assetKind:'lut'}]});
 expect(()=>layerInputPlan({layers:[owner],assets:{grade:{kind:'lut'}}},{definition})).not.toThrow();
 expect(()=>layerInputPlan({layers:[owner],assets:{grade:{kind:'image'}}},{definition})).toThrow('require lut');
 expect(()=>layerInputPlan({layers:[owner]},{definition,assets:{get:()=>({kind:'lut'}),has:()=>false}})).toThrow('missing asset');
});

it('keeps statically disabled and missing effects off the dependency/preparation path while retaining dynamic enables',()=>{
 const make=(on:any,missing=false)=>({layers:[{...layer('a',[{...fx('deleted'),on,missing}]),effectSpace:'source'}]});
 for(const on of [false,{v:false,kf:[],expr:null}]){expect(hasLayerInputs(make(on))).toBe(false);expect(effectMightRender({on})).toBe(false);}
 expect(hasLayerInputs(make(true,true))).toBe(false);expect(hasLayerInputs({layers:[{...layer('a'),effectSpace:'source'}]})).toBe(false);
 for(const on of [true,{v:false,kf:[{t:1,v:true}]},{v:false,expr:'time > 1'}])expect(hasLayerInputs(make(on))).toBe(true);
 expect(hasLayerInputs({layers:[{...layer('a'),effectSpace:'future'}]})).toBe(true);
 const a=layer('a',[fx('a','effects',{v:false,kf:[],expr:null})]);expect(()=>plan([a])).not.toThrow();
});
