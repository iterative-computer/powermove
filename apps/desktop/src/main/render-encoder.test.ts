import {afterEach,describe,it,expect,vi} from 'vitest';
import {mkdtemp,readFile,rm,stat,writeFile,symlink,readlink,chmod,copyFile} from 'node:fs/promises';
import * as fs from 'node:fs/promises';
import {EventEmitter} from 'node:events';
import os from 'node:os';
import path from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const native = vi.hoisted(() => ({ on: vi.fn(), showSaveDialog: vi.fn(), window: { isDestroyed: () => false } }));
vi.mock('electron',()=>({app:{ getAppPath: () => '.', getPath: () => '/tmp', on: native.on },
  BrowserWindow: { fromWebContents: () => native.window }, dialog:{ showSaveDialog: native.showSaveDialog }}));
vi.mock('node:fs/promises', async importOriginal => {
 const actual = await importOriginal<typeof import('node:fs/promises')>();
 return {...actual, link: vi.fn(actual.link)};
});
afterEach(() => vi.restoreAllMocks());
import {registerRenderEncoder,RenderEncoder,validateEncoderOptions,encoderArgs,deliverEncodedVideo,routeToCurrentExporter,takeRoutedDestination} from './render-encoder';
import {resolveFfmpeg} from './ffmpeg-binary';
const tools=resolveFfmpeg({packaged:false,resourcesPath:'',appPath:path.resolve(__dirname,'../..')}),binary=tools.ffmpeg;
/** ffprobe's view of the first video stream. */
async function probe(file:string){const {stdout}=await promisify(execFile)(tools.ffprobe,['-v','error','-count_frames','-select_streams','v:0','-show_entries','stream=codec_name,codec_tag_string,profile,pix_fmt,color_range,color_space,color_transfer,color_primaries,nb_read_frames,r_frame_rate','-of','json',file]);return JSON.parse(stdout).streams[0];}
const renderSender = () => Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
function videoAtoms(bytes: Buffer): string[] {
 const atoms: string[] = [];
 for (let offset = 0; offset + 8 <= bytes.length;) {
  let length = bytes.readUInt32BE(offset);
  atoms.push(bytes.toString('ascii', offset + 4, offset + 8));
  if (length === 0) break;
  if (length === 1) length = Number(bytes.readBigUInt64BE(offset + 8));
  if (length < 8) throw new Error('Invalid video atom');
  offset += length;
 }
 return atoms;
}
describe('finished video delivery', () => {
 it('keeps a new output intact after temporary source cleanup without copying its data', async () => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-video-delivery-'));
  try {
   const source = path.join(folder, 'encoded.mov'), destination = path.join(folder, 'chosen.mov');
   const bytes = Uint8Array.from({length: 1024 * 1024}, (_, i) => i % 251);
   await writeFile(source, bytes);
   await deliverEncodedVideo(source, destination);
   expect((await stat(destination)).ino).toBe((await stat(source)).ino);
   await rm(source);
   expect(await readFile(destination)).toEqual(Buffer.from(bytes));
   expect((await stat(destination)).nlink).toBe(1);
  } finally { await rm(folder, {recursive: true, force: true}); }
 });
 it.each([false, true])('preserves overwrite behavior and permissions for an existing destination, symlink=%s', async symbolic => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-video-overwrite-'));
  try {
   const source = path.join(folder, 'encoded.mp4'), target = path.join(folder, 'existing.mp4'), reference = path.join(folder, 'reference.mp4');
   const destination = symbolic ? path.join(folder, 'chosen.mp4') : target;
   await writeFile(source, 'finished video');
   await writeFile(target, 'old video');
   await writeFile(reference, 'old video');
   await chmod(target, 0o640);
   await chmod(reference, 0o640);
   await copyFile(source, reference);
   if (symbolic) await symlink(target, destination);
   await deliverEncodedVideo(source, destination);
   await rm(source);
   expect(await readFile(target, 'utf8')).toBe('finished video');
   expect((await stat(target)).mode & 0o777).toBe((await stat(reference)).mode & 0o777);
   if (symbolic) expect(await readlink(destination)).toBe(target);
  } finally { await rm(folder, {recursive: true, force: true}); }
 });
 it.each(['EXDEV', 'ENOTSUP', 'EPERM'])('copies safely when hard links are unavailable (%s)', async code => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-video-copy-'));
  try {
   const source = path.join(folder, 'encoded.mp4'), destination = path.join(folder, 'chosen.mp4');
   await writeFile(source, 'finished video');
   vi.mocked(fs.link).mockRejectedValueOnce(Object.assign(new Error('Cannot link'), {code}));
   await deliverEncodedVideo(source, destination);
   await rm(source);
   expect(await readFile(destination, 'utf8')).toBe('finished video');
  } finally { await rm(folder, {recursive: true, force: true}); }
 });
 it('surfaces delivery errors without deleting the temporary video', async () => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-video-delivery-failure-'));
  try {
   const source = path.join(folder, 'encoded.mp4');
   await writeFile(source, 'finished video');
   await expect(deliverEncodedVideo(source, path.join(folder, 'missing', 'chosen.mp4'))).rejects.toThrow();
   expect(await readFile(source, 'utf8')).toBe('finished video');
  } finally { await rm(folder, {recursive: true, force: true}); }
 });
});
describe('native frame encoder',()=>{
 it.each([
  {format: 'mp4' as const, audio: false}, {format: 'mp4' as const, audio: true},
  {format: 'prores' as const, audio: false}, {format: 'prores' as const, audio: true},
 ])('preserves decoded $format frames, timing, audio and container layout with an audio hint, actual audio=$audio', async ({format, audio}) => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-audio-hint-parity-'));
  const service = new RenderEncoder(binary, folder);
  const exec = promisify(execFile);
  try {
   const wav = path.join(folder, 'tone.wav');
   if (audio) await exec(binary, ['-v','error','-f','lavfi','-i','sine=frequency=600:sample_rate=48000:duration=0.6','-ac','2','-c:a','pcm_s16le',wav]);
   const outputs: Array<{video: string; audio?: Buffer; faststart: boolean}> = [];
   for (const expectsAudio of [undefined, true]) {
    const token = await service.start({width: 32, height: 16, fps: 24000/1001, format, alpha: format === 'prores', name: 'Parity', expectsAudio}, 1);
    try {
     const intermediate = service.job(token, 1).file;
     for (let frame = 0; frame < 12; frame++) {
      const bytes = Uint8Array.from({length: 32 * 16 * 4}, (_, index) => index % 4 === 3 ? (index % 32 < 16 ? 128 : 255) : (frame * 19 + index * 7) % 256);
      await service.write(token, 1, bytes);
     }
     if (audio) await service.write(token, 1, await readFile(wav), true);
     const file = await service.finish(token, 1);
     const initialAtoms = videoAtoms(await readFile(intermediate));
     expect(initialAtoms.indexOf('moov') < initialAtoms.indexOf('mdat')).toBe(!expectsAudio);
     // framemd5 includes decoded pixels, timebase, frame order and timestamps.
     const video = await exec(binary, ['-v','error','-i',file,'-map','0:v','-pix_fmt','rgba','-f','framemd5','pipe:1']);
     expect(video.stdout.split('\n').filter(line => line && !line.startsWith('#'))).toHaveLength(12);
     const samples = audio ? (await exec(binary, ['-v','error','-i',file,'-map','0:a','-c:a','pcm_f32le','-f','f32le','pipe:1'], {encoding: 'buffer', maxBuffer: 1024 * 1024})).stdout : undefined;
     if (samples) expect(samples.length).toBeGreaterThan(48000 * .45 * 2 * 4);
     const atoms = videoAtoms(await readFile(file));
     expect(atoms).toContain('moov'); expect(atoms).toContain('mdat');
     outputs.push({video: video.stdout, audio: samples, faststart: atoms.indexOf('moov') < atoms.indexOf('mdat')});
    } finally { await service.release(token, 1); }
   }
   expect(outputs[1]).toEqual(outputs[0]);
   expect(outputs[1]!.faststart).toBe(!audio);
  } finally {
   for (const token of service.jobs.keys()) await service.release(token, 1);
   await rm(folder, {recursive: true, force: true});
  }
 }, 30000);
 it.each(['mp4', 'prores'] as const)('repeats complete %s frames in order at a fractional frame rate',async format=>{
  const temp=await mkdtemp(path.join(os.tmpdir(),'pm-repeat-test-')),service=new RenderEncoder(binary,temp);
  const token=await service.start({width:16,height:16,fps:24000/1001,format,alpha:format==='prores',name:'Repeated'},1);
  const frame=(green:boolean)=>Uint8Array.from({length:16*16*4},(_,i)=>i%4===3?(green?255:128):i%4===(green?1:0)?255:0);
  try{
   await expect(service.repeat(token,1,1)).rejects.toThrow('No complete frame');
   await expect(service.repeat(token,2,1)).rejects.toThrow('Unknown render job');
   await service.write(token,1,frame(false).slice(0,512),false,true);
   await expect(service.repeat(token,1,1)).rejects.toThrow('No complete frame');
   await service.write(token,1,frame(false).slice(512),false,true);
   await expect(service.repeat(token,1,9)).rejects.toThrow('Invalid render repeat count');
   await service.repeat(token,1,4);
   await service.write(token,1,frame(true),false,true);
   await service.repeat(token,1,2);
   const file=await service.finish(token,1);
   const {stdout}=await promisify(execFile)(binary,['-v','error','-i',file,'-f','rawvideo','-pix_fmt','rgba','pipe:1'],{encoding:'buffer',maxBuffer:1024*1024});
   expect(stdout.length).toBe(8*16*16*4);
   for(let i=0;i<8;i++){
    const offset=i*16*16*4;
    expect(stdout[offset+(i<5?0:1)]).toBeGreaterThan(245);
    expect(stdout[offset+(i<5?1:0)]).toBeLessThan(10);
    // ProRes converts 8-bit alpha through its 10-bit pixel format.
    expect(Math.abs(stdout[offset+3]!-(format==='prores'&&i<5?128:255))).toBeLessThanOrEqual(2);
   }
  }finally{await service.release(token,1);await rm(temp,{recursive:true,force:true});}
 },30000);
 it('rejects invalid sizes and unauthorized render chunks',async()=>{expect(()=>validateEncoderOptions({width:8193})).toThrow();const service=new RenderEncoder(binary,os.tmpdir());expect(()=>service.job('unknown',1)).toThrow();});
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
 expect(()=>validateEncoderOptions({...options,expectsAudio:'true'})).toThrow('Invalid render audio expectation');
});

import { IPC } from '../shared/ipc';
import { withAgentExport } from './agent-export';
it.each(['mp4', 'prores'])('selects the %s destination before starting the encoder and handles cancellation', async format => {
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const start = vi.spyOn(RenderEncoder.prototype, 'start');
  let choose!: (value: any) => void;
  native.showSaveDialog.mockImplementationOnce(() => new Promise(resolve => { choose = resolve; }));
  const pending = handlers.get(IPC.renderStart)({ sender: renderSender() }, { width: 16, height: 16, fps: 30, format, alpha: false, name: 'Export' });
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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: output });
  // The real encoder binary with real frame delivery.
  const event = { sender: renderSender() };
  try {
    await withAgentExport(event.sender, automatic ? folder : undefined, async () => {
      const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Export' });
      await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
      const destination = automatic ? path.join(folder, 'Export.mp4') : output;
      expect(await handlers.get(IPC.renderFinish)(event, { token })).toEqual({ path: destination });
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
  const event = { sender: Object.assign(renderSender(), { id: 9 }) };
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

it('releases the real encoder and temporary files after a renderer crash, then permits a fresh export', async () => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-crash-'));
  const destination = path.join(folder, 'finished.mp4');
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const event = { sender: renderSender() };
  native.on.mock.calls.filter(([name]) => name === 'web-contents-created').at(-1)![1]({}, event.sender);
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: destination });
  const originalStart = RenderEncoder.prototype.start;
  let service!: RenderEncoder;
  vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(function (this: RenderEncoder, options, owner) {
    service = this;
    Object.defineProperty(this, 'binary', { value: binary });
    return originalStart.call(this, options, owner);
  });
  const options = { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Crash' };
  try {
    const token = await handlers.get(IPC.renderStart)(event, options);
    const job = service.job(token, event.sender.id);
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
    event.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect(event.sender.isDestroyed()).toBe(false);
    await vi.waitFor(() => expect(service.jobs.size).toBe(0));
    await vi.waitFor(() => expect(job.process.signalCode).toBe('SIGKILL'));
    await vi.waitFor(async () => expect(await stat(job.dir).catch(() => null)).toBeNull());
    event.sender.emit('did-finish-load');
    const fresh = await handlers.get(IPC.renderStart)(event, options);
    await handlers.get(IPC.renderWrite)(event, { token: fresh, data: new Uint8Array(16 * 16 * 4) });
    expect(await handlers.get(IPC.renderFinish)(event, { token: fresh })).toEqual({ path: destination });
    expect((await readFile(destination)).length).toBeGreaterThan(0);
    event.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    expect((await readFile(destination)).length).toBeGreaterThan(0);
  } finally {
    for (const token of service?.jobs.keys() ?? []) await service.release(token, event.sender.id);
    await rm(folder, { recursive: true, force: true });
  }
});

it.each([
  { stage: 'dialog', cleanupFails: false },
  { stage: 'encoder', cleanupFails: false },
  { stage: 'encoder', cleanupFails: true },
])('rejects an old export finishing $stage setup after recovery, cleanup failure=$cleanupFails', async ({ stage, cleanupFails }) => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-setup-'));
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const event = { sender: renderSender() };
  native.on.mock.calls.filter(([name]) => name === 'web-contents-created').at(-1)![1]({}, event.sender);
  let resume!: () => void;
  const pause = new Promise<void>(resolve => { resume = resolve; });
  let waiting = false;
  native.showSaveDialog.mockReset().mockImplementation(async () => {
    if (stage === 'dialog') { waiting = true; await pause; }
    return { canceled: false, filePath: path.join(folder, 'old.mp4') };
  });
  const originalStart = RenderEncoder.prototype.start;
  const originalRelease = RenderEncoder.prototype.release;
  if (cleanupFails) {
    vi.spyOn(RenderEncoder.prototype, 'release').mockImplementation(async function (this: RenderEncoder, token, owner) {
      await originalRelease.call(this, token, owner);
      throw new Error('Temporary cleanup failed');
    });
  }
  let service: RenderEncoder | undefined;
  const start = vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(async function (this: RenderEncoder, options, owner) {
    service = this;
    Object.defineProperty(this, 'binary', { value: binary });
    if (stage === 'encoder') { waiting = true; await pause; }
    return originalStart.call(this, options, owner);
  });
  const pending = handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Old' })
    .then((token: string) => ({ token }), (error: Error) => ({ error }));
  try {
    await vi.waitFor(() => expect(waiting).toBe(true));
    event.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    event.sender.emit('did-finish-load');
    resume();
    const result = await pending;
    expect(result).toEqual({ error: expect.objectContaining({ message: 'Export renderer is unavailable' }) });
    if (stage === 'dialog') expect(start).not.toHaveBeenCalled();
    expect(service?.jobs.size ?? 0).toBe(0);
  } finally {
    resume();
    await pending;
    for (const token of service?.jobs.keys() ?? []) await originalRelease.call(service!, token, event.sender.id);
    await rm(folder, { recursive: true, force: true });
  }
});

it.each([false, true])('surfaces cleanup errors only when export has no primary failure, encode failure=%s', async encodeFails => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-cleanup-'));
  const destination = path.join(folder, 'output.mp4');
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const event = { sender: renderSender() };
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: destination });
  const originalStart = RenderEncoder.prototype.start;
  const originalRelease = RenderEncoder.prototype.release;
  let service!: RenderEncoder;
  vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(function (this: RenderEncoder, options, owner) {
    service = this;
    Object.defineProperty(this, 'binary', { value: binary });
    return originalStart.call(this, options, owner);
  });
  const encodeError = new Error('Video encoding failed');
  const cleanupError = new Error('Temporary cleanup failed');
  if (encodeFails) vi.spyOn(RenderEncoder.prototype, 'finish').mockRejectedValue(encodeError);
  vi.spyOn(RenderEncoder.prototype, 'release').mockImplementation(async function (this: RenderEncoder, token, owner) {
    await originalRelease.call(this, token, owner);
    throw cleanupError;
  });
  try {
    const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Cleanup' });
    const job = service.job(token, event.sender.id);
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
    await expect(handlers.get(IPC.renderFinish)(event, { token })).rejects.toBe(encodeFails ? encodeError : cleanupError);
    expect(service.jobs.size).toBe(0);
    expect(await stat(job.dir).catch(() => null)).toBeNull();
    if (!encodeFails) expect((await readFile(destination)).length).toBeGreaterThan(0);
  } finally {
    for (const token of service?.jobs.keys() ?? []) await originalRelease.call(service, token, event.sender.id);
    await rm(folder, { recursive: true, force: true });
  }
});

it.each([
  { audio: false, interruption: 'crash' }, { audio: true, interruption: 'crash' },
  { audio: false, interruption: 'cancel' }, { audio: true, interruption: 'cancel' },
])('terminates the final mux after $interruption and preserves the interruption error, actual audio=$audio', async ({ audio, interruption }) => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-mux-'));
  const wrapper = path.join(folder, 'ffmpeg-wrapper');
  const marker = path.join(folder, 'mux-pid');
  // Encode real frames, then hold the mux process open so cancellation is
  // deterministic rather than racing a tiny ffmpeg mux on a fast machine.
  await writeFile(wrapper, `#!/usr/bin/env node
const {spawn} = require('node:child_process');
const {writeFileSync} = require('node:fs');
if (process.argv.includes('rawvideo')) {
  spawn(${JSON.stringify(binary)}, process.argv.slice(2), {stdio: 'inherit'}).once('close', code => process.exit(code ?? 1));
} else {
  writeFileSync(${JSON.stringify(marker)}, String(process.pid));
  setInterval(() => {}, 1000);
}
`, { mode: 0o755 });
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true, ffmpeg: binary });
  const event = { sender: renderSender() };
  native.on.mock.calls.filter(([name]) => name === 'web-contents-created').at(-1)![1]({}, event.sender);
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: path.join(folder, 'output.mp4') });
  let service!: RenderEncoder;
  const originalStart = RenderEncoder.prototype.start;
  vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(function (this: RenderEncoder, options, owner) {
    service = this;
    Object.defineProperty(this, 'binary', { value: wrapper });
    return originalStart.call(this, options, owner);
  });
  const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Audio', expectsAudio: true });
  let pending: Promise<unknown> | undefined;
  let muxPid: number | undefined;
  let result: { file?: string; error?: Error } | undefined;
  try {
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
    if (audio) await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array([1]), audio: true });
    const job = service.job(token, 1);
    pending = handlers.get(IPC.renderFinish)(event, { token }).then((file: string) => { result = { file }; }, (error: Error) => { result = { error }; });
    await vi.waitFor(async () => { muxPid = Number(await readFile(marker, 'utf8')); expect(muxPid).toBeGreaterThan(0); });
    if (interruption === 'cancel') await handlers.get(IPC.renderCancel)(event, { token });
    else event.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
    await vi.waitFor(() => expect(result).toEqual({ error: expect.objectContaining({ message: 'Video muxer stopped' }) }));
    expect(() => process.kill(muxPid!, 0)).toThrow();
    await vi.waitFor(async () => expect(await stat(job.dir).catch(() => null)).toBeNull());
  } finally {
    if (muxPid) { try { process.kill(muxPid, 'SIGKILL'); } catch {} }
    if (service.jobs.has(token)) await service.release(token, 1);
    await pending;
    await rm(folder, { recursive: true, force: true });
  }
});

it.each([
  {stage: 'encoder', audio: false}, {stage: 'mux', audio: false}, {stage: 'mux', audio: true},
])('cleans up a hinted export after a real $stage failure, actual audio=$audio', async ({stage, audio}) => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-audio-hint-failure-'));
  const wrapper = path.join(folder, 'ffmpeg-wrapper'), destination = path.join(folder, 'output.mp4');
  await writeFile(wrapper, `#!/usr/bin/env node
const {spawn} = require('node:child_process');
if (process.argv.includes('rawvideo') && ${JSON.stringify(stage)} === 'mux') {
  spawn(${JSON.stringify(binary)}, process.argv.slice(2), {stdio: 'inherit'}).once('close', code => process.exit(code ?? 1));
} else {
  process.stderr.write('Deliberate ${stage} failure', () => process.exit(9));
}
`, {mode: 0o755});
  const handlers = new Map<string, any>();
  registerRenderEncoder({handle: (channel: string, handler: any) => handlers.set(channel, handler)} as any, {isTrustedSender: () => true, ffmpeg: binary});
  const event = {sender: renderSender()};
  native.showSaveDialog.mockReset().mockResolvedValue({canceled: false, filePath: destination});
  const originalStart = RenderEncoder.prototype.start;
  let service!: RenderEncoder;
  vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(function (this: RenderEncoder, options, owner) {
    service = this;
    Object.defineProperty(this, 'binary', {value: wrapper});
    return originalStart.call(this, options, owner);
  });
  const token = await handlers.get(IPC.renderStart)(event, {width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Failure', expectsAudio: true});
  const job = service.job(token, 1);
  try {
    await handlers.get(IPC.renderWrite)(event, {token, data: new Uint8Array(16 * 16 * 4)});
    if (audio) await handlers.get(IPC.renderWrite)(event, {token, data: new Uint8Array([1]), audio: true});
    await expect(handlers.get(IPC.renderFinish)(event, {token})).rejects.toThrow(`Deliberate ${stage} failure`);
    expect(service.jobs.size).toBe(0);
    expect(await stat(job.dir).catch(() => null)).toBeNull();
    expect(await stat(destination).catch(() => null)).toBeNull();
  } finally {
    if (service.jobs.has(token)) await service.release(token, 1);
    await rm(folder, {recursive: true, force: true});
  }
});
