/*
 * IPC between main and renderers for the native engine (behind "Use New
 * Engine"). Engine ports themselves are posted to the window's main frame
 * only (never to extension sandbox frames) and handed to the page with
 * `window.postMessage`; everything else here is plain IPC.
 */
import type { CrashReport, EngineProcess } from './engine-protocol';

export const ENGINE_IPC = {
  /** main → preload: a `MessagePort` to one engine (`EnginePortMessage`). */
  port: 'engine:port',
  /** main → preload: an engine crashed and is restarting (`CrashReport`). */
  crash: 'engine:crash',
  /** main → preload: engine process states (`EngineStatusMessage`). */
  status: 'engine:status-changed',
  /** renderer → main (invoke): whether the new engine is on, and its state. */
  getStatus: 'engine:status',
  /** renderer → main (sendSync): whether this launch runs the new engine
   *  (the renderer decides its document mirror and transport at boot). */
  enabledSync: 'engine:enabled-sync',
  /** renderer → main (invoke): the shared host clock in µs (`HostMicros`). */
  hostTime: 'engine:host-time',
  /** renderer → main (invoke): stage a large resync document as a cache file. */
  stageDocument: 'engine:stage-document',
  /** renderer → main (send): the window's definition updates. */
  definitions: 'engine:definitions'
} as const;

/** The `window.postMessage` types the preload uses to hand things to the page. */
export const ENGINE_WINDOW_MESSAGE = {
  port: 'powermove:engine-port',
  crash: 'powermove:engine-crash',
  status: 'powermove:engine-status'
} as const;

export type EnginePortMessage = { process: EngineProcess; port: number; generation: number };

export type EngineProcessState = 'stopped' | 'starting' | 'ready' | 'restarting';

export type EngineStatusMessage = {
  enabled: boolean;
  processes: Record<EngineProcess, { state: EngineProcessState; generation: number; attempts: number }>;
};

export type EngineCrashMessage = CrashReport;

export type StageDocumentRequest = { process: EngineProcess; session: number; bytes: Uint8Array };
export type StageDocumentResult = { handle: string; length: number };

/** The renderer's view (`window.powermove.nativeEngine`, present on the
 *  desktop host). Ports, crashes, and status arrive as window messages
 *  (`ENGINE_WINDOW_MESSAGE`) because a `MessagePort` cannot cross the
 *  context bridge. */
export interface EngineBridge {
  status(): Promise<EngineStatusMessage>;
  /** Whether this launch runs the new engine (read once at launch). */
  enabledSync(): boolean;
  /** The shared host clock (`mach_absolute_time` in µs). */
  hostTime(): Promise<number>;
  stageDocument(request: StageDocumentRequest): Promise<StageDocumentResult>;
  pushDefinitions(update: import('./engine-protocol').DefinitionsUpdate): void;
}
