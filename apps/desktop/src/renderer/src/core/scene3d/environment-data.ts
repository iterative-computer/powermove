import { z } from 'zod';

const n = (min=-100000,max=100000) => z.number().finite().min(min).max(max);
const c = z.string().regex(/^#[0-9a-f]{6}$/i);
const channel = (type:z.ZodType, fallback:unknown) => z.union([type,z.object({v:type,kf:z.array(z.object({t:n(0),v:type}).passthrough()).max(2400),expr:z.string().max(2000).nullable().optional()}).passthrough()])
  .transform((v:any)=>v && typeof v==='object' && 'v' in v?{...v,expr:v.expr??null}:{v,kf:[],expr:null}).default(()=>({v:fallback,kf:[],expr:null}));
const props = (fields:Record<string,[z.ZodType,unknown]>) => z.object(Object.fromEntries(Object.entries(fields).map(([k,[type,value]])=>[k,channel(type,value)]))).strict();
export const environmentSourceSchema = z.union([z.string().min(1).max(160),z.object({compositionId:z.string().min(1).max(160)}).strict(),z.object({layerId:z.string().min(1).max(160)}).strict()]);
export const worldSchema = z.object({
  mode:z.enum(['none','sky','hdri']).default('sky'),
  preset:z.enum(['clear-noon','golden-hour','overcast','blue-hour','night']).default('clear-noon'),
  source:environmentSourceSchema.nullable().default(null),
  p:props({intensity:[n(0,10000),1],rotation:[n(-360000,360000),0],visible:[z.boolean(),true],metersPerUnit:[n(.000001,1000),.1],
    elevation:[n(-90,90),45],azimuth:[n(-360000,360000),0],haze:[n(1,10),2],sunIntensity:[n(0,10000),1],sunRadius:[n(.05,10),.266],overcast:[n(0,1),0],
    fogDensity:[n(0,100),0],fogColor:[c,'#E6E6E6'],fogAnisotropy:[n(-.95,.95),0],fogBaseHeight:[n(),0],fogHeightFalloff:[n(0,100),0],fogDistance:[n(.001,100000),10000],
    adaptive:[z.boolean(),false],minSamples:[n(1,4096),64],relativeError:[n(.00001,1),.025],absoluteError:[n(.000001,1),.001]}).prefault({})
}).strict();
export const fogSchema = z.object({p:props({shape:[z.enum(['box','sphere']),'box'],width:[n(.0001),2],height:[n(.0001),2],depth:[n(.0001),2],density:[n(0,100),.02],color:[c,'#E6E6E6'],anisotropy:[n(-.95,.95),0]}).prefault({})}).strict();
export const terrainRegionSchema = z.object({p:props({minHeight:[n(),-100000],maxHeight:[n(),100000],minSlope:[n(0,90),0],maxSlope:[n(0,90),90],color:[c,'#737B47'],roughness:[n(.02,1),.8],metalness:[n(0,1),0]}).prefault({})}).strict();
export const terrainSchema = z.object({
  heightSource:environmentSourceSchema.nullable().default(null),heights:z.array(n(-1,1)).max(66049).default([]),
  regions:z.array(terrainRegionSchema).max(8).default([]),
  p:props({width:[n(.001,10000),20],depth:[n(.001,10000),20],resolution:[n(2,256),64],amplitude:[n(0,10000),2],frequency:[n(.00001,1000),.1],octaves:[n(1,12),5],seed:[n(0,4294967295),1],ridged:[z.boolean(),false]}).prefault({})
}).strict();
export const scatterSchema = z.object({model:z.string().min(1).max(160),surface:z.string().min(1).max(160),
  p:props({count:[n(0,100000),100],density:[n(0,1),1],seed:[n(0,4294967295),1],scaleMin:[n(.0001,1000),.8],scaleMax:[n(.0001,1000),1.2],rotationMin:[n(-360000,360000),0],rotationMax:[n(-360000,360000),360],minHeight:[n(),-100000],maxHeight:[n(),100000],minSlope:[n(0,90),0],maxSlope:[n(0,90),60]}).prefault({})
}).strict();
export const materialOptionsSchema = z.object({p:props({castsShadows:[z.enum(['off','on','only']),'on'],acceptsShadows:[z.boolean(),true],acceptsLights:[z.boolean(),true],appearsInReflections:[z.boolean(),true]}).prefault({})}).strict();
export const ENVIRONMENT_SCHEMAS = {world:worldSchema,fog:fogSchema,terrain:terrainSchema,scatter:scatterSchema,materialOptions:materialOptionsSchema};
export type WorldData = z.output<typeof worldSchema>;
export type WorldInput=z.input<typeof worldSchema>;
export type FogData=z.output<typeof fogSchema>;
export type TerrainData=z.output<typeof terrainSchema>;
export type ScatterData=z.output<typeof scatterSchema>;
export type MaterialOptionsData=z.output<typeof materialOptionsSchema>;
export const isEnvironmentNode=(node:any)=>!!(node?.world || node?.fog || node?.terrain || node?.scatter);
/** Timeline names, matching the inspector's rows (After Effects' wording for Material Options). */
const GROUP_LABELS:Record<string,string>={world:'World',fog:'Fog Volume',terrain:'Terrain',scatter:'Scatter',materialOptions:'Material Options'};
const CHANNEL_LABELS:Record<string,Record<string,string>>={
  world:{intensity:'Intensity',rotation:'Rotation',visible:'Visible to Camera',metersPerUnit:'Meters per Unit',elevation:'Sun Height',azimuth:'Sun Direction',haze:'Haze',sunIntensity:'Sun Strength',sunRadius:'Sun Size',overcast:'Clouds',
    fogDensity:'Fog',fogColor:'Fog Color',fogAnisotropy:'Fog Forward Glow',fogBaseHeight:'Fog Base',fogHeightFalloff:'Fog Falloff',fogDistance:'Fog Distance',adaptive:'Adaptive Sampling',minSamples:'Minimum Samples',relativeError:'Relative Error',absoluteError:'Absolute Error'},
  fog:{shape:'Shape',width:'Width',height:'Height',depth:'Depth',density:'Density',color:'Color',anisotropy:'Forward Glow'},
  terrain:{width:'Width',depth:'Depth',resolution:'Resolution',amplitude:'Height',frequency:'Feature Size',octaves:'Detail',seed:'Variation',ridged:'Ridges'},
  region:{minHeight:'Above',maxHeight:'Below',minSlope:'Steeper Than',maxSlope:'Flatter Than',color:'Color',roughness:'Roughness',metalness:'Metalness'},
  scatter:{count:'Count',density:'Density',seed:'Variation',scaleMin:'Smallest',scaleMax:'Largest',rotationMin:'Turn From',rotationMax:'Turn To',minHeight:'Above',maxHeight:'Below',minSlope:'Steeper Than',maxSlope:'Flatter Than'},
  materialOptions:{castsShadows:'Casts Shadows',acceptsShadows:'Accepts Shadows',acceptsLights:'Accepts Lights',appearsInReflections:'Appears in Reflections'}
};
export function environmentProperties(node:any):any[]{
  const out:any[]=[];
  const collect=(p:any,prefix:string)=>{
    const region=/^terrain\.region(\d)$/.exec(prefix);
    const labels=CHANNEL_LABELS[region?'region':prefix] ?? {};
    const group=region?`Ground Cover ${Number(region[1])+1}`:GROUP_LABELS[prefix] ?? prefix;
    for(const [key,prop] of Object.entries<any>(p??{}))if(prop && Array.isArray(prop.kf))out.push({key:`${prefix}.${key}`,prop,label:labels[key] ?? key,group});
  };
  for(const kind of Object.keys(ENVIRONMENT_SCHEMAS))collect(node?.[kind]?.p,kind);
  for(const [i,region] of (node?.terrain?.regions??[]).entries())collect(region.p,`terrain.region${i}`);
  return out;
}
export function validateEnvironmentChannel(path:string,property:unknown):boolean{
  path=path.replace(/^o\.[^.]+\./,'');
  const region=/^terrain\.region[0-7]\.([^.]+)$/.exec(path);
  const [kind,key,...rest]=path.split('.');
  const schema=region?terrainRegionSchema:ENVIRONMENT_SCHEMAS[kind as keyof typeof ENVIRONMENT_SCHEMAS];
  if(!schema)return false;
  const field=schema.shape.p.unwrap().shape[region?.[1]??key!];
  if(!field || !region && rest.length)throw new Error(`Unknown environment channel: ${path}`);
  field.parse(property);return true;
}
/** All source references remain structural; all evaluated settings are normal channels. */
export function evaluateEnvironmentNode(node:any,evaluate:(prop:any,path:string)=>any):any{
  const result={...node};
  for(const key of Object.keys(ENVIRONMENT_SCHEMAS))if(node[key]){
    result[key]={...node[key],p:Object.fromEntries(Object.entries<any>(node[key].p).map(([k,prop])=>[k,{v:evaluate(prop,`${key}.${k}`),kf:[],expr:null}]))};
    if(key==='terrain')result[key].regions=node[key].regions.map((r:any,i:number)=>({...r,p:Object.fromEntries(Object.entries<any>(r.p).map(([k,prop])=>[k,{v:evaluate(prop,`terrain.region${i}.${k}`),kf:[],expr:null}]))}));
  }
  return result;
}
export const channelValues=(p:any)=>Object.fromEntries(Object.entries<any>(p??{}).map(([k,v])=>[k,v.v]));

export function validateEnvironmentNode(node:any):void{
  const checkRanges=(p:any,label:string)=>{
    for(const [a,b] of [['minSlope','maxSlope'],['minHeight','maxHeight'],['scaleMin','scaleMax'],['rotationMin','rotationMax']] as const)
      if(p[a] && p[b] && p[a].v>p[b].v)throw new Error(`${label}: ${a} must not exceed ${b}`);
  };
  if(node.terrain){
    const data=node.terrain,resolution=Math.round(data.p.resolution.v);
    if(data.heights.length && data.heights.length!==(resolution+1)**2)throw new Error('Terrain height count must match (resolution + 1) squared');
    for(const region of data.regions)checkRanges(region.p,'Terrain region');
  }
  if(node.scatter)checkRanges(node.scatter.p,'Scatter');
}
