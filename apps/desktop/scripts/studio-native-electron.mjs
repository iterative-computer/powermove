/** Hidden Electron 44 integration/latency run. No server and no editor profile. */
import { _electron as electron } from '@playwright/test';
import { build } from 'esbuild';
import { mkdtemp, mkdir, writeFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = process.env.PM_STUDIO_SHOTS ?? '/tmp/pm-shots/studio3d/laneB';
await mkdir(out, { recursive: true });
const fixture = await mkdtemp(path.join(tmpdir(), 'pm-studio-electron-'));
const host = process.env.POWERMOVE_STUDIO_HOST ?? path.join(desktop, 'resources/studio/pm-studio-host');
const main = `
const { app, BrowserWindow, ipcMain } = require('electron');
const { registerStudioNativeIpc } = require(${JSON.stringify(path.join(desktop, 'src/main/studio-native/ipc.ts'))});
app.setPath('userData', ${JSON.stringify(path.join(fixture, 'profile'))});
app.commandLine.appendSwitch('enable-unsafe-webgpu');
const dispose=registerStudioNativeIpc(ipcMain,${JSON.stringify(host)},()=>true);app.on('will-quit',dispose);
app.whenReady().then(()=>{const win=new BrowserWindow({show:false,width:1920,height:1080,webPreferences:{sandbox:true,contextIsolation:true,backgroundThrottling:false,preload:${JSON.stringify(path.join(fixture, 'preload.cjs'))}}});win.loadFile(${JSON.stringify(path.join(fixture, 'index.html'))});});
`;
const preload = `const {contextBridge}=require('electron');const {studioNativeBridge}=require(${JSON.stringify(path.join(desktop,'src/preload/studio-native.ts'))});contextBridge.exposeInMainWorld('studioTest',studioNativeBridge());`;
await build({stdin:{contents:preload,resolveDir:desktop,sourcefile:'studio-test-preload.cjs'},bundle:true,platform:'node',format:'cjs',external:['electron'],outfile:path.join(fixture,'preload.cjs')});
await build({ stdin: { contents: main, resolveDir: desktop, sourcefile: 'studio-test-main.cjs' }, bundle: true, platform: 'node', format: 'cjs', external: ['electron'], outfile: path.join(fixture, 'main.cjs') });
const renderer = `
import * as THREE from 'three';
import {nativeEnvironment} from './scripts/studio-native-environments';
import {NativeStudioBackend} from './src/renderer/src/core/studio-native/backend';
import {RecoveringStudioBackend} from './src/renderer/src/core/studio-native/recovery';
import {WebGpuStudioBackend} from './src/renderer/src/core/studio/webgpu-backend';
const bridge=window.studioTest;
const capture=source=>{const canvas=document.createElement('canvas');canvas.width=source.width;canvas.height=source.height;canvas.getContext('2d').drawImage(source,0,0);return canvas.toDataURL('image/png').split(',')[1];};
const median=values=>[...values].sort((a,b)=>a-b)[Math.floor(values.length/2)];
window.runStudioNative=async(name,size,frames)=>{
 let environment=null,scene,m,maps=[];if(name==='room'||name==='room-ortho'||name==='forest'){environment=nativeEnvironment(name);scene=environment.scene;m={view:environment.view};}else{
 await bridge.request('corpus',{op:'preset',name});const snapshot=await bridge.request('corpus',{op:'snapshot'});await bridge.release('corpus');
 m=snapshot.result;const b=snapshot.binary;const bytes=r=>b.slice(r.offset,r.offset+r.length);const floats=r=>new Float32Array(bytes(r).buffer);
 const allNodes=bytes(m.nodes);const topBytes=m.topNodes*64;
 scene={positions:floats(m.positions),tris:floats(m.tris),materials:floats(m.materials),sdata:floats(m.sdata),tlasNodes:allNodes.slice(0,topBytes).buffer,blasNodes:allNodes.slice(topBytes).buffer,geometryKey:name,instanceCount:m.instanceCount,lightCount:m.lightCount,objects:m.objects.map(o=>o.layer),triangles:m.tris.length/96,maps:[],bounds:new THREE.Box3(new THREE.Vector3(...m.bounds.min),new THREE.Vector3(...m.bounds.max))};
 if(m.texture){const t=m.texture,rgba=bytes(t.bytes);const input=new OffscreenCanvas(t.width,t.height);input.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rgba),t.width,t.height),0,0);const map=new OffscreenCanvas(1024,1024);map.getContext('2d').drawImage(input,0,0,1024,1024);maps=[map];}
 }
 const native=new NativeStudioBackend(bridge);const recovery=new RecoveringStudioBackend(native,()=>new WebGpuStudioBackend(),{invalidate:()=>{}});await recovery.initialize();const description=()=>{throw new Error('binary fast path should not build JSON mesh copies');};recovery.setPreparedScene(scene,maps,description);recovery.setView(m.view);
 const request={tier:'live',size,samples:1,background:true};const coldStart=performance.now();let frame=await native.render(request);const inputSize={width:native.lastFrame.host.inputWidth,height:native.lastFrame.host.inputHeight};frame.release?.();const coldMs=performance.now()-coldStart;
 const inputPixels=[],hostTimes=[],traceTimes=[],postTimes=[],roundTrips=[],canvasMs=[];
 const display=document.createElement('canvas');display.width=size.width;display.height=size.height;document.body.append(display);const ctx=display.getContext('2d',{willReadFrequently:true});
 for(let i=0;i<frames;i++){
  const start=performance.now();const view=structuredClone(m.view);view.camera.position[0]+=i*.02;native.setView(view);native.setTime(i/60);frame=await native.render(request);ctx.drawImage(frame.source,0,0,size.width,size.height);ctx.getImageData(size.width>>1,size.height>>1,1,1);inputPixels.push(performance.now()-start);hostTimes.push(native.lastFrame.host.timings.hostMs);traceTimes.push(native.lastFrame.host.timings.traceMs);postTimes.push(native.lastFrame.host.timings.postMs);roundTrips.push(native.lastFrame.roundTripMs);canvasMs.push(native.lastFrame.pixelsMs);frame.release?.();
 }
 const livePng=capture(display);const liveTransport=native.lastFrame.host.transport??'rgba-pipe';native.setView(m.view);native.setTime(0);const refineStart=performance.now();const refined=await native.render({tier:'refine',size,samples:16,background:true,accumulate:false});const refineSeconds=(performance.now()-refineStart)/1000;const nativePng=capture(refined.source);refined.release?.();
 const nativeMetrics={...native.lastFrame};
 const picks=[],pickTimes=[];for(const point of [[.5,.5],[.66,.59],[.35,.6]]){const pickStart=performance.now();const p=await native.pick(...point);pickTimes.push(performance.now()-pickStart);if(p && (!Number.isFinite(p.normal.x)||Math.abs(Math.hypot(p.normal.x,p.normal.y,p.normal.z)-1)>.01))throw new Error("Invalid native surface normal");picks.push(p);}const pickMedianMs=median(pickTimes);const bounds=await native.bounds();if(!bounds || bounds.min.x>=bounds.max.x)throw new Error("Invalid native bounds");
 // Compare both engines at the same resolution/sample count with denoising off.
 const compareSize={width:960,height:540};const nativeReference=await native.render({tier:'final',size:compareSize,samples:64,denoise:false,background:true,accumulate:false});
 const nativeComparePng=capture(nativeReference.source);
 const portable=new WebGpuStudioBackend();await portable.initialize();portable.setPreparedScene(scene,maps,description);portable.setView(m.view);portable.setTime(0);const webStart=performance.now();const web=portable.render({tier:'final',size:compareSize,samples:64,denoise:false,background:true});const webCanvas=document.createElement('canvas');webCanvas.width=compareSize.width;webCanvas.height=compareSize.height;webCanvas.getContext('2d').drawImage(web.source,0,0);const webPixels=webCanvas.getContext('2d').getImageData(0,0,compareSize.width,compareSize.height).data;const nativeCanvas=nativeReference.source;const nativePixels=nativeCanvas.getContext('2d').getImageData(0,0,compareSize.width,compareSize.height).data;
 let absolute=0,peak=0;for(let i=0;i<webPixels.length;i++){if(i%4===3)continue;const d=Math.abs(webPixels[i]-nativePixels[i]);absolute+=d;peak=Math.max(peak,d);}const meanCodeDifference=absolute/(compareSize.width*compareSize.height*3);
 const webSeconds=(performance.now()-webStart)/1000;const webPng=capture(webCanvas);
 const peakNativeRssMiB=(await bridge.request(native.session,{op:"hello"})).result.peakRssMiB;
 let crashRecovery=null;
 if(${JSON.stringify(process.env.PM_STUDIO_RECOVERY === '1')}){
   const hello=await bridge.request(native.session,{op:'hello'});await window.killNativeHostForTest(hello.result.pid);
   const start=performance.now();const recovered=await recovery.render({tier:'live',size:compareSize,samples:1,background:true});const canvas=document.createElement('canvas');canvas.width=compareSize.width;canvas.height=compareSize.height;const ctx=canvas.getContext('2d');ctx.drawImage(recovered.source,0,0,canvas.width,canvas.height);const pixels=ctx.getImageData(0,0,canvas.width,canvas.height).data;let min=255,max=0;for(let i=0;i<pixels.length;i+=4){min=Math.min(min,pixels[i]);max=Math.max(max,pixels[i]);}if(recovery.id!=='webgpu'||max-min<16)throw new Error('Native crash recovery failed');crashRecovery={backend:recovery.id,inputToPixelsMs:performance.now()-start};recovered.release?.();
 }
 const sorted=[...inputPixels].sort((a,b)=>a-b);const result={scene:name,size,inputSize,triangles:scene.triangles,instances:scene.instanceCount,lights:scene.lightCount,coldMs,peakNativeRssMiB,crashRecovery,pickMedianMs,picks,inputToPixelsMedianMs:median(inputPixels),inputToPixelsP95Ms:sorted[Math.floor((sorted.length-1)*.95)],hostMedianMs:median(hostTimes),traceMedianMs:median(traceTimes),postMedianMs:median(postTimes),pipeMedianMs:median(roundTrips),canvasUploadMedianMs:median(canvasMs),refineSeconds,hardwareRt:nativeMetrics.host.hardwareRt,denoiser:nativeMetrics.host.denoiser,transport:nativeMetrics.host.transport??'rgba-pipe',webFinal64Seconds:webSeconds,meanCodeDifference,peakCodeDifference:peak,liveTransport,livePng,nativePng,nativeComparePng,webPng};
 nativeReference.release?.();recovery.dispose();portable.dispose();environment?.dispose();display.remove();return result;
};
window.studioReady=true;
`;
await build({ stdin: { contents: renderer, resolveDir: desktop, sourcefile: 'studio-test-renderer.ts' }, bundle: true, platform: 'browser', format: 'esm', outfile: path.join(fixture, 'renderer.js'), loader: { '.wgsl': 'text' }, plugins: [{ name: 'raw-wgsl', setup(build) { build.onResolve({ filter: /\.wgsl\?raw$/ }, args => ({ path: path.resolve(args.resolveDir, args.path.slice(0, -4)) })); } }] });
await writeFile(path.join(fixture, 'index.html'), '<!doctype html><html><body style="margin:0"><script type="module" src="renderer.js"></script></body></html>');
const env = { ...process.env }; delete env.ELECTRON_RUN_AS_NODE;
const app = await electron.launch({ args: [path.join(fixture, 'main.cjs')], cwd: desktop, env });
try {
  const page = await app.firstWindow();
  await page.exposeFunction('killNativeHostForTest',pid=>{if(!Number.isInteger(pid)||pid<=1)throw new Error('Invalid native test process');process.kill(pid,'SIGKILL');});
  const errors=[];page.on('pageerror',error=>errors.push(error.message));page.on('console',message=>{if(message.type()==='error')console.error(message.text());});
  await page.waitForFunction(() => window.studioReady === true, { timeout: 30_000 });
  const rows=[];
  for(const scene of (process.env.PM_STUDIO_SCENES??'hero,chrome').split(',')){
    const result=await page.evaluate(async({scene,frames})=>window.runStudioNative(scene,{width:1920,height:1080},frames),{scene,frames:Number(process.env.PM_STUDIO_FRAMES??30)});
    for(const [field,name]of [['livePng',`${scene}-electron-native-live.png`],['nativePng',`${scene}-electron-native-refine.png`],['nativeComparePng',`${scene}-native-final64.png`],['webPng',`${scene}-webgpu-final64.png`]]){await writeFile(path.join(out,name),Buffer.from(result[field],'base64'));delete result[field];}
    console.log(JSON.stringify(result));rows.push(result);await writeFile(path.join(out,'electron-benchmark.json'),JSON.stringify(rows,null,2));
    if(result.meanCodeDifference>3)throw new Error(`Native/WebGPU output differs too much: ${result.meanCodeDifference}`);
  }
  if(errors.length)throw new Error(errors.join('\n'));
  console.log('PASS Electron native transport, pixels and WebGPU parity');
} finally { await app.close(); }
