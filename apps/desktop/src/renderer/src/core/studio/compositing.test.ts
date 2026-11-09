import {describe,it,expect} from 'vitest';
import {combineOpaqueDepthSet} from './compositing';
describe('opaque Studio depth-set output',()=>{
  const ids=new Set(['a','b']);
  const pm=(patch:any={})=>({proj:{layers:[{id:'a',blend:'normal'},{id:'b',blend:'normal',...patch}]},evP:(_l:any,p:any)=>p.v});
  it('combines a plain set but retains single-layer and separated outputs',()=>{
    expect(combineOpaqueDepthSet(pm(),ids,new Set(),0)).toBe(true);
    expect(combineOpaqueDepthSet(pm(),new Set(['a']),new Set(),0)).toBe(false);
    expect(combineOpaqueDepthSet(pm(),ids,new Set(['b']),0)).toBe(false);
  });
  it.each([{blend:'multiply'},{blend:{v:'screen'}},{matteSource:'m'},{matteSources:[{source:'m'}]},{transitionIn:{}},{transitionOut:{}}])('retains styled layer boundaries: %j',patch=>{
    expect(combineOpaqueDepthSet(pm(patch),ids,new Set(),0)).toBe(false);
  });
  it('uses the active composition pass and evaluates animated blend modes',()=>{
    const PM:any=pm({blend:{v:'normal'}});
    expect(combineOpaqueDepthSet(PM,ids,new Set(),1)).toBe(true);
    PM._renderComposition3D={layers:[{id:'a'}]};
    expect(combineOpaqueDepthSet(PM,ids,new Set(),1)).toBe(false);
  });
  it('keeps transmissive glass on the established individual-output path',()=>{
    const runtime={objects:new Map([['b',{traverseVisible:(visit:any)=>visit({isMesh:true,material:{transmission:1}})}]])};
    expect(combineOpaqueDepthSet(pm(),ids,new Set(),0,runtime)).toBe(false);
  });
});
