import { create, type Font } from 'fontkit';
import { resolveContent } from '../../legacy/core/content-properties';
import { isExtrudedLayer } from './extrusion-data';
import { sourceTime } from '../../legacy/core/retiming';
import { layerMaterialMaps } from './material-maps';
import { materialCompositionSample } from './composition-textures';
import { styleDescriptor } from '../../typography/font-catalog';
import { reportLayerFailure } from './layer-diagnostic';

// fontkit v2 accepts Uint8Array in browsers; its community types still say Buffer.
const decode=create as unknown as (bytes:Uint8Array,name?:string)=>Font|import('fontkit').FontCollection;
const decodedSources=new WeakMap<ArrayBuffer,Font|import('fontkit').FontCollection>();
const parse=(bytes:ArrayBuffer,name?:string)=>{let font=decodedSources.get(bytes);if(!font){font=decode(new Uint8Array(bytes));decodedSources.set(bytes,font);}if(!font)throw new Error('Font face is unavailable');if('fonts' in font)return font.fonts.find(face=>face.postscriptName===name) || font.fonts[0]!;return font;};

type Entry={font?:Font;error?:string;pending?:Promise<void>};
const fonts=new WeakMap<object,Map<string,Entry>>();
const resources=new WeakMap<object,{catalog?:Promise<any[]>;faces:Map<string,Entry>;variables:Map<string,Font>}>();
function resourceCache(PM:any){let state=resources.get(PM);if(!state){state={faces:new Map(),variables:new Map()};resources.set(PM,state);}return state;}
const normalize=(value:any)=>String(value || '').trim().replace(/^['"]|['"]$/g,'').toLowerCase();
export const extrusionFontKey=(d:any)=>JSON.stringify([normalize(d.font || 'SF Pro Display'),Number(d.weight)||500,!!d.italic]);
function cache(PM:any):Map<string,Entry>{let c=fonts.get(PM);if(!c){c=new Map();fonts.set(PM,c);}return c;}
/** Font bytes are editor-local resources, never embedded into project JSON. */
export function requestExtrusionFont(PM:any,d:any):Entry {
  const key=extrusionFontKey(d),c=cache(PM);if(c.has(key))return c.get(key)!;
  const entry:Entry={};c.set(key,entry);if(c.size>128)c.delete(c.keys().next().value!);
  const state=resourceCache(PM),familyKey=JSON.stringify([normalize(d.font || 'SF Pro Display'),!!d.italic]);
  const variable=state.variables.get(familyKey);if(variable?.variationAxes.wght){entry.font=variable;return entry;}
  // The standalone player already owns packaged font bytes.
  const packaged=PM.fontSource?.(d.font);
  if(packaged){try{entry.font=parse(packaged);}catch(error){entry.error=String(error);}return entry;}
  entry.pending=(async()=>{
    try{
      if(typeof (globalThis as any).queryLocalFonts!=='function')throw new Error('Font outlines are unavailable; provide the font in the player package');
      const all=await (state.catalog ??= (globalThis as any).queryLocalFonts()),family=normalize(d.font || 'SF Pro Display');
      const candidates=all.filter((face:any)=>[face.family,face.fullName,face.postscriptName].some(name=>normalize(name)===family));
      candidates.sort((a:any,b:any)=>{
        const score=(face:any)=>{const style=styleDescriptor(face.style || face.fullName || '');return Math.abs(style.weight-(Number(d.weight)||500))+(style.italic!==!!d.italic?2000:0);};
        return score(a)-score(b);
      });
      let reason=candidates.length?'':'No installed face matched';
      for(const face of candidates){try{
        const faceKey=String(face.postscriptName || face.fullName || face.family);
        let parsed=state.faces.get(faceKey);
        if(!parsed){
          parsed={};state.faces.set(faceKey,parsed);if(state.faces.size>32)state.faces.delete(state.faces.keys().next().value!);
          const target=parsed;target.pending=(async()=>{target.font=parse(await (await face.blob()).arrayBuffer(),face.postscriptName);})();
        }
        await parsed.pending;entry.font=parsed.font;
        if(entry.font?.variationAxes.wght){state.variables.set(familyKey,entry.font);if(state.variables.size>32)state.variables.delete(state.variables.keys().next().value!);}
        return;
      }catch(error){reason=String(error);}}
      throw new Error(`Cannot read outlines for font “${d.font || 'SF Pro Display'}”: ${reason}`);
    }catch(error){entry.error=error instanceof Error?error.message:String(error);}
    finally{PM._scene3dRevision=(PM._scene3dRevision || 0)+1;PM.invalidate?.('render');}
  })();
  return entry;
}
export function installExtrusionFont(PM:any,d:any,bytes:ArrayBuffer):void {cache(PM).set(extrusionFontKey(d),{font:parse(bytes)});}
export function hasExtrudedText(PM:any,project:any):boolean {
  const has=(comp:any)=>(comp.layers || []).some((layer:any)=>isExtrudedLayer(layer) && layer.type==='text');
  return has(project) || Object.values(PM.proj?.comps || {}).some(has);
}
/** Capture/export awaits outlines at each composition's own remapped time. */
export async function prepareExtrusionFonts(PM:any,project:any,time:number,seen=new Set<string>(),sourceLayerId?:string):Promise<void>{
  const key=JSON.stringify([project.compId || project.id,time,sourceLayerId]);
  if(seen.has(key))return;if(seen.size>128)throw new Error('Extrusion font preparation exceeded composition nesting budget');seen.add(key);
  const pending:Promise<void>[]=[];
  PM.scope?.push?.(project);
  try{for(const layer of project.layers || []){
    if(sourceLayerId && layer.id!==sourceLayerId)continue;
    if(!sourceLayerId && (PM.active? !PM.active(layer,time):layer.on===false))continue;
    // A layer-as-material supplies its flat source artwork; it needs no mesh.
    if(!sourceLayerId && isExtrudedLayer(layer) && layer.type==='text'){
      const entry=requestExtrusionFont(PM,resolveContent(PM,layer,time));
      pending.push(Promise.resolve(entry.pending).then(()=>{if(entry.error)reportLayerFailure(PM,layer.id,'font preparation',entry.error);}));
      for(const range of layer.d.styles || []){
        const d=resolveContent(PM,layer,time),style=Object.fromEntries(Object.entries<any>(range.p || {}).map(([key,prop])=>[key,PM.evP?.(layer,prop,time,`ts.${range.id}.${key}`) ?? prop.v]));
        const styled=requestExtrusionFont(PM,{...d,...style});pending.push(Promise.resolve(styled.pending).then(()=>{if(styled.error)reportLayerFailure(PM,layer.id,'font preparation',styled.error);}));
      }
    }
    if(layer.type==='precomp'){
      const sub=PM.proj?.comps?.[layer.d?.comp];
      if(sub)pending.push(prepareExtrusionFonts(PM,sub,sourceTime(PM,layer,time),seen));
    }
    for(const map of layerMaterialMaps(layer))if(typeof map==='object'){
      const sample=materialCompositionSample(PM,layer,map,time);
      if(sample)pending.push(prepareExtrusionFonts(PM,sample.comp,sample.time,seen,sample.sourceLayerId));
    }
  }}finally{PM.scope?.pop?.();}
  await Promise.all(pending);
}
