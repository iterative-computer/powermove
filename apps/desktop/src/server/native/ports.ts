/** MessagePort semantics over Node child IPC. Main alone creates/transfers ports;
 * a child can address only endpoints transferred to that launch. */
import { EventEmitter } from 'node:events';
import { fork, type ChildProcess } from 'node:child_process';
import type { ChannelFactory, ChildLike, PortLike, SpawnEngine } from '../../main/engine/ports';

type Packet = { t: 'port'; id: number; data: unknown; ports: number[] } | { t: 'close'; id: number } | { t: 'bootstrap'; data: unknown; ports: number[] } | { t: 'notice'; data: unknown };

class LocalPort extends EventEmitter implements PortLike {
  peer!: LocalPort;
  remote: { child: ChildProcess; id: number } | null = null;
  private started = false;
  private ended = false;
  private pending: Array<{ data: unknown; ports: PortLike[] }> = [];
  constructor(private readonly transport: NodeEngineTransport) { super(); }
  postMessage(data: unknown, ports: PortLike[] = []): void {
    if (!this.ended) this.peer.deliver(data, ports);
    else for (const port of ports) port.close();
  }
  deliver(data: unknown, ports: PortLike[] = []): void {
    if (this.ended) { for (const port of ports) port.close(); return; }
    if (this.remote) {
      this.transport.send(this.remote.child, { t: 'port', id: this.remote.id, data, ports: this.transport.transfer(this.remote.child, ports) });
    } else {
      this.pending.push({ data, ports });
      this.drain();
    }
  }
  transferTo(child: ChildProcess, id: number): void {
    this.remote = { child, id };
    // Messages posted before the endpoint was transferred are still owned by
    // it. Deliver them to the child, where start() drains in the same order.
    for (const pending of this.pending.splice(0)) this.deliver(pending.data, pending.ports);
  }
  start(): void { this.started = true; this.drain(); }
  private drain(): void {
    while (this.started && !this.ended && this.pending.length) this.emit('message', this.pending.shift()!);
  }
  close(): void { this.end(); this.peer.end(); }
  private end(): void {
    if (this.ended) return;
    this.ended = true;
    this.pending.length = 0;
    if (this.remote) { this.transport.send(this.remote.child, { t: 'close', id: this.remote.id }); this.transport.forget(this.remote.id); }
    this.emit('close');
  }
}

export class NodeEngineTransport {
  private nextId = 1;
  private readonly assigned = new WeakMap<ChildProcess, Set<number>>();
  forget(id: number): void { this.endpoints.delete(id); }
  private endpoints = new Map<number, { port: LocalPort; child: ChildProcess }>();
  readonly channel: ChannelFactory = () => {
    const a = new LocalPort(this), b = new LocalPort(this);
    a.peer = b; b.peer = a;
    return { port1: a, port2: b };
  };
  send(child: ChildProcess, packet: Packet): void {
    if (child.connected) child.send(packet, error => { if (error && child.connected) child.disconnect(); });
  }
  transfer(child: ChildProcess, ports: PortLike[]): number[] {
    return ports.map(value => {
      if (!(value instanceof LocalPort) || value.remote) throw new Error('An engine port can be transferred once, by its host.');
      const id = this.nextId++;
      let assigned = this.assigned.get(child);
      if (!assigned) { assigned = new Set(); this.assigned.set(child, assigned); }
      assigned.add(id);
      this.endpoints.set(id, { port: value, child });
      value.transferTo(child, id);
      return id;
    });
  }
  spawn(entry: string, addon: string, log: (line: string) => void, env: Record<string, string> = {}): SpawnEngine {
    return role => {
      const child = fork(entry, [], { serialization: 'advanced', stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
        env: { ...process.env, ...env, POWERMOVE_ENGINE_PROCESS: role, POWERMOVE_ENGINE_ADDON: addon } });
      const events = new EventEmitter();
      child.stdout?.on('data', bytes => log(`[${role}] ${bytes.toString().trimEnd()}`));
      child.stderr?.on('data', bytes => log(`[${role}] ${bytes.toString().trimEnd()}`));
      child.on('message', (raw: unknown) => {
        const packet = raw as Packet;
        if (packet?.t === 'notice') { events.emit('message', packet.data); return; }
        if (packet?.t !== 'port' && packet?.t !== 'close') return;
        const endpoint = this.endpoints.get(packet.id);
        if (!Number.isSafeInteger(packet.id) || !this.assigned.get(child)?.has(packet.id)) { child.kill(); return; }
        if (!endpoint) return; // A message already in flight when the host closed.
        if (endpoint.child !== child) { child.kill(); return; }
        if (packet.t === 'close') { this.endpoints.delete(packet.id); endpoint.port.close(); }
        else if (Array.isArray(packet.ports) && !packet.ports.length) endpoint.port.postMessage(packet.data);
        else child.kill(); // Only the host creates/transfers endpoints.
      });
      child.on('spawn', () => events.emit('spawn'));
      child.on('error', error => log(`native-engine-child: ${error.message}`));
      child.on('exit', (code, signal) => {
        for (const [id, endpoint] of this.endpoints) if (endpoint.child === child) { this.endpoints.delete(id); endpoint.port.close(); }
        events.emit('exit', code ?? (signal === 'SIGKILL' ? 9 : -1));
      });
      return {
        get pid() { return child.pid; },
        postMessage: (data, ports = []) => this.send(child, { t: 'bootstrap', data, ports: this.transfer(child, ports) }),
        on: (event: string, listener: (...args: any[]) => void) => events.on(event, listener),
        kill: () => child.kill('SIGKILL'),
      } as ChildLike;
    };
  }
}

/** Child-side endpoints: the common engine runtime never knows its carrier. */
export function childParent(): Pick<PortLike, 'postMessage' | 'on'> {
  const parent = new EventEmitter();
  const ports = new Map<number, ChildPort>();
  const send = (packet: Packet) => { if (process.connected) process.send?.(packet); };
  class ChildPort extends EventEmitter implements PortLike {
    private active = false;
    private closed = false;
    private pending: Array<{ data: unknown; ports: PortLike[] }> = [];
    constructor(readonly id: number) { super(); }
    postMessage(data: unknown): void { if (!this.closed) send({ t: 'port', id: this.id, data, ports: [] }); }
    receive(data: unknown, transferred: PortLike[]): void { this.pending.push({ data, ports: transferred }); this.drain(); }
    start(): void { this.active = true; this.drain(); }
    private drain(): void { while (this.active && !this.closed && this.pending.length) this.emit('message', this.pending.shift()!); }
    close(): void {
      if (this.closed) return;
      this.closed = true; ports.delete(this.id); this.pending.length = 0;
      send({ t: 'close', id: this.id }); this.emit('close');
    }
  }
  const get = (id: number): ChildPort => {
    let port = ports.get(id);
    if (!port) { port = new ChildPort(id); ports.set(id, port); }
    return port;
  };
  process.on('message', (raw: unknown) => {
    const packet = raw as Packet;
    if (packet?.t === 'bootstrap') parent.emit('message', { data: packet.data, ports: packet.ports.map(get) });
    else if (packet?.t === 'port') get(packet.id).receive(packet.data, packet.ports.map(get));
    else if (packet?.t === 'close') ports.get(packet.id)?.close();
  });
  process.on('disconnect', () => process.exit(0));
  return Object.assign(parent, { postMessage: (data: unknown) => send({ t: 'notice', data }) }) as Pick<PortLike, 'postMessage' | 'on'>;
}
