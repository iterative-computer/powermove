import {materialSourceLayers} from './material-maps';

export type MaterialSourceBounds={x0:number;y0:number;x1:number;y1:number;w:number;h:number};
type Entry={stamp:unknown[];bounds:MaterialSourceBounds};
const caches=new WeakMap<object,Map<any,Entry>>();
export function clearMaterialSourceBounds(PM:any):void {caches.delete(PM);}

/** One source-local canvas for the whole group duration. Child entrances,
 * exits and motion never change its origin, UV mapping or allocation size.
 * Include every frame and property key time; do not filter by current visibility. */
export function stableMaterialSourceBounds(PM:any,comp:any,group:any,outerTime:number):MaterialSourceBounds {
  let cache=caches.get(PM);if(!cache){cache=new Map();caches.set(PM,cache);}
  const stamp=[PM.proj,comp.layers,comp.w,comp.h,comp.fps,comp.dur,PM.proj.revision,PM._scene3dRevision,PM.animVersion?.()];
  const previous=cache.get(group);
  if(previous && stamp.every((value,index)=>Object.is(value,previous.stamp[index])))return previous.bounds;
  const fps=Math.max(1,Number(comp.fps)||30),start=Math.max(0,Number(group.from)||0);
  const end=Math.max(start,Math.min(Number(comp.dur)||0,start+(Number(group.dur)||Number(comp.dur)||0)));
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  try{
    for(const child of materialSourceLayers(comp,group.id)){
      if(['group','audio','null','adjustment'].includes(child.type))continue;
      const lo=Math.max(start,Number(child.from)||0),hi=Math.min(end,(Number(child.from)||0)+(Number(child.dur)||0));
      if(hi<lo)continue;
      const times=new Set([lo,hi]);
      for(let frame=Math.ceil(lo*fps);frame<=Math.floor(hi*fps);frame++)times.add(frame/fps);
      // Sub-frame keys can be used by motion-blurred exports.
      const visit=(value:any,seen=new Set<object>())=>{
        if(!value||typeof value!=='object'||seen.has(value))return;seen.add(value);
        for(const key of value.kf || []){const t=Number(child.from||0)+Number(key.t);if(t>=lo&&t<=hi)times.add(t);}
        for(const nested of Object.values(value))visit(nested,seen);
      };visit(child);
      for(const time of times){
        PM.beginEval?.(time);
        const bounds=PM.GL?.bounds?.(child,time);if(!bounds)continue;
        const world=PM.worldMatrix(group,time),det=world[0]*world[3]-world[1]*world[2];
        if(!Number.isFinite(det)||Math.abs(det)<1e-10)continue;
        const inverse=[world[3]/det,-world[1]/det,-world[2]/det,world[0]/det,
          (world[2]*world[5]-world[3]*world[4])/det,(world[1]*world[4]-world[0]*world[5])/det];
        const local=PM.mul(inverse,PM.worldMatrix(child,time));
        for(const x of [bounds.x0,bounds.x1])for(const y of [bounds.y0,bounds.y1]){
          const px=local[0]*x+local[2]*y+local[4],py=local[1]*x+local[3]*y+local[5];
          if(!Number.isFinite(px+py))continue;
          x0=Math.min(x0,px);y0=Math.min(y0,py);x1=Math.max(x1,px);y1=Math.max(y1,py);
        }
      }
    }
  }finally{PM.beginEval?.(outerTime);}
  const bounds=Number.isFinite(x0)?{x0,y0,x1,y1,w:Math.max(1,x1-x0),h:Math.max(1,y1-y0)}:{x0:0,y0:0,x1:1,y1:1,w:1,h:1};
  cache.set(group,{stamp,bounds});return bounds;
}
