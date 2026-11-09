import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { StudioPacketDecoder, studioPacket } from './protocol';
import { STUDIO_PROTOCOL, type StudioNativeCommand, type StudioNativeReply, type StudioNativeStatus } from '../../shared/studio-native';

type Pending = { resolve(value: StudioNativeReply): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout>; started: number };
export type HostOptions = {
  executable: string | null;
  args?: string[];
  env?: NodeJS.ProcessEnv;
  timeoutMs?: number;
  surfaceService?: string;
  spawnHost?: typeof spawn;
};
const OPS = new Set(['hello', 'scene', 'preset', 'patch', 'screen', 'render', 'pick', 'bounds', 'shutdown', 'prepared', 'preparedPatch', 'preparedView', 'textures', 'snapshot', 'releaseFrame']);

/** One GPU process per active editor session. A failure rejects all pending work;
 * the next connection starts a fresh process and a new scene generation. */
export class StudioHostManager {
  private child: ChildProcessWithoutNullStreams | null = null;
  private pending = new Map<number, Pending>();
  private nextId = 1;
  private generation = 0;
  private failure: string | null = null;
  private starting: Promise<void> | null = null;
  private disposed = false;
  private crashes = 0;
  private retryAfter = 0;
  constructor(private readonly options: HostOptions) {}

  status(): StudioNativeStatus { return { available: !!this.options.executable && !this.disposed, running: !!this.child, generation: this.generation, failure: this.failure }; }
  async connect(): Promise<void> {
    if (this.disposed) throw new Error('Studio session is closed');
    if (this.child && !this.starting) return;
    if (this.starting) return this.starting;
    if (!this.options.executable) throw new Error('Native Studio is not installed');
    if (performance.now() < this.retryAfter) throw new Error(this.failure ?? 'Native Studio is recovering');
    this.starting = (async () => {
      const env = { ...process.env, ...this.options.env };
      delete env.ELECTRON_RUN_AS_NODE;
      if(this.options.surfaceService)env.STUDIO_SURFACE_SERVICE=this.options.surfaceService;
      const child = (this.options.spawnHost ?? spawn)(this.options.executable!, this.options.args ?? [], { stdio: ['pipe', 'pipe', 'pipe'], env, windowsHide: true });
      this.child = child; this.generation++;
      const decoder = new StudioPacketDecoder();
      const failed = (error: Error) => { if (this.child === child) this.fail(error); };
      child.once('error', failed);
      child.once('exit', (code, signal) => failed(new Error(`Native Studio exited (${signal ?? code ?? 'unknown'})`)));
      child.stdin.on('error', failed);
      child.stdout.on('data', (chunk: Buffer) => {
        if (this.child !== child) return;
        try {
          for (const packet of decoder.push(chunk)) {
            const waiting = this.pending.get(packet.json.id);
            if (!waiting) continue;
            this.pending.delete(packet.json.id); clearTimeout(waiting.timer);
            if (!packet.json.ok) waiting.reject(new Error(packet.json.error ?? 'Studio request failed'));
            else waiting.resolve({ result: packet.json.result, binary: packet.binary, generation: this.generation, roundTripMs: performance.now() - waiting.started });
          }
        } catch (error) { failed(error instanceof Error ? error : new Error(String(error))); }
      });
      // Keep the pipe draining. Scene contents and local operational details are not logged.
      child.stderr.on('data', () => {});
      const hello = await this.send({ op: 'hello' });
      if (hello.result?.protocol !== STUDIO_PROTOCOL) throw new Error('Native Studio protocol version mismatch');
      this.failure = null;
    })().catch(error => { this.fail(error instanceof Error ? error : new Error(String(error))); throw error; }).finally(() => { this.starting = null; });
    return this.starting;
  }
  async request(command: StudioNativeCommand, binary?: Uint8Array): Promise<StudioNativeReply> {
    if (!command || typeof command !== 'object' || !OPS.has(command.op)) throw new Error('Invalid Studio command');
    await this.connect();
    return this.send(command, binary);
  }
  private send(command: StudioNativeCommand, binary?: Uint8Array): Promise<StudioNativeReply> {
    const child = this.child;
    if (!child) return Promise.reject(new Error('Native Studio is unavailable'));
    if (this.pending.size >= 8 && command.op!=='releaseFrame') return Promise.reject(new Error('Native Studio queue is full'));
    const id = this.nextId++;
    // Validate/serialize before adding a pending promise or touching stdin.
    const parts = studioPacket(id, command, binary);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.fail(new Error('Native Studio response timed out')), this.options.timeoutMs ?? (command.op === 'render' ? 120_000 : 30_000));
      timer.unref?.();
      this.pending.set(id, { resolve, reject, timer, started: performance.now() });
      child.stdin.cork();
      for (const part of parts) child.stdin.write(part);
      child.stdin.uncork();
    });
  }
  private fail(error: Error): void {
    const child = this.child; this.child = null; this.failure = error.message;
    if (child) { child.stdin.destroy(); child.kill(); this.crashes++; this.retryAfter = performance.now() + Math.min(10_000, 100 * 2 ** Math.min(this.crashes - 1, 6)); }
    for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(error); }
    this.pending.clear();
  }
  dispose(): void { this.disposed = true; this.fail(new Error('Studio session is closed')); }
}

export function studioHostExecutable(appPath: string, resourcesPath?: string): string | null {
  const filename = process.platform === 'win32' ? 'pm-studio-host.exe' : 'pm-studio-host';
  const candidates = resourcesPath ? [path.join(resourcesPath, 'studio', filename)] : [
    process.env.POWERMOVE_STUDIO_HOST,
    path.join(appPath, 'resources', 'studio', filename),
    path.join(process.env.CARGO_TARGET_DIR ?? path.join(homedir(), 'Developer', 'pm-target'), 'release', filename),
    path.resolve(appPath, '../../engine/target/release', filename),
  ];
  return candidates.find((p): p is string => !!p && existsSync(p)) ?? null;
}
