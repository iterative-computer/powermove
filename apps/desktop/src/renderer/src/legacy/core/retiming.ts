import { evaluatedValue, isProperty, resolveContent } from './content-properties';

const integrals=new WeakMap<object,{version:number;step:number;sums:number[]}>();

/** One source-time mapping for playback, seeking, nested comps and export. */
export function sourceTime(PM: any, layer: any, time: number): number {
  const d=resolveContent(PM,layer,time), local=time-layer.from;
  if (d.timeRemap && isProperty(layer.d.sourceTime)) return Number(PM.evP(layer,layer.d.sourceTime,time,'c.sourceTime')) || 0;
  const speed=layer.d.speed;
  if (!isProperty(speed) || (!speed.kf.length && !speed.expr)) return local*Number(d.speed ?? 1)+Number(d.trim || 0);
  // Integrate a varying playback rate; changing a rate must not move all elapsed time.
  const end=Math.max(0,local),step=1/(Math.max(30,PM.proj.fps)*2),version=PM.animVersion();
  let cached=integrals.get(speed);if(!cached||cached.version!==version||cached.step!==step){cached={version,step,sums:[0]};integrals.set(speed,cached);}
  const full=Math.floor(end/step);for(let i=cached.sums.length-1;i<full;i++)cached.sums.push(cached.sums[i]!+Number(evaluatedValue(PM,layer,speed,layer.from+(i+.5)*step,'c.speed'))*step);
  const remainder=end-full*step;
  return cached.sums[full]!+Number(evaluatedValue(PM,layer,speed,layer.from+full*step+remainder/2,'c.speed'))*remainder+Number(d.trim||0);
}

export function enableTimeRemap(PM: any, layer: any, mode: 'remap'|'freeze'|'reverse' = 'remap'): void {
  const start=sourceTime(PM,layer,layer.from), end=sourceTime(PM,layer,layer.from+Math.max(0,layer.dur-1/PM.proj.fps)), current=sourceTime(PM,layer,PM.time);
  const frame=1/PM.proj.fps;
  PM.Edit.mutate(mode==='freeze'?'Freeze frame':mode==='reverse'?'Reverse time':'Enable time remapping',()=>{
    layer.d.timeRemap=PM.P(true); layer.d.sourceTime=PM.P(mode==='freeze'?current:start);
    if(mode!=='freeze') {
      PM.setKeyOn(layer.d.sourceTime,0,mode==='reverse'?end:start,'linear',PM.proj.fps);
      PM.setKeyOn(layer.d.sourceTime,Math.max(frame,layer.dur-frame),mode==='reverse'?start:end,'linear',PM.proj.fps);
    } else { const key=PM.setKeyOn(layer.d.sourceTime,PM.time-layer.from,current,'linear',PM.proj.fps);key.hold=true; }
    PM.touch();
  },{origin:'inspector'});
  PM.TL?.reveal?.(layer,['c.sourceTime']); PM.Inspector?.refresh?.(); PM.invalidate();
}
