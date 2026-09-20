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
import {registerRenderEncoder,RenderEncoder,validateEncoderOptions,encoderArgs} from './render-encoder';
const binary=path.resolve('node_modules/ffmpeg-static/ffmpeg');
describe('native frame encoder',()=>{
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
it.each(['mp4', 'prores'])('selects the %s destination before starting the encoder and handles cancellation', async format => {
  const handlers = new Map<string, any>();
  registerRenderEncoder({ handle: (channel: string, handler: any) => handlers.set(channel, handler) } as any, { isTrustedSender: () => true });
  const start = vi.spyOn(RenderEncoder.prototype, 'start');
  let choose!: (value: any) => void;
  native.showSaveDialog.mockImplementationOnce(() => new Promise(resolve => { choose = resolve; }));
  const pending = handlers.get(IPC.renderStart)({ sender: { id: 1 } }, { width: 16, height: 16, fps: 30, format, alpha: false, name: 'Export' });
  expect(start).not.toHaveBeenCalled();
  choose({ canceled: true });
  expect(await pending).toBeNull();
  expect(start).not.toHaveBeenCalled();
});

it('finishes video at its original destination without another save dialog', async () => {
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
  const event = { sender: { id: 1, isDestroyed: () => false } };
  try {
    const token = await handlers.get(IPC.renderStart)(event, { width: 16, height: 16, fps: 30, format: 'mp4', alpha: false, name: 'Export' });
    await handlers.get(IPC.renderWrite)(event, { token, data: new Uint8Array(16 * 16 * 4) });
    expect(await handlers.get(IPC.renderFinish)(event, { token })).toEqual({ path: output });
    expect((await readFile(output)).length).toBeGreaterThan(0);
    expect(native.showSaveDialog).toHaveBeenCalledOnce();
  } finally { await rm(folder, { recursive: true, force: true }); }
});
