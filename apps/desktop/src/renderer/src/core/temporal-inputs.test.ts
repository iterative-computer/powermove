import {expect,it} from 'vitest';
import {hasLayerInputs,inputKey,layerInputPlan,temporalTimes} from './effect-inputs';
const echo={on:true,type:'echo',p:{step:-.125,count:2}},map=(id:string,stage='effects')=>({on:true,type:'map',inputs:{map:{layerId:id,stage}}});
const layer=(id:string,fx:any[]=[])=>({id,name:id,type:'shader',from:0,dur:2,fx});
const definition=(fx:any)=>fx.type==='echo'?{params:[],sourceProcessing:{},temporal:{step:'step',count:'count',maxSamples:32}}:{params:[{k:'map',type:'layer'}],sourceProcessing:{}};
const plan=(layers:any[],options:any={})=>layerInputPlan({layers,dur:2},{definition,time:1,value:(_l,fx,key,t)=>typeof fx.p[key]==='function'?fx.p[key](t):fx.p[key],...options});
it('orders historical prefix dependencies at provider timestamps and separates complete/source/mask/space/time keys',()=>{
 const owner=layer('owner',[map('hidden','masks'),echo,map('after')]),hidden=layer('hidden'),after=layer('after');
 const p=plan([owner,hidden,after]);const keys=p.requests.map(n=>n.key);
 for(const t of [.875,.75]){expect(keys.indexOf(inputKey('hidden','masks','composition',t))).toBeLessThan(keys.indexOf(inputKey('owner','effects','composition',t,1)));expect(keys).not.toContain(inputKey('after','effects','composition',t));}
 expect(keys).toContain(inputKey('after','effects','composition',1));expect(new Set(['source','masks','effects'].flatMap(s=>[inputKey('owner',s as any,'source',1,0),inputKey('owner',s as any,'composition',1,0),inputKey('owner',s as any,'composition',.875,0)] )).size).toBe(9);
});
it('multiple Echo prefixes terminate including zero offsets and evaluate preceding controls at historical times',()=>{
 const a=layer('a',[echo,{...echo,p:{step:0,count:2}}]);const p=plan([a]);expect(p.requests.length).toBeLessThan(10);expect(p.requests.some(n=>n.prefix===0)).toBe(true);expect(p.requests.some(n=>n.prefix===1)).toBe(true);
 const dynamic=plan([layer('a',[{...echo,p:{step:-.25,count:(t:number)=>t<1?1:2}},echo])]);expect(dynamic.requests.every(n=>n.prefix!==undefined)).toBe(true);
});
it('bounds temporal expansion and treats off-timeline prefixes as transparent leaves',()=>{
 expect(()=>temporalTimes(definition(echo),k=>k==='step'?-.5:33,1)).toThrow('limits');
 expect(()=>temporalTimes(definition(echo),()=>NaN,1)).toThrow('limits');
 const p=plan([layer('a',[{...echo,p:{step:-1,count:2}}])],{time:0});expect(p.requests.map(n=>n.time)).toEqual([-1,-2]);expect(p.order.filter(n=>n.time!<0).every(n=>!n.deps.length)).toBe(true);
 const many=Array.from({length:10},(_,i)=>layer(String(i),[{...echo,p:{step:-.001,count:32}}]));expect(()=>plan(many)).toThrow('frame request limit');
});
it('preserves cycle/missing diagnostics at historical times and ignores disabled temporal definitions',()=>{
 expect(()=>plan([layer('a',[map('b'),echo]),layer('b',[map('a')])])).toThrow('cycle');
 expect(()=>plan([layer('a',[map('deleted'),echo])])).toThrow('missing input');
 const disabled={...echo,on:false};expect(hasLayerInputs({layers:[layer('a',[disabled])]},definition)).toBe(false);expect(hasLayerInputs({layers:[layer('a',[echo])]},definition)).toBe(true);
 const source={...layer('a',[echo]),effectSpace:'source'};expect(plan([source]).requests.every(n=>n.space==='source')).toBe(true);
});

it('evaluates an animated enable at the provider time instead of the requesting time',()=>{const gated={...map('missing'),on:(t:number)=>t<1};expect(()=>plan([layer('a',[gated,echo])],{enabled:(_l:any,fx:any,t:number)=>typeof fx.on==='function'?fx.on(t):fx.on})).toThrow('missing input');});

it('rejects temporal backdrop prefixes instead of rendering them over an empty stack',()=>{const backdrop={type:'backdrop',on:true};const def=(fx:any)=>fx.type==='backdrop'?{params:[],backdrop:true}:definition(fx);expect(()=>plan([layer('a',[backdrop,echo])],{definition:def})).toThrow('surrounding backdrop');expect(()=>layerInputPlan({layers:[layer('a',[backdrop,echo])]},{definition:def})).toThrow('surrounding backdrop');});
