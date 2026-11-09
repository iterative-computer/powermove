import {expect,it,vi} from 'vitest';
import {stableMaterialGroupBounds} from './material-group-bounds';
it('retains the same box for a child entering midway and invalidates on edits',()=>{
  const PM:any={proj:{dur:2,fps:2,revision:0}},group={id:'g',dur:2};
  const sample=vi.fn((t:number)=>({x0:0,y0:0,x1:t<1?10:100,y1:20,w:t<1?10:100,h:20}));
  const first=stableMaterialGroupBounds(PM,group,sample);
  expect(first.w).toBe(100);expect(stableMaterialGroupBounds(PM,group,sample)).toBe(first);expect(sample).toHaveBeenCalledTimes(5);
  PM.proj.revision++;expect(stableMaterialGroupBounds(PM,group,sample)).not.toBe(first);
});
