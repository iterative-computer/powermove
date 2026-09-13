import { adoptTemporalEase, easeFromUnitHandle, unitHandleFromEase, valueAtTime, refreshAutoBezier, segmentControlPoints, solveCurveParam } from '../../core/anim/temporal-ease';
const owners = new WeakMap<object,any[]>();
type PreparedTrack = {version:number; length:number; first:any; last:any; curves:any[]; times?:Float64Array; values?:Float64Array; evaluatedTime?:number; evaluatedValue?:number; evaluatedSpring?:(time:number,config:any)=>number};
const prepared = new WeakMap<any[], PreparedTrack>();
/** Legacy controls are views of native speed/influence, never persisted copies. */
export function temporalKeys(keys: any[], version?:number): any[] {
  const cached=prepared.get(keys);
  if(version !== undefined && cached?.version===version && cached.length===keys.length && cached.first===keys[0] && cached.last===keys.at(-1)) return keys;
  if(version !== undefined) prepared.set(keys,{version,length:keys.length,first:keys[0],last:keys.at(-1),curves:[]});
  if (keys.some(k => typeof k.v !== 'number')) return keys;
  if (!keys.length) return keys;
  if (keys.every(key=>owners.get(key)===keys)) { refreshAutoBezier(keys); return keys; }
  adoptTemporalEase(keys);
  for (const key of keys) {
    owners.set(key,keys);
    for (const [legacy,side] of [['eo','out'],['ei','in']] as const) {
      Object.defineProperty(key,legacy,{ configurable:true,enumerable:false,
        get() {
          const i = keys.indexOf(key), other = keys[i+(side==='out'?1:-1)] || keys[i+(side==='out'?-1:1)];
          if (!other) return side==='out'?[0,0]:[1,1];
          if (key[side+'Interp'] !== 'bezier') return side==='out'?[0,0]:[1,1];
          return unitHandleFromEase(key[side+'Ease'],Math.abs(other.t-key.t),(other.v-key.v)*Math.sign(other.t-key.t),side)?.map(v=>Number(v.toPrecision(15))) ?? null;
        },
        set(handle) {
          prepared.delete(keys);
          const i=keys.indexOf(key), other=keys[i+(side==='out'?1:-1)] || keys[i+(side==='out'?-1:1)];
          key[side+'Interp']='bezier'; key.autoBezier=false;
          key[side+'Ease']=easeFromUnitHandle(handle,other?Math.abs(other.t-key.t):1,other?(other.v-key.v)*Math.sign(other.t-key.t):1,side);
        }
      });
    }
    Object.defineProperty(key,'hold',{configurable:true,enumerable:false,get:()=>key.outInterp==='hold',set:(v)=>{
      prepared.delete(keys);
      if (v) key.outInterp='hold';
      else if (key.outInterp==='hold') key.outInterp='linear';
    }});
    Object.defineProperty(key,'bezierMode',{configurable:true,enumerable:false,get:()=>key.continuous?'continuous':'split',set:(v)=>{key.continuous=v==='continuous';}});
  }
  return keys;
}
export function temporalValue(keys: any[], left: number, time: number, state = prepared.get(keys)): number {
  if(!state) return valueAtTime(keys[left],keys[left+1],time);
  let c=state.curves[left];
  if(!c){
    const a=keys[left],b=keys[left+1],span=b.t-a.t,start=a.t;
    const mode=a.outInterp==='hold'||b.inInterp==='hold'?0:a.outInterp==='linear'&&b.inInterp==='linear'?1:a.spring?2:3;
    const curve=segmentControlPoints(a,b);
    c=state.curves[left]={mode,start,span,v:a.v,d:b.v-a.v,spring:a.spring,
      x1:(curve.p1.t-start)/span,x2:(curve.p2.t-start)/span,
      cy:3*(curve.p1.v-a.v),by:3*(curve.p2.v-2*curve.p1.v+a.v),ay:b.v-3*curve.p2.v+3*curve.p1.v-a.v};
  }
  if(c.mode===0)return c.v;
  if(c.mode===1)return c.v+c.d*(time-c.start)/c.span;
  if(c.mode===2)return c.v+c.d*activeSpring!((time-c.start),c.spring);
  const u=solveCurveParam(c.x1,c.x2,(time-c.start)/c.span);
  return ((c.ay*u+c.by)*u+c.cy)*u+c.v;
}

// Dense evaluation data avoids repeatedly walking accessor-bearing key objects.
// PM.touch invalidates it after source edits; no animation values are persisted here.
let activeSpring:((time:number,config:any)=>number)|undefined;
export function temporalEvaluate(keys:any[],time:number,version:number,spring:(time:number,config:any)=>number):number {
  let state=prepared.get(keys);
  if(!state||state.version!==version||state.length!==keys.length||state.first!==keys[0]||state.last!==keys[keys.length-1]){
    // Evaluation needs native timing data, not a legacy UI accessor on every key.
    adoptTemporalEase(keys);
    state={version,length:keys.length,first:keys[0],last:keys[keys.length-1],curves:[]};prepared.set(keys,state);
  }
  if (state.evaluatedTime === time && state.evaluatedSpring === spring) return state.evaluatedValue!;
  state.times ||= Float64Array.from(keys,k=>k.t);state.values ||= Float64Array.from(keys,k=>k.v);
  const times=state.times,values=state.values,n=times.length;
  if(time<=times[0]!)return values[0]!;if(time>=times[n-1]!)return values[n-1]!;
  let lo=0,hi=n-1;while(hi-lo>1){const m=(lo+hi)>>1;times[m]!<=time?lo=m:hi=m;}
  activeSpring=spring;
  const value = temporalValue(keys,lo,time,state);
  state.evaluatedTime = time; state.evaluatedValue = value; state.evaluatedSpring = spring;
  return value;
}
