/*
 * Supervises one engine utility process (plan section 19, "Crash
 * handling"): starts it, gives it its host port, negotiates, and restarts
 * it with exponential backoff when it exits unexpectedly. The crash
 * report (attempt, exit, suspects from the last breadcrumb, items marked
 * failed, restart delay) goes to listeners, which tell the windows and
 * clean up the launch's handles, outputs, sessions, and leases.
 */
import type { CrashReport, EngineProcess, ErrorScope, Event, InFlight, Request, Response } from '../../shared/engine-protocol';
import { HostConnection, type HostHandler, type Log } from './connection';
import { CrashLedger, crashReport, describeExit, RestartPolicy } from './crash';
import type { ChannelFactory, ChildLike, EngineControl, PortLike, SpawnEngine } from './ports';
import { isEngineNotice } from './ports';

export type SupervisorState = 'stopped' | 'starting' | 'ready' | 'restarting';

/** One engine launch, as listeners see it. */
export type Launch = {
  process: EngineProcess;
  generation: number;
  hostPort: number;
  connection: HostConnection;
  readonly pid: number | undefined;
  /** Posts a control message (with transferred ports) to the engine. */
  control(message: EngineControl, transfer?: PortLike[]): void;
  /** What the engine reported when it started (build, pid, clock check). */
  started?: { build: string; pid: number; clockSkewUs: number | null };
};

export type SupervisorOptions = {
  process: EngineProcess;
  spawn: SpawnEngine;
  channel: ChannelFactory;
  /** Allocates port ids shared by everything main connects. */
  nextPortId: () => number;
  /** Serves engine → host requests and events for a launch. */
  handler: (launch: () => Launch | null) => HostHandler;
  ledger: CrashLedger;
  log: Log;
  /** The shared host clock in µs, to check the engine reads the same one. */
  hostNow?: () => number;
  now?: () => number;
  /** Delays restarts (tests pass a fake). */
  schedule?: (fn: () => void, ms: number) => { cancel(): void };
};

export interface SupervisorListener {
  /** The engine is up and negotiated. */
  ready?(launch: Launch): void | Promise<void>;
  /** The launch ended (crash or stop). Called before any restart. */
  ended?(launch: Launch, report: CrashReport | null): void;
  /** State changed. */
  state?(state: SupervisorState, generation: number): void;
}

const defaultSchedule = (fn: () => void, ms: number): { cancel(): void } => {
  const timer = setTimeout(fn, ms);
  return { cancel: () => clearTimeout(timer) };
};

export class EngineSupervisor {
  private child: ChildLike | null = null;
  private launch: Launch | null = null;
  private generation = 0;
  private stateValue: SupervisorState = 'stopped';
  private stopping = false;
  private pendingRestart: { cancel(): void } | null = null;
  private readonly restart: RestartPolicy;
  private lastInFlight: ErrorScope[] = [];
  private listeners = new Set<SupervisorListener>();
  private stopped: Promise<void> | null = null;
  private resolveStopped: (() => void) | null = null;

  constructor(private readonly options: SupervisorOptions) {
    this.restart = new RestartPolicy(options.now);
  }

  get process(): EngineProcess {
    return this.options.process;
  }

  get state(): SupervisorState {
    return this.stateValue;
  }

  get current(): Launch | null {
    return this.stateValue === 'ready' ? this.launch : null;
  }

  get attempts(): number {
    return this.restart.attempts;
  }

  listen(listener: SupervisorListener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** The latest crash breadcrumb from the running launch. */
  recordInFlight(crumb: InFlight): void {
    this.lastInFlight = crumb.suspects;
  }

  private setState(state: SupervisorState): void {
    this.stateValue = state;
    for (const l of this.listeners) l.state?.(state, this.generation);
  }

  start(): void {
    if (this.child || this.stateValue === 'starting') return;
    this.stopping = false;
    this.spawn();
  }

  private spawn(): void {
    this.pendingRestart = null;
    this.generation++;
    const generation = this.generation;
    const process = this.options.process;
    this.lastInFlight = [];
    this.setState(this.generation === 1 ? 'starting' : 'restarting');
    let child: ChildLike;
    try {
      child = this.options.spawn(process);
    } catch (error) {
      this.options.log('error', `could not start the ${process} engine: ${error instanceof Error ? error.message : String(error)}`);
      this.onExit(null, -1);
      return;
    }
    this.child = child;
    const { port1, port2 } = this.options.channel();
    const hostPort = this.options.nextPortId();
    const launchRef = (): Launch | null => (this.launch?.generation === generation ? this.launch : null);
    const connection = new HostConnection(process, hostPort, port2, this.options.handler(launchRef), this.options.log);
    const launch: Launch = {
      process,
      generation,
      hostPort,
      connection,
      // Known once the process has spawned.
      get pid() {
        return child.pid;
      },
      control: (message, transfer) => {
        if (this.child === child) child.postMessage(message, transfer);
      }
    };
    this.launch = launch;
    let fatal: string | null = null;
    child.on('message', (notice: unknown) => {
      if (!isEngineNotice(notice)) return;
      if (notice.type === 'log') this.options.log(notice.level, `[${process} engine] ${notice.message}`);
      else if (notice.type === 'fatal') {
        // The engine cannot run in this installation (the addon is missing
        // or does not load): restarting would only repeat it.
        fatal = notice.message;
        this.options.log('error', `[${process} engine] ${notice.message}`);
      }
      else if (notice.type === 'started') {
        // The engine stamped `clock` just before posting; main reads its
        // own host clock on arrival. A large or negative difference means
        // the two do not share the clock contract 13 depends on.
        const skew = this.options.hostNow ? this.options.hostNow() - notice.clock : null;
        launch.started = { build: notice.build, pid: notice.pid, clockSkewUs: skew };
        this.options.log('info', `the ${process} engine started (pid ${notice.pid}, ${notice.build}${skew === null ? '' : `, clock delivered after ${skew} µs`})`);
      }
    });
    child.on('exit', (code: number) => this.onExit(launch, code, fatal));
    child.postMessage({ type: 'connect', port: hostPort, peer: 'host' } satisfies EngineControl, [port1]);
    connection.open().then(
      async () => {
        if (this.launch !== launch) return;
        this.restart.started();
        this.setState('ready');
        for (const l of this.listeners) {
          try {
            await l.ready?.(launch);
          } catch (error) {
            this.options.log('error', `${process} engine setup failed: ${error instanceof Error ? error.message : String(error)}`);
          }
        }
      },
      (error: unknown) => {
        if (this.launch !== launch) return;
        this.options.log('error', `the ${process} engine handshake failed: ${error instanceof Error ? error.message : String(error)}`);
        // A launch that cannot negotiate is killed; the exit restarts it.
        child.kill();
      }
    );
  }

  private onExit(launch: Launch | null, code: number, fatal: string | null = null): void {
    if (launch && this.launch !== launch) return;
    const process = this.options.process;
    this.child = null;
    this.launch = null;
    launch?.connection.close();
    if (this.stopping) {
      if (launch) for (const l of this.listeners) l.ended?.(launch, null);
      this.setState('stopped');
      this.resolveStopped?.();
      return;
    }
    if (fatal !== null) {
      const report = crashReport(process, this.restart.attempts, `${describeExit(code < 0 ? null : code)}: ${fatal}`, [], [], 0);
      this.options.log('error', `the ${process} engine cannot run here and stays stopped: ${fatal}`);
      if (launch) for (const l of this.listeners) l.ended?.(launch, report);
      this.setState('stopped');
      return;
    }
    const { attempt, delayMs } = this.restart.crashed();
    const suspects = this.lastInFlight;
    const markedFailed = this.options.ledger.record(suspects);
    const report = crashReport(process, attempt, describeExit(code < 0 ? null : code), suspects, markedFailed, delayMs);
    this.options.log('error', `the ${process} engine ${report.exit}; restart ${attempt} in ${delayMs} ms${suspects.length ? ` (${suspects.length} suspect item(s), ${markedFailed.length} now failed)` : ''}`);
    if (launch) for (const l of this.listeners) l.ended?.(launch, report);
    this.setState('restarting');
    const schedule = this.options.schedule ?? defaultSchedule;
    this.pendingRestart = schedule(() => {
      if (!this.stopping) this.spawn();
    }, delayMs);
  }

  /** Stops the engine (app quit, or the flag turned off). */
  stop(timeoutMs = 3000): Promise<void> {
    this.stopping = true;
    this.pendingRestart?.cancel();
    this.pendingRestart = null;
    const child = this.child;
    if (!child) {
      this.setState('stopped');
      return Promise.resolve();
    }
    this.stopped ??= new Promise<void>((resolve) => {
      this.resolveStopped = resolve;
    });
    child.postMessage({ type: 'shutdown' } satisfies EngineControl);
    const timer = setTimeout(() => {
      if (this.child === child) child.kill();
    }, timeoutMs);
    return this.stopped.finally(() => clearTimeout(timer));
  }

  /** A request to the running launch. */
  request(body: Request, session: number | null = null): Promise<Response> {
    const launch = this.current;
    if (!launch) return Promise.reject(new Error(`the ${this.options.process} engine is not running`));
    return launch.connection.request(body, session);
  }

  event(body: Event, session: number | null = null): void {
    this.current?.connection.event(body, session);
  }
}
