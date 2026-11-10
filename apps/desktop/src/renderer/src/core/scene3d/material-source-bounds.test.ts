import {expect,it,vi} from 'vitest';
import {makePM} from '../../legacy/__tests__/make-pm';
import {stableMaterialSourceBounds} from './material-source-bounds';

it('unions children across the source duration and keeps a stable origin across entrances, exits and motion',()=>{
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({w:300,h:100,dur:4,fps:30});
  const group=PM.mkLayer('group',{from:0,dur:4});group.id='art';group.on=false;
  const left=PM.mkLayer('solid',{from:0,dur:1}),right=PM.mkLayer('solid',{from:2,dur:2});
  left.group=right.group=group.id;left.p['position.x'].v=-100;right.p['position.x'].v=100;
  right.p['position.x'].kf=[PM.KF(0,100),PM.KF(2,200)];PM.proj.layers=[group,left,right];
  PM.GL.bounds=vi.fn(()=>({x0:0,y0:0,x1:20,y1:10,w:20,h:10}));PM.touch();
  const first=stableMaterialSourceBounds(PM,PM.proj,group,.5);
  expect(first).toEqual({x0:-100,y0:0,x1:220,y1:10,w:320,h:10});
  const calls=PM.GL.bounds.mock.calls.length;
  expect(stableMaterialSourceBounds(PM,PM.proj,group,2.5)).toBe(first);
  expect(PM.GL.bounds).toHaveBeenCalledTimes(calls);
  right.p['position.x'].kf[1].v=300;PM.touch();
  expect(stableMaterialSourceBounds(PM,PM.proj,group,2.5).x1).toBe(320);
});
