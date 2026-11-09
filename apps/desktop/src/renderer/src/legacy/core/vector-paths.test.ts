import {expect,it} from 'vitest';
import {pathRasterGeometry} from './vector-paths';
it.each([undefined,'round','bevel'])('keeps a 10px rectangle stroke close to its corners (%s)',join=>{
  const vertices=[[0,0],[100,0],[100,100],[0,100]].map(([x,y])=>({x,y,inX:0,inY:0,outX:0,outY:0}));
  const geometry=pathRasterGeometry({mul:(_:any,b:any)=>b},{d:{}},0,1,[{v:{vertices,closed:true,copies:1,repeatRotation:0,repeatX:0,repeatY:0,strokeWidth:10,join},matrix:[1,0,0,1,0,0]}]);
  expect(geometry.selection.x0).toBe(-6);expect(geometry.selection.x1).toBe(106);
});
