import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdtemp, rm, stat, copyFile, appendFile } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { app, BrowserWindow, dialog, type IpcMain } from 'electron';
import { IPC } from '../shared/ipc';

export type EncoderOptions={width:number;height:number;fps:number;format:'prores'|'mp4';alpha:boolean;name:string;bitrateMbps?:number};
type Job={destination?:string;dir:string;file:string;audio:string;process:ChildProcessWithoutNullStreams;done:Promise<void>;bytes:number;frameBytes:number;frames:number;options:EncoderOptions;owner:number;error:string};
export function encoderArgs(options:EncoderOptions,output:string):string[]{
  return ['-hide_banner','-loglevel','error','-f','rawvideo','-pixel_format','rgba','-video_size',`${options.width}x${options.height}`,'-framerate',String(options.fps),'-i','pipe:0','-an',
    '-vf','scale=in_range=full:out_range=limited:out_color_matrix=bt709',
    ...(options.format==='prores'?['-c:v','prores_ks','-profile:v','4','-pix_fmt',options.alpha?'yuva444p10le':'yuv444p10le','-alpha_bits','16']:['-c:v','libx264','-pix_fmt','yuv420p','-b:v',`${options.bitrateMbps ?? 16}M`]),
    '-color_primaries','bt709','-color_trc','iec61966-2-1','-colorspace','bt709','-color_range','tv','-movflags','+faststart','-y',output];
}
export function validateEncoderOptions(raw:any):EncoderOptions {
  if(!raw || ![raw.width,raw.height].every(n=>Number.isInteger(n)&&n>=2&&n<=8192&&n%2===0)||!Number.isFinite(raw.fps)||raw.fps<=0||raw.fps>240||!['mp4','prores'].includes(raw.format)||typeof raw.name!=='string'||raw.name.length>240)throw new Error('Invalid render settings');
  if(raw.bitrateMbps!==undefined&&(!Number.isFinite(raw.bitrateMbps)||raw.bitrateMbps<1||raw.bitrateMbps>100))throw new Error('Invalid video bitrate');
  return {...raw,alpha:raw.format==='prores'&&!!raw.alpha};
}
export class RenderEncoder {
  jobs=new Map<string,Job>();
  constructor(readonly binary:string,readonly tempRoot:string){}
  async start(raw:unknown,owner:number){const options=validateEncoderOptions(raw),dir=await mkdtemp(path.join(this.tempRoot,'powermove-render-')),file=path.join(dir,options.format==='prores'?'video.mov':'video.mp4');
    const proc=spawn(this.binary,encoderArgs(options,file),{stdio:['pipe','pipe','pipe']});let error='';proc.stderr.on('data',d=>{error=(error+d.toString()).slice(-8000);});
    const done=new Promise<void>((resolve,reject)=>{proc.once('error',reject);proc.once('close',code=>code===0?resolve():reject(new Error(error||'Video encoder stopped')));});done.catch(()=>undefined);proc.stdin.on('error',()=>undefined);
    const token=randomUUID();this.jobs.set(token,{dir,file,audio:path.join(dir,'audio.wav'),process:proc,done,bytes:0,frameBytes:options.width*options.height*4,frames:0,options,owner,error});return token;
  }
  job(token:string,owner:number){const job=this.jobs.get(token);if(!job||job.owner!==owner)throw new Error('Unknown render job');return job;}
  async write(token:string,owner:number,data:Uint8Array,audio=false){const job=this.job(token,owner);if(!(data instanceof Uint8Array)||data.byteLength===0||data.byteLength>4*1024*1024)throw new Error('Invalid render chunk');if(audio){await appendFile(job.audio,data);return;}if(job.process.exitCode!==null)await job.done;await new Promise<void>((resolve,reject)=>job.process.stdin.write(data,e=>e?reject(e):resolve()));job.bytes+=data.byteLength;}
  async finish(token:string,owner:number){const job=this.job(token,owner);if(!job.bytes||job.bytes%job.frameBytes)throw new Error('Incomplete render frame');job.process.stdin.end();await job.done;
    const audio=await stat(job.audio).catch(()=>null);if(audio?.size){const mux=path.join(job.dir,job.options.format==='prores'?'final.mov':'final.mp4');await new Promise<void>((resolve,reject)=>{const p=spawn(this.binary,['-hide_banner','-loglevel','error','-i',job.file,'-i',job.audio,'-map','0:v','-map','1:a','-c:v','copy','-c:a',job.options.format==='prores'?'pcm_s24le':'aac','-shortest','-y',mux]);let error='';p.stderr.on('data',d=>error+=d);p.on('error',reject);p.on('close',code=>code===0?resolve():reject(new Error(error)));});job.file=mux;}return job.file;
  }
  async release(token:string,owner:number){const job=this.job(token,owner);this.jobs.delete(token);if(job.process.exitCode===null)job.process.kill();await job.done.catch(()=>undefined);await rm(job.dir,{recursive:true,force:true});}
}
export function registerRenderEncoder(ipc:IpcMain,ctx:{isTrustedSender:(event:any)=>boolean}) {
  const binary=app.isPackaged?path.join(process.resourcesPath,'encoder','ffmpeg'):path.join(app.getAppPath(),'node_modules','ffmpeg-static','ffmpeg');
  const encoder=new RenderEncoder(binary,app.getPath('temp'));
  const trusted=(event:any)=>{if(!ctx.isTrustedSender(event))throw new Error('Unauthorized encoder request');return event.sender.id;};
  ipc.handle(IPC.renderStart,async(e,r)=>{
    const owner=trusted(e),options=validateEncoderOptions(r);
    const window=BrowserWindow.fromWebContents(e.sender);
    if(!window||window.isDestroyed())throw new Error('Export window is unavailable');
    const extension=options.format==='prores'?'mov':'mp4';
    const result=await dialog.showSaveDialog(window,{defaultPath:options.name.replace(/[\\/:]/g,'_')+'.'+extension,filters:[{name:'Video',extensions:[extension]}]});
    if(result.canceled||!result.filePath||window.isDestroyed())return null;
    const token=await encoder.start(options,owner);
    if(e.sender.isDestroyed()){await encoder.release(token,owner);return null;}
    encoder.job(token,owner).destination=result.filePath;
    return token;
  });
  ipc.handle(IPC.renderWrite,(e,r)=>encoder.write(r.token,trusted(e),r.data,!!r.audio));
  ipc.handle(IPC.renderFinish,async(e,r)=>{
    const owner=trusted(e),job=encoder.job(r.token,owner);
    try{
      if(!job.destination)throw new Error('Export destination is unavailable');
      const file=await encoder.finish(r.token,owner);
      await copyFile(file,job.destination);
      return {path:job.destination};
    }finally{await encoder.release(r.token,owner);}
  });
  ipc.handle(IPC.renderCancel,(e,r)=>encoder.release(r.token,trusted(e)));
  app.on('web-contents-created',(_e,contents)=>contents.once('destroyed',()=>{for(const [token,job] of encoder.jobs)if(job.owner===contents.id)void encoder.release(token,job.owner);}));
}
