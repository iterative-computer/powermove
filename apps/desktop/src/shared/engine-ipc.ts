/*
 * IPC between main and renderers for the native engine (behind "Use New
 * Engine"). Engine ports themselves are posted to the window's main frame
 * only (never to extension sandbox frames) and handed to the page with
 * `window.postMessage`; everything else here is plain IPC.
 */
import { type DeliveryOptions, migrateFormat, normalizeTimecode } from './delivery';
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
  /** renderer → main (invoke): add an export to the render queue
   *  (`EngineQueueRequest`); main asks for its destination. */
  queueAdd: 'engine:queue-add',
  /** renderer → main (invoke): the render queue's jobs (`RenderQueueState`). */
  queueList: 'engine:queue-list',
  /** renderer → main (invoke): a queue action (`RenderQueueAction`). */
  queueAction: 'engine:queue-action',
  /** main → renderer: the render queue changed (`RenderQueueState`). */
  queueChanged: 'engine:queue-changed',
  /** main → renderer: locate offline media (`string[]` asset ids) after the
   *  offline message's "Locate Media…". */
  locateMedia: 'engine:locate-media'
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

/** The export sheet's settings for one export (exporter.ts), in composition
 *  seconds. `format` is a delivery format id (shared/delivery.ts); today's
 *  `mp4` and `prores` are accepted as H.264 and ProRes 4444. */
export type SheetExportOptions = {
  format: string;
  alpha: boolean;
  /** Export frame rate (may differ from the composition's). */
  fps: number;
  /** Output scale (0.01 to 16): size `round(w × scale / 2) × 2`. */
  scale: number;
  /** An exact size instead of the scale. */
  width?: number;
  height?: number;
  start: number;
  end: number;
  audio: boolean;
  motionBlur: boolean;
  quality: 'draft' | 'high' | 'max';
  delivery?: Partial<DeliveryOptions>;
  /** Captions delivered as a caption file instead of burned in. */
  hideCaptions?: boolean;
  /** "Render using: Proxies (draft)" (section 14.8). */
  draft?: boolean;
};

/** Checks the shape and limits of sheet export options (IPC input). */
export function validSheetExportOptions(value: unknown): value is SheetExportOptions {
  if (!value || typeof value !== 'object') return false;
  const o = value as Record<string, unknown>;
  const finite = (k: string) => typeof o[k] === 'number' && Number.isFinite(o[k] as number);
  if (typeof o['format'] !== 'string' || migrateFormat(o['format'], 'web') === 'web' || o['format'] === 'json') return false;
  if (!['fps', 'scale', 'start', 'end'].every(finite)) return false;
  const fps = o['fps'] as number, scale = o['scale'] as number, start = o['start'] as number, end = o['end'] as number;
  if (!(fps > 0 && fps <= 240 && scale >= 0.01 && scale <= 16 && start >= 0 && end > start)) return false;
  if (!['draft', 'high', 'max'].includes(o['quality'] as string)) return false;
  if (!['alpha', 'audio', 'motionBlur'].every((k) => typeof o[k] === 'boolean')) return false;
  for (const k of ['width', 'height']) {
    if (o[k] !== undefined && !(Number.isInteger(o[k]) && (o[k] as number) >= 1 && (o[k] as number) <= 16384)) return false;
  }
  if ((o['width'] === undefined) !== (o['height'] === undefined)) return false;
  for (const k of ['hideCaptions', 'draft']) if (o[k] !== undefined && typeof o[k] !== 'boolean') return false;
  if (o['delivery'] !== undefined && (typeof o['delivery'] !== 'object' || o['delivery'] === null)) return false;
  const d = (o['delivery'] ?? {}) as Record<string, unknown>;
  if (d['timecode'] !== undefined && d['timecode'] !== '' && !normalizeTimecode(d['timecode'])) return false;
  return true;
}

/** A caption file written beside a delivered video (export-sidecar.ts). */
export type CaptionSidecar = { suffix?: string; extension: 'srt' | 'vtt'; text: string };

/** The export sheet's request (exporter.ts): the project as JSON and its
 *  settings; main shows the save panel (and, first, the offline message). */
export type EngineExportRequest = {
  project: string;
  /** The suggested file name without extension. */
  name: string;
  options: SheetExportOptions;
  /** Caption files to write beside the output once it is delivered. */
  sidecars?: CaptionSidecar[];
};

/** The export sheet's "Add to Queue": the request and the queue job's policy. */
export type EngineQueueRequest = EngineExportRequest & {
  /** The preset the settings came from, for the queue's list. */
  preset?: string;
};

/** `accepted`: the job runs and statuses follow; `cancelled`: the user
 *  cancelled the save panel; `unavailable`: the job engine is not running
 *  (use the current exporter). */
export type EngineExportStart =
  | { status: 'accepted'; jobId: string; frames: number; path: string }
  | { status: 'cancelled' }
  | { status: 'locate'; assets: string[] }
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
  /** The latest encoded frame (the progress sheet's live preview). */
  preview?: { frame: number; width: number; height: number; jpeg: Uint8Array };
  /** Why a job waits ("2 clips are offline: …"). */
  waiting?: string;
};

export type RenderQueueJobState = 'pending' | 'queued' | 'running' | 'waitingForMedia' | 'done' | 'failed' | 'cancelled';

/** One render queue job as the panel shows it. */
export type RenderQueueJob = {
  id: string;
  /** `user`: from the export sheet, persisted; `agent`: an agent's export_video. */
  source: 'user' | 'agent';
  name: string;
  /** "ProRes 422 HQ · 1920×1080 · 23.976 fps". */
  summary: string;
  destination: string;
  state: RenderQueueJobState;
  progress: number;
  error?: string;
  /** Why it waits (offline media). */
  waiting?: string;
  path?: string;
  createdAt: number;
  finishedAt?: number;
};

export type RenderQueueState = { running: boolean; jobs: RenderQueueJob[] };

export type RenderQueueAction =
  | { kind: 'start' }
  | { kind: 'pause' }
  | { kind: 'remove'; id: string }
  | { kind: 'cancel'; id: string }
  | { kind: 'retry'; id: string }
  | { kind: 'move'; id: string; to: number }
  | { kind: 'reveal'; id: string }
  | { kind: 'clearFinished' };
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
  /** Asks main for fresh ports to every engine (window messages follow). */
  requestPorts(): void;
  stageDocument(request: StageDocumentRequest): Promise<StageDocumentResult>;
  pushDefinitions(update: import('./engine-protocol').DefinitionsUpdate): void;
  exportVideo(request: EngineExportRequest): Promise<EngineExportStart>;
  /** Adds an export to the render queue (main asks for its destination). */
  queueAdd(request: EngineQueueRequest): Promise<{ status: 'added'; id: string } | { status: 'cancelled' } | { status: 'unavailable'; reason: string }>;
  queueList(): Promise<RenderQueueState>;
  queueAction(action: RenderQueueAction): Promise<RenderQueueState>;
  /** Render queue changes; returns an unsubscribe. */
  onQueueChanged(listener: (state: RenderQueueState) => void): () => void;
  /** "Locate Media…" chosen in the offline message; returns an unsubscribe. */
  onLocateMedia(listener: (assets: string[]) => void): () => void;
  cancelExport(jobId: string): Promise<void>;
  /** Export job statuses for this window; returns an unsubscribe. */
  onExportStatus(listener: (status: EngineExportStatus) => void): () => void;
  /** The editor's transport started (`true`) or stopped playing. */
  playback(playing: boolean): void;
}
