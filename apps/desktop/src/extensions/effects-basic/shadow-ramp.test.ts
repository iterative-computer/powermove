import { expect,it } from 'vitest';
import { EFFECTS,LEGACY_EFFECTS } from './effects';
import { parameterLayout } from '../inspector/parameter-layout';
it('keeps historical Shadow and Ramp complete while revising current controls',()=>{
 for(const id of ['shadow','gradient']){
  const current=EFFECTS.find(e=>e.id===id)!,old=LEGACY_EFFECTS.find(e=>e.id===id)!;
  expect(current.version).toBe(2);expect(current.legacy).toEqual([{...old,version:1}]);
 }
 expect(EFFECTS.find(e=>e.id==='shadow')!.params.map(p=>[p.k,p.def])).toEqual([['color','#000000'],['opacity',50],['angle',135],['distance',5],['softness',0],['shadowOnly',false]]);
});
it('exposes independently keyframeable Ramp points and colors through native widgets',()=>{
 const ramp=EFFECTS.find(e=>e.id==='gradient')!;
 expect(parameterLayout(ramp.params,ramp.ui).widgets).toEqual(expect.arrayContaining([{kind:'point',x:'startX',y:'startY',label:'Start Of Ramp'},{kind:'point',x:'endX',y:'endY',label:'End Of Ramp'}]));
 expect(ramp.actions).toEqual([{kind:'swap',label:'Swap Colors',keys:['startColor','endColor']}]);
 expect(ramp.params.find(p=>p.k==='endY')!.defaultFrom).toEqual({dimension:'height',factor:1});
});
