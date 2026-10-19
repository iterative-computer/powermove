/*
 * The Electron surfaces the engine host uses, as narrow interfaces so the
 * host logic runs under unit tests with in-memory fakes and under Electron
 * with `MessageChannelMain` and `utilityProcess`.
 */
import type { EngineProcess } from '../../shared/engine-protocol';

/** `MessagePortMain`. Posting on a port whose other end is gone (closed,
 *  or its process died) drops the message; it never crashes main. */
export interface PortLike {
  postMessage(message: unknown, transfer?: PortLike[]): void;
  on(event: 'message', listener: (event: { data: unknown; ports?: PortLike[] }) => void): unknown;
  on(event: 'close', listener: () => void): unknown;
  start(): void;
  close(): void;
}

export type ChannelPair = { port1: PortLike; port2: PortLike };

/** `new MessageChannelMain()`. */
export type ChannelFactory = () => ChannelPair;

/** `UtilityProcess`. Main posts to it exactly once, right after the fork
 *  (`EngineBootstrap`): `UtilityProcess.postMessage` to a child that is
 *  exiting or has exited crashes Electron's main process (SIGSEGV), and
 *  main cannot know a child is exiting until its `exit` event. Everything
 *  else goes over the control port, a `MessageChannelMain` that simply
 *  drops messages once the child is gone. */
export interface ChildLike {
  readonly pid?: number | undefined;
  postMessage(message: unknown, transfer?: PortLike[]): void;
  on(event: 'spawn', listener: () => void): unknown;
  on(event: 'exit', listener: (code: number) => void): unknown;
  on(event: 'message', listener: (message: unknown) => void): unknown;
  kill(): boolean;
}

/** Starts one engine utility process. */
export type SpawnEngine = (process: EngineProcess) => ChildLike;

/** The one message main posts over an engine's parent port: it carries
 *  the control port (`ports[0]`). */
export type EngineBootstrap = { type: 'control' };

/** Control messages from main to an engine process, over its control
 *  port. The engine exits when the control port closes, as on
 *  `shutdown`. */
export type EngineControl =
  /** `owner` names the window a renderer client's port belongs to (the
   *  engine measures its text with that window's font cache seeds). */
  | { type: 'connect'; port: number; peer: 'host' | 'renderer' | 'peer' | 'fonts'; owner?: number }
  | { type: 'disconnect'; port: number }
  /** Destroy the viewer presenter's GPU device as a real loss would (the
   *  recovery test; interactive engine). */
  | { type: 'simulateGpuLoss' }
  /** The engine's status report (sessions, lease, presenter counters,
   *  routes, audio): diagnostics and measurements. */
  | { type: 'status'; id: number }
  | { type: 'shutdown' };

/** Messages from an engine process to main (over its parent port). */
export type EngineNotice =
  | { type: 'started'; build: string; pid: number; clock: number }
  | { type: 'log'; level: 'info' | 'warn' | 'error'; message: string }
  | { type: 'fatal'; message: string }
  /** The answer to a `status` control message (pm-node's status JSON). */
  | { type: 'status'; id: number; json: string };

export function isEngineNotice(value: unknown): value is EngineNotice {
  if (typeof value !== 'object' || value === null) return false;
  const type = (value as { type?: unknown }).type;
  return type === 'started' || type === 'log' || type === 'fatal' || type === 'status';
}

/** What a client (a renderer window) receives for each engine connection. */
export type ClientPortInfo = {
  process: EngineProcess;
  /** Main's id for the port (the engine knows it by the same id). */
  port: number;
  /** Engine launch count; a new generation means re-open sessions and resync. */
  generation: number;
};
