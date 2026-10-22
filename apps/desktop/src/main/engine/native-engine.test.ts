// NativeEngine (host services, supervision, client ports) against fake
// engine processes that speak the protocol through main's own codec. The
// real engines run in scripts/engine-host-integration.mjs.
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { PROTOCOL_BUILD, type CrashReport, type EngineProcess, type Envelope, type Event, type Request, type Response } from '../../shared/engine-protocol';
import { NativeEngine, PROBE_SUPERSEDED } from './native-engine';
import type { ChildLike, ClientPortInfo, EngineControl, PortLike } from './ports';
import { decodeEnvelope, encodeEnvelope, envelope, eventMessage, localRanges, requestMessage, responseMessage } from './protocol';

/* ── in-memory MessageChannelMain ───────────────────────────────────── */

class FakePort implements PortLike {
  other: FakePort | null = null;
  closed = false;
  private started = false;
  private queue: Array<{ data: unknown; ports: PortLike[] }> = [];
  private onMessage: Array<(e: { data: unknown; ports: PortLike[] }) => void> = [];
  private onClose: Array<() => void> = [];
  postMessage(message: unknown, transfer: PortLike[] = []): void {
    const other = this.other;
    if (this.closed || !other || other.closed) {
      // Like MessageChannelMain: dropped; transferred ports die with it.
      for (const port of transfer) port.close();
      return;
    }
    other.queue.push({ data: message, ports: transfer });
    queueMicrotask(() => other.flush());
  }
  on(event: 'message' | 'close', listener: never): this {
    if (event === 'message') this.onMessage.push(listener);
    else this.onClose.push(listener);
    return this;
  }
  start(): void {
    this.started = true;
    queueMicrotask(() => this.flush());
  }
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const l of this.onClose) l();
    if (this.other && !this.other.closed) this.other.close();
  }
  private flush(): void {
    if (!this.started || this.closed) return;
    while (this.queue.length) {
      const event = this.queue.shift()!;
      for (const l of this.onMessage) l(event);
    }
  }
}

const channel = () => {
  const port1 = new FakePort();
  const port2 = new FakePort();
  port1.other = port2;
  port2.other = port1;
  return { port1, port2 };
};

/* ── a fake engine process ──────────────────────────────────────────── */

class FakeEngine implements ChildLike {
  static nextPid = 100;
  readonly pid = FakeEngine.nextPid++;
  ports = new Map<number, { port: FakePort; peer: string }>();
  received: Envelope[] = [];
  private exitListeners: Array<(code: number) => void> = [];
  private messageListeners: Array<(message: unknown) => void> = [];
  dead = false;
  control: FakePort | null = null;
  /** Posts main made to the utility process itself (only the bootstrap). */
  parentPosts: unknown[] = [];
  /** Control messages that reached the engine. */
  controls: EngineControl[] = [];

  constructor(readonly process: EngineProcess) {}

  on(event: string, listener: never): this {
    if (event === 'exit') this.exitListeners.push(listener);
    if (event === 'message') this.messageListeners.push(listener);
    return this;
  }

  /** A notice from the engine process to main (over its parent port). */
  notify(notice: unknown): void {
    for (const l of this.messageListeners) l(notice);
  }

  postMessage(message: unknown, transfer?: PortLike[]): void {
    // Electron's main process crashes when it posts to a child that is
    // exiting or gone; the fake makes that a test failure.
    if (this.dead) throw new Error(`main posted ${JSON.stringify(message)} to an exited ${this.process} engine`);
    this.parentPosts.push(message);
    if ((message as { type?: string }).type !== 'control') throw new Error('only the bootstrap goes over the parent port');
    this.control = transfer![0] as FakePort;
    this.control.on('message', ((e: { data: unknown; ports: PortLike[] }) => this.onControl(e.data as EngineControl, e.ports)) as never);
    this.control.on('close', (() => this.exit(0)) as never);
    this.control.start();
  }

  private onControl(control: EngineControl, transfer: PortLike[]): void {
    if (this.dead) return;
    this.controls.push(control);
    if (control.type === 'connect') {
      const port = transfer[0] as FakePort;
      this.ports.set(control.port, { port, peer: control.peer });
      port.on('message', ((e: { data: unknown }) => this.receive(control.port, e.data as Uint8Array)) as never);
      port.start();
    } else if (control.type === 'disconnect') {
      this.ports.get(control.port)?.port.close();
      this.ports.delete(control.port);
    } else if (control.type === 'status') {
      if (!this.silent) queueMicrotask(() => this.notify({ type: 'status', id: control.id, json: JSON.stringify({ process: this.process, presenter: { framesDelivered: 3 } }) }));
    } else if (control.type === 'shutdown') {
      this.exit(0);
    }
  }
  /** Leaves status requests unanswered (a hung engine). */
  silent = false;

  kill(): boolean {
    this.exit(137);
    return true;
  }

  exit(code: number): void {
    if (this.dead) return;
    this.dead = true;
    for (const { port } of this.ports.values()) port.close();
    this.control?.close();
    queueMicrotask(() => { for (const l of this.exitListeners) l(code); });
  }

  hostPort(): FakePort {
    return [...this.ports.values()].find((p) => p.peer === 'host')!.port;
  }

  private send(port: number, value: Envelope): void {
    this.ports.get(port)?.port.postMessage(encodeEnvelope(value));
  }

  /** Engine → host request; resolves with the host's answer. */
  hostId = 1000;
  answers = new Map<number, (m: Envelope) => void>();
  ask(body: Request, session: number | null = null): Promise<Envelope> {
    const id = this.hostId++;
    const port = [...this.ports.entries()].find(([, p]) => p.peer === 'host')![0];
    const answer = new Promise<Envelope>((resolve) => this.answers.set(id, resolve));
    this.send(port, envelope(requestMessage(id, body), session));
    return answer;
  }

  tell(body: Event): void {
    const port = [...this.ports.entries()].find(([, p]) => p.peer === 'host')![0];
    this.send(port, envelope(eventMessage(body)));
  }

  private receive(port: number, bytes: Uint8Array): void {
    const message = decodeEnvelope(bytes);
    this.received.push(message);
    const body = message.message;
    if (body.kind === 'hello') {
      this.send(port, envelope({
        kind: 'welcome',
        role: this.process === 'interactive' ? 'interactiveEngine' : 'jobEngine',
        wire: 1,
        build: PROTOCOL_BUILD,
        contracts: localRanges().map((r) => ({ contract: r.contract, version: 0 })),
        maxMessageBytes: 1 << 24
      }));
    } else if (body.kind === 'request' && body.body.type === 'grantLease') {
      const lease = body.body;
      const response: Response = {
        type: 'leaseUsage', process: lease.process, epoch: lease.epoch, total: 0, ack: true,
        categories: { frameCache: 0, renderCache: 0, blenderSurfaces: 0, geometry3d: 0, surfacePool: 0, decoders: 0, pipelines: 0, other: 0 }
      };
      this.send(port, envelope(responseMessage(body.id, response)));
    } else if (body.kind === 'request' && body.body.type === 'probeMedia') {
      // A job engine resolves the handle through the host, then answers.
      const id = body.id;
      const handle = body.body.handle;
      void this.ask({ type: 'resolveHandle', handle, session: 0 }).then((answer) => {
        const resolved = answer.message.kind === 'response' && answer.message.body.type === 'handleResolved' ? answer.message.body : null;
        const facts: Response = {
          type: 'mediaFacts', kind: 'video', facts: [{ label: 'Path', value: resolved?.path ?? 'refused' }],
          settings: [], issues: [], displaySize: [320, 180], duration: 2
        };
        this.send(port, envelope(responseMessage(id, facts)));
      });
    } else if (body.kind === 'response' || body.kind === 'error') {
      this.answers.get(body.id)?.(message);
    }
  }

  events(type: string): Event[] {
    return this.received.flatMap((e) => (e.message.kind === 'event' && e.message.body.type === type ? [e.message.body] : []));
  }
}

const settle = async (rounds = 20): Promise<void> => {
  for (let i = 0; i < rounds; i++) await new Promise((r) => setImmediate(r));
};

describe('NativeEngine with fake engines', () => {
  let dir: string;
  let engines: FakeEngine[];
  let timers: Array<{ fn: () => void; ms: number; cancelled: boolean }>;
  let host: NativeEngine;
  let logs: string[];
  let now: number;

  beforeEach(async () => {
    dir = await mkdtemp(path.join(os.tmpdir(), 'pm-native-'));
    engines = [];
    timers = [];
    logs = [];
    now = 0;
    host = new NativeEngine({
      spawn: (process) => {
        const engine = new FakeEngine(process);
        engines.push(engine);
        return engine;
      },
      channel,
      cacheDirectory: path.join(dir, 'cache'),
      automaticOutputDirectory: path.join(dir, 'Powermove'),
      physicalMemory: 16 * 1024 ** 3,
      log: (level, message) => logs.push(`${level}: ${message}`),
      hostNow: () => 42,
      now: () => now,
      schedule: (fn, ms) => {
        const timer = { fn, ms, cancelled: false };
        timers.push(timer);
        return { cancel: () => { timer.cancelled = true; } };
      }
    });
    await host.start();
    await settle();
  });

  afterEach(async () => {
    await host.stop();
    await rm(dir, { recursive: true, force: true });
  });

  const live = (process: EngineProcess) => engines.filter((e) => e.process === process && !e.dead).at(-1)!;

  it('pauses viewer probes for overlapping exports and resumes only after the last one ends', async () => {
    const signals = () => live('interactive').events('deadline').map((e) => (e as { signal: string }).signal);
    const status = (job: string, state: 'running' | 'done' | 'failed' | 'cancelled', kind: 'userJob' | 'generation' = 'userJob') =>
      live('job').tell({ type: 'jobStatus', job, class: kind, state, progress: 0, path: null, error: null });
    status('generation', 'running', 'generation');
    status('a', 'running');
    status('b', 'running');
    host.legacyExportActivity('legacy', true);
    await settle();
    expect(signals()).toEqual(['exportStarted']);
    status('a', 'done');
    status('b', 'failed');
    await settle();
    expect(signals()).toEqual(['exportStarted']);
    host.legacyExportActivity('legacy', false);
    await settle();
    expect(signals()).toEqual(['exportStarted', 'exportStopped']);
    status('c', 'running');
    await settle();
    live('interactive').exit(139);
    await settle();
    timers.at(-1)!.fn();
    await settle();
    expect(signals()).toEqual(['exportStarted']);
    status('c', 'cancelled');
    await settle();
    expect(signals()).toEqual(['exportStarted', 'exportStopped']);
  });

  it('negotiates both engines, grants leases, sends the slicing policy, and links them', () => {
    expect(host.status().interactive.state).toBe('ready');
    expect(host.status().job.state).toBe('ready');
    for (const engine of engines) {
      const hello = engine.received[0]!.message;
      expect(hello.kind).toBe('hello');
      expect(engine.received.some((e) => e.message.kind === 'request' && e.message.body.type === 'grantLease')).toBe(true);
      expect([...engine.ports.values()].filter((p) => p.peer === 'peer')).toHaveLength(1);
    }
    expect(live('job').events('slicing')).toHaveLength(1);
    expect(live('interactive').events('slicing')).toHaveLength(0);
    expect(host.memory.isAcknowledged('interactive')).toBe(true);
  });

  it('gives each client a port per engine, on the main frame only, with the launch generation', async () => {
    const delivered: ClientPortInfo[] = [];
    host.addClient(1, (info) => delivered.push(info));
    await settle();
    expect(delivered.map((d) => [d.process, d.generation])).toEqual([['interactive', 1], ['job', 1]]);
    expect([...live('job').ports.values()].filter((p) => p.peer === 'renderer')).toHaveLength(1);
    // Each connect names the window, so an engine measures its text with
    // that window's font cache seeds.
    expect(live('interactive').controls.filter((c) => c.type === 'connect' && c.peer === 'renderer')).toEqual([expect.objectContaining({ owner: 1 })]);
    host.reconnectClient(1);
    await settle();
    expect(delivered).toHaveLength(4);
    expect([...live('job').ports.values()].filter((p) => p.peer === 'renderer')).toHaveLength(1);
    host.removeClient(1);
    await settle();
    expect([...live('job').ports.values()].filter((p) => p.peer === 'renderer')).toHaveLength(0);
  });

  it('gives main\'s font client one font port to each interactive launch, and none to the job engine', async () => {
    const delivered: ClientPortInfo[] = [];
    host.addClient(-1, (info) => delivered.push(info), { peer: 'fonts', processes: ['interactive'] });
    await settle();
    expect(delivered.map((d) => [d.process, d.generation])).toEqual([['interactive', 1]]);
    expect([...live('interactive').ports.values()].filter((p) => p.peer === 'fonts')).toHaveLength(1);
    expect([...live('job').ports.values()].filter((p) => p.peer === 'fonts')).toHaveLength(0);
    expect(live('interactive').controls.find((c) => c.type === 'connect' && c.peer === 'fonts')).not.toHaveProperty('owner');
    // A restarted interactive engine gets a new font port; a restarted job engine none.
    live('interactive').exit(139);
    await settle();
    timers.at(-1)!.fn();
    await settle();
    live('job').exit(139);
    await settle();
    timers.at(-1)!.fn();
    await settle();
    expect(delivered.map((d) => [d.process, d.generation])).toEqual([['interactive', 1], ['interactive', 2]]);
    expect([...live('interactive').ports.values()].filter((p) => p.peer === 'fonts')).toHaveLength(1);
    host.removeClient(-1);
    await settle();
    expect([...live('interactive').ports.values()].filter((p) => p.peer === 'fonts')).toHaveLength(0);
  });

  it('posts to a utility process only once, and sends nothing to an engine after shutdown', async () => {
    host.addClient(1, () => {});
    await settle();
    const [interactive, job] = [live('interactive'), live('job')];
    for (const engine of [interactive, job]) expect(engine.parentPosts).toEqual([{ type: 'control' }]);
    // Each engine's exit during stop() must not reach the other, which is
    // shutting down too (a post to an exiting child crashed main).
    const stopped = host.stop();
    host.removeClient(1);
    await stopped;
    for (const engine of [interactive, job]) {
      expect(engine.dead).toBe(true);
      const shutdown = engine.controls.findIndex((c) => c.type === 'shutdown');
      expect(shutdown).toBeGreaterThanOrEqual(0);
      expect(engine.controls.slice(shutdown + 1)).toEqual([]);
    }
    expect(host.supervisor('job').current).toBeNull();
  });

  it('asks an engine for its status report, and answers null when it exits or does not answer', async () => {
    expect(await host.report('interactive')).toEqual({ process: 'interactive', presenter: { framesDelivered: 3 } });
    expect(await host.report('job')).toEqual({ process: 'job', presenter: { framesDelivered: 3 } });
    live('interactive').silent = true;
    const pending = host.report('interactive');
    live('interactive').kill();
    expect(await pending).toBeNull();
  });

  it("keeps the interactive engine's fallback texture bands for the snapshot answer they precede", async () => {
    const key = 'c'.repeat(64);
    const band = { type: 'shaderFallback', fallback: 'input', key, texture: 0, width: 2, height: 1, row: 0, rows: 1, rgba16f: new Uint8Array(16).fill(5) } as unknown as Event;
    live('job').tell(band);
    await settle();
    expect(host.fallbackTextures(key, 1, 2, 1)).toBeNull();
    live('interactive').tell(band);
    await settle();
    expect(host.fallbackTextures(key, 1, 2, 1)).toEqual([new Uint8Array(16).fill(5)]);
    expect(host.fallbackTextures(key, 1, 2, 1)).toBeNull();
  });

  it('drops control messages to a launch that is exiting and closes the ports they carried', async () => {
    const launch = host.supervisor('job').current!;
    launch.connection.close();
    await settle();
    timers.at(-1)!.fn();
    await settle();
    // The host connection ended: the supervisor killed the engine.
    expect(launch.exiting).toBe(true);
    const { port1, port2 } = channel();
    launch.control({ type: 'connect', port: 999, peer: 'renderer' }, [port1]);
    expect(port1.closed && port2.closed).toBe(true);
  });

  it('restarts a crashed engine with backoff, attributes suspects, and reconnects clients for a resync', async () => {
    const delivered: ClientPortInfo[] = [];
    host.addClient(1, (info) => delivered.push(info));
    const reports: CrashReport[] = [];
    host.listen({ crash: (r) => reports.push(r) });
    const suspect = { kind: 'asset' as const, asset: 'clip' };

    for (let attempt = 1; attempt <= 2; attempt++) {
      const victim = live('interactive');
      victim.tell({ type: 'inFlight', suspects: [suspect] });
      await settle();
      victim.exit(139);
      await settle();
      expect(host.status().interactive.state).toBe('restarting');
      const timer = timers.at(-1)!;
      expect(timer.ms).toBe(attempt === 1 ? 250 : 500);
      timer.fn();
      await settle();
      expect(host.status().interactive.state).toBe('ready');
    }
    expect(reports.map((r) => [r.attempt, r.markedFailed.length])).toEqual([[1, 0], [2, 1]]);
    expect(reports[1]!.suspects).toEqual([suspect]);
    // The restarted engine is told which items failed; clients got new ports.
    const crash = live('interactive').events('crash') as Array<Extract<Event, { type: 'crash' }>>;
    expect(crash[0]!.markedFailed).toEqual([suspect]);
    expect(delivered.filter((d) => d.process === 'interactive').map((d) => d.generation)).toEqual([1, 2, 3]);
    // The direct port was rebuilt for the new launch.
    expect([...live('job').ports.values()].filter((p) => p.peer === 'peer')).toHaveLength(1);
    // A healthy minute resets the attempt count.
    now += 61_000;
    live('interactive').exit(139);
    await settle();
    expect(timers.at(-1)!.ms).toBe(250);
  });

  it('serves handles and outputs only on the launch that owns them, and cleans up after a crash', async () => {
    const job = live('job');
    const minted = await job.ask({ type: 'mintHandle', kind: { kind: 'cache', cache: 'renderCache' }, session: 4 });
    expect(minted.message.kind).toBe('response');
    const handle = minted.message.kind === 'response' && minted.message.body.type === 'handle' ? minted.message.body.handle : '';
    const resolved = await job.ask({ type: 'resolveHandle', handle, session: 4 });
    expect(resolved.message.kind === 'response' && resolved.message.body.type === 'handleResolved' && resolved.message.body.path).toBe(path.join(dir, 'cache', 'Render Cache'));
    // The interactive engine cannot use the job engine's handle.
    const stolen = await live('interactive').ask({ type: 'resolveHandle', handle, session: 4 });
    expect(stolen.message.kind === 'error' && stolen.message.error.code).toBe('capabilityRejected');
    const wrongSession = await job.ask({ type: 'resolveHandle', handle, session: 5 });
    expect(wrongSession.message.kind === 'error' && wrongSession.message.error.code).toBe('capabilityRejected');
    // Outputs: only the job engine; agents never pick a path.
    const fromInteractive = await live('interactive').ask({ type: 'outputDestination', job: 'j', choice: { kind: 'agentAutomatic', suggestedName: 'Cut', extension: 'mov' }, overwrite: 'uniqueName' });
    expect(fromInteractive.message.kind === 'error' && fromInteractive.message.error.code).toBe('capabilityRejected');
    const escape = await job.ask({ type: 'outputDestination', job: 'j', choice: { kind: 'agentAutomatic', suggestedName: '../x', extension: 'mov' }, overwrite: 'uniqueName' });
    expect(escape.message.kind === 'error' && escape.message.error.code).toBe('invalidMessage');
    const out = await job.ask({ type: 'outputDestination', job: 'j', choice: { kind: 'agentAutomatic', suggestedName: 'Cut', extension: 'mov' }, overwrite: 'uniqueName' }, 4);
    expect(out.message.kind === 'response' && out.message.body.type).toBe('outputDestination');
    expect(host.outputs.size).toBe(1);
    // Hardware sessions and unsupported Blender requests.
    const hw = await job.ask({ type: 'hardwareSessions', kind: 'proResEncode', delta: 3 });
    expect(hw.message.kind === 'response' && hw.message.body).toEqual({ type: 'hardwareSessions', kind: 'proResEncode', held: 2 });
    const blender = await job.ask({ type: 'blenderStatus' });
    expect(blender.message.kind === 'error' && blender.message.error.code).toBe('unsupported');

    job.exit(139);
    await settle();
    expect(host.handles.size).toBe(0);
    expect(host.outputs.size).toBe(0);
    expect(host.hardware.total('proResEncode')).toBe(0);
  });

  it('relays playback signals, moves leases, and stops playback for a dead interactive engine', async () => {
    const playback: boolean[] = [];
    host.listen({ playback: (p) => playback.push(p) });
    const before = host.memory.current('job')!.bytes;
    live('interactive').tell({ type: 'deadline', signal: 'playbackStarted', at: 1, rate: { num: 1, den: 1 }, frameBudgetUs: 16_666 });
    await settle();
    const relayed = live('job').events('deadline');
    expect(relayed).toHaveLength(1);
    expect(host.memory.current('job')!.bytes).toBeLessThan(before);
    expect(host.memory.current('job')!.reason).toBe('playbackStarted');
    // A job engine that restarts during playback hears about it again.
    live('job').exit(139);
    await settle();
    timers.at(-1)!.fn();
    await settle();
    expect(live('job').events('deadline').map((e) => (e as { signal: string }).signal)).toEqual(['playbackStarted']);
    live('interactive').exit(139);
    await settle();
    expect(live('job').events('deadline').map((e) => (e as { signal: string }).signal)).toEqual(['playbackStarted', 'playbackStopped']);
    expect(playback).toEqual([true, false]);
    // The job engine may not send deadline signals.
    live('job').tell({ type: 'deadline', signal: 'playbackStopped', at: 3 });
    await settle();
    expect(logs.some((l) => l.includes('only the interactive engine'))).toBe(true);
  });

  it('slices job engine work while an editor plays, merged with the interactive engine\'s playback', async () => {
    const playback: boolean[] = [];
    host.listen({ playback: (p) => playback.push(p) });
    const signals = () => live('job').events('deadline').map((e) => {
      const d = e as { signal: string; frameBudgetUs?: number };
      return d.signal === 'playbackStarted' ? `started ${d.frameBudgetUs}` : d.signal;
    });
    // Two editors: the job engine hears a start, then the shorter budget of
    // the two once both play, and one stop when the last stops.
    host.editorPlayback(5, true, 16_667);
    host.editorPlayback(6, true, 8_333);
    await settle();
    expect(signals()).toEqual(['started 16667', 'started 8333']);
    expect(host.memory.current('job')!.reason).toBe('playbackStarted');
    host.editorPlayback(5, false);
    await settle();
    expect(signals()).toEqual(['started 16667', 'started 8333']);
    // The interactive engine starts and stops while editor 6 plays: its own
    // signal is relayed, and its stop hands back to the editor's budget.
    live('interactive').tell({ type: 'deadline', signal: 'playbackStarted', at: 1, rate: { num: 1, den: 1 }, frameBudgetUs: 16_666 });
    await settle();
    live('interactive').tell({ type: 'deadline', signal: 'playbackStopped', at: 2 });
    await settle();
    expect(signals()).toEqual(['started 16667', 'started 8333', 'started 16666', 'started 8333']);
    host.editorPlayback(6, false);
    await settle();
    expect(signals()).toEqual(['started 16667', 'started 8333', 'started 16666', 'started 8333', 'playbackStopped']);
    expect(playback).toEqual([true, false]);
    expect(host.memory.isPlaying).toBe(false);
    // An editor stopping twice, or a window that never played, changes nothing.
    host.editorPlayback(6, false);
    host.editorPlayback(9, false);
    await settle();
    expect(signals()).toHaveLength(5);
    // A dead interactive engine does not stop an editor's playback.
    host.editorPlayback(7, true, 1);
    live('interactive').tell({ type: 'deadline', signal: 'playbackStarted', at: 3, rate: { num: 1, den: 1 }, frameBudgetUs: 16_666 });
    await settle();
    live('interactive').exit(139);
    await settle();
    expect(signals().slice(5)).toEqual(['started 2000', 'started 16666', 'started 2000']);
    expect(host.memory.isPlaying).toBe(true);
    host.editorPlayback(7, false);
    await settle();
    expect(signals().at(-1)).toBe('playbackStopped');
    expect(playback).toEqual([true, false, true, false]);
  });

  it('stages resync documents as cache files bound to the session and launch', async () => {
    const staged = await host.stageDocument('interactive', 3, new TextEncoder().encode('{"a":1}'));
    expect(staged.length).toBe(7);
    const engine = live('interactive');
    const ok = await engine.ask({ type: 'resolveHandle', handle: staged.handle, session: 3 });
    expect(ok.message.kind === 'response' && ok.message.body.type === 'handleResolved' && ok.message.body.access).toBe('readWrite');
    const otherEngine = await live('job').ask({ type: 'resolveHandle', handle: staged.handle, session: 3 });
    expect(otherEngine.message.kind).toBe('error');
  });

  it('probes media through a mediaRead handle bound to the job engine and session 0, revoked after', async () => {
    const file = path.join(dir, 'clip.mov');
    const probing = new NativeEngine({
      spawn: (process) => {
        const engine = new FakeEngine(process);
        engines.push(engine);
        return engine;
      },
      channel,
      cacheDirectory: path.join(dir, 'cache-probe'),
      automaticOutputDirectory: path.join(dir, 'Powermove'),
      physicalMemory: 16 * 1024 ** 3,
      log: () => {},
      hostNow: () => 42,
      resolveAsset: async (asset) => (asset === 'clip' ? { path: file, contentHash: null } : null),
      schedule: (fn, ms) => {
        const timer = { fn, ms, cancelled: false };
        timers.push(timer);
        return { cancel: () => { timer.cancelled = true; } };
      }
    });
    await probing.start();
    await settle();
    try {
      const facts = await probing.probeMedia('clip', '{"range":"full"}');
      expect(facts.facts[0]).toEqual({ label: 'Path', value: file });
      expect(facts.displaySize).toEqual([320, 180]);
      const job = engines.filter((e) => e.process === 'job' && !e.dead).at(-1)!;
      const probe = job.received.find((m) => m.message.kind === 'request' && m.message.body.type === 'probeMedia');
      const body = probe?.message.kind === 'request' && probe.message.body.type === 'probeMedia' ? probe.message.body : null;
      expect(body?.interpret).toBe('{"range":"full"}');
      // The handle is gone once the answer arrived.
      const again = await job.ask({ type: 'resolveHandle', handle: body!.handle, session: 0 });
      expect(again.message.kind).toBe('error');
      await expect(probing.probeMedia('missing', null)).rejects.toThrow('not available');
      // One probe per asset in flight: a request waiting behind it is
      // superseded by a newer one and never reaches the engine.
      const sent = () => job.received.filter((m) => m.message.kind === 'request' && m.message.body.type === 'probeMedia').length;
      const before = sent();
      const first = probing.probeMedia('clip', '{"range":"limited"}');
      const second = probing.probeMedia('clip', '{"range":"full"}');
      const third = probing.probeMedia('clip', null);
      await expect(second).rejects.toThrow(PROBE_SUPERSEDED);
      await expect(first).resolves.toMatchObject({ displaySize: [320, 180] });
      await expect(third).resolves.toMatchObject({ displaySize: [320, 180] });
      const bodies = job.received
        .slice()
        .filter((m) => m.message.kind === 'request' && m.message.body.type === 'probeMedia')
        .slice(before)
        .map((m) => (m.message.kind === 'request' && m.message.body.type === 'probeMedia' ? m.message.body.interpret : undefined));
      expect(sent() - before).toBe(2);
      expect(bodies).toEqual(['{"range":"limited"}', null]);
      // The slot is free again: the next probe goes straight out.
      await expect(probing.probeMedia('clip', '{"range":"full"}')).resolves.toMatchObject({ duration: 2 });
      expect(sent() - before).toBe(3);
    } finally {
      await probing.stop();
    }
  });

  it('does not restart an engine that cannot load', async () => {
    const reports: CrashReport[] = [];
    host.listen({ crash: (r) => reports.push(r) });
    const engine = live('job');
    engine.notify({ type: 'fatal', message: 'could not load the engine: image not found' });
    engine.exit(70);
    await settle();
    expect(host.status().job.state).toBe('stopped');
    expect(timers.filter((t) => !t.cancelled)).toHaveLength(0);
    expect(reports[0]).toMatchObject({ process: 'job', restartInMs: 0 });
    expect(reports[0]!.exit).toContain('image not found');
  });

  it('replaces an engine whose host connection ends while it keeps running', async () => {
    const engine = live('interactive');
    engine.hostPort().close();
    await settle();
    // It gets a moment to exit on its own (a dying engine closes its ports
    // first), then main kills it.
    expect(engine.dead).toBe(false);
    timers.at(-1)!.fn();
    await settle();
    expect(engine.dead).toBe(true);
    expect(host.status().interactive.state).toBe('restarting');
    timers.at(-1)!.fn();
    await settle();
    expect(host.status().interactive.state).toBe('ready');
    expect(host.status().interactive.generation).toBe(2);
  });

  it('starts again after a stop, and the second stop waits for the exits even when one engine crashes', async () => {
    await host.stop();
    await host.start();
    await settle();
    expect(host.status().interactive.state).toBe('ready');
    const [interactive, job] = [live('interactive'), live('job')];
    interactive.kill();
    await host.stop();
    expect(interactive.dead && job.dead).toBe(true);
    expect(host.status().interactive.state).toBe('stopped');
    expect(host.status().job.state).toBe('stopped');
    expect(timers.filter((t) => !t.cancelled)).toHaveLength(0);
  });

  it('stops without restarting', async () => {
    await host.stop();
    expect(host.status().interactive.state).toBe('stopped');
    expect(timers.filter((t) => !t.cancelled)).toHaveLength(0);
  });
});
