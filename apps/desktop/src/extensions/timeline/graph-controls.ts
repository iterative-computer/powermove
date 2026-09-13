import { temporalKeys, type PowermoveAPI } from 'powermove';

export function graphSample(api: Pick<PowermoveAPI, 'anim' | 'project'>, axis: any, time: number, speed: boolean): number {
  if (!speed) return Number(api.anim.evP(axis.L,axis.prop,time,axis.key));
  const h = 1/(Math.max(1,api.project.get().fps)*20);
  return (Number(api.anim.evP(axis.L,axis.prop,time+h,axis.key))-Number(api.anim.evP(axis.L,axis.prop,time-h,axis.key)))/(2*h);
}

export function velocityDialog(api: Pick<PowermoveAPI, 'anim' | 'history' | 'transport' | 'ui'>, entries: any[]): void {
  entries = entries.filter(e => typeof e.key.v === 'number' && !e.L?.lock);
  if (!entries.length) { api.ui.toast('Select numeric keyframes first'); return; }
  entries.forEach(e => temporalKeys(e.prop.kf));
  const body = document.createElement('div');
  const fields: Record<string,HTMLInputElement> = {};
  for (const side of ['in','out']) for (const field of ['speed','influence']) {
    const label = document.createElement('label'), input = document.createElement('input');
    label.textContent = `${side === 'in'?'Incoming':'Outgoing'} ${field}${field==='influence'?' (%)':''}`;
    input.type='number'; input.step='any'; input.setAttribute('aria-label',label.textContent);
    input.value=String(entries[0].key[side+'Ease']?.[field] ?? (field==='speed'?0:33.333));
    if (field==='influence') { input.min='0.1'; input.max='100'; }
    label.style.cssText='display:grid;grid-template-columns:1fr 120px;gap:12px;margin:10px 0';
    label.append(input); body.append(label); fields[side+field]=input;
  }
  const message = document.createElement('div'); message.setAttribute('role','status'); body.append(message);
  api.ui.modal({title:`Keyframe Velocity · ${entries.length} keys`,body,actions:[{label:'Cancel'},{label:'Apply',pri:true,run:()=>{
    if (Object.values(fields).some(input => !input.checkValidity() || !Number.isFinite(input.valueAsNumber))) { message.textContent='Enter finite speeds and influences from 0.1% to 100%.'; return false; }
    api.history.do('Keyframe velocity',()=>{
      for (const e of entries) for (const side of ['in','out']) {
        e.key[side+'Interp']='bezier'; e.key[side+'Ease']={speed:fields[side+'speed']!.valueAsNumber,influence:fields[side+'influence']!.valueAsNumber};
        e.key.autoBezier=false;
      }
      api.anim.touch();
    }); api.transport.invalidate();
  }}]});
}

export function scaleGraphDialog(api: Pick<PowermoveAPI, 'anim' | 'history' | 'project' | 'transport' | 'ui' | 'util'>, entries: any[]): void {
  entries = entries.filter(e => typeof e.key.v==='number' && !e.L.lock);
  if (!entries.length) { api.ui.toast('Select keyframes first'); return; }
  const body=document.createElement('div'), time=document.createElement('input'), value=document.createElement('input');
  for (const [input,label] of [[time,'Time scale (%)'],[value,'Value scale (%)']] as const) {
    input.type='number';input.value='100'; input.setAttribute('aria-label',label);
    const row=document.createElement('label');row.textContent=label;row.append(input);body.append(row);
  }
  api.ui.modal({title:'Scale selected keyframes',body,actions:[{label:'Cancel'},{label:'Apply',pri:true,run:()=>{
    const ts=time.valueAsNumber/100, vs=value.valueAsNumber/100;
    if (!Number.isFinite(ts)||!Number.isFinite(vs)||ts<=0) return false;
    const t0=Math.min(...entries.map(e=>e.L.from+e.key.t)), v0=Math.min(...entries.map(e=>e.key.v));
    const plan=entries.map(e=>({...e,t:api.util.snapF(t0+(e.L.from+e.key.t-t0)*ts-e.L.from,api.project.get().fps)}));
    if (plan.some((e,i)=>plan.some((other,j)=>i!==j&&e.prop===other.prop&&Math.abs(e.t-other.t)<.5/api.project.get().fps)) || plan.some(e=>e.t<0 || e.t>e.L.dur || e.prop.kf.some((k:any)=>!entries.some(a=>a.key===k)&&Math.abs(k.t-e.t)<.5/api.project.get().fps))) { api.ui.toast('Scaling would overlap another key or exceed a layer boundary.'); return false; }
    api.history.do('Scale keyframes',()=>{
      for (const e of plan) { e.key.t=e.t; e.key.v=v0+(e.key.v-v0)*vs; for (const side of ['in','out']) if(e.key[side+'Ease'])e.key[side+'Ease'].speed*=vs/ts; }
      for (const prop of new Set(entries.map(e=>e.prop))) prop.kf.sort((a:any,b:any)=>a.t-b.t);
      api.anim.touch();
    });api.transport.invalidate();
  }}]});
}
