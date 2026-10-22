import {afterEach,describe,it,expect,vi} from 'vitest';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const native = vi.hoisted(() => ({ showSaveDialog: vi.fn(), window: { isDestroyed: () => false } }));
vi.mock('electron',()=>({app:{ getAppPath: () => '.', getPath: () => '/tmp', on: vi.fn() },
  BrowserWindow: { fromWebContents: () => native.window }, dialog:{ showSaveDialog: native.showSaveDialog }}));
afterEach(() => vi.restoreAllMocks());
import {registerRenderEncoder,RenderEncoder,validateEncoderOptions,encoderArgs,routeToCurrentExporter,takeRoutedDestination} from './render-encoder';
import {resolveFfmpeg} from './ffmpeg-binary';
const tools=resolveFfmpeg({packaged:false,resourcesPath:'',appPath:path.resolve(__dirname,'../..')}),binary=tools.ffmpeg;
/** ffprobe's view of the first video stream. */
async function probe(file:string){const {stdout}=await promisify(execFile)(tools.ffprobe,['-v','error','-count_frames','-select_streams','v:0','-show_entries','stream=codec_name,codec_tag_string,profile,pix_fmt,color_range,color_space,color_transfer,color_primaries,nb_read_frames,r_frame_rate','-of','json',file]);return JSON.parse(stdout).streams[0];}
describe('native frame encoder',()=>{
 it('rejects invalid sizes and unauthorized render chunks',async()=>{expect(()=>validateEncoderOptions({width:8193})).toThrow();const service=new RenderEncoder(binary,os.tmpdir());expect(()=>service.job('unknown',1)).toThrow();});
 it('clears export activity when a live encoder is cancelled', async () => {
   const activity: Array<[string, boolean]> = [];
   const service = new RenderEncoder(binary, os.tmpdir(), (token, active) => activity.push([token, active]));
   const token = await service.start({width:16,height:16,fps:24,format:'mp4',alpha:false,name:'Cancel'},1);
   expect(activity).toEqual([[token, true]]);
   await service.release(token, 1);
   expect(activity).toEqual([[token, true], [token, false]]);
   expect(service.jobs.size).toBe(0);
 });
 it('encodes exact fractional-rate ProRes with retained alpha',async()=>{const temp=await mkdtemp(path.join(os.tmpdir(),'pm-encoder-test-')),service=new RenderEncoder(binary,temp);let token='';try{token=await service.start({width:16,height:16,fps:24000/1001,format:'prores',alpha:true,name:'Test'},1);const bytes=new Uint8Array(16*16*4);for(let i=0;i<256;i++){bytes[i*4]=255;bytes[i*4+3]=i%16<8?255:0;}for(let i=0;i<3;i++)await service.write(token,1,bytes);const file=await service.finish(token,1);expect((await readFile(file)).length).toBeGreaterThan(1000);const {stdout}=await promisify(execFile)(binary,['-hide_banner','-loglevel','error','-i',file,'-f','rawvideo','-pix_fmt','rgba','pipe:1'],{encoding:'buffer',maxBuffer:1024*1024});expect(stdout.length).toBe(16*16*4*3);expect(stdout[3]).toBeGreaterThan(250);expect(stdout[8*4+3]).toBeLessThan(5);expect(stdout[0]).toBeGreaterThan(250);expect(stdout[1]).toBeLessThan(5);
   expect(await probe(file)).toMatchObject({codec_name:'prores',codec_tag_string:'ap4h',profile:'4444',pix_fmt:expect.stringMatching(/^yuva444p/),color_primaries:'bt709',color_transfer:'iec61966-2-1',color_space:'bt709',color_range:'tv',nb_read_frames:'3',r_frame_rate:'24000/1001'});}finally{if(token)await service.release(token,1);await rm(temp,{recursive:true,force:true});}},30000);
});

it('encodes through VideoToolbox only: no libx264 or prores_ks in Powermove\'s FFmpeg build', () => {
 const base = {width:64,height:64,fps:30,alpha:false,name:'Codec'};
 const mp4 = encoderArgs({...base,format:'mp4'},'out.mp4'), prores = encoderArgs({...base,format:'prores'},'out.mov'), alpha = encoderArgs({...base,format:'prores',alpha:true},'out.mov');
 expect(mp4.slice(mp4.indexOf('-c:v'),mp4.indexOf('-c:v')+2)).toEqual(['-c:v','h264_videotoolbox']);
 expect(prores.slice(prores.indexOf('-c:v'),prores.indexOf('-c:v')+2)).toEqual(['-c:v','prores_videotoolbox']);
 expect(prores[prores.indexOf('-pix_fmt')+1]).toBe('p416le');
 expect(alpha[alpha.indexOf('-pix_fmt')+1]).toBe('ayuv64le');
 for (const args of [mp4,prores,alpha]) {
  expect(args.join(' ')).not.toMatch(/libx264|prores_ks|alpha_bits/);
  // VideoToolbox reads primaries and transfer from the frames, so the filter graph tags them.
  expect(args[args.indexOf('-vf')+1]).toContain('setparams=color_primaries=bt709:color_trc=iec61966-2-1:colorspace=bt709:range=tv');
 }
});

it('encodes opaque ProRes 4444 without an alpha channel and H.264 MP4 with Rec.709 (QuickTime) tags', async () => {
 const temp=await mkdtemp(path.join(os.tmpdir(),'pm-encoder-test-')),service=new RenderEncoder(binary,temp);const tokens:string[]=[];
 try{
  const frame=new Uint8Array(64*64*4);for(let i=0;i<64*64;i++){frame[i*4]=255;frame[i*4+3]=255;}
  const opaque=await service.start({width:64,height:64,fps:25,format:'prores',alpha:false,name:'Opaque'},1);tokens.push(opaque);
  for(let i=0;i<2;i++)await service.write(opaque,1,frame);
  expect(await probe(await service.finish(opaque,1))).toMatchObject({codec_name:'prores',codec_tag_string:'ap4h',pix_fmt:expect.stringMatching(/^yuv444p/),color_transfer:'iec61966-2-1',color_range:'tv',nb_read_frames:'2'});
  const mp4=await service.start({width:64,height:64,fps:30000/1001,format:'mp4',alpha:false,name:'Web',bitrateMbps:8},1);tokens.push(mp4);
  for(let i=0;i<30;i++)await service.write(mp4,1,frame);
  const file=await service.finish(mp4,1);
  expect(await probe(file)).toMatchObject({codec_name:'h264',codec_tag_string:'avc1',profile:'High',pix_fmt:'yuv420p',color_primaries:'bt709',color_transfer:'iec61966-2-1',color_space:'bt709',color_range:'tv',nb_read_frames:'30',r_frame_rate:'30000/1001'});
  const {stdout}=await promisify(execFile)(binary,['-v','error','-i',file,'-frames:v','1','-vf','crop=2:2:32:32','-f','rawvideo','-pix_fmt','rgb24','pipe:1'],{encoding:'buffer'});
  expect(Math.abs(stdout[0]!-255)).toBeLessThanOrEqual(2);expect(stdout[1]).toBeLessThanOrEqual(2);expect(stdout[2]).toBeLessThanOrEqual(2);
 }finally{for(const token of tokens)await service.release(token,1);await rm(temp,{recursive:true,force:true});}
},30000);

it('applies the selected MP4 bitrate and rejects invalid bitrates', () => {
 const options = {width:64,height:64,fps:30,format:'mp4' as const,alpha:false,name:'Quality',bitrateMbps:4};
 expect(encoderArgs(options,'out.mp4')).toContain('4M');
 expect(encoderArgs({...options,bitrateMbps:40},'out.mp4')).toContain('40M');
 expect(()=>validateEncoderOptions({...options,bitrateMbps:Infinity})).toThrow('Invalid video bitrate');
});

import { IPC } from '../shared/ipc';
import { withAgentExport } from './agent-export';
it.each(['mp4', 'prores'])('selects the %s destination before starting the encoder and handles cancellation', async format => {
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const start = vi.spyOn(RenderEncoder.prototype, 'start');
  let choose!: (value: any) => void;
  native.showSaveDialog.mockImplementationOnce(() => new Promise(resolve => { choose = resolve; }));
  const pending = handlers.get(IPC.renderStart)({ sender: { id: 1 } }, { width: 16, height: 16, fps: 30, format, alpha: false, name: 'Export' });
  expect(start).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(typeof choose).toBe('function'));
  choose({ canceled: true });
  expect(await pending).toBeNull();
  expect(start).not.toHaveBeenCalled();
});

it.each([false, true])('finishes real video with automatic agent delivery=%s', async automatic => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-ipc-'));
  const output = path.join(folder, 'chosen.mp4');
  const handlers = new Map<string, any>();
  const activity: Array<[string, boolean]> = [];
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary, onActivity: (token, active) => activity.push([token, active]) });
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: output });
  // The real encoder binary with real frame delivery.
  const event = { sender: { id: 1, isDestroyed: () => false } };
  try {
    await withAgentExport(event.sender, automatic ? folder : undefined, async () => {
      const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Export' });
      expect(activity).toEqual([[token, true]]);
      await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
      const destination = automatic ? path.join(folder, 'Export.mp4') : output;
      expect(await handlers.get(IPC.renderFinish)(event, { token })).toEqual({ path: destination });
      expect(activity).toEqual([[token, true], [token, false]]);
      expect((await readFile(destination)).length).toBeGreaterThan(0);
      expect(native.showSaveDialog).toHaveBeenCalledTimes(automatic ? 0 : 1);
    });
  } finally { await rm(folder, { recursive: true, force: true }); }
});
describe('exports the job engine routed here',()=>{
 it('write the file the user already chose, once, with its extension, while fresh',()=>{
  routeToCurrentExporter(7,'/Users/x/Cut.mov',1000);
  expect(takeRoutedDestination(8,'mov',1000)).toBeUndefined();
  expect(takeRoutedDestination(7,'mp4',1000)).toBeUndefined();
  routeToCurrentExporter(7,'/Users/x/Cut.mov',1000);
  expect(takeRoutedDestination(7,'mov',2000)).toBe('/Users/x/Cut.mov');
  expect(takeRoutedDestination(7,'mov',2000)).toBeUndefined();
  routeToCurrentExporter(7,'/Users/x/Cut.mov',0);
  expect(takeRoutedDestination(7,'mov',11*60_000)).toBeUndefined();
 });
});
it('an agent export never takes a routed destination the user chose', async () => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-routed-'));
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  native.showSaveDialog.mockReset();
  const event = { sender: { id: 9, isDestroyed: () => false } };
  const chosen = path.join(folder, 'User cut.mp4');
  try {
    // An export sheet job routed here never reached renderStart.
    routeToCurrentExporter(9, chosen);
    await withAgentExport(event.sender, folder, async () => {
      const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Agent cut' });
      await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
      expect(await handlers.get(IPC.renderFinish)(event, { token })).toEqual({ path: path.join(folder, 'Agent cut.mp4') });
    });
    await expect(readFile(chosen)).rejects.toThrow();
    // The user's own export still takes it.
    expect(takeRoutedDestination(9, 'mp4')).toBe(chosen);
  } finally { await rm(folder, { recursive: true, force: true }); }
});
