export function installRenderQueue(PM:any,X:any) {
  const saved=PM.store.get('renderQueue',[]);X.queue=Array.isArray(saved)?saved.map((job:any)=>({...job,status:job.status==='rendering'?'pending':job.status})):[];
  const persist=()=>{PM.store.set('renderQueue',X.queue);PM.bus.emit('renderqueue');};
  X.enqueue=(options:any)=>{const job={id:PM.uid('render'),name:options.name||PM.proj.name,options:structuredClone(options),project:JSON.parse(JSON.stringify(PM.proj)),status:'pending',error:''};X.queue.push(job);persist();PM.toast('Added to render queue');return job;};
  X.queueDialog=()=>{
    const body=PM.h('div'),list=PM.h('div');body.append(list);
    const draw=()=>{list.replaceChildren();for(const job of X.queue){const row=PM.h('div',{style:{padding:'10px 0',borderBottom:'1px solid var(--line)'}}),title=PM.h('div',`${job.name} · ${job.options.format.toUpperCase()} · ${job.status}`);row.append(title);if(job.error)row.append(PM.h('p',{style:{color:'var(--danger,#ef8989)',fontSize:'11px'}},job.error));const remove=PM.h('button.chip',job.status==='failed'||job.status==='cancelled'?'Retry':'Remove');remove.disabled=job.status==='rendering';remove.onclick=()=>{if(job.status==='failed'||job.status==='cancelled'){job.status='pending';job.error='';}else X.queue=X.queue.filter((j:any)=>j.id!==job.id);persist();draw();};row.append(remove);list.append(row);}if(!X.queue.length)list.append(PM.h('p','The render queue is empty.'));};draw();
    const off=PM.bus.on('renderqueue',draw);const modal=PM.modal({title:'Render queue',body,width:480,actions:[{label:'Close',run:()=>off?.()},{label:'Render pending',pri:true,run:()=>{off?.();void X.runQueue();}}]});return modal;
  };
  X.runQueue=async()=>{if(X.queueBusy||X.busy)return;X.queueBusy=true;const original=PM.proj,time=PM.time;
    try{for(const job of X.queue.filter((j:any)=>j.status==='pending')){job.status='rendering';job.error='';persist();PM.proj=JSON.parse(JSON.stringify(job.project));PM.ProjectIndex?.invalidate?.();PM.touch();try{const result=await X.run(job.options);job.status=result?.cancelled?'cancelled':'complete';if(result?.error){job.status='failed';job.error=result.error;}}catch(e){job.status='failed';job.error=(e as Error).message;}finally{PM.proj=original;PM.ProjectIndex?.invalidate?.();PM.touch();persist();}if(job.status!=='complete')break;}}
    finally{PM.proj=original;PM.time=time;X.queueBusy=false;PM.bus.emit('project');PM.invalidate();X.queueDialog();}
  };
}
export function wavBytes(buffer:AudioBuffer):Uint8Array {
  const channels=buffer.numberOfChannels,frames=buffer.length,data=new Uint8Array(44+frames*channels*2),view=new DataView(data.buffer),text=(offset:number,s:string)=>{for(let i=0;i<s.length;i++)data[offset+i]=s.charCodeAt(i);};
  text(0,'RIFF');view.setUint32(4,data.length-8,true);text(8,'WAVEfmt ');view.setUint32(16,16,true);view.setUint16(20,1,true);view.setUint16(22,channels,true);view.setUint32(24,buffer.sampleRate,true);view.setUint32(28,buffer.sampleRate*channels*2,true);view.setUint16(32,channels*2,true);view.setUint16(34,16,true);text(36,'data');view.setUint32(40,data.length-44,true);
  const planes=Array.from({length:channels},(_,i)=>buffer.getChannelData(i));for(let i=0;i<frames;i++)for(let c=0;c<channels;c++){const value=Math.max(-1,Math.min(1,planes[c]![i]!));view.setInt16(44+(i*channels+c)*2,Math.round(value*(value<0?32768:32767)),true);}return data;
}
