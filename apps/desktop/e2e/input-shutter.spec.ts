import {expect,test,repoRoot} from './helpers/app';
import {execFileSync} from 'node:child_process';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {createServer} from 'node:http';

test('referenced video samples exact shutter times through reverse seeks and repeated precomps',async({session},info)=>{
 test.setTimeout(60_000);await session.openEditor();const directory=await mkdtemp(path.join(os.tmpdir(),'powermove-input-shutter-'));
 try{
  const file=path.join(directory,'step.mp4'),raw=Buffer.concat(Array.from({length:64},(_,i)=>Buffer.alloc(32*32*3,i<32?16:240)));
  execFileSync(path.join(repoRoot,'node_modules/ffmpeg-static/ffmpeg'),['-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','32x32','-r','64','-i','pipe:0','-c:v','libx264','-crf','0','-pix_fmt','yuv444p',file],{input:raw});
  await session.app.evaluate(({ipcMain})=>{const state={chunks:[] as number[][],started:0};(globalThis as any).__shutterExport=state;for(const n of ['start','write','finish','cancel'])ipcMain.removeHandler(`render:${n}`);ipcMain.handle('render:start',()=>{state.started++;return'shutter';});ipcMain.handle('render:write',(_e,args)=>{state.chunks.push([...args.data]);});ipcMain.handle('render:finish',()=>({path:'/test/shutter'}));ipcMain.handle('render:cancel',()=>{});});
  const result=await session.page.evaluate(async(bytes)=>{
   const PM=(window as any).PM;PM.pause();PM.agentFrameCapture=true;PM.quality=1;PM.AutomaticPreview?.cancel();
   const project=PM.mkProject({name:'Input shutter',w:64,h:32,fps:4,dur:1});project.colorDepth=16;project.shutter=1;project.backgroundFill={type:'none',stops:[],angle:0};PM.replaceProject(project);
   const asset=await PM.assets.add(new File([new Uint8Array(bytes)],'step.mp4',{type:'video/mp4'}),{silent:true});
   const group=PM.mkLayer('group',{name:'Sampled group',p:{'position.x':0,'position.y':0}},project);group.mblur=true;
   const owner=PM.mkLayer('shape',{name:'Matte owner',d:{w:32,h:32,radius:0,color:'#FF0000'},p:{'position.x':16,'position.y':16}},project);owner.group=group.id;
   const video=PM.mkLayer('video',{name:'Hidden step',d:{asset:asset.id,w:32,h:32,speed:1,trim:0,embeddedAudio:false},p:{'position.x':16,'position.y':16}},project);video.on=false;
   const effect=PM.mkEffect('setMatte',undefined,owner);effect.inputs.map={layerId:video.id,stage:'source'};effect.p.channel.v=4;owner.fx=[effect];project.layers=[group,owner,video];PM.touch();
   const pixel=(p:any,x=16)=>[...p.slice(((32-1-16)*64+x)*4,((32-1-16)*64+x)*4+4)];
   const samples:any[]=[];
   for(const shutter of [1,-1])for(const time of [.5,.25,.75,.5]){const options={mblur:true,mbSamples:4,shutter,transparent:true};await PM.prepareFrame(time,options);const requests=PM.GL.frameVideoRequests(project,time,options);samples.push({time,shutter,count:requests.length,pixel:pixel(PM.GL.renderToPixels(time,64,32,options)),keys:[...PM.preparedVideoFrames.keys()]});}
   // The same exact shutter contract must hold for a source raster that has
   // no visible composition pixels and whose ordinary opacity is zero.
   effect.inputs.map.space='source';video.p.opacity.v=0;video.p['position.x'].v=10000;video.p['position.y'].v=-10000;PM.touch();
   for(const shutter of [1,-1])for(const time of [.5,.25,.75,.5]){const options={mblur:true,mbSamples:4,shutter,transparent:true};await PM.prepareFrame(time,options);samples.push({time,shutter,count:PM.GL.frameVideoRequests(project,time,options).length,pixel:pixel(PM.GL.renderToPixels(time,64,32,options)),space:'source'});}
   group.mblur=false;owner.mblur=true;owner.effectSpace='source';PM.touch();
   for(const shutter of [1,-1])for(const time of [.5,.25,.75,.5]){const options={mblur:true,mbSamples:4,shutter,transparent:true};await PM.prepareFrame(time,options);samples.push({time,shutter,count:PM.GL.frameVideoRequests(project,time,options).length,pixel:pixel(PM.GL.renderToPixels(time,64,32,options)),ownerSpace:'source'});}
   // Stop the editor loop while exercising the live viewer's private subframe
   // preparation directly. It must not substitute the central streaming frame.
   PM.preparedVideoFrames=null;PM.preparedVideoExactInputs=null;PM.agentFrameCapture=false;PM.Export.busy=true;PM.GL.resize(64,32);
   const liveOptions={mblur:true,mbSamples:4,shutter:1};let liveReady=false,live:any;
   try{for(let retry=0;retry<400;retry++){if(PM.GL.render(.5,liveOptions)!==false){liveReady=true;break;}await new Promise(resolve=>setTimeout(resolve,10));}
    const data=new Uint8Array(64*32*4);PM.GL.gl.readPixels(0,0,64,32,PM.GL.gl.RGBA,PM.GL.gl.UNSIGNED_BYTE,data);
    live={pixel:pixel(data),ready:liveReady,presented:PM.GL.inputPreviewPresentedTime};
   }finally{PM.GL.cancelInputPreview();PM.Export.busy=false;PM.agentFrameCapture=true;}
   const nativeOptions={mblur:true,mbSamples:16,shutter:1,opaque:true,topDownOpaque:true},native:number[]=[];
   for(const time of [0,.25,.5,.75]){await PM.prepareFrame(time,nativeOptions);native.push(...PM.GL.renderToPixels(time,64,32,nativeOptions));}
   const web=await PM.Export.buildWeb(),exported=await PM.Export.run({format:'mp4',alpha:false,scale:1,fps:4,range:'all',quality:'draft',audio:false,mblur:true});
   const portableOptions={mblur:true,mbSamples:6,shutter:1,transparent:true};await PM.prepareFrame(.5,portableOptions);const portable=[...PM.GL.renderToPixels(.5,64,32,portableOptions)];
   const files=[...web.files].map(([name,bytes]:any)=>[name,[...bytes]]);
   const child={...project,id:PM.uid('comp'),w:32,h:32,layers:[group,owner,video],comps:{}};project.comps[child.id]=child;
   const a=PM.mkLayer('precomp',{name:'A',d:{comp:child.id,w:32,h:32,trim:.25},p:{'position.x':0,'position.y':0}},project),b=PM.mkLayer('precomp',{name:'B',d:{comp:child.id,w:32,h:32,trim:0},p:{'position.x':32,'position.y':0}},project);b.p['position.x'].v=32;project.layers=[a,b];PM.touch();
   const options={mblur:true,mbSamples:4,shutter:1,transparent:true};await PM.prepareFrame(.25,options);let p;try{p=PM.GL.renderToPixels(.25,64,32,options);}catch(e:any){throw Error(e.message+' Prepared: '+JSON.stringify([...PM.preparedVideoFrames.keys()]));}
   const nested={left:pixel(p),right:pixel(p,48),keys:[...PM.preparedVideoFrames.keys()],a:a.id,b:b.id};
   const missingKey=[...PM.preparedVideoFrames.keys()].find((k:any)=>k.startsWith(a.id+'/')&&!k.endsWith('@0.5'));PM.preparedVideoFrames.delete(missingKey);let missing='';try{PM.GL.renderToPixels(.25,64,32,options);}catch(e:any){missing=e.message;}
   return{samples,live,native,exported,files,portable,nested,missing,errors:[...PM.GL.errors]};
  },[...await readFile(file)]);
  for(const sample of result.samples){const expected=sample.time===.5?128:sample.time===.25?16:240;expect(Math.abs(sample.pixel[0]-expected)).toBeLessThanOrEqual(2);expect(sample.pixel[1]).toBe(0);expect(sample.pixel[2]).toBe(0);expect(Math.abs(sample.pixel[3]-expected)).toBeLessThanOrEqual(2);expect(sample.count).toBe(5);}
  expect(result.live.ready).toBe(true);expect(result.live.presented).toBe(.5);expect(Math.abs(result.live.pixel[0]-128)).toBeLessThanOrEqual(2);expect(result.live.pixel.slice(1)).toEqual([0,0,255]);
  expect(result.exported).toEqual({cancelled:false});const delivered=await session.app.evaluate(()=>(globalThis as any).__shutterExport);expect(delivered.started).toBe(1);expect(delivered.chunks.flat()).toEqual(result.native);
  const files=new Map<string,Buffer>(result.files.map(([name,bytes]:any)=>[name,Buffer.from(bytes)]));const server=createServer((request,response)=>{const name=request.url==='/'?'index.html':request.url!.slice(1),data=files.get(name);response.statusCode=data?200:404;response.setHeader('Content-Type',name.endsWith('.js')?'text/javascript':name.endsWith('.json')?'application/json':name.endsWith('.mp4')?'video/mp4':'text/html');response.end(data||'Not found');});await new Promise<void>(resolve=>server.listen(0,'127.0.0.1',resolve));
  try{const pending=session.app.waitForEvent('window');await session.app.evaluate(({BrowserWindow},url)=>{const window=new BrowserWindow({show:false,webPreferences:{nodeIntegration:false,contextIsolation:true,sandbox:true}});(globalThis as any).__shutterPlayer=window;void window.loadURL(url);},`http://127.0.0.1:${(server.address() as any).port}`);const page=await pending;await page.waitForLoadState('domcontentloaded');const portable=await page.evaluate(async()=>{const{createPlayer}=await import(/* @vite-ignore */location.origin+'/player.js'),canvas=document.createElement('canvas');canvas.style.width=`${64/devicePixelRatio}px`;canvas.style.height=`${32/devicePixelRatio}px`;document.body.append(canvas);const player=await createPlayer({canvas,scene:'./scene.json',transparent:true,audio:false});try{await player.seek(.5);const gl=canvas.getContext('webgl2')!,pixels=new Uint8Array(64*32*4);gl.readPixels(0,0,64,32,gl.RGBA,gl.UNSIGNED_BYTE,pixels);return[...pixels];}finally{player.destroy();}});expect(Math.max(...portable.map((v,i)=>Math.abs(v-result.portable[i]!)))).toBeLessThanOrEqual(1);
  }finally{await session.app.evaluate(()=>(globalThis as any).__shutterPlayer?.destroy());await new Promise<void>(resolve=>server.close(()=>resolve()));}
  expect(Math.abs(result.nested.left[0]-128)).toBeLessThanOrEqual(2);expect(Math.abs(result.nested.right[0]-16)).toBeLessThanOrEqual(2);expect(result.nested.keys.some((key:string)=>key.startsWith(result.nested.a+'/'))).toBe(true);expect(result.nested.keys.some((key:string)=>key.startsWith(result.nested.b+'/'))).toBe(true);expect(result.missing).toContain('prepared video frame');expect(result.errors).toEqual([]);expect(session.diagnostics.pageErrors).toEqual([]);
  await info.attach('shutter-reference-pixels',{body:JSON.stringify({...result,files:undefined,native:undefined,portable:undefined},null,2),contentType:'application/json'});
 }finally{await rm(directory,{recursive:true,force:true});}
});
