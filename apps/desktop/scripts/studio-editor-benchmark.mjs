/** Visible, real editor + native host acceptance. No server or private profile. */
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdir,mkdtemp,readFile,writeFile,rm,cp} from 'node:fs/promises';
import {constants} from 'node:fs';
import {createHash} from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {_electron} from 'playwright';
import {distribution,presentationMetrics,memoryTrend,overlappingTasks} from './studio-editor-metrics.mjs';
import {corpusGlb} from './studio-corpus-glb.mjs';
import {studioCargoTarget} from './studio-cargo-target.mjs';
import {flipFuses,FuseVersion,FuseV1Options} from '@electron/fuses';

const desktop=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const command=(file,args)=>execFileSync(file,args,{encoding:'utf8',maxBuffer:8*1024*1024}).trim();
const safeCommand=(file,args)=>{try{return command(file,args);}catch{return 'unavailable';}};
function processes() {return command('/bin/ps',['-axo','pid=,ppid=,rss=,comm=']).split('\n').map(line=>line.trim().match(/^(\d+)\s+(\d+)\s+(\d+)\s+(.+)$/)).filter(Boolean).map(m=>({pid:+m[1],parent:+m[2],rssMiB:+m[3]/1024,name:m[4]}));}
function machine(owned=new Set()) {
 const rows=processes();
 let listed='';try{listed=command('/usr/bin/pgrep',['-fl','playwright|Electron|cargo|rustc|pm-studio-host']);}catch(error){if(error.status!==1)throw error;}
 const active=listed.split('\n').map(line=>line.match(/^(\d+)\s+(.+)$/)).filter(Boolean).map(m=>({pid:+m[1],name:m[2].slice(0,200)})).filter(p=>!owned.has(p.pid)&&!p.name.includes('chrome_crashpad_handler'));
 return {date:new Date().toISOString(),host:os.cpus()[0]?.model,ramGiB:os.totalmem()/1024**3,os:safeCommand('/usr/bin/sw_vers',['-productVersion']),
  power:safeCommand('/usr/bin/pmset',['-g','batt']),powerSettings:safeCommand('/usr/bin/pmset',['-g','custom']),thermal:safeCommand('/usr/bin/pmset',['-g','therm']),
  swap:safeCommand('/usr/sbin/sysctl',['vm.swapusage']),pressure:safeCommand('/usr/bin/memory_pressure',['-Q']),disk:safeCommand('/usr/sbin/diskutil',['info','/']).split('\n').find(l=>l.includes('Container Free Space'))?.trim(),
  active,quiet:active.length===0,processes:rows};
}
function processTree(state,pid) {
 const owned=new Set([pid]);let changed=true;
 while(changed){changed=false;for(const p of state.processes)if(owned.has(p.parent)&&!owned.has(p.pid)){owned.add(p.pid);changed=true;}}
 return {owned,rows:state.processes.filter(p=>owned.has(p.pid))};
}
function memorySample(pid) {
 const state={processes:processes(),swap:safeCommand('/usr/sbin/sysctl',['vm.swapusage'])},tree=processTree(state,pid),hosts=tree.rows.filter(p=>p.name.endsWith('/pm-studio-host'));
 return {at:Date.now(),rssMiB:tree.rows.reduce((n,p)=>n+p.rssMiB,0),hostMiB:hosts.reduce((n,p)=>n+p.rssMiB,0),maxHostMiB:Math.max(0,...hosts.map(p=>p.rssMiB)),processes:tree.rows,swap:state.swap};
}
function measurementState(pid) {
 const state=machine(),owned=processTree(state,pid).owned;owned.add(process.pid);
 const result=machine(owned);delete result.processes;return result;
}
const swapMiB=text=>Number(text.match(/used =\s*([\d.]+)M/)?.[1]??NaN);
function options() {
 const value=(key,fallback)=>{const i=process.argv.indexOf(key);return i<0?fallback:process.argv[i+1];};
 return {dev:process.argv.includes('--dev'),seconds:Number(value('--seconds','30')),runs:Number(value('--runs','3')),finalFrames:Number(value('--final-frames','30')),
  stressMinutes:Number(value('--stress-minutes','0')),stressWarmupMinutes:Number(value('--stress-warmup-minutes','5')),stressFixedProjects:process.argv.includes('--stress-fixed-projects'),diagnostic:process.argv.includes('--diagnostic'),profile:process.argv.includes('--cpu-profile'),scenes:value('--scenes','hero,edge,macro,back,chrome,materials,caustics,wet,landscape,room,street,4view').split(',').filter(Boolean),
  evidence:path.resolve(process.env.PM_STUDIO_SHOTS??'/tmp/pm-shots/studio3d/laneP4'),fixtures:value('--fixtures',path.join(studioCargoTarget(),'release/studio-environments'))};
}
async function installProbe(page) {
 await page.evaluate(()=>{
  const PM=window.PM,gl=PM.GL.gl,probe={active:null,presentations:[],inputs:[],tasks:[],pending:[],polling:false};window.__studioPerf=probe;
  if(typeof gl.fenceSync!=='function')throw new Error('WebGL2 completion fences are required');
  const poll=()=>{for(let i=probe.pending.length-1;i>=0;i--){const item=probe.pending[i],result=gl.clientWaitSync(item.sync,0,0);if(result===gl.ALREADY_SIGNALED||result===gl.CONDITION_SATISFIED||result===gl.WAIT_FAILED){gl.deleteSync(item.sync);probe.pending.splice(i,1);if(result===gl.WAIT_FAILED)item.row.pixelError='GPU fence failed';else item.row.pixelAt=performance.now();}}if(probe.pending.length)requestAnimationFrame(poll);else probe.polling=false;};
  probe.observer=new PerformanceObserver(list=>{for(const task of list.getEntries())probe.tasks.push({at:task.startTime,ms:task.duration,name:task.name});});probe.observer.observe({type:'longtask',buffered:false});
  PM.bus.on('preview:presented',frame=>{
   if(!probe.active)return;const status=PM.Studio?.status();if(!status)return;
   const jobs=(PM.Studio.diagnostics?.()??[]).filter(j=>j.version),signature=jobs.length?jobs.map(j=>`${j.slot}:${j.version}`).sort().join('|'):String(status.frames);
   const row={at:performance.now(),time:frame.time,signature,backend:status.backend,jobs};probe.presentations.push(row);
   if(probe.pending.length>=32){const old=probe.pending.shift();gl.deleteSync(old.sync);old.row.pixelError='GPU completion queue full';}
   const sync=gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE,0);if(!sync){row.pixelError='No GPU completion fence';return;}probe.pending.push({sync,row});gl.flush();if(!probe.polling){probe.polling=true;requestAnimationFrame(poll);}
  });
  window.addEventListener('pointermove',e=>{if(probe.active==='drag'&&e.buttons)probe.inputs.push({at:performance.now(),kind:'pointer',time:PM.time,x:e.clientX,y:e.clientY,eventTime:e.timeStamp});},true);
  PM.bus.on('time',()=>{if(probe.active==='playback'){const time=Math.floor(PM.time*PM.proj.fps+1e-7)/PM.proj.fps;if(time!==probe.lastTime){probe.lastTime=time;probe.inputs.push({at:performance.now(),kind:'time',time});}}});
 });
}
async function resetScene(page,name) {
 await page.evaluate(name=>{
  const PM=window.PM;window.__studioPerf.tasks=[];window.__studioPerf.sceneStarted=performance.now();PM.pause();PM.AutomaticPreview?.cancel();PM.assets.clear();PM.replaceProject(PM.mkProject({name:`Studio acceptance: ${name}`,w:1920,h:1080,fps:60,dur:90}));PM.setTime(0);PM.ProjectsScreen.hide();PM.Studio?.setPreview('quality');PM.perf.auto=false;PM.quality=1;PM.selectLayers([]);PM.Kernel.api('studio-perf').scene3d.views.setLayout(1);
 },name);
}
async function author(page,name,config) {
 await resetScene(page,name);
 if(name==='landscape'||name==='landscape-2x'||name==='forest-100k'){
  // Call the actual Add menu contribution. No copied defaults or density edits.
  await page.evaluate(async()=>{await window.PM.cmd('3d.add-menu');});
  await page.locator('[data-svelte-overlay-menu] .menu-panel .di .label').filter({hasText:/^Terrain$/}).click();
 }else{
  const sourceName=name==='4view'?'hero':name==='room'?'env-room':name==='street'?'env-street':name;
  command(config.fixtures,['--scene',sourceName,'--expand','--json-only','--out',config.evidence]);
  const scene=JSON.parse(await readFile(path.join(config.evidence,`${sourceName}-scene.json`),'utf8'));
  let screen;try{screen=await readFile(path.join(config.evidence,`${sourceName}-screen.png`));}catch{}
  const bytes=corpusGlb(scene,screen);
  await page.evaluate(async({base64,scene})=>{
   const PM=window.PM,api=PM.Kernel.api('studio-perf').scene3d;
   const check=r=>{if(r?.ok===false)throw new Error(JSON.stringify(r));};
   const c=scene.camera,p=c.position,t=c.target;
   check(api.edit({operation:'add_camera',id:'perf-camera',camera:{p:{x:p.x,y:p.y,z:p.z,targetX:t.x,targetY:t.y,targetZ:t.z,fov:c.fov,fStop:c.f_stop,far:100000}}}));
   const data=Uint8Array.from(atob(base64),c=>c.charCodeAt(0)),packed=await api.prepareImport([new File([data],'corpus.glb',{type:'model/gltf-binary'})]);
   const imported=await PM.cmd('3d.import-obj',packed);check(imported);
   // Ordinary imports normalize to a unit sphere. Restore the canonical corpus
   // transform through ordinary layer channels, keeping its original camera.
   const model=PM.L(imported.data.result.id),asset=PM.assets.get(model.d.data.object.source.assetId),normalization=asset.object3d.children[0],original=normalization.children[0],radius=1/normalization.scale.x;
   check(api.edit({operation:'update_object',target:model.id,patch:{p:{x:-original.position.x,y:-original.position.y,z:-original.position.z,sx:radius,sy:radius,sz:radius}}}));
   const hex=v=>'#'+[v.x,v.y,v.z].map(x=>Math.round(Math.max(0,Math.min(1,x<=.0031308?x*12.92:1.055*x**(1/2.4)-.055))*255).toString(16).padStart(2,'0')).join('');
   check(api.edit({operation:'set_environment',patch:{samples:64,background:hex(scene.backdrop),shadowCatcher:scene.shadow_catcher,p:{ambient:Math.max(scene.environment[0].x,scene.environment[0].y,scene.environment[0].z),ambientColor:hex({x:scene.environment[0].x/(Math.max(scene.environment[0].x,scene.environment[0].y,scene.environment[0].z)||1),y:scene.environment[0].y/(Math.max(scene.environment[0].x,scene.environment[0].y,scene.environment[0].z)||1),z:scene.environment[0].z/(Math.max(scene.environment[0].x,scene.environment[0].y,scene.environment[0].z)||1)}),exposure:scene.exposure}}}));
   for(const [i,l] of scene.lights.entries()){
    const center=l.center,u=l.u,v=l.v,n={x:u.y*v.z-u.z*v.y,y:u.z*v.x-u.x*v.z,z:u.x*v.y-u.y*v.x},radiance=l.radiance,peak=Math.max(radiance.x,radiance.y,radiance.z,1e-8),length=Math.hypot(n.x,n.y,n.z)||1;n.x/=length;n.y/=length;n.z/=length;
    check(api.edit({operation:'add_light',light:{id:`perf-light-${i}`,type:'area',p:{x:center.x,y:center.y,z:center.z,targetX:center.x-n.x,targetY:center.y-n.y,targetZ:center.z-n.z,width:2*Math.hypot(u.x,u.y,u.z),height:2*Math.hypot(v.x,v.y,v.z),color:hex({x:radiance.x/peak,y:radiance.y/peak,z:radiance.z/peak}),intensity:peak}}}));
   }
   if(scene.world?.sky){const s=scene.world.sky,a=scene.world.atmosphere;check(api.edit({operation:'set_world',world:{mode:'sky',p:{metersPerUnit:scene.meters_per_unit,elevation:s.elevation,azimuth:s.azimuth,haze:s.turbidity,intensity:s.intensity,sunIntensity:s.sun_intensity,...(a?{fogDensity:a.density,fogDistance:a.distance,fogHeightFalloff:a.height_falloff,fogAnisotropy:a.anisotropy,fogBaseHeight:a.base_height,fogColor:hex(a.color)}:{} )}}}));}
   else check(api.edit({operation:'set_world',world:{mode:'hdri',p:{metersPerUnit:scene.meters_per_unit,intensity:0,fogDensity:0}}}));
   for(const [i,v] of (scene.world?.volumes??[]).entries()){
    const p=v.center,s=v.size;check(api.edit({operation:'add_fog',id:`perf-volume-${i}`,fog:{p:{shape:v.shape,width:s.x*2,height:s.y*2,depth:s.z*2,density:v.density,color:hex(v.color),anisotropy:v.anisotropy}}}));
    check(api.edit({operation:'update_object',target:`perf-volume-${i}`,patch:{p:{x:p.x,y:p.y,z:p.z}}}));
   }
   PM.selectLayers([]);PM.touch();
   for(const [i,l] of (scene.extra_lights??[]).entries()){
    const color={x:l.radiance[0],y:l.radiance[1],z:l.radiance[2]},peak=Math.max(color.x,color.y,color.z,1e-8),radius=l.u[0];
    if(l.pos[3]!==1)throw new Error('Unsupported corpus analytic light');
    check(api.edit({operation:'add_light',light:{id:`perf-extra-${i}`,type:'point',p:{x:l.pos[0],y:l.pos[1],z:l.pos[2],color:hex({x:color.x/peak,y:color.y/peak,z:color.z/peak}),intensity:peak*Math.PI*radius*radius}}}));
   }
  },{base64:bytes.toString('base64'),scene:{...scene,objects:[],materials:[],instances:[]}});
 }
 await page.evaluate(({name})=>{
  const PM=window.PM,api=PM.Kernel.api('studio-perf').scene3d;
  if(name==='landscape-2x'||name==='forest-100k')for(const layer of PM.proj.layers){const scatter=layer.d?.data?.object?.scatter;if(!scatter)continue;const count=name==='landscape-2x'?scatter.p.count.v*2:layer.name==='Meadow'?50000:layer.name==='Pines'?70000:layer.name==='Birches'?20000:layer.name==='Oaks'?10000:2000;const r=api.edit({operation:'update_scatter',target:layer.id,patch:{p:{count}}});if(!r.ok)throw new Error(r.message);}
  window.__studioPerf.allPanesDynamic=name==='4view';
  if(name==='4view'){
   api.views.setLayout(4);
   // Camera motion leaves the other three orthographic views unchanged. Move
   // the product too, so every pane must produce genuinely new native images.
   const model=PM.proj.layers.find(l=>l.d?.data?.object?.source?.assetId);
   if(!model)throw new Error('Four-view workload needs a moving product');
   const y=model.p['position.y'].v;
   PM.Edit.apply({type:'replace_keyframes',target:model.id,path:'position.y',keyframes:[{time:0,value:y,ease:'linear'},{time:90,value:y+.5,ease:'linear'}]});
  }
  const camera=PM.proj.layers.find(l=>l.d?.data?.camera);if(camera){const x=camera.p['position.x'].v,z=camera.p['position.z'].v;PM.Edit.apply({type:'replace_keyframes',target:camera.id,path:'position.x',keyframes:[{time:0,value:x,ease:'linear'},{time:90,value:x+Math.max(1,Math.abs(z)*.15),ease:'linear'}]});}
  api.edit({operation:'set_environment',patch:{samples:64}});
  PM.selectLayers([]);PM.invalidate();
 },{name});
}
async function nativeReady(page) {
 try {
  await page.waitForFunction(()=>{const s=window.PM.Studio?.status(),jobs=window.PM.Studio?.diagnostics?.()??[],layout=window.PM.Kernel.api('studio-perf').scene3d.views.get().layout;return s?.declined||s?.failure||jobs.some(j=>j.backend==='webgpu')||s?.backend==='native-studio'&&s.ready&&!s.preparing&&jobs.length>=layout&&jobs.every(j=>j.version);},undefined,{timeout:120000});
 }catch(error){throw new Error(`Native readiness timed out: ${JSON.stringify(await page.evaluate(()=>({status:window.PM.Studio?.status(),jobs:window.PM.Studio?.diagnostics?.(),layers:window.PM.proj.layers.map(l=>({id:l.id,name:l.name}))})))}`,{cause:error});}
 const state=await page.evaluate(()=>({status:window.PM.Studio.status(),jobs:window.PM.Studio.diagnostics?.()??[]}));
 assert.equal(state.status.backend,'native-studio',JSON.stringify(state));assert.equal(state.status.declined,'',JSON.stringify(state));assert(!state.status.failure,JSON.stringify(state));
 assert(state.jobs.length&&state.jobs.every(j=>j.backend==='native-studio'),`Every viewer pane must stay native: ${JSON.stringify(state)}`);return state;
}

async function interval(page,kind,seconds) {
 const start=await page.evaluate(kind=>{const p=window.__studioPerf;p.active=kind;p.presentations=[];p.inputs=[];p.tasks=[];p.lastTime=null;const start=performance.now();if(kind==='drag'){window.PM.Kernel.api('studio-perf').scene3d.setView('editor');window.PM.Kernel.api('studio-perf').scene3d.setNavigationMode('orbit');if(p.allPanesDynamic){window.PM.setTime(0);window.PM.play();}}if(kind==='playback'){window.PM.Kernel.api('studio-perf').scene3d.setView('camera');window.PM.setTime(0);window.PM.play();}return start;},kind);
 if(kind==='drag'){
  const stage=await page.locator('#stage').boundingBox();assert(stage,'Visible editor stage required');
  await page.keyboard.down('Alt');await page.mouse.move(stage.x+stage.width*.5,stage.y+stage.height*.5);await page.mouse.down();
  const stop=Date.now()+seconds*1000;let i=0;
  while(Date.now()<stop){await page.mouse.move(stage.x+stage.width*(.5+.12*Math.sin(i*.055)),stage.y+stage.height*(.5+.08*Math.sin(i*.037)));i++;}
  await page.mouse.up();await page.keyboard.up('Alt');assert.equal(await page.evaluate(()=>window.PM.Kernel.api('studio-perf').scene3d.getView()),'editor','A real Free View orbit must occur');
 }else await page.waitForTimeout(seconds*1000);
 const end=await page.evaluate(()=>{const p=window.__studioPerf;p.active=null;const end=performance.now();window.PM.pause();window.PM.Kernel.api('studio-perf').scene3d.setNavigationMode('select');return end;});
 await page.waitForFunction(()=>window.__studioPerf.pending.length===0,undefined,{timeout:10000});
 const raw=await page.evaluate(({start,end})=>{const p=window.__studioPerf;return {start,end,presentations:p.presentations,inputs:p.inputs,tasks:p.tasks};},{start,end});
 raw.tasks=overlappingTasks(raw.tasks,start,end);return raw;
}
function summarize(raw,hz) {
 const metrics=presentationMetrics(raw.presentations,raw.start,raw.end,hz);
 const latencies=[];
 // Time edits match the host's rendered time, not the newest compositor time.
 for(const input of raw.inputs)if(input.kind==='time'){
  const frame=raw.presentations.find(p=>Number.isFinite(p.pixelAt)&&p.at>=input.at&&p.jobs.some(j=>!j.slot&&j.inputAt>=input.at&&(j.native?.host?.time??j.time)>=input.time-1e-7));if(frame)latencies.push(frame.pixelAt-input.at);
 }
 for(const input of raw.inputs)if(input.kind==='pointer'){
  const frame=raw.presentations.find(p=>Number.isFinite(p.pixelAt)&&p.at>=input.at&&p.jobs.some(j=>!j.slot&&j.inputAt>=input.at));if(frame)latencies.push(frame.pixelAt-input.at);
 }
 const slots=[...new Set(raw.presentations.flatMap(p=>p.jobs.map(j=>j.slot)))],panes=slots.map(slot=>({slot,...presentationMetrics(raw.presentations.map(p=>({...p,signature:p.jobs.find(j=>j.slot===slot)?.version??''})),raw.start,raw.end,hz)}));
 const seen=new Set(),native=[];for(const p of raw.presentations)for(const j of p.jobs){const key=`${j.slot}:${j.version}`;if(j.version&&!seen.has(key)){seen.add(key);native.push(j.native?.host?.timings??{});}}
 return {...metrics,nativeTimingMs:{host:distribution(native.map(n=>n.hostMs)),trace:distribution(native.map(n=>n.traceMs)),post:distribution(native.map(n=>n.postMs))},pass:metrics.pass&&panes.every(p=>p.pass),panes,inputToPixelMs:distribution(latencies),matchedInputs:latencies.length,totalInputs:raw.inputs.length,longTasks:distribution(raw.tasks.map(t=>t.ms)),peakGpuMiB:Math.max(0,...raw.presentations.map(p=>[...new Map(p.jobs.map(j=>[j.hostPid??j.session,j.native?.host?.gpuAllocatedMiB??0])).values()].reduce((a,b)=>a+b,0))),nativeOnly:raw.presentations.every(p=>p.backend==='native-studio'&&p.jobs.every(j=>j.backend==='native-studio'))};
}
export async function runEditorBenchmark(source=path.join(desktop,'dist/mac-arm64/Powermove.app')) {
 const config=options();await mkdir(config.evidence,{recursive:true});
 assert(config.seconds>0&&Number.isFinite(config.seconds)&&Number.isInteger(config.runs)&&config.runs>0&&Number.isInteger(config.finalFrames)&&config.finalFrames>=0&&Number.isFinite(config.stressMinutes)&&config.stressMinutes>=0&&Number.isFinite(config.stressWarmupMinutes)&&config.stressWarmupMinutes>=0,'Invalid benchmark duration/count');
 if(config.fixtures===path.join(studioCargoTarget(),'release/studio-environments'))execFileSync(process.env.CARGO??path.join(os.homedir(),'.cargo/bin/cargo'),['build','--manifest-path',path.resolve(desktop,'../../engine/Cargo.toml'),'--target-dir',studioCargoTarget(),'--release','--locked','-p','pm-studio','--features','oidn,metalfx,yardstick','--bin','studio-environments'],{env:{...process.env,CARGO_BUILD_JOBS:'2',CARGO_INCREMENTAL:'0',CARGO_PROFILE_RELEASE_DEBUG:'0'},stdio:['ignore','ignore','pipe']});
 assert(config.evidence.startsWith('/tmp/')||config.evidence.startsWith('/private/tmp/'),'Evidence must stay in a temporary directory');
 const sourceState=command('git',['status','--porcelain']);
 const initial=machine(new Set([process.pid])),report={date:initial.date,commit:command('git',['rev-parse','HEAD']),dirty:!!sourceState,changedFiles:sourceState.split('\n').filter(Boolean),harnessSha256:createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).digest('hex'),acceptanceEligible:!config.diagnostic&&initial.quiet,config,machine:initial,rows:[],errors:[]};
 if(!config.diagnostic)assert(initial.quiet,`Quiet-machine gate blocked by ${initial.active.map(p=>`${p.pid} ${p.name}`).join(', ')}`);
 const temporary=await mkdtemp(path.join(os.tmpdir(),'pm-p4-editor-')),profile=path.join(temporary,'profile');await mkdir(profile);await writeFile(path.join(profile,'onboarding-v1.json'),'{"version":1}\n');
 const env={...process.env,POWERMOVE_USER_DATA:profile,POWERMOVE_BACKGROUND_TEST:'0',POWERMOVE_DEVTOOLS:'0'};for(const k of ['ELECTRON_RUN_AS_NODE','POWERMOVE_STUDIO_HOST','PM_STUDIO_TEST_WEBGPU','STUDIO_LIVE_SCALE','STUDIO_ADAPTIVE_LIVE','STUDIO_NO_RT','STUDIO_NO_RIS','STUDIO_NO_METALFX','STUDIO_NO_SHARED_TEXTURE'])delete env[k];
 let app,memoryTimer,memoryPending,watchdog,lastProgress=Date.now();const save=()=>{lastProgress=Date.now();return writeFile(path.join(config.evidence,'editor-results.json'),JSON.stringify(report,null,2)+'\n');};
 try {
  let launchSource=source;
  if(!config.dev){
    launchSource=path.join(temporary,'Applications/Powermove.app');await cp(source,launchSource,{recursive:true,mode:constants.COPYFILE_FICLONE,verbatimSymlinks:true});
    await flipFuses(launchSource,{version:FuseVersion.V1,resetAdHocDarwinSignature:true,[FuseV1Options.EnableNodeCliInspectArguments]:true});
    command('/usr/bin/codesign',['--force','--sign','-','--options','runtime','--entitlements',path.join(desktop,'resources/entitlements.mac.adhoc.plist'),launchSource]);
    report.inspectorEnabledInDisposableCopy=true;
  }
  app=await _electron.launch({...(config.dev?{args:[path.join(desktop,'out/main/index.js')],cwd:desktop}:{executablePath:path.join(launchSource,'Contents/MacOS/Powermove')}),env,timeout:60000});
  watchdog=setInterval(()=>{if(Date.now()-lastProgress>180000){report.errors.push('Editor benchmark made no progress for three minutes');app.process().kill('SIGTERM');}},1000);
  app.process().stderr?.on('data',data=>{const message=String(data);void writeFile(path.join(config.evidence,'main-stderr.log'),message,{flag:'a'});if(/PM_STUDIO_BENCHMARK_FATAL|Could not release Studio surface/.test(message))report.errors.push(message.trim());});
  const pid=app.process().pid;report.hostSha256=createHash('sha256').update(await readFile(path.join(config.dev?path.join(desktop,'resources'):path.join(launchSource,'Contents/Resources'),'studio/pm-studio-host'))).digest('hex');
  report.addonSha256=createHash('sha256').update(await readFile(path.join(config.dev?path.join(desktop,'resources'):path.join(launchSource,'Contents/Resources'),'studio/studio-surface.node'))).digest('hex');
  await app.evaluate(({BrowserWindow,Menu,sharedTexture,dialog,app})=>{
   process.prependListener('uncaughtException',error=>{process.stderr.write(`PM_STUDIO_BENCHMARK_FATAL ${String(error)}\n`);app.exit(1);});
   globalThis.__perfTransfers=0;const send=sharedTexture.sendSharedTexture;sharedTexture.sendSharedTexture=(...args)=>{globalThis.__perfTransfers++;return Reflect.apply(send,sharedTexture,args);};
   dialog.showMessageBox=async()=>({response:2,checkboxChecked:false});
  });
  const page=await app.firstWindow();page.on('pageerror',e=>report.errors.push(String(e)));page.on('console',message=>{if(message.type()==='warning'||message.type()==='error')void writeFile(path.join(config.evidence,'renderer-console.log'),message.text()+'\n',{flag:'a'});});await page.waitForFunction(()=>!!window.PM?.app&&!!window.PM.Kernel,undefined,{timeout:60000});
  await page.evaluate(async()=>{const PM=window.PM;await PM.projectReady;await PM.Kernel.loader.builtinsReady;if(PM.ProjectsScreen.isOpen){window.dispatchEvent(new CustomEvent('pm-open-project',{detail:PM.mkProject({name:'Studio benchmark'})}));PM.ProjectsScreen.hide();}});
  await page.waitForFunction(()=>!!window.PM.GL?.gl&&!!document.querySelector('#stage'));
  await installProbe(page);
  report.window=await app.evaluate(({BrowserWindow,app,screen})=>({packaged:app.isPackaged,windows:BrowserWindow.getAllWindows().map(w=>({visible:w.isVisible(),minimized:w.isMinimized(),bounds:w.getBounds()})),displays:screen.getAllDisplays().map(d=>({size:d.size,scaleFactor:d.scaleFactor,refreshHz:d.displayFrequency})),electron:process.versions.electron}));
  assert(report.window.windows.some(w=>w.visible&&!w.minimized),'A normal visible window is required');
  const hz=report.window.displays[0]?.refreshHz||60;report.memory=[];memoryTimer=setInterval(()=>{if(memoryPending)return;memoryPending=(async()=>{const sample=memorySample(pid);const gpu=await page.evaluate(()=>[...new Map((window.PM.Studio?.diagnostics?.()??[]).filter(j=>j.backend==='native-studio').map(j=>[j.hostPid??j.session,{pid:j.hostPid,currentMiB:j.native?.host?.gpuAllocatedMiB??0,peakMiB:j.native?.host?.peakGpuMiB??0}])).values()]);sample.gpu=gpu;sample.gpuMiB=gpu.reduce((n,g)=>n+g.currentMiB,0);report.memory.push(sample);})().catch(error=>report.errors.push(`memory sampler: ${error}`)).finally(()=>{memoryPending=null;});},2000);
  for(const name of config.scenes){
   const before=machine(),owned=processTree(before,pid).owned;owned.add(process.pid);const state=machine(owned),row={name,machine:state,runs:[]};delete row.machine.processes;report.rows.push(row);await save();
   if(!config.diagnostic)assert(state.quiet,'Another lane became active before measurement');
   try {
    const cold=Date.now();await author(page,name,config);row.native=await nativeReady(page);row.coldSeconds=(Date.now()-cold)/1000;
    const coldTasks=await page.evaluate(()=>window.__studioPerf.tasks.filter(t=>t.at+t.ms>window.__studioPerf.sceneStarted));row.coldLongTasks=distribution(coldTasks.map(t=>t.ms));await writeFile(path.join(config.evidence,`${name}-cold-long-tasks.json`),JSON.stringify(coldTasks));
    row.scene=await page.evaluate(()=>({layers:window.PM.proj.layers.map(l=>({name:l.name,scatter:l.d?.data?.object?.scatter?.p?.count?.v})),allPanesDynamic:!!window.__studioPerf.allPanesDynamic,motionBlur:window.PM.proj.layers.some(l=>!!(typeof l.mblur==='object'?l.mblur?.v:l.mblur)),shutter:window.PM.proj.shutter,viewer:{width:document.querySelector('#gl').width,height:document.querySelector('#gl').height,pixelRatio:devicePixelRatio}}));
    await page.waitForTimeout(1500);
    for(let run=0;run<config.runs;run++){
     // Warmup through real camera navigation, then retain at least 30 completions.
     let warmed=0,attempts=0;while(warmed<30&&attempts++<30){const warm=await interval(page,'drag',2);warmed+=presentationMetrics(warm.presentations,warm.start,warm.end,hz).presentations;}
     assert(warmed>=30,'Warmup requires at least 30 native presentations');
     const result={run,warmupPresentations:warmed};
     for(const kind of ['drag','playback']){
      const machineBefore=measurementState(pid);if(!config.diagnostic)assert(machineBefore.quiet,'Another lane became active before measurement');
      let cdp,clock;if(config.profile){cdp=await page.context().newCDPSession(page);await cdp.send('Profiler.enable');await cdp.send('Profiler.setSamplingInterval',{interval:1000});clock=await page.evaluate(()=>performance.now());await cdp.send('Profiler.start');}
      const raw=await interval(page,kind,config.seconds);
      if(cdp){const profile=await cdp.send('Profiler.stop');await writeFile(path.join(config.evidence,`${name}-${run}-${kind}.cpuprofile`),JSON.stringify({...profile,rendererClockAtStart:clock}));await cdp.detach();}
      const machineAfter=measurementState(pid);await writeFile(path.join(config.evidence,`${name}-${run}-${kind}.json`),JSON.stringify(raw));result[kind]={...summarize(raw,hz),machineBefore,machineAfter};if(!config.diagnostic)assert(machineAfter.quiet,'Another lane became active during measurement');await nativeReady(page);
     }
     row.runs.push(result);await save();console.log(name,JSON.stringify({run,drag:{...result.drag,machineBefore:undefined,machineAfter:undefined},playback:{...result.playback,machineBefore:undefined,machineAfter:undefined}}));
    }
    const release=await page.evaluate(()=>{const PM=window.PM;PM.pause();PM.Kernel.api('studio-perf').scene3d.setView('camera');const start=performance.now();PM.setTime(0);PM.invalidate('render');return start;});
    await page.waitForFunction(release=>(window.PM.Studio.diagnostics?.()??[]).some(j=>!j.slot&&j.version&&j.inputAt>=release&&j.native?.host?.tier==='refine'&&j.native.host.samples>=32),release,{timeout:120000});
    row.refineViewer=await page.evaluate(release=>({seconds:(performance.now()-release)/1000,frames:window.PM.Studio.diagnostics(),longTasks:window.__studioPerf.tasks.filter(t=>t.at+t.ms>release)}),release);
    await page.screenshot({path:path.join(config.evidence,`${name}-refine-editor.png`)});
    row.refineMachine=measurementState(pid);if(!config.diagnostic)assert(row.refineMachine.quiet,'Another lane became active before Refine');
    row.refine=await page.evaluate(async()=>{const PM=window.PM;PM.pause();PM.Kernel.api('studio-perf').scene3d.setView('camera');PM.Studio.setPreview('quality');PM.agentFrameCapture=true;const start=performance.now();try{
      await PM.prepareFrame(0,{studioTier:'refine',studioSamples:32,width:1920,height:1080,mblur:false});const canvas=PM.renderFrameTo(0,1920,1080,{mblur:false});const frames=PM.Studio.captureDiagnostics();PM.agentFrameCapture=false;PM.GL.render(0,{mblur:false});const seconds=(performance.now()-start)/1000,check=new OffscreenCanvas(64,36),ctx=check.getContext('2d');ctx.drawImage(canvas,0,0,64,36);const rgb=[...ctx.getImageData(0,0,64,36).data].filter((_,i)=>i%4!==3),pixelRange=Math.max(...rgb)-Math.min(...rgb);return {seconds,pixelRange,frames,presented:PM.Studio.diagnostics(),png:canvas.toDataURL('image/png')};
    }finally{PM.agentFrameCapture=false;}});
    await writeFile(path.join(config.evidence,`${name}-refine.png`),Buffer.from(row.refine.png.split(',')[1],'base64'));delete row.refine.png;
    assert(row.refine.pixelRange>8,'A visible nonblank corpus frame is required');
    assert(row.refine.frames.length&&row.refine.frames.every(f=>f.backend==='native-studio'&&f.native?.host?.denoiser==='oidn'&&f.samples>=32&&f.width===1920&&f.height===1080),'Refine must be a native 1080p/32-sample OIDN image');
    row.final=[];
    row.finalMachineBefore=measurementState(pid);if(!config.diagnostic)assert(row.finalMachineBefore.quiet,'Another lane became active before Final');
    await page.evaluate(()=>{window.PM.agentFrameCapture=true;});
    try{
    for(let i=0;i<(config.finalFrames?config.finalFrames+1:0);i++){
     const frame=await page.evaluate(async({i,mblur})=>{const PM=window.PM,start=performance.now();await PM.prepareFrame(i/60,{exporting:true,width:1920,height:1080,mblur,mbSamples:16});const canvas=PM.renderFrameTo(i/60,1920,1080,{exporting:true,mblur,mbSamples:16}),seconds=(performance.now()-start)/1000;return {seconds,status:PM.Studio.status(),frames:PM.Studio.captureDiagnostics(),...(i===0?{png:canvas.toDataURL('image/png')}:{})};},{i,mblur:row.scene.motionBlur});
     if(frame.png){await writeFile(path.join(config.evidence,`${name}-final.png`),Buffer.from(frame.png.split(',')[1],'base64'));delete frame.png;}
     assert.equal(frame.status.backend,'native-studio');assert(frame.frames.length&&frame.frames.every(f=>f.backend==='native-studio'&&f.width===1920&&f.height===1080&&f.native?.host?.tier==='final'&&f.native.host.denoiser==='oidn'));assert(frame.frames.reduce((n,f)=>n+f.samples,0)>=64);row.final.push(frame);await save();
    }
    }finally{await page.evaluate(()=>{window.PM.agentFrameCapture=false;});}
    row.finalSeconds=distribution(row.final.slice(1).map(f=>f.seconds));
    row.finalMachineAfter=measurementState(pid);if(!config.diagnostic)assert(row.finalMachineAfter.quiet,'Another lane became active during Final');
    await page.screenshot({path:path.join(config.evidence,`${name}-editor.png`)});
   }catch(error){row.error=String(error);report.errors.push(`${name}: ${error}`);console.error(row.error);await save();}
  }
  if(config.stressMinutes>0){
   try{
   report.stress={started:Date.now(),samples:[],warmupSamples:[],cycles:0,warmupCycles:0,minutes:config.stressMinutes,warmupMinutes:config.stressWarmupMinutes,workingSet:config.stressFixedProjects?'two saved projects with retained Undo history':'fresh file identities, growing saved-project catalog',operations:'Save/reopen real synthetic projects; import/delete the tiny animated FBX; regenerate default landscape scatter; resize; GC and idle samples'};
   const projects=[];
   for(const name of ['hero','landscape']){
     await author(page,name,config);await nativeReady(page);const file=path.join(temporary,`${name}.pmv`);
     await app.evaluate(({dialog},file)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath:file});},file);
     assert(await page.evaluate(()=>window.PM.saveProject({saveAs:true})),'Synthetic stress project must save');projects.push({file,id:await page.evaluate(()=>window.PM.proj.id)});
   }
   await page.evaluate(async()=>{const PM=window.PM;for(const id of [...PM.Tabs.list()])if(!await PM.Tabs.close(id))throw new Error('Could not close a saved stress template');});
   report.stress.projectCountStart=await page.evaluate(()=>window.PM.Projects.list().length);
   const importedBytes=(await readFile(path.resolve(desktop,'../../engine/hosts/studio-electron/fixtures/tiny-animated.fbx'))).toString('base64');
   const workingFiles=[];
   report.stress.started=Date.now();report.stress.rendererStart=await page.evaluate(()=>{window.__studioPerf.tasks=[];return performance.now();});await save();
   const warmupEnd=Date.now()+config.stressWarmupMinutes*60000;let end=Infinity;
   while(Date.now()<end){
    if(!report.stress.measurementStarted&&Date.now()>=warmupEnd){report.stress.measurementStarted=Date.now();report.stress.warmupCycles=report.stress.cycles;end=Date.now()+config.stressMinutes*60000;report.stress.rendererStart=await page.evaluate(()=>{window.__studioPerf.tasks=[];return performance.now();});console.log('stress measurement started',JSON.stringify({warmupCycles:report.stress.warmupCycles,warmupMinutes:(Date.now()-report.stress.started)/60000}));await save();}
    const project=projects[report.stress.cycles%2];
    if(config.stressFixedProjects){
     await page.evaluate(async id=>{const PM=window.PM;PM.pause();if(!await PM.openProjectHere(id))throw new Error('Could not reopen saved stress project');PM.ProjectsScreen.hide();PM.AutomaticPreview?.cancel();PM.Studio.setPreview('quality');PM.invalidate('render');},project.id);
    }else{
     const file=path.join(temporary,`cycle-${report.stress.cycles}.pmv`);await cp(project.file,file,{mode:constants.COPYFILE_FICLONE});workingFiles.push(file);
     await app.evaluate(({dialog},file)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:[file]});},file);
     await page.evaluate(async()=>{const PM=window.PM;PM.pause();await PM.openProject();PM.AutomaticPreview?.cancel();PM.Studio.setPreview('quality');PM.invalidate('render');});
    }
    console.log('stress opened',report.stress.cycles);await nativeReady(page);
    while(workingFiles.length>2)await rm(workingFiles.shift(),{force:true});
    await page.evaluate(async base64=>{
      const PM=window.PM,api=PM.Kernel.api('studio-perf').scene3d,before=new Set(PM.proj.layers.map(l=>l.id));
      const bytes=Uint8Array.from(atob(base64),c=>c.charCodeAt(0)),prepared=await api.prepareImport([new File([bytes],'stress.fbx')]);const imported=await PM.cmd('3d.import-obj',prepared);if(imported?.ok===false)throw new Error(imported.message);
      const added=PM.proj.layers.filter(l=>!before.has(l.id));PM.GL.render(PM.time,{mblur:false});await PM.Studio.idle();
      for(const layer of added){const removed=api.edit({operation:'remove',target:layer.id});if(!removed.ok)throw new Error(removed.message);}
      const scatter=PM.proj.layers.find(l=>l.d?.data?.object?.scatter);if(scatter){const r=api.edit({operation:'update_scatter',target:scatter.id,patch:{p:{seed:scatter.d.data.object.scatter.p.seed.v+1}}});if(!r.ok)throw new Error(r.message);}
      PM.selectLayers([]);PM.GL.render(PM.time,{mblur:false});await PM.Studio.idle();if(!await PM.saveProject())throw new Error('Stress save failed');
    },importedBytes);
    const edited=await nativeReady(page),hostStats=edited.jobs.map(j=>j.native?.host).filter(Boolean);
    report.stress.peakHostMiB=Math.max(report.stress.peakHostMiB??0,...hostStats.map(h=>h.peakRssMiB??0));
    report.stress.peakGpuMiB=Math.max(report.stress.peakGpuMiB??0,...hostStats.map(h=>h.peakGpuMiB??0));
    await page.evaluate(async()=>{const PM=window.PM;if(!await PM.Tabs.close(PM.proj.id))throw new Error('Stress project close failed');});
    await app.evaluate(({BrowserWindow})=>{const w=BrowserWindow.getAllWindows()[0],b=w.getBounds();w.setSize(b.width===1400?1380:1400,b.height);});
    report.stress.cycles++;console.log('stress cycle',report.stress.cycles);if(report.stress.cycles%10===0){await page.requestGC();await page.waitForTimeout(2000);const sample=memorySample(pid);sample.editor=await page.evaluate(()=>({projects:window.PM.Projects.list().length,tabs:window.PM.Tabs.list().length,attachedNodes:document.querySelectorAll('*').length}));assert.equal(sample.editor.tabs,0,'Stress samples require closed project tabs');if(config.stressFixedProjects)assert.equal(sample.editor.projects,report.stress.projectCountStart,'Fixed-project stress must not grow the project catalog');sample.machine=measurementState(pid);if(!config.diagnostic)assert(sample.machine.quiet,'Another lane became active during stress');(report.stress.measurementStarted?report.stress.samples:report.stress.warmupSamples).push(sample);await save();console.log('stress',report.stress.cycles,JSON.stringify({rssMiB:sample.rssMiB,hostMiB:sample.hostMiB,projects:sample.editor.projects}));}
   }
   report.stress.totalElapsedMinutes=(Date.now()-report.stress.started)/60000;report.stress.elapsedMinutes=(Date.now()-report.stress.measurementStarted)/60000;report.stress.measuredCycles=report.stress.cycles-report.stress.warmupCycles;
   report.stress.trend=memoryTrend(report.stress.samples);
   const tasks=await page.evaluate(start=>window.__studioPerf.tasks.filter(t=>t.at>=start),report.stress.rendererStart);report.stress.longTasks=distribution(tasks.map(t=>t.ms));await writeFile(path.join(config.evidence,'stress-long-tasks.json'),JSON.stringify(tasks));
   }catch(error){report.stress.error=String(error);report.errors.push(`stress: ${error}`);console.error(String(error));await save();}
  }
  report.inputLatencyMethod='Trusted pointer/time input to matching native image composed by the editor and completed WebGL2 GPU fence; asynchronous rAF polling adds up to one display interval; display scanout is not measured';
  report.sharedTextureTransfers=await app.evaluate(()=>globalThis.__perfTransfers);assert(report.sharedTextureTransfers>0,'Shared textures must reach the real editor');
  clearInterval(memoryTimer);await memoryPending;report.peakRssMiB=Math.max(0,...report.memory.map(s=>s.rssMiB));report.peakHostMiB=Math.max(0,...report.memory.map(s=>s.maxHostMiB));
  const hosts=report.rows.flatMap(r=>[...(r.native?.jobs??[]),...(r.refine?.frames??[]),...(r.final??[]).flatMap(f=>f.frames)]).map(f=>f.native?.host).filter(Boolean);
  report.peakHostMiB=Math.max(report.peakHostMiB,report.stress?.peakHostMiB??0,...hosts.map(h=>h.peakRssMiB??0));
  report.peakGpuMiB=Math.max(0,report.stress?.peakGpuMiB??0,...report.memory.map(s=>s.gpuMiB??0),...hosts.map(h=>h.peakGpuMiB??0),...report.rows.flatMap(r=>r.runs.flatMap(run=>[run.drag.peakGpuMiB,run.playback.peakGpuMiB])));report.gpuMemory='Metal currentAllocatedSize high-water per host; overlaps unified RSS and is not added to it';
  report.swapGrowthMiB=swapMiB(safeCommand('/usr/sbin/sysctl',['vm.swapusage']))-swapMiB(initial.swap);
  report.gates={memory:report.peakRssMiB<=6144&&report.peakHostMiB<=3072&&report.swapGrowthMiB<=256,stress:report.acceptanceEligible&&config.stressWarmupMinutes>=5&&report.errors.length===0&&!!report.stress&&!report.stress.error&&report.stress.elapsedMinutes>=30&&report.stress.measuredCycles>=200&&report.stress.trend.pass&&report.orphanedHostPids?.length===0};
  for(const row of report.rows){const limit=['room','street','landscape-2x','forest-100k'].includes(row.name)?null:row.name==='landscape'?{refine:5,final:3}:{refine:2,final:1},intervals=row.runs.flatMap(r=>[r.drag,r.playback]),complete=report.acceptanceEligible&&row.runs.length>=3&&row.runs.every(r=>r.warmupPresentations>=30)&&intervals.every(r=>r.seconds>=30);row.protocol={liveComplete:complete,finalWarmFramesComplete:row.finalSeconds?.count>=30};row.gates={native:!row.error&&intervals.length>0&&intervals.every(r=>r.nativeOnly),live:!row.error&&complete&&intervals.every(r=>r.pass),input:!row.error&&complete&&intervals.every(r=>r.inputToPixelMs.count>0&&r.matchedInputs/r.totalInputs>=.95&&r.inputToPixelMs.p95<=50&&r.inputToPixelMs.p99<=83.3),mainThread:!row.error&&complete&&intervals.every(r=>(r.longTasks.max??0)<=50)&&(row.coldLongTasks?.max??0)<=50&&(row.refineViewer?.longTasks??[]).every(t=>t.ms<=50),refine:limit?report.acceptanceEligible&&!row.error&&row.refine?.seconds<=limit.refine&&row.refineViewer?.seconds<=limit.refine:null,final:limit?report.acceptanceEligible&&!row.error&&row.finalSeconds?.count>=30&&row.finalSeconds.p95<=limit.final:null};}
  await save();
 }finally{
  clearInterval(watchdog);
  clearInterval(memoryTimer);
  const hostPids=new Set((report.memory??[]).flatMap(s=>s.processes.filter(p=>p.name.endsWith('/pm-studio-host')).map(p=>p.pid)));
  if(app){await app.evaluate(async({BrowserWindow})=>{for(const w of BrowserWindow.getAllWindows())await w.webContents.executeJavaScript('window.PM?.prepareToClose?.({flush:true})');}).catch(()=>{});const p=app.process(),timer=setTimeout(()=>p.kill('SIGKILL'),6000);try{await app.close({timeout:10000});}catch{p.kill('SIGKILL');}clearTimeout(timer);}
  report.orphanedHostPids=processes().filter(p=>hostPids.has(p.pid)).map(p=>p.pid);if(report.gates&&report.stress)report.gates.stress=report.acceptanceEligible&&config.stressWarmupMinutes>=5&&report.errors.length===0&&!report.stress.error&&report.stress.elapsedMinutes>=30&&report.stress.measuredCycles>=200&&report.stress.trend?.pass&&report.orphanedHostPids.length===0;report.finished=Date.now();await save();await rm(temporary,{recursive:true,force:true});
 }
 console.log(`Evidence: ${path.join(config.evidence,'editor-results.json')}`);
 return report;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url))await runEditorBenchmark(process.argv.find(a=>a.endsWith('.app'))??path.join(desktop,'dist/mac-arm64/Powermove.app'));
