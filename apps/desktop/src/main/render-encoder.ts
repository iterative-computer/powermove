import { agentExportDestination } from './agent-export';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdtemp, rm, stat, copyFile, appendFile, link } from 'node:fs/promises';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { app, BrowserWindow, dialog, type IpcMain } from 'electron';
import { IPC } from '../shared/ipc';
import { rememberExport } from './export-sidecar';
import { isRendererUnavailable } from './renderer-recovery';

export type EncoderOptions={width:number;height:number;fps:number;format:'prores'|'mp4';alpha:boolean;name:string;bitrateMbps?:number;expectsAudio?:boolean;frames?:number};
/** The FFmpeg video encoder a job uses. */
export type VideoEncoderName='h264_videotoolbox'|'libx264'|'prores_ks';
type Job={destination?:string;dir:string;file:string;audio:string;process:ChildProcessWithoutNullStreams;done:Promise<void>;mux?:{process:ChildProcessWithoutNullStreams;done:Promise<void>};bytes:number;frameBytes:number;frames:number;options:EncoderOptions;owner:number;error:string;encoder:VideoEncoderName;started:number;inflight:Promise<void>[];writeError?:unknown;retained?:Uint8Array;pendingFrame?:Uint8Array};
/** Frames FFmpeg's pipe may have queued before a write waits for it. Awaiting
 * every chunk instead lets the pipe run dry between IPC messages, roughly
 * halving encoder throughput at 4K. */
const QUEUED_FRAMES=2;
// VideoToolbox cannot signal the sRGB transfer libx264 writes, and drops it.
// Rewrite the stream's color description so both encoders tag identically.
const H264_COLOR='h264_metadata=video_full_range_flag=0:colour_primaries=1:transfer_characteristics=13:matrix_coefficients=1';
export function encoderArgs(options:EncoderOptions,output:string,encoder:VideoEncoderName=options.format==='prores'?'prores_ks':'libx264'):string[]{
  const bitrate=`${options.bitrateMbps ?? 16}M`;
  return ['-hide_banner','-loglevel','error','-f','rawvideo','-pixel_format','rgba','-video_size',`${options.width}x${options.height}`,'-framerate',String(options.fps),'-i','pipe:0','-an',
    '-vf','scale=in_range=full:out_range=limited:out_color_matrix=bt709',
    ...(options.format==='prores'?['-c:v','prores_ks','-profile:v','4','-pix_fmt',options.alpha?'yuva444p10le':'yuv444p10le','-alpha_bits','16']
      :encoder==='h264_videotoolbox'?['-c:v','h264_videotoolbox','-allow_sw','0','-profile:v','high','-pix_fmt','yuv420p','-b:v',bitrate,'-bsf:v',H264_COLOR]
      :['-c:v','libx264','-pix_fmt','yuv420p','-b:v',bitrate]),
    '-color_primaries','bt709','-color_trc','iec61966-2-1','-colorspace','bt709','-color_range','tv',
    // Audio will remux this entire file. Moving its header first would rewrite
    // every video byte only for that temporary layout to be discarded.
    ...(options.expectsAudio ? [] : ['-movflags','+faststart']),'-y',output];
}
export function validateEncoderOptions(raw:any):EncoderOptions {
  if(!raw || ![raw.width,raw.height].every(n=>Number.isInteger(n)&&n>=2&&n<=8192&&n%2===0)||!Number.isFinite(raw.fps)||raw.fps<=0||raw.fps>240||!['mp4','prores'].includes(raw.format)||typeof raw.name!=='string'||raw.name.length>240)throw new Error('Invalid render settings');
  if(raw.bitrateMbps!==undefined&&(!Number.isFinite(raw.bitrateMbps)||raw.bitrateMbps<1||raw.bitrateMbps>100))throw new Error('Invalid video bitrate');
  if(raw.expectsAudio!==undefined&&typeof raw.expectsAudio!=='boolean')throw new Error('Invalid render audio expectation');
  if(raw.frames!==undefined&&(!Number.isInteger(raw.frames)||raw.frames<1||raw.frames>10_000_000))throw new Error('Invalid render frame count');
  return {...raw,alpha:raw.format==='prores'&&!!raw.alpha};
}
export async function deliverEncodedVideo(source: string, destination: string): Promise<void> {
  // Encoding and muxing are finished, so the temporary file will never change.
  // A new destination on the same filesystem can share its data until release
  // removes the temporary name, avoiding another full video read and write.
  // Existing files (including symlinks), other filesystems, and filesystems
  // without hard links keep the established copy/overwrite behavior.
  try { await link(source, destination); }
  catch { await copyFile(source, destination); }
}
/** Below this size libx264 already outpaces rendering; hardware only pays off above it. */
const HARDWARE_MIN_PIXELS=1280*720;
const probes=new Map<string,Promise<boolean>>();
/** Whether this FFmpeg can open a hardware H.264 session at this exact size.
 * VideoToolbox can be missing (sandboxes, virtual machines) or refuse a size;
 * a two-frame trial decides before any rendered frame is committed to it. */
export function probeHardwareH264(binary:string,width:number,height:number):Promise<boolean>{
  if(process.platform!=='darwin')return Promise.resolve(false);
  const key=`${binary}\0${width}x${height}`;
  let probe=probes.get(key);
  if(!probe){
    probe=new Promise<boolean>(resolve=>{
      const proc=spawn(binary,['-hide_banner','-loglevel','error','-f','lavfi','-i',`color=c=black:s=${width}x${height}:r=30`,'-frames:v','2',
        '-c:v','h264_videotoolbox','-allow_sw','0','-profile:v','high','-pix_fmt','yuv420p','-f','null','-'],{stdio:'ignore'});
      const timer=setTimeout(()=>{proc.kill('SIGKILL');resolve(false);},10_000);
      proc.once('error',()=>{clearTimeout(timer);resolve(false);});
      proc.once('close',code=>{clearTimeout(timer);resolve(code===0);});
    });
    probes.set(key,probe);
  }
  return probe;
}
export class RenderEncoder {
  jobs=new Map<string,Job>();
  /** `hardware` decides whether an MP4 may use VideoToolbox (default: never). */
  constructor(readonly binary:string,readonly tempRoot:string,readonly hardware:(options:EncoderOptions)=>Promise<boolean>=async()=>false){}
  private async encoderFor(options:EncoderOptions):Promise<VideoEncoderName>{
    if(options.format==='prores')return 'prores_ks';
    if(options.width*options.height<HARDWARE_MIN_PIXELS)return 'libx264';
    return await this.hardware(options).catch(()=>false)?'h264_videotoolbox':'libx264';
  }
  /** Frames the encoder has accepted so far, for a long export's progress. */
  progress(owner:number){
    for(const job of this.jobs.values())if(job.owner===owner)return {frames:Math.floor(job.bytes/job.frameBytes),total:job.options.frames,encoder:job.encoder,seconds:(Date.now()-job.started)/1000};
    return null;
  }
  async start(raw:unknown,owner:number){const options=validateEncoderOptions(raw),encoder=await this.encoderFor(options),dir=await mkdtemp(path.join(this.tempRoot,'powermove-render-')),file=path.join(dir,options.format==='prores'?'video.mov':'video.mp4');
    const proc=spawn(this.binary,encoderArgs(options,file,encoder),{stdio:['pipe','pipe','pipe']});let error='';proc.stderr.on('data',d=>{error=(error+d.toString()).slice(-8000);});
    const done=new Promise<void>((resolve,reject)=>{proc.once('error',reject);proc.once('close',code=>code===0?resolve():reject(new Error(error||'Video encoder stopped')));});done.catch(()=>undefined);proc.stdin.on('error',()=>undefined);
    const token=randomUUID();this.jobs.set(token,{dir,file,audio:path.join(dir,'audio.wav'),process:proc,done,bytes:0,frameBytes:options.width*options.height*4,frames:0,options,owner,error,encoder,started:Date.now(),inflight:[]});return token;
  }
  job(token:string,owner:number){const job=this.jobs.get(token);if(!job||job.owner!==owner)throw new Error('Unknown render job');return job;}
  /** Queue bytes for FFmpeg in order, then wait only while it is behind. */
  private async queue(job:Job,data:Uint8Array){
    // A broken pipe means FFmpeg is exiting; report its own error, not EPIPE.
    if(job.writeError||job.process.exitCode!==null||job.process.signalCode!==null){await job.done;throw job.writeError??new Error('Video encoder stopped');}
    const written=new Promise<void>((resolve,reject)=>job.process.stdin.write(data,error=>error?reject(error):resolve()));
    const entry=written.then(()=>{job.inflight.shift();},error=>{job.inflight.shift();job.writeError??=error;throw error;});
    entry.catch(()=>undefined);
    job.inflight.push(entry);
    job.bytes+=data.byteLength;
    // Callbacks settle in order, so waiting on the oldest keeps newer data queued.
    while(job.process.stdin.writableLength>QUEUED_FRAMES*job.frameBytes&&job.inflight.length)await job.inflight[0];
  }
  async write(token: string, owner: number, data: Uint8Array, audio = false, retain = false) {
    const job = this.job(token, owner);
    if (!(data instanceof Uint8Array) || data.byteLength === 0 || data.byteLength > 4 * 1024 * 1024) {
      throw new Error('Invalid render chunk');
    }
    if (audio) { await appendFile(job.audio, data); return; }
    if (retain) {
      // Retain one explicitly marked frame, assembled from bounded IPC chunks.
      // Ordinary animated delivery never allocates this extra frame buffer.
      const offset = job.bytes % job.frameBytes;
      if (data.byteLength > job.frameBytes - offset) throw new Error('Invalid retained frame chunk');
      if (offset === 0) {
        job.retained = undefined;
        job.pendingFrame = new Uint8Array(job.frameBytes);
      }
      if (!job.pendingFrame) throw new Error('Incomplete retained frame');
      job.pendingFrame.set(data, offset);
    } else {
      job.retained = undefined;
      job.pendingFrame = undefined;
    }
    await this.queue(job, data);
    if (retain && job.bytes % job.frameBytes === 0) {
      job.retained = job.pendingFrame;
      job.pendingFrame = undefined;
    }
  }
  async repeat(token: string, owner: number, count: number) {
    const job = this.job(token, owner);
    if (!Number.isInteger(count) || count < 1 || count > 8) throw new Error('Invalid render repeat count');
    const frame = job.retained;
    if (!frame || job.bytes % job.frameBytes) throw new Error('No complete frame to repeat');
    for (let i = 0; i < count; i++) {
      await this.queue(job, frame);
    }
  }
  async finish(token:string,owner:number){
    const job=this.job(token,owner);
    if(!job.bytes||job.bytes%job.frameBytes)throw new Error('Incomplete render frame');
    job.process.stdin.end();await job.done;
    if(job.writeError)throw job.writeError;
    const audio=await stat(job.audio).catch(()=>null);
    // Cancellation can occur while waiting for the encoder or filesystem.
    this.job(token,owner);
    if(audio?.size || job.options.expectsAudio){
      const mux=path.join(job.dir,job.options.format==='prores'?'final.mov':'final.mp4');
      // A composition can anticipate audio but produce no offline mix. Keep
      // silent exports' existing faststart layout in that case, without
      // decoding or re-encoding their video.
      const args=audio?.size
        ? ['-hide_banner','-loglevel','error','-i',job.file,'-i',job.audio,'-map','0:v','-map','1:a','-c:v','copy','-c:a',job.options.format==='prores'?'pcm_s24le':'aac','-shortest','-y',mux]
        : ['-hide_banner','-loglevel','error','-i',job.file,'-map','0:v','-c:v','copy','-an','-movflags','+faststart','-y',mux];
      const proc=spawn(this.binary,args);
      let error='';proc.stderr.on('data',d=>{error=(error+d.toString()).slice(-8000);});
      const done=new Promise<void>((resolve,reject)=>{
        proc.once('error',reject);
        proc.once('close',code=>code===0?resolve():reject(new Error(error||'Video muxer stopped')));
      });
      job.mux={process:proc,done};
      await done;
      this.job(token,owner);
      job.file=mux;
    }
    return job.file;
  }
  // A cancelled job's output is discarded. ffmpeg defers SIGTERM while it is
  // blocked reading frames from the still-open stdin, so close the pipe and
  // kill it outright; otherwise release never settles and the export stays busy.
  async release(token:string,owner:number){
    const job=this.job(token,owner);this.jobs.delete(token);
    for(const proc of [job.process,job.mux?.process]){
      if(proc&&proc.exitCode===null&&proc.signalCode===null){proc.stdin.destroy();proc.kill('SIGKILL');}
    }
    await Promise.all([job.done,job.mux?.done].map(done=>done?.catch(()=>undefined)));
    await rm(job.dir,{recursive:true,force:true});
  }
}
let activeEncoder:RenderEncoder|null=null;
/** Progress of the native export a renderer owns, read without asking that busy renderer. */
export function nativeExportProgress(owner:number){return activeEncoder?.progress(owner)??null;}
export function registerRenderEncoder(ipc:IpcMain,ctx:{isTrustedSender:(event:any)=>boolean}) {
  const binary=app.isPackaged?path.join(process.resourcesPath,'encoder','ffmpeg'):path.join(app.getAppPath(),'node_modules','ffmpeg-static','ffmpeg');
  // POWERMOVE_EXPORT_ENCODER=software keeps every MP4 on libx264.
  const hardware=process.env.POWERMOVE_EXPORT_ENCODER==='software'?async()=>false
    :(options:EncoderOptions)=>probeHardwareH264(binary,options.width,options.height);
  const encoder=activeEncoder=new RenderEncoder(binary,app.getPath('temp'),hardware);
  const trusted=(event:any)=>{if(!ctx.isTrustedSender(event))throw new Error('Unauthorized encoder request');return event.sender.id;};
  ipc.handle(IPC.renderStart,async(e,r)=>{
    const owner=trusted(e),options=validateEncoderOptions(r);
    const window=BrowserWindow.fromWebContents(e.sender);
    if(!window||window.isDestroyed())throw new Error('Export window is unavailable');
    let rendererGone=false;
    const gone=()=>{rendererGone=true;};
    const checkRenderer=()=>{
      if(rendererGone||isRendererUnavailable(e.sender))throw new Error('Export renderer is unavailable');
    };
    e.sender.once('render-process-gone',gone);
    let token:string|undefined;
    try{
      checkRenderer();
      const extension=options.format==='prores'?'mov':'mp4';
      const automatic=await agentExportDestination(e.sender,options.name+'.'+extension);
      checkRenderer();
      const result=automatic?{canceled:false,filePath:automatic}:await dialog.showSaveDialog(window,{defaultPath:options.name.replace(/[\\/:]/g,'_')+'.'+extension,filters:[{name:'Video',extensions:[extension]}]});
      checkRenderer();
      if(result.canceled||!result.filePath||window.isDestroyed())return null;
      token=await encoder.start(options,owner);
      checkRenderer();
      encoder.job(token,owner).destination=result.filePath;
      return token;
    }catch(error){
      if(token&&encoder.jobs.has(token))await encoder.release(token,owner).catch(()=>undefined);
      throw error;
    }finally{e.sender.removeListener('render-process-gone',gone);}
  });
  ipc.handle(IPC.renderWrite,(e,r)=>r.repeat!==undefined?encoder.repeat(r.token,trusted(e),r.repeat):encoder.write(r.token,trusted(e),r.data,!!r.audio,!!r.retain));
  ipc.handle(IPC.renderFinish,async(e,r)=>{
    const owner=trusted(e),job=encoder.job(r.token,owner);
    let failed=false;
    try{
      if(!job.destination)throw new Error('Export destination is unavailable');
      const file=await encoder.finish(r.token,owner);
      await deliverEncodedVideo(file,job.destination);
      rememberExport(owner,job.destination);
      return {path:job.destination,encoder:job.encoder};
    }catch(error){
      failed=true;
      throw error;
    }finally{
      // Renderer lifecycle cleanup may already have released this job.
      // Keep the export failure if cleanup also fails; a cleanup-only error
      // still reaches the caller after otherwise successful delivery.
      if(encoder.jobs.has(r.token)){
        const cleanup=encoder.release(r.token,owner);
        if(failed)await cleanup.catch(()=>undefined);
        else await cleanup;
      }
    }
  });
  ipc.handle(IPC.renderCancel,(e,r)=>encoder.release(r.token,trusted(e)));
  app.on('web-contents-created',(_e,contents)=>{
    const release=()=>{
      for(const [token,job] of encoder.jobs){
        if(job.owner===contents.id)void encoder.release(token,job.owner).catch(error=>console.error('[encoder] cleanup failed',error));
      }
    };
    contents.once('destroyed',release);
    contents.on('render-process-gone',release);
  });
}
