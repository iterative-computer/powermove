type Bounds={x0:number;y0:number;x1:number;y1:number;w:number;h:number};
const MAX_SAMPLES=240;
const cache=new WeakMap<object,{stamp:unknown[];groups:Map<string,Bounds>}>();
/** Union all frame samples once per edit, including children outside their in/out points. */
export function stableMaterialGroupBounds(PM:any,group:any,sample:(time:number)=>Bounds|null):Bounds {
  const comp=PM.curComp?.() || PM.proj,stamp=[comp,PM.proj.revision,PM._scene3dRevision,PM.animVersion?.()];
  let entry=cache.get(comp);
  if(!entry || stamp.some((value,i)=>!Object.is(value,entry!.stamp[i]))){entry={stamp,groups:new Map()};cache.set(comp,entry);}
  const cached=entry.groups.get(group.id);if(cached)return cached;
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  const fps=Math.max(1,Number(comp.fps)||30),from=Math.max(0,group.from || 0),end=Math.min(comp.dur,from+(group.dur ?? comp.dur));
  // Long groups are sampled at most MAX_SAMPLES times so an edit stays cheap.
  const first=Math.floor(from*fps),last=Math.ceil(end*fps),step=Math.max(1,Math.ceil((last-first)/MAX_SAMPLES));
  for(let frame=first;frame<=last;frame=frame<last&&frame+step>last?last:frame+step){
    const b=sample(frame/fps);if(!b)continue;
    x0=Math.min(x0,b.x0);y0=Math.min(y0,b.y0);x1=Math.max(x1,b.x1);y1=Math.max(y1,b.y1);
  }
  const bounds=Number.isFinite(x0)?{x0,y0,x1,y1,w:x1-x0,h:y1-y0}:{x0:0,y0:0,x1:1,y1:1,w:1,h:1};
  entry.groups.set(group.id,bounds);return bounds;
}
