import { EventEmitter } from 'node:events';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { STUDIO_NATIVE_IPC } from '../../shared/studio-native';

const mock=vi.hoisted(()=>({workers:[] as any[],request:vi.fn()}));
vi.mock('./surface',()=>({StudioSurfaceReceiver:{open:()=>null}}));
vi.mock('./manager',()=>({StudioHostManager:class {
  request=mock.request;dispose=vi.fn();status=()=>({running:true,generation:1});
  constructor(){mock.workers.push(this);}
}}));
import { registerStudioNativeIpc } from './ipc';

beforeEach(()=>{mock.workers=[];mock.request.mockReset().mockResolvedValue({binary:new Uint8Array([1]),result:{}});});
function setup(){
  const handlers=new Map<string,Function>(),ipc={handle:(key:string,fn:Function)=>handlers.set(key,fn),removeHandler:vi.fn()};
  const sender=Object.assign(new EventEmitter(),{id:1}),event={sender};
  const stop=registerStudioNativeIpc(ipc as any,'/test/host',()=>true);
  return {handlers,event,stop,request:(binary=new Uint8Array([1]))=>handlers.get(STUDIO_NATIVE_IPC.request)!(event,'fbx:test',{op:'importFbx'},binary)};
}
describe('FBX worker lifecycle',()=>{
  it('isolates imports from renderer sessions and disposes each worker after success',async()=>{
    const s=setup(),pending=s.request(),worker=mock.workers[0];
    expect(worker.request).toHaveBeenCalledWith({op:'importFbx'},new Uint8Array([1]));
    await pending;expect(worker.dispose).toHaveBeenCalledOnce();
    await s.request();expect(mock.workers).toHaveLength(2);s.stop();
  });
  it('bounds imports to one per window and releases a worker when its owner leaves',async()=>{
    const s=setup();let reject!:(error:Error)=>void;
    mock.request.mockImplementationOnce(()=>new Promise((_resolve,no)=>{reject=no;}));
    const pending=s.request(),worker=mock.workers[0];
    await expect(s.request()).rejects.toThrow('already running');
    s.event.sender.emit('destroyed');expect(worker.dispose).toHaveBeenCalled();
    reject(new Error('Studio session is closed'));
    await expect(pending).rejects.toThrow('closed');s.stop();
  });
  it('cleans up failed imports so the next import can start',async()=>{
    const s=setup();mock.request.mockRejectedValueOnce(new Error('Cannot read FBX'));
    await expect(s.request()).rejects.toThrow('Cannot read FBX');
    expect(mock.workers[0].dispose).toHaveBeenCalledOnce();
    await s.request();expect(mock.workers).toHaveLength(2);s.stop();
  });
  it('rejects invalid bytes before allocating a worker',async()=>{
    const s=setup();await expect(s.request(new Uint8Array())).rejects.toThrow('empty');
    expect(mock.workers).toHaveLength(0);s.stop();
  });
});
