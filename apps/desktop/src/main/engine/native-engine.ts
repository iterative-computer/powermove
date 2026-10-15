/*
 * Host services in main for the native engine (plan section 19; contracts
 * 7, 8, 12, 13, 14).
 *
 * Two engine utility processes, the interactive engine and the job engine,
 * each supervised and restarted on a crash. Main is their host: it grants
 * memory leases from one global budget, counts hardware codec sessions,
 * mints capability handles bound to a session and an engine launch,
 * creates output destinations and commits outputs, relays the interactive
 * engine's deadline signals to the job engine (and shrinks the job
 * engine's lease while playing), and keeps the definition snapshots main
 * needs for sessions it opens.
 *
 * Every client (a renderer window) gets its own `MessageChannelMain` per
 * engine, and the two engines get a direct port for per-frame signals.
 * When an engine restarts, every client gets new ports with a new
 * generation and re-sends its document (resync).
 */
import { randomBytes } from 'node:crypto';
import { mkdir, open, rm, unlink } from 'node:fs/promises';
import path from 'node:path';

import type {
  ContentHash,
  CrashReport,
  DeadlineSignal,
  EngineProcess,
  Event,
  InFlight,
  JobStatus,
  Request,
  Response,
  SlicingPolicy
} from '../../shared/engine-protocol';
import type { HardwareArbiter, LeaseGrant, MemoryArbiter } from './arbiter';
import type { HostHandler, Log } from './connection';
import { CrashLedger, isAttributable } from './crash';
import { DefinitionRelay } from './definitions';
import type { HandleTable } from './handles';
import type { OutputDestinations } from './outputs';
import type { ChannelFactory, ClientPortInfo, PortLike, SpawnEngine } from './ports';
import { isRecord, isSafeUint, ProtocolError } from './protocol';
import { CACHE_DIRS, HostServicesCore } from './services';
import { EngineSupervisor, type Launch, type SupervisorState } from './supervisor';

export const DEFAULT_SLICING_POLICY: SlicingPolicy = {
  maxCommandBufferUs: 2000,
  targetCommandBufferUs: 1000,
  maxInFlight: 1,
  alignGuardUs: 1000,
  yieldAfterMissUs: 100_000,
  blender: { kind: 'pause' }
};

/** Staged resync documents kept per engine launch before the oldest goes. */
const STAGED_PER_LAUNCH = 16;

export type NativeEngineOptions = {
  spawn: SpawnEngine;
  channel: ChannelFactory;
  /** Engine caches (one directory per cache kind). */
  cacheDirectory: string;
  /** Where agent outputs go (today `~/Powermove`). */
  automaticOutputDirectory: string;
  physicalMemory: number;
  log: Log;
  /** The shared host clock in µs (contract 13). */
  hostNow: () => number;
  /** Resolves an asset to the file the engine may read (the media store
   *  lane provides it); `null` when the asset is offline. */
  resolveAsset?: (asset: string) => Promise<{ path: string; contentHash: ContentHash | null } | null>;
  now?: () => number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

export type EngineProcessStatus = { state: SupervisorState; generation: number; pid: number | null; attempts: number };
export type NativeEngineStatus = Record<EngineProcess, EngineProcessStatus>;

export type ClientDeliver = (info: ClientPortInfo, port: PortLike) => void;

type Client = {
  deliver: ClientDeliver;
  ports: Map<EngineProcess, { port: number; generation: number }>;
  kind: ClientKind;
};

/** What a client's ports lead to in the engine: protocol ports for a
 *  renderer window, or main's own font port to the interactive engine's
 *  font service (engine-fonts.ts; not a protocol peer). */
export type ClientKind = { peer: 'renderer' | 'fonts'; processes: readonly EngineProcess[] };

export const RENDERER_CLIENT: ClientKind = { peer: 'renderer', processes: ['interactive', 'job'] };

export interface NativeEngineListener {
  crash?(report: CrashReport): void;
  status?(status: NativeEngineStatus): void;
  /** Playback started or stopped (Blender supervision pauses its workers). */
  playback?(playing: boolean): void;
  jobStatus?(status: JobStatus): void;
}

function validInFlight(value: unknown): value is InFlight {
  return isRecord(value) && Array.isArray(value['suspects']) && value['suspects'].length <= 256 &&
    value['suspects'].every((s: unknown) => isRecord(s) && typeof s['kind'] === 'string' && isAttributable(s as never));
}

export class NativeEngine {
  readonly services: HostServicesCore;
  readonly definitions = new DefinitionRelay();
  readonly ledger = new CrashLedger();
  private readonly supervisors: Record<EngineProcess, EngineSupervisor>;
  private nextPort = 1;
  private clients = new Map<number, Client>();
  private peer: { interactive: number; job: number; generations: [number, number] } | null = null;
  private listeners = new Set<NativeEngineListener>();
  private lastPlaybackStarted: DeadlineSignal | null = null;
  private staged = new Map<number, string[]>();
  /** Set by `stop()`: no control messages go to engines that are shutting
   *  down (they would be dropped anyway; see `Launch.exiting`). */
  private stopping = false;

  get handles(): HandleTable {
    return this.services.handles;
  }

  get outputs(): OutputDestinations {
    return this.services.outputs;
  }

  get memory(): MemoryArbiter {
    return this.services.memory;
  }

  get hardware(): HardwareArbiter {
    return this.services.hardware;
  }

  constructor(private readonly options: NativeEngineOptions) {
    this.services = new HostServicesCore({
      cacheDirectory: options.cacheDirectory,
      automaticOutputDirectory: options.automaticOutputDirectory,
      physicalMemory: options.physicalMemory,
      ...(options.resolveAsset ? { resolveAsset: options.resolveAsset } : {}),
      ...(options.now ? { now: options.now } : {})
    });
    const make = (process: EngineProcess): EngineSupervisor => {
      const supervisor = new EngineSupervisor({
        process,
        spawn: options.spawn,
        channel: options.channel,
        nextPortId: () => this.nextPort++,
        handler: (launch) => this.handlerFor(process, launch),
        ledger: this.ledger,
        log: options.log,
        hostNow: options.hostNow,
        ...(options.now ? { now: options.now } : {}),
        ...(options.schedule ? { schedule: options.schedule } : {})
      });
      supervisor.listen({
        ready: (launch) => this.onReady(launch),
        ended: (launch, report) => this.onEnded(launch, report),
        state: () => this.emitStatus()
      });
      return supervisor;
    };
    this.supervisors = { interactive: make('interactive'), job: make('job') };
  }

  listen(listener: NativeEngineListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async start(): Promise<void> {
    this.stopping = false;
    await this.prepareCaches();
    this.supervisors.interactive.start();
    this.supervisors.job.start();
  }

  async stop(): Promise<void> {
    this.stopping = true;
    await Promise.all([this.supervisors.interactive.stop(), this.supervisors.job.stop()]);
  }

  supervisor(process: EngineProcess): EngineSupervisor {
    return this.supervisors[process];
  }

  status(): NativeEngineStatus {
    const one = (s: EngineSupervisor): EngineProcessStatus => ({
      state: s.state,
      generation: s.current?.generation ?? 0,
      pid: s.current?.pid ?? null,
      attempts: s.attempts
    });
    return { interactive: one(this.supervisors.interactive), job: one(this.supervisors.job) };
  }

  private emitStatus(): void {
    const status = this.status();
    for (const l of this.listeners) l.status?.(status);
  }

  /* ── clients (renderer windows) ─────────────────────────────────────── */

  /** Connects a client to every running engine now and after every restart. */
  addClient(id: number, deliver: ClientDeliver, kind: ClientKind = RENDERER_CLIENT): void {
    this.removeClient(id);
    const client: Client = { deliver, ports: new Map(), kind };
    this.clients.set(id, client);
    for (const process of kind.processes) {
      const launch = this.supervisors[process].current;
      if (launch) this.connectClient(client, launch);
    }
  }

  /** The client's page reloaded: its ports died with it; give it new ones. */
  reconnectClient(id: number): void {
    const client = this.clients.get(id);
    if (client) this.addClient(id, client.deliver, client.kind);
  }

  removeClient(id: number): void {
    const client = this.clients.get(id);
    if (!client) return;
    this.clients.delete(id);
    if (!this.stopping) {
      for (const [process, { port, generation }] of client.ports) {
        const launch = this.supervisors[process].current;
        if (launch?.generation === generation) launch.control({ type: 'disconnect', port });
      }
    }
    this.definitions.forget(id);
  }

  private connectClient(client: Client, launch: Launch): void {
    if (launch.exiting) return;
    const previous = client.ports.get(launch.process);
    if (previous && previous.generation === launch.generation) launch.control({ type: 'disconnect', port: previous.port });
    const { port1, port2 } = this.options.channel();
    const port = this.nextPort++;
    launch.control({ type: 'connect', port, peer: client.kind.peer }, [port1]);
    client.ports.set(launch.process, { port, generation: launch.generation });
    client.deliver({ process: launch.process, port, generation: launch.generation }, port2);
  }

  /** The client (window) holding `port` to an engine's current launch, if any
   *  (the frame bridge sends a viewer's surfaces to that window). */
  clientForPort(process: EngineProcess, port: number): number | null {
    const generation = this.supervisors[process].current?.generation;
    for (const [id, client] of this.clients) {
      const held = client.ports.get(process);
      if (held && held.port === port && held.generation === generation) return id;
    }
    return null;
  }

  /** The port a client holds to an engine (for handle binding by clients). */
  clientPort(id: number, process: EngineProcess): { port: number; generation: number } | null {
    return this.clients.get(id)?.ports.get(process) ?? null;
  }

  /* ── services a client asks main for ────────────────────────────────── */

  /** Stages a large resync document as a scratch cache file and mints a
   *  handle the engine can resolve for `session` (contract 9: bulk data
   *  travels through cache files). */
  async stageDocument(process: EngineProcess, session: number, bytes: Uint8Array): Promise<{ handle: string; length: number }> {
    const launch = this.supervisors[process].current;
    if (!launch) throw new Error(`the ${process} engine is not running`);
    if (!isSafeUint(session)) throw new Error('a staged document names a session');
    const directory = path.join(this.options.cacheDirectory, CACHE_DIRS.scratch);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    const file = path.join(directory, `resync-${randomBytes(12).toString('hex')}.json`);
    const handleFile = await open(file, 'wx', 0o600);
    try {
      await handleFile.writeFile(bytes);
    } finally {
      await handleFile.close();
    }
    const list = this.staged.get(launch.hostPort) ?? [];
    list.push(file);
    while (list.length > STAGED_PER_LAUNCH) await unlink(list.shift()!).catch(() => {});
    this.staged.set(launch.hostPort, list);
    const handle = this.handles.mint({ kind: { kind: 'cache', cache: 'scratch' }, session, port: launch.hostPort, path: file });
    return { handle, length: bytes.byteLength };
  }

  /** A window's kernel pushed definitions (kept for sessions main opens). */
  updateDefinitions(clientId: number, update: unknown): void {
    this.definitions.update(clientId, update);
  }

  /** A request main itself sends to an engine (sessions main opens for
   *  exports and agent snapshots; tests). `revision` is the envelope
   *  revision (an agent frame request's read-your-writes revision). */
  request(process: EngineProcess, body: Request, session: number | null = null, revision: number | null = null): Promise<Response> {
    return this.supervisors[process].request(body, session, revision);
  }

  /** A destination the user chose for a job, as a write handle bound to the
   *  job engine session that renders it and the current launch's host port
   *  (contract 14: the engine never sees the path). */
  chooseOutput(file: string, job: string, session: number): string {
    const launch = this.supervisors.job.current;
    if (!launch) throw new Error('the job engine is not running');
    return this.outputs.chooseDestination(file, job, session, launch.hostPort);
  }

  /** Whether an engine's current launch is connected and answering. */
  isReady(process: EngineProcess): boolean {
    return this.supervisors[process].state === 'ready';
  }

  /* ── engine launches ────────────────────────────────────────────────── */

  private async prepareCaches(): Promise<void> {
    // Scratch holds staged documents of the last run; nothing in it survives a launch.
    await rm(path.join(this.options.cacheDirectory, CACHE_DIRS.scratch), { recursive: true, force: true });
    await mkdir(this.options.cacheDirectory, { recursive: true, mode: 0o700 });
    await this.outputs.sweepAutomaticDirectory();
  }

  private async onReady(launch: Launch): Promise<void> {
    const { process } = launch;
    this.options.log('info', `the ${process} engine is ready (generation ${launch.generation}, pid ${launch.pid ?? '?'})`);
    this.sendGrants(this.memory.initial(process));
    if (process === 'job') {
      launch.connection.event({ type: 'slicing', ...DEFAULT_SLICING_POLICY });
      if (this.memory.isPlaying && this.lastPlaybackStarted) launch.connection.event({ type: 'deadline', ...this.lastPlaybackStarted });
    }
    const failed = this.ledger.failed();
    if (failed.length) {
      launch.connection.event({
        type: 'crash',
        process,
        attempt: this.supervisors[process].attempts,
        exit: 'restarted',
        suspects: [],
        markedFailed: failed,
        restartInMs: 0
      });
    }
    for (const client of this.clients.values()) if (client.kind.processes.includes(launch.process)) this.connectClient(client, launch);
    this.connectPeer();
    this.emitStatus();
  }

  private onEnded(launch: Launch, report: CrashReport | null): void {
    const { process } = launch;
    void this.services.endLaunch(process, launch.hostPort).then((n) => {
      if (n) this.options.log('info', `removed ${n} unfinished output(s) of the ${process} engine`);
    });
    for (const file of this.staged.get(launch.hostPort) ?? []) void unlink(file).catch(() => {});
    this.staged.delete(launch.hostPort);
    const wasPlaying = this.memory.isPlaying;
    this.sendGrants(this.memory.exited(process));
    if (process === 'interactive' && wasPlaying) {
      // Main stops playback on behalf of a dead interactive engine so the
      // job engine never waits on it (contract 13 invariant 9).
      this.supervisors.job.event({ type: 'deadline', signal: 'playbackStopped', at: this.options.hostNow() });
      this.lastPlaybackStarted = null;
      for (const l of this.listeners) l.playback?.(false);
    }
    if (this.peer) {
      // While stopping, the survivor is shutting down too: leave it alone.
      const survivor: EngineProcess = process === 'interactive' ? 'job' : 'interactive';
      if (!this.stopping) this.supervisors[survivor].current?.control({ type: 'disconnect', port: this.peer[survivor] });
      this.peer = null;
    }
    for (const client of this.clients.values()) {
      if (client.ports.get(process)?.generation === launch.generation) client.ports.delete(process);
    }
    if (report) for (const l of this.listeners) l.crash?.(report);
  }

  /** The direct interactive → job port for per-frame signals (amendment
   *  12). Replaced whenever either engine restarts. */
  private connectPeer(): void {
    const interactive = this.supervisors.interactive.current;
    const job = this.supervisors.job.current;
    if (!interactive || !job || interactive.exiting || job.exiting || this.stopping) return;
    if (this.peer && this.peer.generations[0] === interactive.generation && this.peer.generations[1] === job.generation) return;
    if (this.peer) {
      interactive.control({ type: 'disconnect', port: this.peer.interactive });
      job.control({ type: 'disconnect', port: this.peer.job });
    }
    const { port1, port2 } = this.options.channel();
    const a = this.nextPort++;
    const b = this.nextPort++;
    // The job engine first: it must be listening when the interactive
    // engine's hello arrives.
    job.control({ type: 'connect', port: b, peer: 'peer' }, [port2]);
    interactive.control({ type: 'connect', port: a, peer: 'peer' }, [port1]);
    this.peer = { interactive: a, job: b, generations: [interactive.generation, job.generation] };
  }

  private sendGrants(grants: LeaseGrant[]): void {
    for (const grant of grants) {
      const supervisor = this.supervisors[grant.process];
      if (!supervisor.current) continue;
      supervisor.request({ type: 'grantLease', ...grant.lease }).then(
        (response) => {
          if (response.type !== 'leaseUsage') return;
          try {
            this.sendGrants(this.memory.report(response));
          } catch (error) {
            this.options.log('error', `lease answer from the ${grant.process} engine: ${error instanceof Error ? error.message : String(error)}`);
          }
        },
        (error: unknown) => this.options.log('warn', `lease grant to the ${grant.process} engine failed: ${error instanceof Error ? error.message : String(error)}`)
      );
    }
  }

  /* ── engine → host ──────────────────────────────────────────────────── */

  private handlerFor(process: EngineProcess, launch: () => Launch | null): HostHandler {
    return {
      request: (body, session) => {
        const current = launch();
        if (!current) return Promise.reject(new ProtocolError('internal', 'the engine launch ended'));
        return this.services.serve(process, current.hostPort, body, session);
      },
      event: (body, session) => this.onEvent(process, body, session)
    };
  }

  private onEvent(process: EngineProcess, body: Event, _session: number | null): void {
    switch (body.type) {
      case 'inFlight':
        if (!validInFlight(body)) throw new Error('a breadcrumb lists at most 256 attributable items');
        this.supervisors[process].recordInFlight(body);
        return;
      case 'leaseUsage':
        this.sendGrants(this.memory.report(body));
        return;
      case 'deadline':
        if (process !== 'interactive') throw new Error('only the interactive engine sends deadline signals');
        this.onDeadline(body);
        return;
      case 'jobStatus':
        if (process !== 'job') throw new Error('only the job engine reports jobs');
        for (const l of this.listeners) l.jobStatus?.(body);
        return;
      default:
        this.options.log('warn', `the ${process} engine sent ${body.type} to the host; ignored`);
    }
  }

  private onDeadline(signal: DeadlineSignal & { type?: string }): void {
    const { type: _type, ...rest } = signal;
    const plain = rest as DeadlineSignal;
    // Relay first: the job engine reacts to the miss before main does any bookkeeping.
    this.supervisors.job.event({ type: 'deadline', ...plain });
    if (plain.signal === 'playbackStarted') {
      this.lastPlaybackStarted = plain;
      this.sendGrants(this.memory.playbackStarted());
      for (const l of this.listeners) l.playback?.(true);
    } else if (plain.signal === 'playbackStopped') {
      this.lastPlaybackStarted = null;
      this.sendGrants(this.memory.playbackStopped());
      for (const l of this.listeners) l.playback?.(false);
    }
  }
}
