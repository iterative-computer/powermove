import {afterEach,describe,it,expect,vi} from 'vitest';
import {mkdtemp,readFile,rm,stat,writeFile} from 'node:fs/promises';
import {EventEmitter} from 'node:events';
import os from 'node:os';
import path from 'node:path';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
const native = vi.hoisted(() => ({ on: vi.fn(), showSaveDialog: vi.fn(), window: { isDestroyed: () => false } }));
vi.mock('electron',()=>({app:{ getAppPath: () => '.', getPath: () => '/tmp', on: native.on },
  BrowserWindow: { fromWebContents: () => native.window }, dialog:{ showSaveDialog: native.showSaveDialog }}));
afterEach(() => vi.restoreAllMocks());
import {registerRenderEncoder,RenderEncoder,validateEncoderOptions,encoderArgs} from './render-encoder';
const binary=path.resolve('node_modules/ffmpeg-static/ffmpeg');
const renderSender = () => Object.assign(new EventEmitter(), { id: 1, isDestroyed: () => false, isCrashed: () => false });
describe('native frame encoder',()=>{
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
 it('encodes exact fractional-rate ProRes with retained alpha',async()=>{const temp=await mkdtemp(path.join(os.tmpdir(),'pm-encoder-test-')),service=new RenderEncoder(binary,temp);let token='';try{token=await service.start({width:16,height:16,fps:24000/1001,format:'prores',alpha:true,name:'Test'},1);const bytes=new Uint8Array(16*16*4);for(let i=0;i<256;i++){bytes[i*4]=255;bytes[i*4+3]=i%16<8?255:0;}for(let i=0;i<3;i++)await service.write(token,1,bytes);const file=await service.finish(token,1);expect((await readFile(file)).length).toBeGreaterThan(1000);const {stdout}=await promisify(execFile)(binary,['-hide_banner','-loglevel','error','-i',file,'-f','rawvideo','-pix_fmt','rgba','pipe:1'],{encoding:'buffer',maxBuffer:1024*1024});expect(stdout.length).toBe(16*16*4*3);expect(stdout[3]).toBeGreaterThan(250);expect(stdout[8*4+3]).toBeLessThan(5);}finally{if(token)await service.release(token,1);await rm(temp,{recursive:true,force:true});}},30000);
});

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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
  native.showSaveDialog.mockReset().mockResolvedValue({ canceled: false, filePath: output });
  // Use the real encoder binary with real frame delivery.
  const originalStart = RenderEncoder.prototype.start;
  vi.spyOn(RenderEncoder.prototype, 'start').mockImplementation(function (this: RenderEncoder, options, owner) {
    Object.defineProperty(this, 'binary', { value: binary });
    return originalStart.call(this, options, owner);
  });
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

it('releases the real encoder and temporary files after a renderer crash, then permits a fresh export', async () => {
  const folder = await mkdtemp(path.join(os.tmpdir(), 'powermove-render-crash-'));
  const destination = path.join(folder, 'finished.mp4');
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
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

it('terminates a crashed renderer’s audio mux and preserves the interruption error', async () => {
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
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
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
  const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Audio' });
  let pending: Promise<unknown> | undefined;
  let muxPid: number | undefined;
  let result: { file?: string; error?: Error } | undefined;
  try {
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array([1]), audio: true });
    const job = service.job(token, 1);
    pending = handlers.get(IPC.renderFinish)(event, { token }).then((file: string) => { result = { file }; }, (error: Error) => { result = { error }; });
    await vi.waitFor(async () => { muxPid = Number(await readFile(marker, 'utf8')); expect(muxPid).toBeGreaterThan(0); });
    event.sender.emit('render-process-gone', {}, { reason: 'crashed', exitCode: 1 });
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
