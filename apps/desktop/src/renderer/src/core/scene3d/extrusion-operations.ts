import { EXTRUSION_SURFACES, parseExtrusion, geometryFields, surfaceFields, type ExtrusionSurface, type SurfacePropertyState } from './extrusion-data';
import { MATERIAL_SLOTS, materialOwner, materialLayerStatus, compositionById } from './material-maps';
import { resolveContent } from '../../legacy/core/content-properties';
import type { Scene3DEdit } from './operations';
const clone=<T>(v:T):T=>JSON.parse(JSON.stringify(v));
export function listExtrusionSurfaces(PM:any,id:string){
  const layer=materialOwner(PM.proj,id)?.layers.find((l:any)=>l.id===id);
  if(!layer || !['text','shape'].includes(layer.type))return [];
  return EXTRUSION_SURFACES.map((surface,index)=>({surface,index,name:surface[0]!.toUpperCase()+surface.slice(1),slots:[...MATERIAL_SLOTS],maps:clone(layer.d?.extrusion?.surfaces?.[surface]?.maps || {})}));
}
export function readSurfaceProperties(PM:any,id:string,surface:ExtrusionSurface,time=PM.time || 0):SurfacePropertyState|null{
  const layer=materialOwner(PM.proj,id)?.layers.find((l:any)=>l.id===id);
  if(!layer || !['text','shape'].includes(layer.type) || !EXTRUSION_SURFACES.includes(surface))return null;
  const properties=layer.d?.extrusion?.surfaces?.[surface]?.properties || {},d=resolveContent(PM,layer,time);
  const values:any={color:d.color || '#E8E2CF',opacity:100,roughness:.4,metallic:0,emissionColor:'#000000',emissionIntensity:0,ambientResponse:100};
  for(const [key,prop] of Object.entries<any>(properties)){const raw=PM.evP?.(layer,prop,time,`ms.${surface}.${key}`) ?? prop.v;try{values[key]=(surfaceFields as any)[key].parse(raw).v;}catch{values[key]=prop.v;}}
  return {properties:clone(properties),values,paths:Object.fromEntries(Object.keys(surfaceFields).map(key=>[key,`ms.${surface}.${key}`])) as SurfacePropertyState['paths']};
}
/** Install sparse channels first; all values/keyframes then cross the ordinary
 * edit boundary so autokey, expressions, source protection and undo stay shared. */
export function extrusionCommands(PM:any,layer:any,args:Scene3DEdit):{commands:any[];id:string}{
  const data=parseExtrusion(layer.d.extrusion),commands:any[]=[],patch=args.patch || {};
  const write=(path:string,value:any)=>{
    if(typeof value==='object' && value!==null){
      commands.push({type:'set_property',target:layer.id,path,value:value.v,mode:'value'});
      commands.push({type:'replace_keyframes',target:layer.id,path,keyframes:(value.kf || []).map((k:any)=>({time:k.t,value:k.v,ease:k.ease || 'linear',hold:k.hold || false})),expression:value.expr || null});
    }else commands.push({type:'set_property',target:layer.id,path,value,mode:PM.autokey?'keyframe':'auto'});
  };
  if(args.operation==='set_extrusion'){
    if(Object.keys(patch).some(key=>key!=='enabled' && !Object.hasOwn(geometryFields,key)))throw new Error('Unknown extrusion property');
    if(patch.enabled!==undefined){if(typeof patch.enabled!=='boolean')throw new Error('Extrusion enabled must be a boolean');data.enabled=patch.enabled;}
    for(const key of Object.keys(geometryFields))if(patch[key]!==undefined){(geometryFields as any)[key].parse(patch[key]);write(`geo.${key}`,patch[key]);}
    if(patch.enabled===true && !layer.threeD)commands.push({type:'set_layer',target:layer.id,patch:{threeD:true}});
  }else{
    const surface=args.surface ?? (args.materialIndex==null?'front':EXTRUSION_SURFACES[args.materialIndex]);
    if(!surface || !EXTRUSION_SURFACES.includes(surface))throw new Error('Choose Front, Bevel, Side or Back');
    const material=data.surfaces[surface] ||= {maps:{},properties:{}};
    if(args.operation==='set_material_map' || args.operation==='set_surface_material_map'){
      if(!args.slot || !MATERIAL_SLOTS.includes(args.slot))throw new Error('Choose a material map slot');
      if(typeof args.map==='object' && args.map){
        if(args.slot!=='color' && args.slot!=='emissive')throw new Error('Layer and composition maps support color and emissive');
        if('compositionId' in args.map && !compositionById(PM.proj,args.map.compositionId))throw new Error('Composition is unavailable');
        if('layerId' in args.map){const status=materialLayerStatus(PM.proj,layer.id,args.map.layerId);if(status==='cycle')throw new Error('Layer material reference would create a cycle');if(status!=='ready')throw new Error('Choose an eligible source layer in the same composition');}
      }
      if(args.map==null)delete material.maps[args.slot];else material.maps[args.slot]=clone(args.map);
    }else if(args.operation==='set_material_properties' || args.operation==='set_surface_material_properties'){
      const current=readSurfaceProperties(PM,layer.id,surface)!;
      for(const [key,value] of Object.entries<any>(patch)){
        if(!Object.hasOwn(surfaceFields,key))throw new Error('Unknown surface material property');
        if(value===null){delete (material.properties as any)[key];continue;}
        (surfaceFields as any)[key].parse(value);
        if(!(material.properties as any)[key])(material.properties as any)[key]={v:(current.values as any)[key],kf:[],expr:null};
        write(`ms.${surface}.${key}`,value);
      }
    }else throw new Error('Choose an extrusion or surface material operation');
  }
  const checked=parseExtrusion(data);
  if(JSON.stringify(checked)!==JSON.stringify(layer.d.extrusion))commands.unshift({type:'set_content',target:layer.id,patch:{extrusion:checked}});
  return {commands,id:layer.id};
}
