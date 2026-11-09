import { z } from 'zod';
import { type MaterialMaps, type MaterialSlot } from './material-maps';
import type { MaterialPropertyChannels, MaterialPropertyPatch, MaterialPropertyValues } from './material-properties';

export const EXTRUSION_SURFACES = ['front', 'bevel', 'side', 'back'] as const;
export type ExtrusionSurface = typeof EXTRUSION_SURFACES[number];
export const BEVEL_STYLES = ['none', 'angular', 'concave', 'convex'] as const;
const MATERIAL_SLOTS = ['color','normal','roughness','metalness','emissive','ao'] as const;
const number = (max:number) => z.number().finite().min(0).max(max);
const color = z.string().regex(/^#[0-9a-f]{6}$/i);
const channel = (value:z.ZodType) => z.union([value, z.object({v:value,kf:z.array(z.object({t:number(100000),v:value}).passthrough()).max(2400).default([]),expr:z.string().max(2000).nullable().optional()}).passthrough()])
  .transform((raw:any) => typeof raw === 'object' && raw !== null && 'v' in raw ? {...raw,expr:raw.expr ?? null} : {v:raw,kf:[],expr:null});
export const geometryFields = {
  depth: channel(number(10000)), bevelStyle: channel(z.enum(BEVEL_STYLES)), bevelDepth: channel(number(1000)),
  holeBevelDepth: channel(number(100)), quality: channel(z.enum(['draft','standard','high']))
};
export const surfaceFields = {
  color:channel(color),opacity:channel(number(100)),roughness:channel(number(1)),metallic:channel(number(1)),
  emissionColor:channel(color),emissionIntensity:channel(number(100)),ambientResponse:channel(number(100))
};
const map=z.union([z.string().min(1).max(160),z.object({compositionId:z.string().min(1).max(160)}).strict(),z.object({layerId:z.string().min(1).max(160)}).strict()]);
const maps=z.object(Object.fromEntries(MATERIAL_SLOTS.map(slot=>[slot,map.optional()]))).strict().superRefine((value,ctx)=>{
  for(const [slot,source] of Object.entries(value))if(typeof source==='object' && slot!=='color' && slot!=='emissive')ctx.addIssue({code:'custom',path:[slot],message:'Composition and layer textures support base color and emissive slots'});
});
const sparseSurface=z.object(Object.fromEntries(Object.entries(surfaceFields).map(([key,schema])=>[key,schema.optional()]))).strict();
const surface=z.object({maps:maps.default({}),properties:sparseSurface.default({})}).strict();
const schema=z.object({version:z.literal(1).default(1),enabled:z.boolean().default(false),
  p:z.object(geometryFields).strict(),surfaces:z.object(Object.fromEntries(EXTRUSION_SURFACES.map(key=>[key,surface.optional()]))).strict().default({})}).strict();
export interface ExtrusionValues {depth:number;bevelStyle:typeof BEVEL_STYLES[number];bevelDepth:number;holeBevelDepth:number;quality:'draft'|'standard'|'high'}
export type ExtrusionKey=keyof ExtrusionValues;
export type ExtrusionPatch={enabled?:boolean}&Partial<{[K in ExtrusionKey]:ExtrusionValues[K]|{v:ExtrusionValues[K];kf:any[];expr?:string|null}}>;
export type SurfacePropertyChannels=MaterialPropertyChannels & {color?:{v:string;kf:any[];expr:string|null}};
export type SurfacePropertyPatch=MaterialPropertyPatch & {color?:string|{v:string;kf:any[];expr?:string|null}|null};
export interface ExtrusionData {version:1;enabled:boolean;p:Record<ExtrusionKey,{v:any;kf:any[];expr:string|null}>;surfaces:Partial<Record<ExtrusionSurface,{maps:MaterialMaps;properties:SurfacePropertyChannels}>>}
export interface ExtrusionState {enabled:boolean;properties:ExtrusionData['p'];values:ExtrusionValues;paths:Record<ExtrusionKey,string>}
export interface ExtrusionSurfaceInfo {surface:ExtrusionSurface;index:number;name:string;slots:MaterialSlot[];maps:MaterialMaps}
export interface SurfacePropertyState {properties:SurfacePropertyChannels;values:MaterialPropertyValues & {color:string};paths:Record<keyof SurfacePropertyChannels,string>}
export const EXTRUSION_DEFAULTS:ExtrusionValues={depth:0,bevelStyle:'none',bevelDepth:2,holeBevelDepth:100,quality:'standard'};
const clone=<T>(value:T):T=>JSON.parse(JSON.stringify(value));
export function extrusionEligible(layer:any):boolean {return layer?.type==='text' || layer?.type==='shape';}
export function isExtrudedLayer(layer:any):boolean {return extrusionEligible(layer) && layer.threeD===true && layer.d?.extrusion?.enabled===true;}
const planeTypes=new Set(['text','shape','solid','image','video','precomp']);
/** Legacy flat projects keep their original renderer. In an extrusion scene,
 * native 3D planes share its camera/depth. Explicit Material Options opt a
 * plane into Studio in other 3D compositions without changing old projects. */
export function isShared3DPlane(comp:any,layer:any):boolean {
  return layer?.threeD===true && planeTypes.has(layer.type) && !isExtrudedLayer(layer) && ((comp?.layers || []).some(isExtrudedLayer) || !!layer.d?.materialOptions);
}
/** Absent data is read as disabled, without rewriting old projects. */
export function parseExtrusion(raw:any):ExtrusionData {
  const p=Object.fromEntries(Object.entries(EXTRUSION_DEFAULTS).map(([key,value])=>[key,{v:value,kf:[],expr:null}]));
  return schema.parse({...raw,p:{...p,...raw?.p}}) as unknown as ExtrusionData;
}
export function extrusionProperties(layer:any):any[] {
  if(!extrusionEligible(layer) || !layer.d?.extrusion)return [];
  const data=layer.d.extrusion,out:any[]=[];
  const labels:Record<string,string>={depth:'Extrusion Depth',bevelStyle:'Bevel Style',bevelDepth:'Bevel Depth',holeBevelDepth:'Hole Bevel Depth',quality:'Geometry Quality',color:'Color',opacity:'Opacity',roughness:'Roughness',metallic:'Metallic',emissionColor:'Emission Color',emissionIntensity:'Emission Intensity',ambientResponse:'Ambient Response'};
  for(const [key,prop] of Object.entries<any>(data.p || {}))out.push({key:`geo.${key}`,prop,label:labels[key],group:'Geometry Options'});
  for(const name of EXTRUSION_SURFACES)for(const [key,prop] of Object.entries<any>(data.surfaces?.[name]?.properties || {}))out.push({key:`ms.${name}.${key}`,prop,label:labels[key],group:`${name[0]!.toUpperCase()+name.slice(1)} Material`});
  return out;
}
export function validateExtrusionChannel(path:string,value:any):boolean {
  const geo=/^geo\.([^.]+)$/.exec(path),surface=/^ms\.(front|bevel|side|back)\.([^.]+)$/.exec(path);
  if(!geo && !surface)return false;
  const field=geo?(geometryFields as any)[geo[1]!]: (surfaceFields as any)[surface![2]!];
  if(!field)throw new Error(`Unknown extrusion property: ${path}`);
  field.parse(value);return true;
}
export function readExtrusion(PM:any,layer:any,time=PM.time || 0):ExtrusionState|null {
  if(!extrusionEligible(layer))return null;
  const data=parseExtrusion(layer.d?.extrusion),values={} as ExtrusionValues;
  for(const key of Object.keys(geometryFields) as ExtrusionKey[]){
    const prop=data.p[key],raw=PM.evP?.(layer,prop,time,`geo.${key}`) ?? prop.v;
    try{(values as any)[key]=geometryFields[key].parse(raw).v;}catch{(values as any)[key]=prop.v;}
  }
  return {enabled:data.enabled,properties:clone(data.p),values,paths:Object.fromEntries(Object.keys(geometryFields).map(key=>[key,`geo.${key}`])) as ExtrusionState['paths']};
}
