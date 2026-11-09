import { compositionById, materialSourceLayers } from './material-maps';
import { sourceTime } from '../../legacy/core/retiming';
import { staticExportPlan, staticMaskSupported } from '../../legacy/core/static-export';
import { staticEffectSupported, type StaticMaskProgram } from '../../legacy/core/static-effects';
import type { EffectRef } from '../../legacy/core/export-effects';

type Plan = { key:(time:number)=>string; effects:EffectRef[]; programs:StaticMaskProgram[]; content:string };

/** Prove that the entire source dependency tree is stationary. A precomp still
 * follows its own clock, including trim/speed and child entrances/exits. */
export function materialStaticPlan(PM:any,comp:any,sourceId?:string):{plan:Plan|null;images:any[]} {
  const visiting=new Set<any>(),observedImages=new Set<any>();
  const memo=new Map<any,Map<string,Plan|null>>();
  const build=(comp:any,sourceId?:string):Plan|null=>{
    if(!comp || visiting.has(comp) || visiting.size>=8)return null;
    let entries=memo.get(comp);if(!entries){entries=new Map();memo.set(comp,entries);}
    const memoKey=sourceId ?? '';if(entries.has(memoKey))return entries.get(memoKey)!;
    visiting.add(comp);
    try{
      const byId=new Map<string,any>(comp.layers.map((layer:any)=>[layer.id,layer]));
      const selected=sourceId?materialSourceLayers(comp,sourceId):[...comp.layers];
      const ids=new Set<string>(selected.map((layer:any)=>layer.id));
      // Mattes can live outside a selected group. Transform parents can too.
      // Include each provider's subtree and its own dependencies before proving
      // constancy, so hidden/animated providers cannot leave stale pixels.
      for(let i=0;i<selected.length;i++){
        const layer=selected[i];
        const add=(id:string,subtree=false)=>{
          if(!id)return;
          for(const dep of subtree?materialSourceLayers(comp,id):[byId.get(id)])if(dep&&!ids.has(dep.id)){
            ids.add(dep.id);selected.push(dep);
          }
        };
        add(layer.parent);add(layer.group);
        add(layer.matteSource,true);
        for(const matte of layer.matteSources || [])add(matte.source,true);
        for(const effect of layer.fx || [])for(const input of Object.values<any>(effect.inputs || {}))add(input?.layerId,true);
      }
      // Preserve authored stack order in the content signature.
      const layers=comp.layers.filter((layer:any)=>ids.has(layer.id));
      const children=new Map<any,Plan>();
      const images=layers.filter((layer:any)=>layer.type==='image');
      for(const image of images)observedImages.add(image);
      const effects:EffectRef[]=[],programs=new Set<StaticMaskProgram>();
      const key=staticExportPlan({...comp,layers},false,layer=>{
        const asset=PM.assets?.get?.(layer.d?.asset),el=asset?.el;
        return asset?.kind==='image' && !asset.imageSequence && asset.format!=='svg'
          && typeof ImageBitmap!=='undefined' && el instanceof ImageBitmap && el.width>0 && el.height>0;
      },effect=>staticEffectSupported(effect?.type),staticMaskSupported,{
        precomp:layer=>{
          const child=build(compositionById(PM.proj,layer.d?.comp));
          if(!child)return false;
          children.set(layer,child);return true;
        },
        matte:layer=>[layer.matteSource,...(layer.matteSources || []).map((m:any)=>m.source)]
          .filter(Boolean).every(id=>ids.has(id)),
      });
      if(!key){entries.set(memoKey,null);return null;}
      for(const layer of layers){
        effects.push(...layer.fx || []);
        if(layer.masks?.length){
          if(layer.masks.some((mask:any)=>mask.path))programs.add('pathMaskApply');
          else {programs.add('masks');programs.add('maskApply');}
        }
        if(layer.matteSource || layer.matteSources?.length)programs.add('trackMatte');
      }
      const content:unknown[]=[comp.w,comp.h,comp.fps,comp.dur,comp.bg,comp.backgroundFill,layers];
      for(const child of new Set(children.values())){
        effects.push(...child.effects);
        for(const program of child.programs)programs.add(program);
        content.push(child.content);
      }
      let previousTime:number|undefined,previousKey='';
      const plan:Plan={effects,programs:[...programs],content:JSON.stringify(content),key:time=>{
        if(time===previousTime)return previousKey;
        previousTime=time;
        return previousKey=JSON.stringify([key(time),...Array.from(children,([layer,child])=>child.key(sourceTime(PM,layer,time)))]);
      }};
      entries.set(memoKey,plan);return plan;
    }finally{visiting.delete(comp);}
  };
  const plan=build(comp,sourceId);
  return {plan,images:[...observedImages]};
}
