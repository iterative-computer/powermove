import { studioCommandTimeoutMs } from './manager';
import { describe, expect, it } from 'vitest';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { StudioHostManager } from './manager';
import { StudioPacketDecoder, studioPacket } from './protocol';

// An actual child process with the same framing. It can exit in the middle of a
// render, exercising pending promise cleanup and a fresh generation on restart.
const fixture = String.raw`
let input=Buffer.alloc(0);
process.stdin.on('data',chunk=>{
 input=Buffer.concat([input,chunk]);
 while(input.length>=12){
  const j=input.readUInt32LE(4),b=input.readUInt32LE(8);if(input.length<12+j+b)return;
  const q=JSON.parse(input.subarray(12,12+j));input=input.subarray(12+j+b);
  if(q.crash)process.exit(9);
  if(q.hang)continue;
  const result=q.op==='hello'?{protocol:1}:{op:q.op,runAsNode:process.env.ELECTRON_RUN_AS_NODE??null};
  const json=Buffer.from(JSON.stringify({id:q.id,ok:true,result}));const h=Buffer.alloc(12);
  h.write('PMS1');h.writeUInt32LE(json.length,4);h.writeUInt32LE(4,8);
  process.stdout.write(Buffer.concat([h,json,Buffer.from([1,2,3,255])]));
 }
});`;
const manager = () => new StudioHostManager({ executable: process.execPath, args: ['-e', fixture], env: { ELECTRON_RUN_AS_NODE: '1' }, timeoutMs: 1000 });

describe('native Studio process', () => {
  it('bounds retained input while the helper is still starting', async () => {
    const host = new StudioHostManager({ executable: process.execPath, args: ['-e', fixture.replace('if(q.hang)continue;', "if(q.hang||q.op==='hello')continue;")], timeoutMs: 100, maxQueuedBytes: 1024 });
    const first = host.request({op:'bounds'},new Uint8Array(700)).catch(()=>{});
    try { await expect(host.request({op:'bounds'},new Uint8Array(700))).rejects.toThrow('queue'); }
    finally { host.dispose(); await first; }
  });
  it('bounds queued bytes and reserves only three frame-release requests', async () => {
    const host = new StudioHostManager({ executable: process.execPath, args: ['-e', fixture], timeoutMs: 10_000, maxQueuedBytes: 1024 });
    const pending: Promise<unknown>[] = [];
    try {
      await host.connect();
      pending.push(host.request({ op: 'bounds', hang: true }, new Uint8Array(700)).catch(() => {}));
      await expect(host.request({ op: 'bounds' }, new Uint8Array(700))).rejects.toThrow('queue');
    } finally { host.dispose(); await Promise.all(pending); }
    const releases = manager();
    try {
      await releases.connect();
      for (let i = 0; i < 11; i++) pending.push(releases.request({ op: 'releaseFrame', frame: i, hang: true }).catch(() => {}));
      await expect(releases.request({ op: 'releaseFrame', frame: 12 })).rejects.toThrow('queue');
    } finally { releases.dispose(); await Promise.all(pending); }
  });
  it('reaps a host that ignores graceful termination', async () => {
    let child!: ReturnType<typeof spawn>;
    const host = new StudioHostManager({
      executable: process.execPath, args: ['-e', `setInterval(()=>{},1000);process.on('SIGTERM',()=>{});${fixture}`], killTimeoutMs: 30,
      spawnHost: ((...args: Parameters<typeof spawn>) => (child = spawn(...args))) as typeof spawn,
    });
    try {
      await host.connect();
      const exited = once(child, 'exit');
      host.dispose();
      let timer: ReturnType<typeof setTimeout>;
      const result = await Promise.race([exited, new Promise<null>(resolve => { timer = setTimeout(() => resolve(null), 1000); })]).finally(() => clearTimeout(timer));
      expect(result).toEqual([null, 'SIGKILL']);
    } finally { child?.kill('SIGKILL'); host.dispose(); }
  });
  it('exchanges binary packets without exposing Electron launch flags', async () => {
    const host = manager();
    try {
      const reply = await host.request({ op: 'bounds' });
      expect(reply.result).toEqual({ op: 'bounds', runAsNode: null });
      expect([...reply.binary]).toEqual([1, 2, 3, 255]);
      expect(host.status().running).toBe(true);
      expect(reply.roundTripMs).toBeGreaterThan(0);
      expect((await host.request({op:'preparedView',key:'test',view:{}})).result.op).toBe('preparedView');
    } finally { host.dispose(); }
  });
  it('rejects a crashed render and restarts with a fresh generation', async () => {
    const host = manager();
    try {
      const before = await host.request({ op: 'bounds' });
      await expect(host.request({ op: 'render', crash: true })).rejects.toThrow('Native Studio exited');
      expect(host.status().running).toBe(false);
      await new Promise(resolve => setTimeout(resolve, 120));
      const after = await host.request({ op: 'bounds' });
      expect(after.generation).toBeGreaterThan(before.generation);
      expect(host.status().failure).toBe(null);
    } finally { host.dispose(); }
  });
  it('lets the caller choose WebGPU when the native binary is missing', async () => {
    const host = new StudioHostManager({ executable: null });
    const chosen = await host.connect().then(() => 'native', () => 'webgpu');
    expect(chosen).toBe('webgpu');
    expect(host.status().available).toBe(false);
    host.dispose();
  });
  it('handles spawn failures and refuses requests after disposal', async () => {
    const host = new StudioHostManager({ executable: '/nonexistent/powermove-studio-host' });
    await expect(host.connect()).rejects.toThrow();
    expect(host.status().running).toBe(false);
    host.dispose();
    await expect(host.request({ op: 'hello' })).rejects.toThrow('closed');
  });
});

describe('native Studio framing', () => {
  it('decodes split headers and several binary packets in one chunk', () => {
    const encode = (id: number) => {
      const [header, json, binary] = studioPacket(id, { ok: true, result: { id } }, new Uint8Array([0, 255]));
      return Buffer.concat([header!, json!, binary!]);
    };
    const bytes = Buffer.concat([encode(1), encode(2)]), decoder = new StudioPacketDecoder();
    const packets = [...decoder.push(bytes.subarray(0, 7)), ...decoder.push(bytes.subarray(7, 21)), ...decoder.push(bytes.subarray(21))];
    expect(packets.map(p => p.json.id)).toEqual([1, 2]);
    expect([...packets[1]!.binary]).toEqual([0, 255]);
  });
  it('rejects invalid and oversized headers before allocating a body', () => {
    expect(() => new StudioPacketDecoder().push(Buffer.alloc(12))).toThrow('magic');
    const header = Buffer.alloc(12); header.write('PMS1'); header.writeUInt32LE(0xffffffff, 4);
    expect(() => new StudioPacketDecoder().push(header)).toThrow('size limit');
  });
});

it('keeps custom quality deadlines longer than their tracing limit',()=>{
  expect(studioCommandTimeoutMs({op:'render',tier:'final',minSamples:4,timeBudgetMs:600000})).toBe(660000);
  expect(studioCommandTimeoutMs({op:'render',tier:'final',minSamples:4,timeBudgetMs:0})).toBe(3660000);
  expect(studioCommandTimeoutMs({op:'render',tier:'live'})).toBe(120000);
  expect(studioCommandTimeoutMs({op:'bounds'})).toBe(30000);
});
