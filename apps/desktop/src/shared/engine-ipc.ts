/*
 * IPC between main and renderers for the native engine (behind "Use New
 * Engine"). Engine ports themselves are posted to the window's main frame
 * only (never to extension sandbox frames) and handed to the page with
 * `window.postMessage`; everything else here is plain IPC.
 */
import type { CrashReport, EngineProcess, MediaFacts } from './engine-protocol';

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
  /** renderer → main (send): the page listens now; give it fresh engine
   *  ports (ports posted before its listener existed were lost). */
  requestPorts: 'engine:request-ports',
  /** renderer → main (invoke): the shared host clock in µs (`HostMicros`). */
  hostTime: 'engine:host-time',
  /** renderer → main (invoke): stage a large resync document as a cache file. */
  stageDocument: 'engine:stage-document',
  /** renderer → main (send): the window's definition updates. */
  definitions: 'engine:definitions',
  /** renderer → main (invoke): export from the export sheet through the job engine. */
  exportVideo: 'engine:export-video',
  /** renderer → main (invoke): cancel one of the window's export jobs. */
  cancelExport: 'engine:export-cancel',
  /** main → renderer: an export job's status (`EngineExportStatus`). */
  exportStatus: 'engine:export-status',
  /** renderer → main (send): the editor's transport plays or stopped
   *  (boolean); the job engine slices its GPU work while anything plays. */
  playback: 'engine:editor-playback',
  /** renderer → main (invoke): Interpret Footage's facts of an asset's file
   *  (`EngineProbeRequest` → `EngineProbeResult`). */
  probeMedia: 'engine:probe-media'
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

/** The export sheet's request (exporter.ts): the project as JSON and its
 *  settings; main shows the save panel. */
export type EngineExportRequest = {
  project: string;
  /** The suggested file name without extension. */
  name: string;
  options: {
    format: 'mp4' | 'prores';
    alpha: boolean;
    fps: number;
    scale: number;
    start: number;
    end: number;
    audio: boolean;
    motionBlur: boolean;
    quality: 'draft' | 'high' | 'max';
  };
};

/** `accepted`: the job runs and statuses follow; `cancelled`: the user
 *  cancelled the save panel; `unavailable`: the job engine is not running
 *  (use the current exporter). */
export type EngineExportStart =
  | { status: 'accepted'; jobId: string; frames: number; path: string }
  | { status: 'cancelled' }
  | { status: 'unavailable'; reason: string };

/** A job status for the export progress sheet. `routed`: the new engine
 *  cannot render this project; the current exporter writes the same file
 *  without asking again. */
export type EngineExportStatus = {
  jobId: string;
  state: 'queued' | 'running' | 'waitingForMedia' | 'done' | 'failed' | 'cancelled';
  progress: number;
  path?: string;
  error?: string;
  routed?: boolean;
};
export type StageDocumentResult = { handle: string; length: number };

/** Interpret Footage asks for an asset's facts, read with `interpret` (the
 *  asset's `interpret` member, or the sheet's draft of it). */
export type EngineProbeRequest = { asset: string; interpret: Record<string, unknown> | null };

/** `facts`: what the file is and how it reads; `unavailable`: the new engine
 *  is off or not running, or the asset's file cannot be found. */
export type EngineProbeResult =
  | { status: 'facts'; facts: MediaFacts }
  | { status: 'unavailable'; reason: string };

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
  /** Asks main for fresh ports to every engine (window messages follow). */
  requestPorts(): void;
  stageDocument(request: StageDocumentRequest): Promise<StageDocumentResult>;
  pushDefinitions(update: import('./engine-protocol').DefinitionsUpdate): void;
  exportVideo(request: EngineExportRequest): Promise<EngineExportStart>;
  cancelExport(jobId: string): Promise<void>;
  /** Export job statuses for this window; returns an unsubscribe. */
  onExportStatus(listener: (status: EngineExportStatus) => void): () => void;
  /** The editor's transport started (`true`) or stopped playing. */
  playback(playing: boolean): void;
  /** Interpret Footage's facts of an asset's file (the job engine reads it). */
  probeMedia(request: EngineProbeRequest): Promise<EngineProbeResult>;
}
