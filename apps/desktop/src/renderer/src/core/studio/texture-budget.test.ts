import { describe, expect, it } from 'vitest';
import { planStudioTextures } from './texture-budget';
const MiB=1024*1024;
describe('Studio texture memory budget',()=>{
  it('retains 4K detail for a small scene and scales down when images multiply',()=>{
    const image={width:4096,height:4096};
    expect(planStudioTextures([image],0).size).toBe(4096);
    const many=planStudioTextures(Array.from({length:18},()=>image),0);
    expect(many.size).toBe(1024);expect(many.bytes).toBeLessThanOrEqual(many.budgetBytes);
  });
  it('reserves memory for geometry and honors device limits',()=>{
    const sources=Array.from({length:4},()=>({width:8192,height:2048}));
    const normal=planStudioTextures(sources,0),busy=planStudioTextures(sources,500*MiB);
    expect(busy.size).toBeLessThan(normal.size);expect(busy.bytes).toBeLessThanOrEqual(12*MiB);
    expect(planStudioTextures(sources,0,{maxDimension:1024}).size).toBe(1024);
    expect(()=>planStudioTextures(Array(257).fill(sources[0]),0)).toThrow('256');
  });
  it('does not allocate a 1024 square array for an untextured scene',()=>{
    expect(planStudioTextures([],0)).toEqual({size:1,bytes:4,budgetBytes:0});
  });
});
