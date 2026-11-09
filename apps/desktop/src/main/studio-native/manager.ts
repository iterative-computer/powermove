import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { StudioPacketDecoder, studioPacket } from './protocol';
import { STUDIO_MAX_BINARY, STUDIO_MAX_JSON, STUDIO_PROTOCOL, type StudioNativeCommand, type StudioNativeReply, type StudioNativeStatus } from '../../shared/studio-native';

type Pending = { resolve(value: StudioNativeReply): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout>; started: number; bytes: number };
export type HostOptions = {
  executable: string | null;
  args?: string[];
  env?: NodeJS.ProcessEnv;
  timeoutMs?: number;
  killTimeoutMs?: number;
  maxQueuedBytes?: number;
  surfaceService?: string;
  spawnHost?: typeof spawn;
};
const OPS = new Set(['hello', 'scene', 'preset', 'patch', 'screen', 'render', 'pick', 'bounds', 'shutdown', 'prepared', 'preparedBegin', 'preparedChunk', 'preparedCommit', 'preparedPatch', 'preparedView', 'cancelRefine', 'textures', 'snapshot', 'releaseFrame', 'importFbx']);

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
  private queuedBytes = 0;
  private retainedBytes = 0;
  private requests = 0;
  constructor(private readonly options: HostOptions) {}

  /** Main-only identity used to authenticate incoming Mach surface messages. */
  get processId(): number | null { return this.child?.pid ?? null; }

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
            this.pending.delete(packet.json.id); this.queuedBytes -= waiting.bytes; clearTimeout(waiting.timer);
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
    // Reserve before awaiting the handshake: otherwise callers can retain
    // unbounded scene payloads while a cold or hung helper is starting.
    const bytes = studioPacket(0, command, binary).reduce((n, part) => n + part.byteLength, 0);
    if (this.requests >= (command.op === 'releaseFrame' ? 11 : 8) || this.retainedBytes + bytes > (this.options.maxQueuedBytes ?? STUDIO_MAX_BINARY + STUDIO_MAX_JSON + 4096)) throw new Error('Native Studio queue memory budget exceeded');
    this.requests++; this.retainedBytes += bytes;
    try { await this.connect(); return await this.send(command, binary); }
    finally { this.requests--; this.retainedBytes -= bytes; }
  }
  private send(command: StudioNativeCommand, binary?: Uint8Array): Promise<StudioNativeReply> {
    const child = this.child;
    if (!child) return Promise.reject(new Error('Native Studio is unavailable'));
    // Three reserved acknowledgements match the IOSurface pool. They cannot
    // bypass the limit indefinitely, even if a renderer sends release spam.
    if (this.pending.size >= (command.op === 'releaseFrame' ? 11 : 8)) return Promise.reject(new Error('Native Studio queue is full'));
    const id = this.nextId++;
    // Validate/serialize before adding a pending promise or touching stdin.
    const parts = studioPacket(id, command, binary);
    const bytes = parts.reduce((n, part) => n + part.byteLength, 0);
    if (this.queuedBytes + bytes > (this.options.maxQueuedBytes ?? STUDIO_MAX_BINARY + STUDIO_MAX_JSON + 4096)) return Promise.reject(new Error('Native Studio queue memory budget exceeded'));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.fail(new Error('Native Studio response timed out')), this.options.timeoutMs ?? (command.op === 'render' ? 120_000 : 30_000));
      timer.unref?.();
      this.pending.set(id, { resolve, reject, timer, started: performance.now(), bytes });
      this.queuedBytes += bytes;
      try {
        child.stdin.cork();
        for (const part of parts) child.stdin.write(part);
        child.stdin.uncork();
      } catch { this.fail(new Error('Native Studio pipe write failed')); }
    });
  }
  private fail(error: Error): void {
    const child = this.child; this.child = null; this.failure = error.message;
    if (child) {
      child.stdin.destroy(); child.kill();
      const reap = setTimeout(() => {
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      }, this.options.killTimeoutMs ?? 1000);
      reap.unref(); child.once('exit', () => clearTimeout(reap));
      this.crashes++; this.retryAfter = performance.now() + Math.min(10_000, 100 * 2 ** Math.min(this.crashes - 1, 6));
    }
    for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(error); }
    this.pending.clear();
    this.queuedBytes = 0;
  }
  dispose(): void { this.disposed = true; this.fail(new Error('Studio session is closed')); }
}

export function studioHostExecutable(appPath: string, resourcesPath?: string): string | null {
  const filename = process.platform === 'win32' ? 'pm-studio-host.exe' : 'pm-studio-host';
  const candidates = resourcesPath ? [path.join(resourcesPath, 'studio', filename)] : [
    process.env.POWERMOVE_STUDIO_HOST,
    path.join(appPath, 'resources', 'studio', filename),
    // Playwright and direct Electron launches use out/main as app.getAppPath().
    // Prefer the built runtime beside the project, including its surface addon.
    path.resolve(appPath, '../../resources/studio', filename),
    // Never guess a helper in a shared Cargo cache: another checkout can have
    // a different ABI. The build copies this checkout's verified runtime above.
    path.resolve(appPath, '../../engine/target/release', filename),
  ];
  return candidates.find((p): p is string => !!p && existsSync(p)) ?? null;
}
