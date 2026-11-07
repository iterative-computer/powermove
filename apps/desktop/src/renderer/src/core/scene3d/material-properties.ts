import * as THREE from 'three';
import { materialOwner } from './material-maps';
export const MATERIAL_PROPERTIES=['opacity','roughness','metallic','emissionColor','emissionIntensity','ambientResponse'] as const;
export type MaterialProperty=typeof MATERIAL_PROPERTIES[number];
export interface MaterialPropertyValues {opacity:number;roughness:number;metallic:number;emissionColor:string;emissionIntensity:number;ambientResponse:number}
export type MaterialPropertyChannels=Partial<Record<MaterialProperty,{v:any;kf:any[];expr:string|null}>>;
export type MaterialPropertyPatch=Partial<Record<MaterialProperty,number|string|{v:any;kf:any[];expr?:string|null}|null>>;
export interface MaterialPropertyState {properties:MaterialPropertyChannels;values:MaterialPropertyValues;paths:Record<MaterialProperty,string>}
export const materialPropertyPath=(index:number|null,key:MaterialProperty)=>`mi.${index==null?'':`${index}.`}${key}`;
export function readMaterialProperties(PM:any,layerId:string,index:number|null=null,time=PM.time || 0):MaterialPropertyState|null {
  const layer=materialOwner(PM.proj,layerId)?.layers.find((l:any)=>l.id===layerId),object=layer?.d?.data?.object;
  if(!object)return null;
  const asset=object.source?.assetId && PM.assets?.get?.(object.source.assetId);
  let original:any;
  asset?.object3d?.traverse((mesh:any)=>{for(const material of (Array.isArray(mesh.material)?mesh.material:[mesh.material]))
    if(material && !original && (index==null || material.userData.sourceMaterialIndex===index))original=material;});
  const ev=(prop:any,path:string)=>prop?(PM.evP?.(layer,prop,time,path) ?? prop.v):undefined;
  const m=object.material?.p || {},source=object.useSourceMaterials && original;
  const values:MaterialPropertyValues={opacity:100,roughness:source?original.roughness ?? .4:ev(m.roughness,'m.roughness') ?? .4,
    metallic:source?original.metalness ?? 0:ev(m.metalness,'m.metalness') ?? 0,
    emissionColor:source?`#${original.emissive?.getHexString() || '000000'}`:ev(m.emissive,'m.emissive') ?? '#000000',
    emissionIntensity:source?original.emissiveIntensity ?? 0:ev(m.emissiveIntensity,'m.emissiveIntensity') ?? 0,ambientResponse:100};
  const properties=index==null?object.materialProperties || {}:object.sourceMaterialProperties?.[index] || {};
  for(const key of MATERIAL_PROPERTIES){const prop=(index==null?undefined:object.materialProperties?.[key]);if(prop)(values as any)[key]=ev(prop,materialPropertyPath(null,key));
    if(properties[key])(values as any)[key]=ev(properties[key],materialPropertyPath(index,key));}
  return {properties:JSON.parse(JSON.stringify(properties)),values,paths:Object.fromEntries(MATERIAL_PROPERTIES.map(key=>[key,materialPropertyPath(index,key)])) as Record<MaterialProperty,string>};
}
/** Only indirect ambient/probe/hemisphere lighting is scaled. Direct lights,
 * shadows and emission keep their normal response. No global light changes. */
export function setAmbientResponse(material:THREE.Material,response:number):void {
  const data=material.userData;
  if(!data.materialAmbientUniform){
    if(response===1)return;
    const uniform={value:response};data.materialAmbientUniform=uniform;
    const previous=material.onBeforeCompile,cacheKey=material.customProgramCacheKey();
    material.onBeforeCompile=function(shader,renderer){
      previous.call(this,shader,renderer);shader.uniforms.pmAmbientResponse=uniform;
      shader.fragmentShader='uniform float pmAmbientResponse;\n'+shader.fragmentShader;
      let chunk=THREE.ShaderChunk.lights_fragment_begin;
      for(const expression of ['getAmbientLightIrradiance( ambientLightColor )','getLightProbeIrradiance( lightProbe, geometryNormal )','getHemisphereLightIrradiance( hemisphereLights[ i ], geometryNormal )'])
        chunk=chunk.replace(expression,`( ${expression} * pmAmbientResponse )`);
      shader.fragmentShader=shader.fragmentShader.replace('#include <lights_fragment_begin>',chunk);
    };
    material.customProgramCacheKey=()=>`${cacheKey}:pm-ambient-v1`;material.needsUpdate=true;
  }
  data.materialAmbientUniform.value=response;
}
export function applyMaterialProperties(material:any,values:Partial<MaterialPropertyValues>,original:any):void {
  material.opacity=values.opacity==null?original.opacity:values.opacity/100;
  const transparent=original.transparent || material.opacity<1 || material.map?.userData?.materialHasAlpha===true;
  if(material.transparent!==transparent)material.needsUpdate=true;
  material.transparent=transparent;material.depthWrite=(values.opacity!=null && material.opacity<1 || material.map?.userData?.materialHasAlpha===true)?false:original.depthWrite;
  if(material.isMeshStandardMaterial){
    material.roughness=values.roughness ?? original.roughness;material.metalness=values.metallic ?? original.metalness;
    if(values.emissionColor!=null)material.emissive.set(values.emissionColor);
    if(values.emissionIntensity!=null)material.emissiveIntensity=values.emissionIntensity;
    setAmbientResponse(material,(values.ambientResponse ?? 100)/100);
  }
}
