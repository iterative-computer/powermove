import { describe, expect, it } from 'vitest';
import { createPathTiming } from './motion-path';

describe('original-curve arc-length timing',()=>{
  it('evaluates a parabola on its original curve, at uniform travelled distance',()=>{
    const route=createPathTiming(t=>[t,t*t],[0,1],0,1);
    const length=(x:number)=>x*Math.sqrt(1+4*x*x)/2+Math.asinh(2*x)/4;
    for(let i=0;i<=200;i++){
      const progress=i/200,[x,y]=route.position(progress);
      expect(y).toBe(x!*x!);
      expect(length(x!)/length(1)).toBeCloseTo(progress,5);
    }
    expect(route.position(.5)[0]).toBeGreaterThan(.5);
  });
  it('removes nonlinear speed even when the original path is a straight line',()=>{
    const route=createPathTiming(t=>[t*t*t,2*t*t*t],[0,1],0,1);
    for(let i=0;i<=100;i++){
      const [x,y]=route.position(i/100);expect(x).toBeCloseTo(i/100,4);expect(y).toBeCloseTo(i/50,4);
    }
  });
  it('retains loops and their direction when the endpoints coincide',()=>{
    const route=createPathTiming(t=>[Math.sin(2*Math.PI*t),1-Math.cos(2*Math.PI*t)],[0,.25,.5,.75,1],0,1);
    expect(route.length).toBeCloseTo(2*Math.PI,5);
    for(const p of [.1,.25,.5,.8]){
      const [x,y]=route.position(p);expect(x!*x!+(y!-1)**2).toBeCloseTo(1,12);expect(x).toBeCloseTo(Math.sin(2*Math.PI*p),5);
    }
  });
  it('supports static axes and clamps progress without extending the path',()=>{
    const route=createPathTiming(t=>[12,t,7],[0,1],0,1);
    expect(route.position(-1)).toEqual([12,0,7]);expect(route.position(2)).toEqual([12,1,7]);
    expect(()=>route.position(Infinity)).toThrow();
  });
  it('rejects invalid routes before they can alter source animation',()=>{
    expect(()=>createPathTiming(()=>[0,0],[],0,1)).toThrow('no movement');
    expect(()=>createPathTiming(()=>[NaN,0],[],0,1)).toThrow('finite');
    expect(()=>createPathTiming(()=>[1],[],1,0)).toThrow('time range');
  });
});
