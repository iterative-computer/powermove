/*
 * Frozen contract for on-device transcription, shared by the transcription
 * engine, the agent's media tools and captions. Models are never bundled:
 * the user downloads one (onboarding, Settings › Transcription, or the
 * on-demand sheet) and it lives in the app's user data directory.
 *
 * Times are seconds relative to the start of the source media (before any
 * layer trim, speed or composition offset). Callers map them into
 * composition time themselves.
 */

export interface TranscriptWord {
  text: string;
  start: number;
  end: number;
  /** 0..1 when the model reports it. */
  confidence?: number;
}

export interface TranscriptSegment {
  text: string;
  start: number;
  end: number;
  words: TranscriptWord[];
}

export interface Transcript {
  modelId: string;
  /** BCP-47 tag when the model reports or was told one. */
  language?: string;
  /** Duration of the transcribed span in seconds. */
  duration: number;
  segments: TranscriptSegment[];
}

export type TranscriptionModelState = 'absent' | 'downloading' | 'ready' | 'error';

export interface TranscriptionModelInfo {
  id: string;
  name: string;
  description: string;
  /** Download size in bytes. */
  size: number;
  /** 'multi' when the model detects and transcribes many languages. */
  languages: 'en' | 'multi';
  /** 0..1 relative scores for the picker, as Handy presents them. */
  speed: number;
  accuracy: number;
  recommended: boolean;
  wordTimestamps: boolean;
  state: TranscriptionModelState;
  /** 0..1 while downloading. */
  progress?: number;
  error?: string;
  /* ── additive (transcription lane) ── */
  /** BCP-47 tags the model transcribes, for the language picker. */
  languageCodes?: string[];
  /** Bytes received so far while downloading. */
  downloadedBytes?: number;
}

export interface TranscriptionStatus {
  models: TranscriptionModelInfo[];
  /** The model transcription uses, or null when none is ready. */
  activeModelId: string | null;
  /* ── additive (transcription lane) ── */
  /** False when this host cannot run on-device transcription at all (a
   *  browser host without the engine, an unsupported platform). Omitted
   *  means available. */
  available?: boolean;
  /** Spoken-language preference: 'auto' or a BCP-47 tag from the active
   *  model's `languageCodes`. Used when a request carries no language. */
  language?: string;
  /** Bytes the downloaded models (and partial downloads) take on disk. */
  storageBytes?: number;
}

export interface TranscribeRequest {
  /** Absolute path to an audio or video file the main process can read. */
  path: string;
  /** Optional source-time span, seconds. */
  start?: number;
  end?: number;
  /** BCP-47 hint; omitted means auto-detect when the model supports it. */
  language?: string;
  /** Opaque id echoed on progress events. */
  requestId?: string;
}

/** What the transcribe IPC returns: errors keep their `code` across the
 *  process boundary (a thrown Error would lose it). Additive. */
export type TranscribeResult =
  | { ok: true; transcript: Transcript }
  | { ok: false; code?: string; message: string };

export interface TranscribeProgress {
  requestId: string;
  /** 0..1 */
  progress: number;
}

/** Error `code` thrown/returned when no model is ready. UIs answer it with
 * the download sheet; agent tools return it so the agent can tell the user. */
export const TRANSCRIPTION_MODEL_MISSING = 'transcription-model-missing';

export const TRANSCRIPTION_IPC = {
  status: 'transcription:status',
  download: 'transcription:download',
  cancelDownload: 'transcription:cancel-download',
  remove: 'transcription:remove',
  setActive: 'transcription:set-active',
  transcribe: 'transcription:transcribe',
  cancelTranscribe: 'transcription:cancel-transcribe',
  /** main → renderer: TranscriptionStatus whenever any model state changes. */
  statusChanged: 'transcription:status-changed',
  /** main → renderer: TranscribeProgress. */
  progress: 'transcription:progress',
  /** main → renderer: an agent tool needs a model; show the download sheet.
   * Payload: { reason: string }. */
  modelRequested: 'transcription:model-requested',
  /* ── additive (transcription lane) ── */
  /** renderer → main: 'auto' or a BCP-47 tag. */
  setLanguage: 'transcription:set-language',
  /** renderer → main: show the models folder in Finder. */
  reveal: 'transcription:reveal'
} as const;

/* ── additive (transcription lane): the preload bridge ── */

/** `transcription` on the editor bridge (preload/index.ts, web-bridge.ts). */
export interface TranscriptionBridge {
  status(): Promise<TranscriptionStatus>;
  /** Starts the download in main (it outlives the window); resolves at once. */
  download(modelId: string): Promise<TranscriptionStatus>;
  cancelDownload(modelId: string): Promise<TranscriptionStatus>;
  remove(modelId: string): Promise<TranscriptionStatus>;
  setActive(modelId: string): Promise<TranscriptionStatus>;
  setLanguage(language: string): Promise<TranscriptionStatus>;
  /** Shows the models folder in Finder. */
  reveal(): Promise<void>;
  transcribe(request: TranscribeRequest): Promise<TranscribeResult>;
  cancelTranscribe(requestId: string): Promise<void>;
  onStatus(callback: (status: TranscriptionStatus) => void): () => void;
  onProgress(callback: (progress: TranscribeProgress) => void): () => void;
  onModelRequested(callback: (request: { reason: string }) => void): () => void;
}

/** The onboarding window's slice: pick and start a download, watch it. */
export type OnboardingTranscriptionBridge = Pick<TranscriptionBridge, 'status' | 'download' | 'cancelDownload' | 'remove' | 'onStatus'>;
