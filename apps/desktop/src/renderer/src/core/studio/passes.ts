import { qualityForComposition } from './sampling';
import { STUDIO_MATERIAL_FLOATS } from './layout';
import { compositionRuntime } from '../scene3d/service';
import { compositionById } from '../scene3d/material-maps';
import { extractScene,MAP_SIZE } from './extract';
import { attachStudioWorld } from '../scene3d/environment-render';
import { initStudioGpu,StudioTarget } from './gpu';
import { viewOf,mapCanvas,cameraOf } from './index';
import { png16 } from './png16';
import * as THREE from 'three';
export const STUDIO_PASSES=['beauty','depth','normal','albedo','object_id','material_id','shadow'] as const;
export type StudioPass=typeof STUDIO_PASSES[number];
export function validatePasses(raw:unknown):StudioPass[]{
  if(!Array.isArray(raw)||!raw.length||raw.length>7||raw.some(p=>!STUDIO_PASSES.includes(p as any)))throw new Error('Choose beauty, depth, normal, albedo, object_id, material_id or shadow');
  return [...new Set(raw)] as StudioPass[];
}
/** Auxiliary passes use the shared portable shader, even when the viewer uses
 * native Metal. No display readback or 8-bit intermediary is involved. */
export async function* renderStudioPasses(PM:any,time:number,width:number,height:number,passes:StudioPass[]){
  validatePasses(passes);
  if(!await initStudioGpu())throw new Error('Render-pass export requires WebGPU');
  const runtime=compositionRuntime(PM,time),scene=extractScene(runtime.scene,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});
  attachStudioWorld(scene,(runtime as any).studioWorld,(runtime as any).metersPerUnit);
  runtime.resizeCamera(width,height);const camera=cameraOf(runtime.camera,width,height);if(!camera)throw new Error('Unsupported pass camera');
  const view=viewOf(runtime,camera,scene.bounds);
  const mapSize=scene.mapSize ?? MAP_SIZE;
  const maps=scene.maps.map(map=>{
    if('source' in map)return mapCanvas(map.source,map.flipY,mapSize);
    const comp=compositionById(PM.proj,map.screen);if(!comp)return new Uint8Array(mapSize*mapSize*4);
    return PM.GL.renderMaterialComposition(comp,time,mapSize,mapSize,time) || new Uint8Array(mapSize*mapSize*4);
  });
  const target=new StudioTarget(width,height),seed=Math.floor(time*1000000)>>>0;
  try{
    target.setScene(scene,maps);target.setView(view);
    for(const pass of passes){
      let pixels:Float32Array;
      if(pass==='beauty'){
        const quality=qualityForComposition(PM);
        target.causticsEnabled=quality.caustics;target.setPrecomputedCaustics(quality.noiseThreshold>=.025);
        target.guideSamples=quality.noiseThreshold>=.06?8:quality.noiseThreshold>=.025?32:64;
        target.setAdaptiveSampling(quality.noiseThreshold>0,quality.minSamples,quality.noiseThreshold);
        await target.traceQuality(quality.maxSamples,true,Math.round(time*9600),quality.timeLimitSeconds*1000,undefined,quality.minSamples,quality.maxBounces);
        target.resolve(0,quality.denoise);pixels=await target.readBeauty();
        for(let i=0;i<pixels.length;i+=4){const alpha=pixels[i+3]!;if(alpha>0)for(let j=0;j<3;j++)pixels[i+j]!/=alpha;}
      }else pixels=await target.dataPass(STUDIO_PASSES.indexOf(pass)-1,seed);
      yield {pass,blob:await png16(width,height,pixels,pass==='beauty'),metadata:{depth:{near:camera.near,far:camera.far,encoding:'(distance - near) / (far - near)'},object_ids:scene.objects,material_ids:Array.from({length:scene.materials.length/STUDIO_MATERIAL_FLOATS},(_,i)=>({id:i+1,base:Array.from(scene.materials.subarray(i*STUDIO_MATERIAL_FLOATS,i*STUDIO_MATERIAL_FLOATS+3)),metallic:scene.materials[i*STUDIO_MATERIAL_FLOATS+3],roughness:scene.materials[i*STUDIO_MATERIAL_FLOATS+4]})),background_id:0,normal:'world XYZ mapped from [-1,1] to [0,1]',albedo:'linear sRGB',shadow:'fraction of direct analytic illumination occluded; 0 lit, 1 blocked',bit_depth:16}};
    }
  }finally{target.destroy();}
}
