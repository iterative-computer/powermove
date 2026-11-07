import * as THREE from 'three';
import { EXTRUSION_SURFACES, isExtrudedLayer, isShared3DPlane, EXTRUSION_DEFAULTS, surfaceFields, type ExtrusionSurface } from './extrusion-data';
import { layerExtrusionOutlines, type ExtrusionOutlineSample } from './extrusion-outlines';
import type { MaterialMaps } from './material-maps';
import type { MaterialPropertyValues } from './material-properties';
import type { CompositionTextureFrame } from './composition-textures';
import { world3D } from '../../legacy/core/space-3d';
import { sizeAnchorOffset } from '../../legacy/core/size-anchor';
export interface ExtrusionMeshSample extends ExtrusionOutlineSample {
  plane?:CompositionTextureFrame;id:string;name:string;matrix:THREE.Matrix4;opacity:number;surfaces:Partial<Record<ExtrusionSurface,{maps:MaterialMaps;values:Partial<MaterialPropertyValues>&{color?:string}}>>;
}
export function compositionExtrusions(PM:any,comp:any,time:number):ExtrusionMeshSample[]{
  const out:ExtrusionMeshSample[]=[],unit=6/Math.max(1,comp.h);
  const bridge=new THREE.Matrix4().makeTranslation(-comp.w*unit/2,3,0).multiply(new THREE.Matrix4().makeScale(unit,-unit,-unit));
  for(const layer of comp.layers || [])if((isExtrudedLayer(layer) || isShared3DPlane(comp,layer)) && (PM.active?PM.active(layer,time):layer.on!==false)){
    let sample= isExtrudedLayer(layer)?layerExtrusionOutlines(PM,layer,time):null;
    let plane:CompositionTextureFrame|undefined;
    if(isShared3DPlane(comp,layer)){
      const bounds=PM.GL?.bounds?.(layer,time);if(!bounds || !PM.GL?.renderMaterialComposition)continue;
      const scale=Math.min(1,1024/Math.max(bounds.w,bounds.h)),width=Math.max(2,Math.round(bounds.w*scale)),height=Math.max(2,Math.round(bounds.h*scale));
      const pixels=PM.GL.renderMaterialComposition(comp,time,width,height,time,layer.id);if(!pixels)continue;
      plane={pixels,width,height,key:`${layer.id}:${time}:${PM._scene3dRevision || 0}`};
      sample={key:JSON.stringify(['plane',bounds.x0,bounds.y0,bounds.w,bounds.h]),color:'#FFFFFF',options:{...EXTRUSION_DEFAULTS},outlines:[{outer:[[bounds.x0,bounds.y0],[bounds.x1,bounds.y0],[bounds.x1,bounds.y1],[bounds.x0,bounds.y1]],holes:[]}]};
    }
    if(!sample)continue;
    const surfaces:ExtrusionMeshSample['surfaces']={};
    for(const surface of EXTRUSION_SURFACES){
      const data=layer.d.extrusion?.surfaces?.[surface];if(!data)continue;
      surfaces[surface]={maps:data.maps || {},values:Object.fromEntries(Object.entries<any>(data.properties || {}).map(([key,prop])=>[key,(()=>{const raw=PM.evP?.(layer,prop,time,`ms.${surface}.${key}`) ?? prop.v;try{return (surfaceFields as any)[key].parse(raw).v;}catch{return prop.v;}})()]))};
    }
    let offset=plane?{x:0,y:0}:sizeAnchorOffset(PM,layer,time);
    if(!plane && layer.type==='text' && layer.d.fontAnchorBounds && PM.raster){const raster=PM.raster(layer,1,time);if(raster?.fontOffset)offset=raster.fontOffset;}
    const matrix=bridge.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,layer,time)))
      .multiply(new THREE.Matrix4().makeTranslation(offset.x,offset.y,0)).multiply(new THREE.Matrix4().makeScale(1,-1,-1));
    out.push({...sample,plane,id:layer.id,name:layer.name,matrix,opacity:PM.worldOpacity?PM.worldOpacity(layer,time):1,surfaces});
  }
  return out;
}
