import {describe,it,expect,vi} from 'vitest';
import {installRenderQueue,wavBytes} from './render-queue';

/** Independent explicit little-endian reference for the established PCM format. */
function referenceWav(buffer: AudioBuffer): Uint8Array {
 const channels = buffer.numberOfChannels, frames = buffer.length;
 const bytes = Buffer.alloc(44 + frames * channels * 2);
 bytes.write('RIFF', 0); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write('WAVEfmt ', 8);
 bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(channels, 22);
 bytes.writeUInt32LE(buffer.sampleRate, 24); bytes.writeUInt32LE(buffer.sampleRate * channels * 2, 28);
 bytes.writeUInt16LE(channels * 2, 32); bytes.writeUInt16LE(16, 34); bytes.write('data', 36); bytes.writeUInt32LE(bytes.length - 44, 40);
 for (let frame = 0; frame < frames; frame++) for (let channel = 0; channel < channels; channel++) {
  const sample = Math.max(-1, Math.min(1, buffer.getChannelData(channel)[frame]!));
  bytes.writeInt16LE(Math.round(sample * (sample < 0 ? 32768 : 32767)), 44 + (frame * channels + channel) * 2);
 }
 return bytes;
}

function pcmFixture(channels: number): AudioBuffer {
 const values = [NaN, Infinity, -Infinity, 0, -0, 1, -1, 1.0001, -1.0001, .5, -.5];
 // Every signed 16-bit half-step catches rounding and clipping differences.
 for (let sample = -32768; sample <= 32767; sample++) values.push((sample + .5) / (sample < 0 ? 32768 : 32767));
 let seed = 0x12345678;
 for (let index = 0; index < 1024; index++) { seed = Math.imul(seed, 1664525) + 1013904223; values.push((seed >>> 0) / 2**32 * 4 - 2); }
 const planes = Array.from({length: channels}, (_, channel) => Float32Array.from(values, (_, index) => values[(index + channel * 11) % values.length]!));
 return {numberOfChannels: channels, length: values.length, sampleRate: 48000, getChannelData: channel => planes[channel]!} as AudioBuffer;
}

describe('WAV PCM byte parity', () => {
 it.each([1, 2, 3, 6])('preserves every PCM byte, rounding edge case and channel order for %i channels', channels => {
  const buffer = pcmFixture(channels), before = Array.from({length: channels}, (_, index) => buffer.getChannelData(index).slice());
  expect(Buffer.compare(wavBytes(buffer), referenceWav(buffer))).toBe(0);
  for (let channel = 0; channel < channels; channel++) expect(buffer.getChannelData(channel)).toEqual(before[channel]);
 });
 it('writes the same explicit little-endian bytes when typed stores need the portable fallback', async () => {
  const NativeUint16Array = Uint16Array;
  vi.stubGlobal('Uint16Array', class extends NativeUint16Array {
   constructor(values: number[]) { super(values); this[0] = 0x100; }
  });
  try {
   vi.resetModules();
   const {wavBytes: portableWav} = await import('./render-queue');
   const buffer = pcmFixture(2);
   expect(Buffer.compare(portableWav(buffer), referenceWav(buffer))).toBe(0);
  } finally { vi.unstubAllGlobals(); }
 });
 it.each([11025, 44100, 48000])('preserves headers and empty payloads at %i Hz', sampleRate => {
  const buffer = {numberOfChannels: 2, length: 0, sampleRate, getChannelData: () => new Float32Array()} as unknown as AudioBuffer;
  expect(Buffer.compare(wavBytes(buffer), referenceWav(buffer))).toBe(0);
 });
});
function queue(){const original={id:'active',name:'Active',layers:[{id:'l',value:1}]},store=new Map<string,any>();const PM:any={proj:original,time:1,uid:()=>String(store.size),store:{get:(k:string,d:any)=>store.get(k)??d,set:(k:string,v:any)=>store.set(k,structuredClone(v))},bus:{emit:vi.fn()},toast:vi.fn(),touch:vi.fn(),invalidate:vi.fn(),ProjectIndex:{invalidate:vi.fn()}};const X:any={run:vi.fn(async()=>({cancelled:false}))};installRenderQueue(PM,X);X.queueDialog=vi.fn();return {PM,X,store,original};}
describe('render queue recovery',()=>{
 it('renders an immutable project snapshot and restores the editing project and time',async()=>{const {PM,X,original,store}=queue();const job=X.enqueue({format:'prores',fps:24000/1001});original.layers[0]!.value=2;X.run.mockImplementation(async()=>{expect(PM.proj.layers[0].value).toBe(1);PM.time=5;return {cancelled:false};});await X.runQueue();expect(job.status).toBe('complete');expect(PM.proj).toBe(original);expect(PM.time).toBe(1);expect(store.get('renderQueue')[0].status).toBe('complete');});
 it('stops after a failed output and leaves following jobs pending',async()=>{const {X,PM,original}=queue();const first=X.enqueue({format:'mp4'}),next=X.enqueue({format:'prores'});X.run.mockResolvedValue({error:'Disk full'});await X.runQueue();expect(first).toMatchObject({status:'failed',error:'Disk full'});expect(next.status).toBe('pending');expect(X.run).toHaveBeenCalledOnce();expect(PM.proj).toBe(original);});
 it('restores interrupted renders as pending and writes interleaved WAV audio',()=>{const {PM}=queue();PM.store.set('renderQueue',[{status:'rendering'}]);const X:any={};installRenderQueue(PM,X);expect(X.queue[0].status).toBe('pending');const bytes=wavBytes({numberOfChannels:2,length:2,sampleRate:48000,getChannelData:(i:number)=>i?new Float32Array([.5,-.5]):new Float32Array([1,-1])} as AudioBuffer);const view=new DataView(bytes.buffer);expect(view.getUint32(24,true)).toBe(48000);expect([44,46,48,50].map(i=>view.getInt16(i,true))).toEqual([32767,16384,-32768,-16384]);});
});
