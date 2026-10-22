/*
 * IPC between main and renderers for the native engine (behind "Use New
 * Engine"). Engine ports themselves are posted to the window's main frame
 * only (never to extension sandbox frames) and handed to the page with
 * `window.postMessage`; everything else here is plain IPC.
 */
import { type DeliveryOptions, migrateFormat, normalizeTimecode } from './delivery';
import type { CrashReport, EngineProcess, MediaFacts } from './engine-protocol';

export const ENGINE_IPC = {
  proxies: 'engine:proxies',
  proxiesChanged: 'engine:proxies-changed',
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
  probeMedia: 'engine:probe-media',
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
  locateMedia: 'engine:locate-media',
  /** renderer → main (sendSync): a snapshot the calling thread waits for
   *  (`EngineSnapshotRequest` → `EngineSnapshotReply`; the synchronous
   *  compatibility path, apiVersion 1 to 3). */
  snapshotSync: 'engine:snapshot-sync',
  /** renderer → main (invoke): the same, asynchronously (async siblings,
   *  thumbnails, library cards, posters, the project thumbnail). */
  snapshot: 'engine:snapshot',
  /** main → renderer: the job engine needs a layer's compatibility pixels
   *  for an export frame (`EngineCompatAsk`); the window answers with
   *  `compatAnswer`. */
  compatAsk: 'engine:compat-ask',
  /** renderer → main (send): the pixels for a `compatAsk`, or why not
   *  (`EngineCompatAnswer`). */
  compatAnswer: 'engine:compat-answer',
  /** main → renderer: run a job engine request in the window's shader
   *  fallback runner (`ShaderFallbackJob`, main/engine/shader-fallback-relay.ts). */
  shaderFallback: 'engine:shader-fallback',
  /** renderer → main (send): the runner's answer (`{ id, ok, rgba16f }` or `{ id, ok: false, reason, message }`). */
  shaderFallbackResult: 'engine:shader-fallback-result'
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

export type RenderQueueJobState = 'pending' | 'queued' | 'running' | 'waitingForMedia' | 'paused' | 'done' | 'failed' | 'cancelled';

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

/** Interpret Footage asks for an asset's facts, read with `interpret` (the
 *  asset's `interpret` member, or the sheet's draft of it). */
export type EngineProbeRequest = { asset: string; interpret: Record<string, unknown> | null };

/** `facts`: what the file is and how it reads; `unavailable`: the new engine
 *  is off or not running, or the asset's file cannot be found. */
export type EngineProbeResult =
  | { status: 'facts'; facts: MediaFacts }
  | { status: 'unavailable'; reason: string };

/* ── The Phase 3c render transport (docs/engine/rfcs/16-render-transport.md) ── */

/** A snapshot the renderer asks main for. `session` and `generation` name
 *  the window's engine session on its current interactive engine launch
 *  (main refuses another window's session and a stale launch's);
 *  `revision` is the document revision the caller has (read-your-writes). */
export type EngineSnapshotRequest = {
  generation: number;
  /** The window's engine session, or null with `document` (main serves the
   *  snapshot from a session of its own for the window). */
  session: number | null;
  revision: number | null;
  snapshot: import('./engine-protocol').SnapshotRequest;
  /** The window's document, for a sync snapshot its own session cannot
   *  serve (none yet, or a staged resync in flight). `json` is left out
   *  when main already has this revision; main answers `needsDocument`
   *  when it does not. */
  document?: EngineSnapshotDocument;
  /** Shader fallback pixels drawn for an earlier `needsRenderer` answer of
   *  a `document` snapshot (a window's own session gets them on its port). */
  fallbackPixels?: { key: string; width: number; height: number; rgba16f: Uint8Array }[];
  /** Level 1 SVG rasters (premultiplied RGBA8, top-down) drawn for an
   *  earlier `needsRenderer` answer of a `document` snapshot: main posts
   *  them on its session for the window before asking again (the window
   *  may have no session of its own yet; decision S1). */
  svgPixels?: { key: import('./engine-protocol').SvgRasterKey; data: Uint8Array }[];
};

export type EngineSnapshotDocument = { id: string; revision: number; hash?: string | null; json?: Uint8Array | null };

/** What a snapshot came back with. `data` is RGBA8, rows top-down, in
 *  `pixels`'s encoding. `empty`: a layer target that draws nothing then
 *  (today's `render.raster()` returns null). `needsRenderer`: draw these with the shader
 *  fallback runner (and the compatibility producers), upload them on the
 *  session's port, and ask again. `failed`: `code` is the engine's error
 *  code (`timeout`, `unsupported`, `unknownSession`, …) or `unavailable`
 *  (the new engine is off or not running) or `refused` (main's caps). */
export type EngineSnapshotReply =
  | {
      status: 'rendered';
      revision: number;
      viewSeq: number | null;
      width: number;
      height: number;
      pixels: import('./engine-protocol').SnapshotPixels;
      data: Uint8Array;
      origin: [number, number];
      /** A layer raster's box in layer pixels (`RasterSurface` w, h). */
      layerBox: [number, number] | null;
      /** A layer raster's canvas, crop window, and selection. */
      raster: import('./engine-protocol').RasterGeometry | null;
      quality: import('./engine-protocol').FrameQuality;
      waitedUs: number;
      renderUs: number;
      /** Time the whole round trip took in main (ms). */
      mainMs: number;
    }
  | {
      status: 'needsRenderer';
      revision: number;
      fallback: import('./engine-protocol').ShaderFallbackRequest[];
      compat: import('./engine-protocol').CompatNeed[];
      /** The textures of the `fallback` requests that sample them (effects
       *  and transitions): binary16 RGBA bytes in GL row order, one per
       *  texture, as the engine sent them to main ahead of the answer
       *  (main/engine/fallback-inputs.ts). */
      fallbackTextures?: { key: string; textures: Uint8Array[] }[];
      /** Level 1 SVG rasters to draw and post before asking again (decision S1). */
      svg?: import('./engine-protocol').SvgRasterRequest[];
    }
  | { status: 'empty'; revision: number }
  | { status: 'failed'; code: string; message: string };

/** main → renderer: compatibility pixels the job engine needs for an export
 *  frame (contract 14 `compatFrame`). */
export type EngineCompatAsk = { ask: number; request: import('./engine-protocol').CompatFrameRequest };

/** A window's `unavailable` answer when another document is open there:
 *  main keeps the more useful reason (the named S1 message, or another
 *  window's) when every window says no. */
export const OTHER_DOCUMENT_OPEN = 'This window has another document open.';

/** renderer → main: the answer to an `EngineCompatAsk`. */
export type EngineCompatAnswer =
  | { ask: number; status: 'pixels'; key: string; width: number; height: number; format: import('./engine-protocol').CompatFormat; origin: [number, number]; scale: number; data: Uint8Array }
  | { ask: number; status: 'unavailable'; message: string };

/** The renderer's view (`window.powermove.nativeEngine`, present on the
 *  desktop host). Ports, crashes, and status arrive as window messages
 *  (`ENGINE_WINDOW_MESSAGE`) because a `MessagePort` cannot cross the
 *  context bridge. */
export interface EngineBridge {
  proxies?: import('./proxies').EngineProxiesBridge;
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
  /** Interpret Footage's facts of an asset's file (the job engine reads it). */
  probeMedia(request: EngineProbeRequest): Promise<EngineProbeResult>;
  /** A snapshot the calling thread waits for (the synchronous
   *  compatibility path). Never throws: failures are `status: 'failed'`. */
  snapshotSync?(request: EngineSnapshotRequest): EngineSnapshotReply;
  /** A snapshot, asynchronously. Never rejects: failures are `status: 'failed'`. */
  snapshot?(request: EngineSnapshotRequest): Promise<EngineSnapshotReply>;
  /** Compatibility pixels the job engine needs for an export frame; the
   *  listener answers with `answerCompat`. Returns an unsubscribe. */
  onCompatAsk?(listener: (ask: EngineCompatAsk) => void): () => void;
  answerCompat?(answer: EngineCompatAnswer): void;
  /** Runs the job engine's fallback requests in this window's runner
   *  (`run` answers each); returns an unsubscribe. */
  onShaderFallback?(run: (job: { request: import('./engine-protocol').ShaderFallbackRequest; textures: Uint16Array[] }) => Promise<Uint16Array>): () => void;
}
