/*
 * Blender supervision in host services (contract 14) for the engines'
 * Blender requests (contract 11: the job engine's exports, the interactive
 * engine's paused viewer), behind "Use New Engine". Design notes:
 * docs/engine/blender.md.
 *
 * The job engine writes each frame's snapshot (`renderSnapshotJsonV1`)
 * into the Blender Surfaces cache area and sends `blenderRender` with one
 * surface request per layer. Main runs the pinned worker
 * (engine/crates/pm-blender/worker/worker.py, the same file pm-blender
 * hashes into every surface key) in a supervised Blender:
 *
 * - least privilege: `--background --factory-startup --disable-autoexec
 *   --offline-mode`, a scrubbed environment (fixed PATH; HOME, TMPDIR and
 *   BLENDER_USER_* in the worker's own scratch directory; nothing of
 *   main's), its own process group, assets copied in as `<id>.<ext>`;
 * - today's bounds: one request at a time, a queue of 16, the 10-minute
 *   timeout, today's snapshot validation (`validateBlenderJob`);
 * - output: each surface is moved to `<area>/<key>.png`, checked against
 *   the per-surface cap, and announced with a `blenderSurface` event (the
 *   crop is the full surface; the engine finds the occupied rectangle when
 *   it decodes); progress follows each one; the request is answered when
 *   its job ends: `ack`, or `blenderMissing`, `blenderFailed`, `cancelled`;
 * - cancel stops Blender: `cancelJobs` for a Blender job kills the worker's
 *   process group; an engine launch ending cancels its jobs.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, open, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';

import type { BlenderJobRequest, Event, ResolvedDevice } from '../../shared/engine-protocol';
import { validateBlenderJob } from '../blender-service';
import WORKER_SCRIPT from '../../../../../engine/crates/pm-blender/worker/worker.py?raw';
import type { HandleTable } from './handles';
import { ProtocolError } from './protocol';

/** SHA-256 of the pinned worker script, as pm-blender computes it. */
export const WORKER_SCRIPT_SHA256 = createHash('sha256').update(WORKER_SCRIPT).digest();
export { WORKER_SCRIPT };

/** Contract 11's worker flags, then a Python failure as the exit status. */
export const WORKER_FLAGS = ['--background', '--factory-startup', '--disable-autoexec', '--offline-mode', '--python-exit-code', '1', '--python'] as const;

const MAX_QUEUED_JOBS = 16;
const MAX_SURFACES = 4096;
const MAX_SNAPSHOT_BYTES = 64 * 1024 * 1024;
const MAX_INPUT_BYTES = 256 * 1024 * 1024;
const MAX_OUTPUT_BYTES_PER_SURFACE = 8192 * 8192 * 8 + 16 * 1024 * 1024;
const ASSET_EXTENSIONS = new Set(['.blend', '.glb', '.gltf', '.obj', '.png', '.jpg', '.jpeg', '.webp', '.exr', '.hdr']);
const safeId = /^[a-zA-Z][a-zA-Z0-9_-]{0,159}$/;

export type BlenderLocation = { available: boolean; path?: string; version?: string; error?: string };
export type BlenderLocator = () => Promise<BlenderLocation>;

/** `Blender 4.5.14 LTS` → `4.5.14`. */
export function blenderVersion(banner: string | undefined): string | null {
  const m = /(\d+)\.(\d+)(?:\.(\d+))?/.exec(banner ?? '');
  return m ? `${m[1]}.${m[2]}.${m[3] ?? 0}` : null;
}

/** The worker's scrubbed environment: nothing inherited from main. */
export function workerEnvironment(root: string): Record<string, string> {
  return {
    PATH: '/usr/bin:/bin:/usr/sbin:/sbin',
    HOME: path.join(root, 'home'),
    TMPDIR: path.join(root, 'tmp'),
    BLENDER_USER_RESOURCES: path.join(root, 'user'),
    BLENDER_USER_CONFIG: path.join(root, 'user/config'),
    BLENDER_USER_SCRIPTS: path.join(root, 'user/scripts'),
    BLENDER_USER_DATAFILES: path.join(root, 'user/datafiles'),
    PYTHONUNBUFFERED: '1',
    PYTHONNOUSERSITE: '1',
    PYTHONDONTWRITEBYTECODE: '1',
    LANG: 'en_US.UTF-8'
  };
}

/** The device a manifest's `resolvedDevice` names. */
export function resolvedDevice(manifest: unknown): ResolvedDevice | null {
  const d = (manifest as { resolvedDevice?: { kind?: unknown; name?: unknown } } | null)?.resolvedDevice;
  const name = typeof d?.name === 'string' ? d.name : '';
  switch (d?.kind) {
    case 'cpu': return { kind: 'cpu' };
    case 'metal': return { kind: 'metal', name };
    case 'optix': return { kind: 'optix', name };
    case 'cuda': return { kind: 'cuda', name };
    case 'hip': return { kind: 'hip', name };
    case 'oneapi': return { kind: 'oneApi', name };
    default: return null;
  }
}

/** Width and height from a PNG's IHDR (no decoding). */
export async function pngSize(file: string): Promise<{ width: number; height: number }> {
  const handle = await open(file, 'r');
  try {
    const head = Buffer.alloc(24);
    await handle.read(head, 0, 24, 0);
    if (!head.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) || head.toString('latin1', 12, 16) !== 'IHDR')
      throw new ProtocolError('blenderFailed', 'Blender wrote a surface that is not a PNG.');
    return { width: head.readUInt32BE(16), height: head.readUInt32BE(20) };
  } finally {
    await handle.close();
  }
}

const hex = (bytes: Uint8Array): string => Buffer.from(bytes).toString('hex');

/** Removes the scratch of processes that no longer run. */
export async function sweepScratch(parent: string): Promise<void> {
  for (const name of await readdir(parent).catch(() => [] as string[])) {
    const pid = Number(name);
    if (!Number.isSafeInteger(pid) || pid <= 0 || pid === process.pid) continue;
    let alive = true;
    try {
      process.kill(pid, 0);
    } catch (error) {
      alive = (error as NodeJS.ErrnoException).code === 'EPERM';
    }
    if (!alive) await rm(path.join(parent, name), { recursive: true, force: true }).catch(() => {});
  }
}

type Pending = { id: string; resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> | null; left: number; since: number };

/** One serialized Blender process running the pinned worker. */
export class PinnedWorker {
  private proc: ChildProcess | null = null;
  private root = '';
  private pending: Pending | null = null;
  private stderr = '';
  private counter = 0;
  private paused = false;

  constructor(readonly executable: string, readonly tempRoot: string, readonly timeoutMs = 10 * 60 * 1000) {}

  get scratch(): string {
    return this.root;
  }

  get pid(): number | null {
    return this.proc?.pid ?? null;
  }

  async start(): Promise<void> {
    if (this.proc) return;
    this.root = await mkdtemp(path.join(this.tempRoot, 'powermove-blender-'));
    for (const d of ['home', 'tmp', 'user/config', 'user/scripts', 'user/datafiles']) await mkdir(path.join(this.root, d), { recursive: true, mode: 0o700 });
    const script = path.join(this.root, 'worker.py');
    await writeFile(script, WORKER_SCRIPT, { mode: 0o600 });
    const proc = spawn(this.executable, [...WORKER_FLAGS, script], {
      cwd: this.root,
      env: workerEnvironment(this.root),
      stdio: ['pipe', 'pipe', 'pipe'],
      detached: true
    });
    this.proc = proc;
    if (this.paused && proc.pid) {
      try { process.kill(-proc.pid, 'SIGSTOP'); } catch {}
    }
    proc.stdin?.on('error', () => {});
    proc.stderr?.on('data', (b: Buffer) => {
      this.stderr = (this.stderr + b.toString()).slice(-8000);
    });
    readline.createInterface({ input: proc.stdout! }).on('line', (line) => {
      if (!line.startsWith('PM_BLENDER:')) return;
      try {
        const msg = JSON.parse(line.slice(11));
        const p = this.pending;
        if (!p || msg.id !== p.id) return;
        this.pending = null;
        if (msg.ok) p.resolve();
        else p.reject(new ProtocolError('blenderFailed', String(msg.error || 'Blender operation failed')));
      } catch {}
    });
    const stopped = (why: string) => {
      if (this.proc !== proc) return;
      this.proc = null;
      const p = this.pending;
      this.pending = null;
      p?.reject(new ProtocolError('blenderFailed', `Blender stopped (${why}). ${this.stderr.slice(-2000)}`));
    };
    proc.on('error', (e) => stopped(e.message));
    proc.on('close', (code, signal) => stopped(String(code ?? signal ?? 'cancelled')));
  }

  /** A new request directory inside the worker's scratch. */
  async requestDir(): Promise<string> {
    await this.start();
    const dir = path.join(this.root, `job-${++this.counter}`);
    await mkdir(dir, { recursive: true, mode: 0o700 });
    return dir;
  }

  /** Runs the request in `requestPath`; rejects on failure, timeout, or kill. */
  run(id: string, requestPath: string): Promise<void> {
    if (!this.proc) return Promise.reject(new ProtocolError('blenderFailed', 'Blender is not running.'));
    if (this.pending) return Promise.reject(new ProtocolError('internal', 'Blender is busy.'));
    return new Promise<void>((resolve, reject) => {
      const pending: Pending = {
        id,
        resolve: () => { if (pending.timer) clearTimeout(pending.timer); resolve(); },
        reject: (e) => { if (pending.timer) clearTimeout(pending.timer); reject(e); },
        timer: null,
        left: this.timeoutMs,
        since: Date.now()
      };
      this.pending = pending;
      if (!this.paused) this.arm(pending);
      this.proc!.stdin!.write(JSON.stringify({ id, path: requestPath }) + '\n');
    });
  }

  /** The timeout counts only while Blender runs (not while paused). */
  private arm(p: Pending): void {
    p.since = Date.now();
    p.timer = setTimeout(() => {
      this.kill();
      p.reject(new ProtocolError('blenderFailed', 'Blender operation timed out'));
    }, Math.max(0, p.left));
  }

  /** Pauses or resumes Blender (SIGSTOP and SIGCONT of its process group):
   *  contract 13's `BlenderThrottle::Pause` while anything plays. */
  setPaused(paused: boolean): void {
    if (paused === this.paused) return;
    this.paused = paused;
    const pid = this.proc?.pid;
    if (pid) {
      try { process.kill(-pid, paused ? 'SIGSTOP' : 'SIGCONT'); } catch {}
    }
    const p = this.pending;
    if (!p) return;
    if (paused) {
      if (p.timer) clearTimeout(p.timer);
      p.timer = null;
      p.left -= Date.now() - p.since;
    } else {
      this.arm(p);
    }
  }

  get isPaused(): boolean {
    return this.paused;
  }

  /** Kills the worker's process group (cancel stops Blender). */
  kill(reason: Error = new ProtocolError('cancelled', 'Blender request cancelled')): void {
    const proc = this.proc;
    this.proc = null;
    if (proc?.pid) {
      try { process.kill(-proc.pid, 'SIGKILL'); } catch {}
      try { proc.kill('SIGKILL'); } catch {}
    }
    const p = this.pending;
    this.pending = null;
    p?.reject(reason);
  }

  async dispose(): Promise<void> {
    this.kill();
    const root = this.root;
    this.root = '';
    if (root) await rm(root, { recursive: true, force: true }).catch(() => {});
  }
}

export type EngineBlenderOptions = {
  locate: BlenderLocator;
  handles: HandleTable;
  tempRoot: string;
  log: (level: 'info' | 'warn' | 'error', message: string) => void;
  timeoutMs?: number;
};

type Queued = { job: string; port: number; cancelled: boolean };

/** Host services' Blender: status, render jobs, and their cancellation. */
export class EngineBlender {
  private worker: PinnedWorker | null = null;
  private tail: Promise<unknown> = Promise.resolve();
  private queued: Queued[] = [];
  private current: Queued | null = null;
  private paused = false;

  /** This process's scratch parent: `<tmp>/powermove-engine-blender/<pid>`.
   *  Scratch of processes that no longer run is removed at start (an app
   *  that was killed never disposed its worker). */
  private readonly scratch: string;

  constructor(private readonly options: EngineBlenderOptions) {
    const parent = path.join(options.tempRoot, 'powermove-engine-blender');
    this.scratch = path.join(parent, String(process.pid));
    void sweepScratch(parent);
  }

  async status(): Promise<{ type: 'blenderStatus'; available: boolean; version: string | null; error: string | null }> {
    const found = await this.options.locate();
    const version = blenderVersion(found.version);
    return { type: 'blenderStatus', available: !!found.available && !!version, version, error: found.available ? null : (found.error ?? null) };
  }

  /** Runs one render job; `send` delivers events to the requesting engine. */
  render(req: BlenderJobRequest, session: number, port: number, send: (event: Event) => void): Promise<void> {
    if (this.queued.length + (this.current ? 1 : 0) >= MAX_QUEUED_JOBS || this.queued.some((q) => q.job === req.job) || this.current?.job === req.job)
      return Promise.reject(new ProtocolError('blenderFailed', 'Blender queue is full or request already exists'));
    const entry: Queued = { job: req.job, port, cancelled: false };
    this.queued.push(entry);
    const work = this.tail.catch(() => {}).then(async () => {
      this.queued = this.queued.filter((q) => q !== entry);
      if (entry.cancelled) throw new ProtocolError('cancelled', 'Blender request cancelled');
      this.current = entry;
      try {
        await this.run(req, session, port, send, entry);
      } finally {
        this.current = null;
      }
    });
    this.tail = work;
    return work;
  }

  /** Cancels a queued or running job: a running one kills Blender. */
  cancel(job: string): boolean {
    const queued = this.queued.find((q) => q.job === job);
    if (queued) {
      queued.cancelled = true;
      return true;
    }
    if (this.current?.job === job) {
      this.current.cancelled = true;
      this.worker?.kill();
      return true;
    }
    return false;
  }

  /** Whether `job` is queued or running for the engine launch on `port`
   *  (an engine cancels only its own Blender jobs). */
  owns(job: string, port: number): boolean {
    return this.queued.some((q) => q.job === job && q.port === port) || (this.current?.job === job && this.current.port === port);
  }

  /** Playback started or stopped anywhere: Blender pauses while anything
   *  plays (`SlicingPolicy.blender` is `pause`, S8). */
  setPaused(paused: boolean): void {
    this.paused = paused;
    this.worker?.setPaused(paused);
  }

  /** An engine launch ended: its Blender jobs stop. */
  endPort(port: number): void {
    for (const q of this.queued) if (q.port === port) q.cancelled = true;
    if (this.current?.port === port) this.cancel(this.current.job);
  }

  async dispose(): Promise<void> {
    for (const q of this.queued) q.cancelled = true;
    await this.worker?.dispose();
    this.worker = null;
    await rm(this.scratch, { recursive: true, force: true }).catch(() => {});
  }

  private area(handle: string, session: number, port: number): string {
    const r = this.options.handles.resolve(handle, session, port);
    if (r.kind.kind !== 'cache' || r.kind.cache !== 'blenderSurfaces' || r.access !== 'readWrite')
      throw new ProtocolError('capabilityRejected', 'Blender jobs read and write the Blender Surfaces cache area.');
    return r.path;
  }

  private async run(req: BlenderJobRequest, session: number, port: number, send: (event: Event) => void, entry: Queued): Promise<void> {
    if (!Buffer.from(req.workerScriptHash).equals(WORKER_SCRIPT_SHA256))
      throw new ProtocolError('invalidMessage', 'The Blender worker script differs from the one host services run.');
    if (!Array.isArray(req.surfaces) || req.surfaces.length < 1 || req.surfaces.length > MAX_SURFACES)
      throw new ProtocolError('invalidMessage', `a Blender job renders 1 to ${MAX_SURFACES} surfaces`);
    const area = this.area(req.snapshot.handle, session, port);
    if (this.area(req.output, session, port) !== area) throw new ProtocolError('capabilityRejected', 'Blender output goes to the Blender Surfaces area.');
    const snapshotPath = path.join(area, 'snapshots', `${hex(req.snapshot.hash)}.json`);
    const info = await stat(snapshotPath).catch(() => null);
    if (!info || info.size !== req.snapshot.bytes || info.size > MAX_SNAPSHOT_BYTES) throw new ProtocolError('invalidMessage', 'The Blender snapshot is missing or does not match its size.');
    const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'));
    // Assets: media files, or converted images in the area.
    const assets: { id: string; file: string }[] = [];
    let inputBytes = 0;
    for (const a of req.assets) {
      const r = this.options.handles.resolve(a.handle, session, port);
      let file: string | null = null;
      if (r.kind.kind === 'mediaRead' && r.kind.asset === a.asset) file = r.path;
      else if (r.kind.kind === 'cache' && r.kind.cache === 'blenderSurfaces') {
        const prefix = hex(a.hash) + '.';
        const name = (await readdir(path.join(r.path, 'assets')).catch(() => [] as string[])).find((n) => n.startsWith(prefix));
        if (name) file = path.join(r.path, 'assets', name);
      }
      if (!file) throw new ProtocolError('blenderFailed', `Missing Blender asset ${a.asset}`);
      if (!ASSET_EXTENSIONS.has(path.extname(file).toLowerCase())) throw new ProtocolError('blenderFailed', 'Unsupported Blender asset format');
      inputBytes += (await stat(file)).size;
      if (inputBytes > MAX_INPUT_BYTES) throw new ProtocolError('blenderFailed', 'Blender inputs exceed 256 MB');
      assets.push({ id: a.asset, file });
    }
    // Today's snapshot rules (sizes, cameras, lights, geometry, materials).
    let checked: any;
    try {
      checked = validateBlenderJob({ id: 'render_check', operation: 'render', snapshot, assets: assets.map((a) => ({ id: a.id, name: path.basename(a.file), data: new Uint8Array(0) })) } as never);
    } catch (error) {
      throw new ProtocolError('invalidMessage', error instanceof Error ? error.message : String(error));
    }
    const found = await this.options.locate();
    if (!found.available || !found.path) throw new ProtocolError('blenderMissing', found.error || 'This composition renders with Blender. Install Blender 4.5 or later, or set its renderer to Native.');
    if (!this.worker || this.worker.executable !== found.path) {
      await this.worker?.dispose();
      await mkdir(this.scratch, { recursive: true, mode: 0o700 });
      this.worker = new PinnedWorker(found.path, this.scratch, this.options.timeoutMs);
    }
    this.worker.setPaused(this.paused);
    const worker = this.worker;
    const layers = req.surfaces.map((s) => s.layer);
    const groups = req.batchLayers ? [layers] : layers.map((l) => [l]);
    let completed = 0;
    for (const [g, outputs] of groups.entries()) {
      if (entry.cancelled) throw new ProtocolError('cancelled', 'Blender request cancelled');
      const dir = await worker.requestDir();
      try {
        const files: Record<string, string> = {};
        for (const a of assets) {
          const target = path.join(dir, a.id + path.extname(a.file).toLowerCase());
          await copyFile(a.file, target);
          files[a.id] = target;
        }
        const id = `render_${g}`;
        const requestPath = path.join(dir, 'request.json');
        await writeFile(requestPath, JSON.stringify({ id, operation: 'render', snapshot: checked.snapshot, assets: files, dir, outputs }), { mode: 0o600 });
        try {
          await worker.run(id, requestPath);
        } catch (error) {
          if (entry.cancelled) throw new ProtocolError('cancelled', 'Blender request cancelled');
          throw error;
        }
        const manifest = JSON.parse(await readFile(path.join(dir, 'manifest.json'), 'utf8'));
        const device = resolvedDevice(manifest);
        if (!device) throw new ProtocolError('blenderFailed', 'Blender did not report its device');
        for (const file of manifest.files || []) {
          if (!safeId.test(file.id) || typeof file.name !== 'string' || path.basename(file.name) !== file.name) throw new ProtocolError('blenderFailed', 'Invalid Blender output');
          const source = path.join(dir, file.name);
          const size = (await stat(source)).size;
          if (size > MAX_OUTPUT_BYTES_PER_SURFACE) throw new ProtocolError('blenderFailed', 'A Blender surface exceeds the per-surface output cap');
          const { width, height } = await pngSize(source);
          for (const [index, s] of req.surfaces.entries()) {
            if (s.layer !== file.id) continue;
            const target = path.join(area, `${hex(s.key)}.png`);
            await copyFile(source, target);
            send({ type: 'blenderSurface', job: req.job, requestIndex: index, key: s.key, size: { width, height }, crop: { x: 0, y: 0, width, height }, handle: req.output, bytes: size, device });
            completed++;
            send({ type: 'blenderProgress', job: req.job, completed, total: req.surfaces.length });
          }
        }
      } finally {
        await rm(dir, { recursive: true, force: true }).catch(() => {});
      }
    }
    if (completed !== req.surfaces.length) throw new ProtocolError('blenderFailed', `Blender rendered ${completed} of ${req.surfaces.length} surfaces`);
  }
}
