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
  MediaFacts,
  Request,
  Response,
  ShaderFallbackRequest,
  SlicingPolicy
} from '../../shared/engine-protocol';
import type { HardwareArbiter, LeaseGrant, MemoryArbiter } from './arbiter';
import type { HostHandler, Log } from './connection';
import { CrashLedger, isAttributable } from './crash';
import { DefinitionRelay } from './definitions';
import { FallbackInputs } from './fallback-inputs';
import { HandleRejected, type HandleTable } from './handles';
import type { OutputDestinations } from './outputs';
import type { ChannelFactory, ClientPortInfo, PortLike, SpawnEngine } from './ports';
import { isRecord, isSafeUint, ProtocolError } from './protocol';
import { serveHostFallback, type ShaderFallbackResult } from './shader-fallback-relay';
import { CACHE_DIRS, HostServicesCore } from './services';
import { EngineBlender, type BlenderLocator } from './blender';
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
  generatedMediaDirectory?: (context?: { process: EngineProcess; session: number }) => string;
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
  resolveAsset?: (asset: string, context?: { process: EngineProcess; session: number }) => Promise<{ path: string; contentHash: ContentHash | null } | null>;
  /** A generated proxy of an offline asset, for draft renders. */
  resolveProxy?: (asset: string, context?: { process: EngineProcess; session: number }) => Promise<{ path: string; contentHash: ContentHash | null } | null>;
  now?: () => number;
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
  /** Finds the user's Blender (contract 11 Blender jobs from the job
   *  engine's exports and the interactive engine's paused viewer); without
   *  it Blender requests answer `unsupported` and the current engine runs
   *  Blender. */
  blender?: { locate: BlenderLocator; tempRoot: string };
  /** Compatibility pixels drawn by the document's window
   *  (compat-frames.ts): job renders and interactive Blender PNG maps.
   *  Without it they answer `notHeadlessRenderable`. */
  compatFrame?: (request: import('../../shared/engine-protocol').CompatFrameRequest) => Promise<Response>;
  /** Runs a job engine fallback request in a window's runner
   *  (`shader-fallback-relay.ts`); without it such requests answer
   *  `unsupported` and the job fails naming the program. */
  shaderFallback?: (session: number, request: ShaderFallbackRequest, textures: Uint16Array[]) => Promise<ShaderFallbackResult>;
};

export type EngineProcessStatus = { state: SupervisorState; generation: number; pid: number | null; attempts: number };
export type NativeEngineStatus = Record<EngineProcess, EngineProcessStatus>;

export type ClientDeliver = (info: ClientPortInfo, port: PortLike) => void;

type Client = {
  /** The window (its webContents id) or main's own client id. */
  id: number;
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

/** A 60 Hz frame, when the display's refresh rate is unknown. */
export const DEFAULT_FRAME_BUDGET_US = 16_667;

/** Why a waiting Interpret Footage probe was dropped. */
export const PROBE_SUPERSEDED = 'A newer probe of the same asset replaced this one.';

interface ProbeSlot {
  waiting: { interpret: string | null; resolve: (facts: MediaFacts) => void; reject: (cause: unknown) => void } | null;
}

/** Events the engine sends for a session, which main ignores for the
 *  sessions it opened itself. */
const SESSION_EVENTS: ReadonlySet<string> = new Set(['patchesApplied', 'renderStats', 'failure', 'shaderFallback', 'compat', 'audio', 'clock', 'surfacePool', 'frame']);

export class NativeEngine {
  readonly services: HostServicesCore;
  /** Blender supervision for both engines (contract 11, `blender.ts`). */
  readonly blender: EngineBlender | null;
  readonly definitions = new DefinitionRelay();
  /** Texture bands the interactive engine sends ahead of a snapshot's
   *  `needsRenderer` answer (fallback-inputs.ts). */
  readonly fallbackInputs = new FallbackInputs();
  readonly ledger = new CrashLedger();
  private readonly supervisors: Record<EngineProcess, EngineSupervisor>;
  private nextPort = 1;
  private clients = new Map<number, Client>();
  private peer: { interactive: number; job: number; generations: [number, number] } | null = null;
  private listeners = new Set<NativeEngineListener>();
  /** The `playbackStarted` the job engine last heard while something plays
   *  (resent to a job engine that starts during playback). */
  private lastPlaybackStarted: DeadlineSignal | null = null;
  /** Whether the interactive engine plays (its own deadline signals). */
  private interactivePlaying = false;
  /** Editor windows whose transport plays, with their frame budget (µs):
   *  the viewer may draw playback without the interactive engine, and the
   *  job engine slices whenever anything plays (contract 13). */
  private editorsPlaying = new Map<number, number>();
  /** Running non-generation jobs and legacy encoder tokens. A transition of
   * their union pauses or resumes the interactive viewer's idle probes. */
  private exportJobs = new Set<string>();
  private legacyExports = new Set<string>();
  private staged = new Map<number, string[]>();
  /** Set by `stop()`: no control messages go to engines that are shutting
   *  down (they would be dropped anyway; see `Launch.exiting`). */
  private stopping = false;
  /** Interpret Footage probes per asset: the one in flight and the newest
   *  request waiting behind it (the sheet probes on every change and keeps
   *  only its latest answer, so a waiting request a newer one replaces is
   *  superseded and never reaches the engine). */
  private probes = new Map<string, ProbeSlot>();

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
      ...(options.generatedMediaDirectory ? { generatedMediaDirectory: options.generatedMediaDirectory } : {}),
      ...(options.resolveAsset ? { resolveAsset: options.resolveAsset } : {}),
      ...(options.resolveProxy ? { resolveProxy: options.resolveProxy } : {}),
      ...(options.now ? { now: options.now } : {})
    });
    this.blender = options.blender
      ? new EngineBlender({ locate: options.blender.locate, handles: this.services.handles, tempRoot: options.blender.tempRoot, log: options.log })
      : null;
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
    this.exportJobs.clear();
    this.legacyExports.clear();
    await Promise.all([this.supervisors.interactive.stop(), this.supervisors.job.stop(), this.blender?.dispose()]);
  }

  supervisor(process: EngineProcess): EngineSupervisor {
    return this.supervisors[process];
  }

  /** Destroys the interactive engine's viewer GPU device as a real loss
   *  would (the recovery test). False when it is not running. */
  simulateGpuLoss(): boolean {
    const launch = this.supervisors.interactive.current;
    if (!launch || launch.exiting) return false;
    launch.control({ type: 'simulateGpuLoss' });
    return true;
  }

  /** An engine's own status report (sessions, lease, the presenter's
   *  counters, routes, and audio): diagnostics, the e2e suite, and the
   *  measurements. Null when it is not running or does not answer. */
  async report(process: EngineProcess = 'interactive'): Promise<unknown> {
    const launch = this.supervisors[process].current;
    if (!launch || launch.exiting) return null;
    return launch.report();
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
    const client: Client = { id, deliver, ports: new Map(), kind };
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
    launch.control({ type: 'connect', port, peer: client.kind.peer, ...(client.kind.peer === 'renderer' ? { owner: client.id } : {}) }, [port1]);
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
  /** Main's own host port on the current launch of `process`. */
  hostPort(process: EngineProcess): { port: number; generation: number } | null {
    const launch = this.supervisors[process].current;
    return launch && !launch.exiting ? { port: launch.hostPort, generation: launch.generation } : null;
  }

  /** Posts an event to `process` for a session main opened. */
  event(process: EngineProcess, body: Event, session: number | null): void {
    this.supervisors[process].event(body, session);
  }

  /** The textures of a snapshot's fallback request (fallback-inputs.ts). */
  fallbackTextures(key: string, count: number, width: number, height: number): Uint8Array[] | null {
    return this.fallbackInputs.take(key, count, width, height);
  }

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
  chooseOutput(file: string, job: string, session: number, overwrite?: 'uniqueName' | 'replace' | 'fail'): string {
    const launch = this.supervisors.job.current;
    if (!launch) throw new Error('the job engine is not running');
    return this.outputs.chooseDestination(file, job, session, launch.hostPort, overwrite);
  }

  /** Marks a job session as a draft render: offline media resolves to proxies. */
  setDraftSession(session: number, draft: boolean): void {
    this.services.setDraftSession(session, draft);
  }

  /** Interpret Footage's facts of an asset's file. Main resolves the asset
   *  (the media store) and mints a `mediaRead` handle bound to the job
   *  engine's host port and session 0; the job engine resolves it, reads
   *  the file, and answers `mediaFacts`. The handle is revoked after. */
  async probeMedia(asset: string, interpret: string | null): Promise<MediaFacts> {
    const slot = this.probes.get(asset);
    if (slot) {
      slot.waiting?.reject(new Error(PROBE_SUPERSEDED));
      return new Promise<MediaFacts>((resolve, reject) => {
        slot.waiting = { interpret, resolve, reject };
      });
    }
    const mine: ProbeSlot = { waiting: null };
    this.probes.set(asset, mine);
    try {
      return await this.probeOnce(asset, interpret);
    } finally {
      this.nextProbe(asset, mine);
    }
  }

  /** Runs the request waiting on `asset`'s probe, or frees the slot. */
  private nextProbe(asset: string, slot: ProbeSlot): void {
    const next = slot.waiting;
    if (!next) {
      this.probes.delete(asset);
      return;
    }
    slot.waiting = null;
    void this.probeOnce(asset, next.interpret)
      .then(next.resolve, next.reject)
      .finally(() => this.nextProbe(asset, slot));
  }

  private async probeOnce(asset: string, interpret: string | null): Promise<MediaFacts> {
    const resolved = await this.options.resolveAsset?.(asset);
    if (!resolved) throw new Error('The asset’s file is not available.');
    return this.probeSource(resolved, asset, interpret);
  }

  /** A machine-local source assessment already resolved by the owning
   * project's media binding. Never accepts a renderer-supplied path. */
  async probeSource(resolved: { path: string }, asset = 'source-analysis', interpret: string | null = null): Promise<MediaFacts> {
    const launch = this.supervisors.job.current;
    if (!launch) throw new Error('the job engine is not running');
    const handle = this.handles.mint({ kind: { kind: 'mediaRead', asset }, session: 0, port: launch.hostPort, path: resolved.path });
    try {
      const answer = await this.request('job', { type: 'probeMedia', handle, interpret });
      if (answer.type !== 'mediaFacts') throw new Error(`the job engine answered ${answer.type}`);
      const { type: _type, ...facts } = answer;
      return facts as MediaFacts;
    } finally {
      this.handles.revoke(handle);
    }
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
    if (process === 'interactive' && this.exportActive()) {
      launch.connection.event({ type: 'deadline', signal: 'exportStarted', at: this.options.hostNow() });
    }
    if (process === 'job') {
      launch.connection.event({ type: 'slicing', ...DEFAULT_SLICING_POLICY });
      if (this.lastPlaybackStarted) launch.connection.event({ type: 'deadline', ...this.lastPlaybackStarted });
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
    if (process === 'job') {
      const wasActive = this.exportActive();
      this.exportJobs.clear();
      this.syncExportActivity(wasActive);
    }
    this.blender?.endPort(launch.hostPort);
    void this.services.endLaunch(process, launch.hostPort).then((n) => {
      if (n) this.options.log('info', `removed ${n} unfinished output(s) of the ${process} engine`);
    });
    for (const file of this.staged.get(launch.hostPort) ?? []) void unlink(file).catch(() => {});
    this.staged.delete(launch.hostPort);
    const wasPlaying = this.memory.isPlaying;
    this.sendGrants(this.memory.exited(process));
    if (process === 'interactive') {
      // Main stops playback on behalf of a dead interactive engine so the
      // job engine never waits on it (contract 13 invariant 9), unless an
      // editor still plays.
      const interactiveWas = this.interactivePlaying;
      this.interactivePlaying = false;
      if (this.editorsPlaying.size) {
        if (wasPlaying && !this.memory.isPlaying) this.sendGrants(this.memory.playbackStarted());
        if (interactiveWas) this.startPlayback(this.editorSignal());
      } else if (interactiveWas) {
        this.stopPlayback(false);
      }
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
        const blender = this.blender;
        if (blender) {
          if (body.type === 'blenderStatus') return blender.status();
          if (body.type === 'blenderRender') {
            // The job engine's exports, and the interactive engine's paused
            // viewer (Material Preview and Rendered shading). Surfaces and
            // progress go to the launch that asked, before the answer.
            const send = (event: Event): void => {
              if (launch() === current) this.supervisors[process].event(event, session);
            };
            return blender.render(body, session ?? 0, current.hostPort, send).then(() => ({ type: 'ack' }) as Response);
          }
          if (body.type === 'cancelJobs' && body.kind === 'job' && (process === 'job' || blender.owns(body.job, current.hostPort))) {
            return Promise.resolve({ type: 'jobsCancelled', jobs: blender.cancel(body.job) ? [body.job] : [] } as Response);
          }
        }
        if (body.type === 'shaderFallback') {
          if (process !== 'job') return Promise.reject(new ProtocolError('capabilityRejected', 'Only the job engine asks the host to run fallback requests.'));
          return this.runShaderFallback(body, session ?? 0, current.hostPort);
        }
        if (body.type === 'compatFrame') {
          // Paused Blender previews use the interactive engine. Admit only
          // its exact-PNG map source; every other compatibility draw remains
          // an export/job capability. The shared window ask still checks the
          // document owner and original content hash; the request's revision
          // is forwarded unchanged with the frozen source identity.
          if (process !== 'job' && (body.source !== 'blenderPng' || !body.blender)) {
            return Promise.reject(new ProtocolError('capabilityRejected', 'Only the job engine asks for this compatibility frame.'));
          }
          if (!this.options.compatFrame) return Promise.reject(new ProtocolError('notHeadlessRenderable', 'This host draws no compatibility frames.'));
          const { type: _type, ...request } = body;
          return this.options.compatFrame(request);
        }
        return this.services.serve(process, current.hostPort, body, session);
      },
      event: (body, session) => this.onEvent(process, body, session)
    };
  }

  /**
   * A job engine fallback request (contract 14): the textures are read
   * from the session's scratch area, the request runs in a window's runner,
   * and the result is written back as the named output file.
   */
  private async runShaderFallback(body: Extract<Request, { type: 'shaderFallback' }>, session: number, hostPort: number): Promise<Response> {
    const run = this.options.shaderFallback;
    await serveHostFallback(body, (handle) => {
      try {
        return this.services.handles.resolve(handle, session, hostPort);
      } catch (error) {
        if (error instanceof HandleRejected) throw new ProtocolError('capabilityRejected', error.message);
        throw error;
      }
    }, run ? (request, textures) => run(session, request, textures) : null);
    return { type: 'ack' };
  }

  private onEvent(process: EngineProcess, body: Event, session: number | null): void {
    // The textures of a snapshot's fallback requests, ahead of its answer
    // (fallback-inputs.ts): for whichever session the snapshot is of.
    if (process === 'interactive' && body.type === 'shaderFallback' && (body as { fallback?: string }).fallback === 'input') {
      if (!this.fallbackInputs.receive(body)) this.options.log('warn', 'the interactive engine sent a damaged fallback texture band; dropped');
      return;
    }
    // What a session main opened itself reports (window snapshot sessions,
    // window-sessions.ts): main only snapshots them.
    if (session !== null && SESSION_EVENTS.has(body.type)) return;
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
        if (body.class !== 'generation') this.jobExportActivity(body.job, body.state === 'running');
        for (const l of this.listeners) l.jobStatus?.(body);
        return;
      default:
        this.options.log('warn', `the ${process} engine sent ${body.type} to the host; ignored`);
    }
  }

  private onDeadline(signal: DeadlineSignal & { type?: string }): void {
    const { type: _type, ...rest } = signal;
    const plain = rest as DeadlineSignal;
    if (plain.signal === 'playbackStarted') {
      this.interactivePlaying = true;
      // Relayed even when an editor already plays: it carries the
      // interactive engine's own rate and frame budget.
      this.startPlayback(plain);
    } else if (plain.signal === 'playbackStopped') {
      this.interactivePlaying = false;
      // While an editor still plays, the job engine keeps slicing, by the
      // editor's frame budget again.
      if (this.editorsPlaying.size) this.startPlayback(this.editorSignal());
      else this.stopPlayback(true, plain);
    } else {
      // Relay first: the job engine reacts to the miss before main does any bookkeeping.
      this.supervisors.job.event({ type: 'deadline', ...plain });
    }
  }

  private exportActive(): boolean {
    return this.exportJobs.size > 0 || this.legacyExports.size > 0;
  }

  private syncExportActivity(wasActive: boolean): void {
    const active = this.exportActive();
    if (wasActive === active) return;
    this.supervisors.interactive.event({
      type: 'deadline', signal: active ? 'exportStarted' : 'exportStopped', at: this.options.hostNow()
    });
  }

  private jobExportActivity(job: string, active: boolean): void {
    const wasActive = this.exportActive();
    if (active) this.exportJobs.add(job);
    else this.exportJobs.delete(job);
    this.syncExportActivity(wasActive);
  }

  /** The current canvas exporter reports its actual encoder lifetime. */
  legacyExportActivity(token: string, active: boolean): void {
    const wasActive = this.exportActive();
    if (active) this.legacyExports.add(token);
    else this.legacyExports.delete(token);
    this.syncExportActivity(wasActive);
  }

  /** An editor window's transport started or stopped (`frameBudgetUs`: its
   *  display's refresh period). The job engine hears `playbackStarted` when
   *  the first thing starts playing and `playbackStopped` when the last
   *  stops, whether that is an editor or the interactive engine. */
  editorPlayback(owner: number, playing: boolean, frameBudgetUs = DEFAULT_FRAME_BUDGET_US): void {
    const budget = Number.isFinite(frameBudgetUs) ? Math.max(2_000, Math.min(100_000, Math.round(frameBudgetUs))) : DEFAULT_FRAME_BUDGET_US;
    const had = this.editorsPlaying.size > 0;
    if (playing) this.editorsPlaying.set(owner, budget);
    else this.editorsPlaying.delete(owner);
    const has = this.editorsPlaying.size > 0;
    if (this.interactivePlaying) return;
    if (has) {
      // Started, or the shortest budget of the playing editors changed.
      const signal = this.editorSignal();
      const last = this.lastPlaybackStarted as { frameBudgetUs?: number } | null;
      if (!had || last?.frameBudgetUs !== (signal as { frameBudgetUs?: number }).frameBudgetUs) this.startPlayback(signal);
    } else if (had) {
      this.stopPlayback(true);
    }
  }

  /** `playbackStarted` for the playing editors: the shortest frame budget. */
  private editorSignal(): DeadlineSignal {
    return {
      signal: 'playbackStarted',
      at: this.options.hostNow(),
      rate: { num: 1, den: 1 },
      frameBudgetUs: Math.min(...this.editorsPlaying.values())
    } as DeadlineSignal;
  }

  private startPlayback(signal: DeadlineSignal): void {
    const was = this.lastPlaybackStarted !== null;
    this.supervisors.job.event({ type: 'deadline', ...signal });
    this.lastPlaybackStarted = signal;
    if (was) return;
    if (DEFAULT_SLICING_POLICY.blender.kind === 'pause') this.blender?.setPaused(true);
    this.sendGrants(this.memory.playbackStarted());
    for (const l of this.listeners) l.playback?.(true);
  }

  private stopPlayback(grants: boolean, signal?: DeadlineSignal): void {
    if (this.lastPlaybackStarted === null) return;
    this.supervisors.job.event({ type: 'deadline', ...(signal ?? { signal: 'playbackStopped', at: this.options.hostNow() }) });
    this.lastPlaybackStarted = null;
    this.blender?.setPaused(false);
    if (grants) this.sendGrants(this.memory.playbackStopped());
    for (const l of this.listeners) l.playback?.(false);
  }
}
