/** Local packaged acceptance; generated scenes and evidence never enter Git.
 * The supplied bundle keeps production fuses. Only a disposable relocated copy
 * enables the Node inspector, which Playwright's Electron driver requires.
 */
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { constants, createReadStream } from 'node:fs';
import { createHash } from 'node:crypto';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { _electron, chromium } from 'playwright';
import { verifyStudioRuntime } from './verify-studio-runtime.mjs';
import { flipFuses, FuseVersion, FuseV1Options, FuseState, getCurrentFuseWire } from '@electron/fuses';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const evidence = path.resolve(process.env.PM_STUDIO_SHOTS ?? '/tmp/pm-shots/studio3d/laneP1');
const source = path.resolve(process.argv[2] ?? path.join(desktop, 'dist/mac-arm64/Powermove.app'));
const report = { date: new Date().toISOString(), sourceCommit: execFileSync('git', ['rev-parse', 'HEAD'], {cwd:desktop,encoding:'utf8'}).trim(), sourceDirty: !!execFileSync('git',['status','--porcelain'],{cwd:desktop,encoding:'utf8'}).trim(), observations: [] };
const note = async (step, data) => { report.observations.push({step,...data}); await writeFile(path.join(evidence, 'packaged-results.json'), JSON.stringify(report,null,2)+'\n'); console.log(step, JSON.stringify(data)); };
const exec = (command, args) => execFileSync(command, args, {encoding:'utf8',maxBuffer:8*1024*1024});
const temporary = await mkdtemp(path.join(os.tmpdir(), 'pm-studio-package-'));
const relocated = path.join(temporary, 'Applications/Powermove.app');
const userData = path.join(temporary, 'profile');
let app, appProcess, ordinary, browser;
await mkdir(evidence, {recursive:true});
await mkdir(userData, {recursive:true});
await writeFile(path.join(userData,'onboarding-v1.json'), '{"version":1}\n');
const env = {...process.env, POWERMOVE_USER_DATA:userData, POWERMOVE_BACKGROUND_TEST:'0', POWERMOVE_DEVTOOLS:'0'};
for (const key of ['ELECTRON_RUN_AS_NODE','POWERMOVE_STUDIO_HOST','STUDIO_NO_SHARED_TEXTURE','PM_STUDIO_TEST_WEBGPU','STUDIO_LIVE_SCALE','STUDIO_ADAPTIVE_LIVE']) delete env[key];
const executable = path.join(relocated,'Contents/MacOS/Powermove');

async function ready(page) {
  await page.waitForFunction(() => !!window.PM?.app && !!window.PM?.Kernel, undefined, {timeout:60_000});
  await page.evaluate(async () => { const PM=window.PM; await PM.projectReady; await PM.Kernel.loader.builtinsReady; });
}
async function openScene(page) {
  await page.evaluate(() => {
    const PM=window.PM;
    if(PM.ProjectsScreen.isOpen){window.dispatchEvent(new CustomEvent('pm-open-project',{detail:PM.mkProject({name:'Packaged acceptance'})}));PM.ProjectsScreen.hide();}
  });
  await page.waitForFunction(()=>!!window.PM.GL?.gl&&!!window.PM.Kernel.services.get('viewer')?.stage?.isConnected);
  await page.evaluate(() => {
    const PM=window.PM; PM.pause(); PM.AutomaticPreview?.cancel();
    PM.replaceProject(PM.mkProject({name:'Packaged Studio acceptance',w:640,h:360,fps:24,dur:.5}));
    PM.ProjectsScreen.hide();
    const api=PM.Kernel.api('packaged-acceptance').scene3d;
    const edit=command=>{const result=api.edit(command);if(result?.ok===false)throw new Error(JSON.stringify(result));return result;};
    edit({operation:'add_camera',id:'camera',camera:{p:{x:6,y:4,z:8,targetX:0,targetY:.5,targetZ:0,fov:45}}});
    edit({operation:'set_world',id:'world',world:{preset:'golden-hour',p:{intensity:.5,sunIntensity:1,fogDensity:0}}});
    edit({operation:'add_terrain',id:'terrain',terrain:{p:{width:12,depth:12,resolution:32,amplitude:.3,frequency:.2,seed:41}}});
    edit({operation:'add_object',object:{id:'model',source:{primitive:'sphere',parameters:{radius:.8}},p:{x:-1,y:1},material:{p:{color:'#E45332',roughness:.35}}}});
    edit({operation:'add_object',object:{id:'prototype',source:{primitive:'cone',parameters:{radius:.12,height:.5,segments:8}},p:{x:100},material:{p:{color:'#3B6545',roughness:.8}}}});
    PM.Edit.apply({type:'set_layer',target:'prototype',patch:{visible:false}});
    edit({operation:'add_scatter',id:'scatter',scatter:{model:'prototype',surface:'terrain',p:{count:60,seed:71,scaleMin:.6,scaleMax:1.2}}});
    PM.Edit.apply({type:'add_layer',id:'title',layerType:'text',content:{text:'Studio',font:'Arial',weight:700,size:72,align:'center',color:'#E5E2FF'},properties:{'position.x':400,'position.y':90}});
    api.setExtrusion('title',{enabled:true,depth:24,bevelDepth:2,quality:'high'});
    edit({operation:'add_light',light:{id:'key',type:'area',p:{x:2,y:4,z:4,intensity:30,width:3,height:2}}});
    edit({operation:'set_environment',patch:{samples:16}});
    PM.Edit.apply({type:'replace_keyframes',target:'model',path:'position.x',keyframes:[{time:0,value:-1.5,ease:'linear'},{time:.5,value:-.5,ease:'linear'}]});
    PM.Studio.setPreview('fast'); PM.touch();
  });
  await page.waitForFunction(()=>window.PM.Studio.status().ready,undefined,{timeout:60_000});
}
async function frame(page, filename, width=640) {
  const image=await page.evaluate(async width => {
    const PM=window.PM, height=Math.round(width*9/16); PM.agentFrameCapture=true;
    try {
      await PM.prepareFrame(0,{mblur:false,width,height});
      const canvas=PM.renderFrameTo(0,width,height,{mblur:false});
      const p=canvas.getContext('2d').getImageData(0,0,width,height).data;
      let min=255,max=0,lit=0; for(let i=0;i<p.length;i+=4){min=Math.min(min,p[i],p[i+1],p[i+2]);max=Math.max(max,p[i],p[i+1],p[i+2]);if(p[i]+p[i+1]+p[i+2]>100)lit++;}
      return {png:canvas.toDataURL('image/png'),status:PM.Studio.status(),range:max-min,lit};
    } finally { PM.agentFrameCapture=false; }
  },width);
  if (filename) await writeFile(path.join(evidence,filename),Buffer.from(image.png.split(',')[1],'base64'));
  assert(image.range>32 && image.lit>1000,`Frame must contain rendered content: ${JSON.stringify({...image,png:undefined})}`);
  delete image.png; return image;
}
async function scene(page) {
  return page.evaluate(() => {
    const PM=window.PM,p=JSON.parse(JSON.stringify(PM.proj));
    const normalize=layers=>layers.map(layer=>({...layer,solo:!!layer.solo}));
    for(const comp of Object.values(p.comps??{}))if(comp.layers)comp.layers=normalize(comp.layers);
    return {layers:normalize(p.layers),comps:p.comps??null,w:p.w,h:p.h,fps:p.fps,dur:p.dur,scene3d:p.scene3d??null};
  });
}
function nativePids(parent) {
  return exec('/bin/ps',['-axo','pid=,ppid=,comm=']).split('\n').map(line=>line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/)).filter(m=>m&&+m[2]===parent&&m[3].endsWith('/pm-studio-host')).map(m=>+m[1]);
}
async function quit() {
  if (!app) return;
  const active=app, child=appProcess;
  await active.evaluate(async ({BrowserWindow})=>{
    for(const window of BrowserWindow.getAllWindows())if(!window.isDestroyed())await window.webContents.executeJavaScript('window.PM?.prepareToClose?.({flush:true}) ?? true');
  });
  const pid=child.pid, owned=nativePids(pid);
  let completed=false;
  await active.evaluate(()=>{process.once('exit',code=>{if(code===0)process.getBuiltinModule('node:fs').writeSync(2,'PM_PACKAGED_CLEAN_EXIT\n');});});
  child.stderr.on('data',data=>{if(String(data).includes('PM_PACKAGED_CLEAN_EXIT'))completed=true;});
  const timer=setTimeout(()=>{if(completed)child.kill('SIGKILL');},5000);timer.unref();
  try { await active.close({timeout:15_000}); } finally {clearTimeout(timer); if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL'); app=null;appProcess=null;}
  assert.equal(nativePids(pid).length,0,'Native hosts must exit on app quit');
  for(const native of owned){let alive=false;try{process.kill(native,0);alive=true;}catch{}assert.equal(alive,false,'Quit must not leave an orphan native host');}
}
async function launch() {
  app=await _electron.launch({executablePath:executable,env,timeout:30_000});
  appProcess=app.process();
  app.process().stderr.on('data',chunk=>void writeFile(path.join(evidence,'packaged-stderr.log'),chunk,{flag:'a'}));
  await app.evaluate(({dialog,sharedTexture})=>{
    dialog.showMessageBox=async()=>({response:2,checkboxChecked:false});
    const send=sharedTexture.sendSharedTexture; globalThis.__studioSharedTransfers=0;
    sharedTexture.sendSharedTexture=(...args)=>{globalThis.__studioSharedTransfers++;return Reflect.apply(send,sharedTexture,args);};
  });
  await note('main-started',await app.evaluate(({app,BrowserWindow})=>({ready:app.isReady(),windows:BrowserWindow.getAllWindows().map(w=>({url:w.webContents.getURL()})),resources:process.getActiveResourcesInfo()})));
  app.on('console',message=>void writeFile(path.join(evidence,'main-console.log'),message.text()+'\n',{flag:'a'}));
  const page=await app.firstWindow({timeout:60_000}).catch(async error=>{await note('main-timeout',await app.evaluate(({app,BrowserWindow})=>({ready:app.isReady(),windows:BrowserWindow.getAllWindows().length,resources:process.getActiveResourcesInfo(),argv:process.argv,modules:Object.keys(process.getBuiltinModule('node:module')._cache).filter(p=>p.includes('out/main')).map(p=>p.split('/out/main/')[1])})));throw error;}); page.on('pageerror',error=>void writeFile(path.join(evidence,'page-errors.log'),String(error)+'\n',{flag:'a'}));
  await ready(page); assert(await app.evaluate(({app})=>app.isPackaged)); return page;
}

try {
  exec('/usr/bin/codesign',['--verify','--deep','--strict',source]);
  const binaries=await verifyStudioRuntime(source,{adHoc:true});
  const hashes={};
  for(const relative of ['Contents/Resources/app.asar','Contents/Resources/studio/pm-studio-host']) {
    const hash=createHash('sha256');for await(const part of createReadStream(path.join(source,relative)))hash.update(part);hashes[relative]=hash.digest('hex');
  }
  await note('hardened-signing',{binaries,notarized:false,hashes});
  await cp(source,relocated,{recursive:true,mode:constants.COPYFILE_FICLONE,verbatimSymlinks:true});
  const fuses=await getCurrentFuseWire(relocated);
  assert.equal(fuses[FuseV1Options.EnableNodeCliInspectArguments],FuseState.DISABLE,'Production Node inspector fuse must be disabled');
  ordinary=spawn(executable,['--remote-debugging-port=0'],{env,stdio:['ignore','pipe','pipe']});
  ordinary.stdout.on('data',data=>void writeFile(path.join(evidence,'ordinary-stdout.log'),data,{flag:'a'}));
  ordinary.stderr.on('data',data=>void writeFile(path.join(evidence,'ordinary-stderr.log'),data,{flag:'a'}));
  const websocket=await new Promise((resolve,reject)=>{
    let log='';const timer=setTimeout(()=>reject(new Error('Ordinary packaged launch timed out')),30_000);
    ordinary.once('exit',(code,signal)=>{clearTimeout(timer);reject(new Error(`Ordinary package exited ${signal??code}`));});
    ordinary.stderr.on('data',data=>{log+=data;const match=/DevTools listening on (ws:\/\/[^\s]+)/.exec(log);if(match){clearTimeout(timer);resolve(match[1]);}});
  });
  browser=await chromium.connectOverCDP(websocket);
  await note('ordinary-cdp',{contexts:browser.contexts().length,pages:browser.contexts().map(c=>c.pages().map(p=>p.url()))});
  const ordinaryPage=browser.contexts()[0].pages().find(p=>p.url().startsWith('app:'))??browser.contexts()[0].pages()[0]??await browser.contexts()[0].waitForEvent('page',{timeout:30_000});
  await ready(ordinaryPage); await openScene(ordinaryPage);
  const normalFrame=await frame(ordinaryPage,'01-production-fuses.png');assert.equal(normalFrame.status.backend,'native-studio');
  await ordinaryPage.screenshot({path:path.join(evidence,'01-production-window.png')});
  await note('ordinary-packaged-editor',{...normalFrame,inspectorFuseEnabled:false,backgroundTest:false,nativeProcesses:nativePids(ordinary.pid).length});
  const disconnect=browser.close();browser=null;
  if(ordinary.exitCode===null&&ordinary.signalCode===null){const exited=new Promise(resolve=>ordinary.once('exit',resolve));ordinary.kill('SIGTERM');const reap=setTimeout(()=>ordinary?.kill('SIGKILL'),1500);await exited;clearTimeout(reap);}ordinary=null;await disconnect;
  // A separate profile avoids any first-launch state from the smoke test.
  await rm(userData,{recursive:true,force:true});await mkdir(userData);await writeFile(path.join(userData,'onboarding-v1.json'),'{"version":1}\n');
  await flipFuses(relocated,{version:FuseVersion.V1,resetAdHocDarwinSignature:true,[FuseV1Options.EnableNodeCliInspectArguments]:true});
  exec('/usr/bin/codesign',['--force','--sign','-','--options','runtime','--entitlements',path.join(desktop,'resources/entitlements.mac.adhoc.plist'),relocated]);
  exec('/usr/bin/codesign',['--verify','--deep','--strict',relocated]);
  let page=await launch();await openScene(page);
  const live=await frame(page,'02-native-live.png');assert.equal(live.status.backend,'native-studio');
  const transfers=await app.evaluate(()=>globalThis.__studioSharedTransfers);assert(transfers>0,'Native shared-texture frames must reach the editor');
  await note('playwright-native-editor',{...live,sharedTextureTransfers:transfers,inspectorCopyOnly:true});
  for(const width of [320,640,480,640,800,640]){const resized=await frame(page,null,width);assert.equal(resized.status.backend,'native-studio');}
  await note('resize',{sizes:[320,640,480,640,800,640],backend:'native-studio'});
  const mp4=path.join(evidence,'final.mp4');
  await app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});},mp4);
  const exported=await page.evaluate(async()=>{const PM=window.PM;const result=await PM.Export.run({format:'mp4',scale:1,fps:24,range:'all',quality:'high',mblur:false,audio:false,reportPath:true});return {result,status:PM.Studio.status()};});
  assert(!exported.result.error,JSON.stringify(exported.result));assert.equal(exported.status.backend,'native-studio');assert(exported.status.frameSamples>=16);
  const probe=JSON.parse(exec('ffprobe',['-v','error','-count_frames','-show_streams','-show_format','-of','json',mp4]));
  const stream=probe.streams.find(s=>s.codec_type==='video');assert.equal(stream.codec_name,'h264');assert.equal(stream.width,640);assert.equal(stream.height,360);assert.equal(+stream.nb_read_frames,12);
  exec(path.join(relocated,'Contents/Resources/encoder/ffmpeg'),['-y','-v','error','-i',mp4,'-frames:v','1',path.join(evidence,'03-final-frame.png')]);
  const frameHashes=exec(path.join(relocated,'Contents/Resources/encoder/ffmpeg'),['-v','error','-i',mp4,'-an','-f','framemd5','pipe:1']).split('\n').filter(line=>line&&!line.startsWith('#')).map(line=>line.split(',').at(-1).trim());
  assert.equal(frameHashes.length,12);assert(new Set(frameHashes).size>1,'Animated model must advance in the encoded video');
  await writeFile(path.join(evidence,'ffprobe.json'),JSON.stringify(probe,null,2)+'\n');await note('native-final-mp4',{status:exported.status,frames:+stream.nb_read_frames,uniqueFrames:new Set(frameHashes).size,duration:+probe.format.duration});
  await page.evaluate(()=>{window.PM.Kernel.api('packaged-acceptance').scene3d.edit({operation:'update_object',target:'model',patch:{p:{y:1.2}}});});
  const beforeCrash=await scene(page),pids=nativePids(app.process().pid);assert(pids.length>0,'Need a real native child to crash');pids.forEach(pid=>process.kill(pid,'SIGKILL'));
  const recovered=await frame(page,'04-recovered.png');assert.deepEqual(await scene(page),beforeCrash);assert.equal(recovered.status.backend,'webgpu');
  await page.evaluate(()=>{window.PM.Kernel.api('packaged-acceptance').scene3d.edit({operation:'update_object',target:'model',patch:{p:{y:1.5}}});});
  const afterEdit=await scene(page);assert.notDeepEqual(afterEdit,beforeCrash);await frame(page,null);
  await note('crash-recovery',{backend:recovered.status.backend,scenePreserved:true,subsequentEdit:true,crashedProcesses:pids.length});
  const project=path.join(temporary,'Acceptance.pmv');
  await app.evaluate(({dialog},filePath)=>{dialog.showSaveDialog=async()=>({canceled:false,filePath});},project);
  assert.equal(await page.evaluate(()=>window.PM.saveProject({saveAs:true})),true);
  const saved=await scene(page);await quit();
  page=await launch();
  await app.evaluate(({dialog},filePath)=>{dialog.showOpenDialog=async()=>({canceled:false,filePaths:[filePath]});},project);
  await page.evaluate(()=>window.PM.openProject());await ready(page);assert.deepEqual(await scene(page),saved);
  const reopened=await frame(page,'05-reopened-native.png');assert.equal(reopened.status.backend,'native-studio');
  await note('save-quit-reopen',{identicalScene:true,backend:reopened.status.backend,layers:saved.layers.length});
  const prior=nativePids(appProcess.pid);
  await page.reload({waitUntil:'domcontentloaded'});await ready(page);
  assert.deepEqual(await scene(page),saved);
  const reloaded=await frame(page,'06-reloaded-native.png');assert.equal(reloaded.status.backend,'native-studio');
  assert(prior.every(pid=>!nativePids(appProcess.pid).includes(pid)),'Reload must release the old host');
  await note('renderer-reload',{identicalScene:true,freshHost:true,backend:reloaded.status.backend});
  const opened=app.waitForEvent('window');
  await app.evaluate(({Menu})=>{const item=Menu.getApplicationMenu().getMenuItemById('newWindow');item.click(item,undefined,{});});
  const second=await opened;await ready(second);await openScene(second);
  const other=await frame(second,'07-second-window.png');assert.equal(other.status.backend,'native-studio');
  assert.deepEqual(await scene(page),saved);
  const secondWindow=await app.browserWindow(second);
  await second.evaluate(()=>window.PM.prepareToClose({flush:true}));
  const closed=second.waitForEvent('close');await secondWindow.evaluate(window=>window.close());await closed;
  const retained=await frame(page,null);assert.equal(retained.status.backend,'native-studio');assert.deepEqual(await scene(page),saved);
  await note('two-windows',{bothNative:true,independentScenes:true,remainingWindowNative:true});
  await quit();
  await writeFile(path.join(evidence,'captions.md'),'# Packaged Studio acceptance\n\n01-production-fuses.png and 01-production-window.png — Relocated, hardened ad-hoc app with unchanged production fuses, ordinary visible window, native backend.\n\n02-native-live.png — Model, Arial extruded text, sky, terrain, seeded scatter and area light in a packaged editor; the disposable Playwright copy enables only the Node inspector fuse.\n\n03-final-frame.png — First decoded frame from the native Final H.264 MP4 (640×360, 24 fps, 12 frames, 16 samples and OIDN).\n\n04-recovered.png — Same unsaved scene after killing the native child; WebGPU fallback preserves edits.\n\n05-reopened-native.png — Saved scene reopened after quitting; complete persisted layer/scene data matches and a fresh native host renders it.\n\n06-reloaded-native.png — Reloaded packaged renderer retains its scene and replaces the old host.\n\n07-second-window.png — A second editor window renders its own scene through a second native session; closing it preserves the first window.\n');
  await note('complete',{packagedAcceptance:true});
} catch(error) {await note('failed',{error:String(error)});throw error;}
finally {
  await browser?.close().catch(()=>{});
  ordinary?.kill('SIGKILL');
  if(app){try{await quit();}catch{appProcess?.kill('SIGKILL');}}
  await rm(temporary,{recursive:true,force:true});
}
