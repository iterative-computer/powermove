/*
 * The Electron surfaces the engine host uses, as narrow interfaces so the
 * host logic runs under unit tests with in-memory fakes and under Electron
 * with `MessageChannelMain` and `utilityProcess`.
 */
import type { EngineProcess } from '../../shared/engine-protocol';

/** `MessagePortMain`. */
export interface PortLike {
  postMessage(message: unknown): void;
  on(event: 'message', listener: (event: { data: unknown }) => void): unknown;
  on(event: 'close', listener: () => void): unknown;
  start(): void;
  close(): void;
}

export type ChannelPair = { port1: PortLike; port2: PortLike };

/** `new MessageChannelMain()`. */
export type ChannelFactory = () => ChannelPair;

/** `UtilityProcess`. */
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

/** Control messages from main to an engine process (over its parent port). */
export type EngineControl =
  | { type: 'connect'; port: number; peer: 'host' | 'renderer' | 'peer' }
  | { type: 'disconnect'; port: number }
  | { type: 'shutdown' };

/** Messages from an engine process to main (over its parent port). */
export type EngineNotice =
  | { type: 'started'; build: string; pid: number; clock: number }
  | { type: 'log'; level: 'info' | 'warn' | 'error'; message: string }
  | { type: 'fatal'; message: string };

export function isEngineNotice(value: unknown): value is EngineNotice {
  if (typeof value !== 'object' || value === null) return false;
  const type = (value as { type?: unknown }).type;
  return type === 'started' || type === 'log' || type === 'fatal';
}

/** What a client (a renderer window) receives for each engine connection. */
export type ClientPortInfo = {
  process: EngineProcess;
  /** Main's id for the port (the engine knows it by the same id). */
  port: number;
  /** Engine launch count; a new generation means re-open sessions and resync. */
  generation: number;
};
