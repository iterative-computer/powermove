import { isProperty, resolveContent } from './content-properties';
import { sourceTime } from './retiming';

/** Invert the renderer's source clock, preferring the parent frame from which
 * this instance was entered when remapping holds or repeats source frames. */
export function compositionParentTime(PM:any,layer:any,time:number,reference:number,comp:any):number {
  const start=Math.max(0,layer.from),end=Math.max(start,Math.min(comp.dur,layer.from+layer.dur));
  const anchor=PM.clamp(reference,start,end),sample=(at:number)=>sourceTime(PM,layer,at);
  const value=sample(anchor);
  if(Math.abs(value-time)<1e-8)return anchor;
  const content=resolveContent(PM,layer,anchor);
  const animated=(value:any)=>isProperty(value) && (value.expr || value.kf.length);
  if(!content.timeRemap && ![layer.d.speed,layer.d.trim,layer.d.timeRemap].some(animated)){
    const rate=sample(anchor+1)-value;
    return Number.isFinite(rate) && Math.abs(rate)>1e-10 ? anchor+(time-value)/rate : anchor;
  }
  // Remaps can reverse, freeze or repeat. Find crossings on the parent frame
  // grid, then refine them; choose the crossing nearest the entry frame.
  const count=Math.min(4096,Math.max(1,Math.ceil((end-start)*Math.max(1,comp.fps))));
  let best=anchor,error=Math.abs(value-time),solution:number|undefined;
  const consider=(at:number,v:number)=>{
    const next=Math.abs(v-time);
    if(Number.isFinite(next) && (next<error-1e-8 || Math.abs(next-error)<1e-8 && Math.abs(at-anchor)<Math.abs(best-anchor))){best=at;error=next;}
  };
  let left=start,a=sample(left);consider(left,a);
  for(let i=1;i<=count;i++){
    const right=start+(end-start)*i/count,b=sample(right);consider(right,b);
    if(Number.isFinite(a) && Number.isFinite(b) && Math.min(a,b)<=time && time<=Math.max(a,b)){
      let lo=left,hi=right;
      if(Math.abs(a-b)<1e-10){lo=hi=PM.clamp(anchor,left,right);}
      else for(let n=0;n<30;n++){
        const mid=(lo+hi)/2,v=sample(mid);
        if((b>a)===(v<time))lo=mid;else hi=mid;
      }
      const candidate=(lo+hi)/2;
      // A hold jumps across intermediate values; that is not an inverse.
      if(Math.abs(sample(candidate)-time)<1e-6 && (solution===undefined || Math.abs(candidate-anchor)<Math.abs(solution-anchor)))solution=candidate;
    }
    left=right;a=b;
  }
  return solution ?? best;
}

type Link={parent:string;child:string;layer:any};
/** Carry the current time through nested instances, including tab switches
 * between siblings. Unrelated compositions share the current global time. */
export function compositionNavigationTime(PM:any,target:string,options:{layerId?:string},
  savedTime:(id:string)=>number|undefined,contexts:Map<string,string>):{time:number;links:Link[]} {
  const project=PM.proj,from=project.compId;
  const comps=new Map<string,any>([[from,project],...Object.entries(project.comps || {})]);
  const links:Link[]=[];
  for(const [parent,comp] of comps)for(const layer of comp.layers || [])
    if(layer.type==='precomp' && comps.has(layer.d?.comp))links.push({parent,child:layer.d.comp,layer});
  const identity=(link:Link)=>`${link.parent}:${link.layer.id}`;
  const rank=(link:Link)=>link.parent===from && link.layer.id===options.layerId ? 0
    : link.parent===from && PM.sel.layers.includes(link.layer.id) ? 1
    : contexts.get(link.child)===identity(link) ? 2
    : (savedTime(link.parent) ?? -Infinity)>=link.layer.from && (savedTime(link.parent) ?? -Infinity)<link.layer.from+link.layer.dur ? 3 : 4;
  links.sort((a,b)=>rank(a)-rank(b));
  const pending=[{id:from,path:[] as Link[],descending:false}],seen=new Set([`${from}:false`]);
  for(let i=0;i<pending.length;i++){
    const current=pending[i]!;
    if(current.id===target){
      let time=PM.time,id=from;
      for(const link of current.path){
        const parent=comps.get(link.parent);
        // Expressions use the same composition scope as nested rendering.
        PM.scope.push(parent);
        try{time=id===link.parent ? sourceTime(PM,link.layer,time)
          : compositionParentTime(PM,link.layer,time,savedTime(link.parent) ?? link.layer.from,parent);}
        finally{PM.scope.pop();}
        id=id===link.parent?link.child:link.parent;
      }
      return {time:Number.isFinite(time)?time:PM.time,links:current.path};
    }
    for(const link of links){
      const down=link.parent===current.id;
      if(!down && current.descending)continue;
      const next=down?link.child:link.child===current.id?link.parent:null;
      const descending=current.descending || down,key=`${next}:${descending}`;
      if(next&&!seen.has(key)){seen.add(key);pending.push({id:next,path:[...current.path,link],descending});}
    }
  }
  return {time:PM.time,links:[]};
}
