/*
 * Shared contract for the agent's "watch and listen" media tools.
 *
 * Main runs the tools (ffmpeg, transcription) on a file path the renderer
 * resolves through `resolveMediaPath` (renderer/src/media/media-path.ts):
 * the original source when it is still on disk and unchanged, otherwise the
 * asset's stored bytes staged once to a cache file main owns (desktop) or the
 * host's own copy (`powermove serve`).
 */

/** Tools main executes with ffmpeg / the transcription seam. */
export const AGENT_MEDIA_TOOL_NAMES = [
  'probe_media', 'sample_media_frames', 'media_contact_sheet', 'media_waveform', 'transcribe_media'
] as const;
export type AgentMediaToolName = typeof AGENT_MEDIA_TOOL_NAMES[number];

/** Renderer-handled structural lint; listed with the media tools for activity labels. */
export const CHECK_PROJECT_TOOL = 'check_project';

/** Internal renderer round trip: asset/layer → readable file + timing. */
export const MEDIA_SOURCE_TOOL = '__media_source';

/** Internal renderer round trip: composition size, rate, duration and work area.
 * Unlike get_project_state it is not a read the run acknowledges, so it never
 * moves the run's edit baseline past changes the agent has not seen. */
export const COMPOSITION_INFO_TOOL = '__composition_info';

export interface AgentCompositionInfo { width: number; height: number; fps: number; duration: number; workArea?: [number, number] }

const MEDIA_TOOL_SET = new Set<string>([...AGENT_MEDIA_TOOL_NAMES, CHECK_PROJECT_TOOL]);

/** `mcp__powermove__probe_media`, `probe_media` → `probe_media`; anything else → null. */
export function mediaToolName(toolName: unknown): string | null {
  if (typeof toolName !== 'string') return null;
  const lower = toolName.trim().toLowerCase();
  const short = lower.startsWith('mcp__') ? (lower.split('__').at(-1) ?? lower) : lower;
  return MEDIA_TOOL_SET.has(short) ? short : null;
}

/* ── media paths (renderer ⇄ main) ──────────────────────── */

export const MEDIA_PATH_IPC = {
  lookup: 'media-path:lookup',
  stageBegin: 'media-path:stage-begin',
  stageChunk: 'media-path:stage-chunk',
  stageFinish: 'media-path:stage-finish',
  stageAbort: 'media-path:stage-abort',
  /** Drop this window's staged files for a project it closed (or all of them). */
  release: 'media-path:release'
} as const;

/** Desktop staging chunk; the web host's own upload uses WEB_UPLOAD_CHUNK_BYTES. */
export const MEDIA_STAGE_CHUNK_BYTES = 4 * 1024 * 1024;

export type MediaPathOrigin = 'source' | 'cache' | 'host';

export interface MediaPathLookupRequest {
  assetId: string;
  projectId?: string;
  /** Absolute path the asset was imported from, when known. */
  sourcePath?: string;
  /** Import fingerprint (`v2:<size>:<sha256>`); a source is used only when it still matches. */
  fingerprint?: string;
  /** Content-addressed store key (`media:…`). */
  storageKey?: string;
  /** Byte size of the stored copy, for the staging cache key. */
  size?: number;
}

export interface MediaPathResult { path: string; origin: MediaPathOrigin }

export interface MediaPathStageRequest {
  assetId: string;
  projectId?: string;
  storageKey?: string;
  name: string;
  /** MIME type of the bytes, so the cache file gets an honest extension. */
  type?: string;
  size: number;
}

export interface MediaPathBridge {
  lookup(request: MediaPathLookupRequest): Promise<MediaPathResult | null>;
  /** Desktop: chunked staging into main's cache. Resolves a path at once when already staged. */
  stageBegin?(request: MediaPathStageRequest): Promise<{ token: string } | MediaPathResult>;
  stageChunk?(token: string, offset: number, data: Uint8Array): Promise<void>;
  stageFinish?(token: string): Promise<MediaPathResult>;
  stageAbort?(token: string): Promise<void>;
  /** Web host: hand the bytes over with the bridge's own upload path. */
  stageFile?(file: File, request: MediaPathStageRequest): Promise<MediaPathResult>;
  release?(projectId?: string): Promise<void>;
}

/* ── __media_source (renderer → main) ───────────────────── */

export interface AgentMediaTiming {
  /** Ascending composition times paired with source times, piecewise linear. */
  samples: Array<[number, number]>;
  /** Constant speed and trim: the two samples are exact. */
  constant: boolean;
}

export interface AgentMediaSource {
  path: string;
  origin: MediaPathOrigin;
  /** What the bytes are (import fingerprint, else store key), stable when the
   * same asset is staged again to a new file; results are cached by it. */
  contentKey?: string;
  asset: {
    id: string;
    name: string;
    kind: 'video' | 'audio';
    duration: number | null;
    hasAudio: boolean;
    /** The stored bytes are Powermove's playback proxy, not the original file. */
    proxy: boolean;
  };
  layer?: {
    id: string;
    name: string;
    type: string;
    from: number;
    duration: number;
    timing: AgentMediaTiming;
    /** Source span the layer plays, seconds. */
    sourceStart: number;
    sourceEnd: number;
  };
}

/* ── activity rows ───────────────────────────────────────── */

/** The few arguments an activity row needs, sanitized for the trace stream. */
export interface AgentMediaToolSubject {
  assetId?: string;
  layerId?: string;
  /** Requested frame count (count or times.length). */
  frames?: number;
  auto?: boolean;
  start?: number;
  end?: number;
  target?: 'source' | 'composition';
}

const ID = /^[A-Za-z0-9_.:-]{1,120}$/;
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);

export function mediaToolSubject(toolName: unknown, args: unknown): AgentMediaToolSubject | undefined {
  const tool = mediaToolName(toolName);
  if (!tool || tool === CHECK_PROJECT_TOOL || !args || typeof args !== 'object' || Array.isArray(args)) return undefined;
  const input = args as Record<string, unknown>;
  const subject: AgentMediaToolSubject = {};
  if (typeof input.assetId === 'string' && ID.test(input.assetId)) subject.assetId = input.assetId;
  if (typeof input.layerId === 'string' && ID.test(input.layerId)) subject.layerId = input.layerId;
  if (Array.isArray(input.times) && input.times.length) subject.frames = Math.min(999, input.times.length);
  else if (finite(input.count) && input.count > 0) subject.frames = Math.min(999, Math.trunc(input.count));
  if (input.auto === true) subject.auto = true;
  if (finite(input.start)) subject.start = input.start;
  if (finite(input.end)) subject.end = input.end;
  if (input.target === 'composition' || input.target === 'source') subject.target = input.target;
  return subject;
}

/** "0:04.20", "1:02:03.40": the clock the media tools label frames with. */
export function formatClock(seconds: number): string {
  if (!Number.isFinite(seconds)) return '';
  const negative = seconds < 0;
  const centis = Math.round(Math.abs(seconds) * 100);
  const hours = Math.floor(centis / 360000);
  const minutes = Math.floor((centis % 360000) / 6000);
  const secs = (centis % 6000) / 100;
  const ss = secs.toFixed(2).padStart(5, '0');
  const body = hours ? `${hours}:${String(minutes).padStart(2, '0')}:${ss}` : `${minutes}:${ss}`;
  return negative ? `-${body}` : body;
}

export interface MediaToolLabels { running: string; done: string; failed: string; detail?: string }

/**
 * Specific, live wording for a media tool row: "Transcribing interview.mov…"
 * while it runs, "Transcribed interview.mov" once done. `name` is the asset or
 * layer the call targets, resolved by the caller (the renderer knows names).
 */
export function mediaToolLabels(toolName: unknown, subject: AgentMediaToolSubject | undefined, name: string | null): MediaToolLabels | null {
  const tool = mediaToolName(toolName);
  if (!tool) return null;
  const what = name || (subject?.layerId ? 'a clip' : 'media');
  const span = subject && (subject.start !== undefined || subject.end !== undefined)
    ? `${subject.start !== undefined ? formatClock(subject.start) : 'start'}–${subject.end !== undefined ? formatClock(subject.end) : 'end'}`
    : undefined;
  const make = (running: string, done: string, failed: string): MediaToolLabels =>
    ({ running: `${running}…`, done, failed, ...(span ? { detail: span } : {}) });
  switch (tool) {
    case 'probe_media': return make(`Probing ${what}`, `Probed ${what}`, `Probe ${what}`);
    case 'sample_media_frames': {
      if (subject?.auto) return make(`Finding scene changes in ${what}`, `Found scene changes in ${what}`, `Find scene changes in ${what}`);
      const n = subject?.frames ?? 1;
      const frames = `${n} ${n === 1 ? 'frame' : 'frames'}`;
      return make(`Sampling ${frames} from ${what}`, `Sampled ${frames} from ${what}`, `Sample ${frames} from ${what}`);
    }
    case 'media_contact_sheet': {
      const of = subject?.target === 'composition' ? 'the composition' : what;
      return make(`Building a contact sheet of ${of}`, `Built a contact sheet of ${of}`, `Build a contact sheet of ${of}`);
    }
    case 'media_waveform': return make(`Mapping silences in ${what}`, `Mapped silences in ${what}`, `Map silences in ${what}`);
    case 'transcribe_media': return make(`Transcribing ${what}`, `Transcribed ${what}`, `Transcribe ${what}`);
    case CHECK_PROJECT_TOOL: return { running: 'Checking the project…', done: 'Checked the project', failed: 'Check the project' };
    default: return null;
  }
}

/* Running → settled wording. The row keeps the label it was given while the
   call ran; once it settles the verb changes tense ("Transcribing x…" →
   "Transcribed x"), or goes back to the plain verb beside "Failed". */
const SETTLED_VERBS: Array<[string, string, string]> = [
  ['Probing ', 'Probed ', 'Probe '],
  ['Finding scene changes', 'Found scene changes', 'Find scene changes'],
  ['Sampling ', 'Sampled ', 'Sample '],
  ['Building ', 'Built ', 'Build '],
  ['Mapping ', 'Mapped ', 'Map '],
  ['Transcribing ', 'Transcribed ', 'Transcribe '],
  ['Checking ', 'Checked ', 'Check ']
];

export function settleMediaLabel(toolName: unknown, label: string, status: 'running' | 'done' | 'error' | 'continued'): string {
  if (!mediaToolName(toolName) || status === 'running') return label;
  const rule = SETTLED_VERBS.find(([running]) => label.startsWith(running));
  if (!rule) return label;
  const rest = label.slice(rule[0].length).replace(/…$/, '');
  return `${status === 'error' ? rule[2] : rule[1]}${rest}`;
}

/* Non-final answers. transcribe_media can answer before there is a transcript:
   a long job is still running ("status":"transcribing", a successful call) or
   no model is downloaded ("status":"model-required", a failed call). Neither
   is "Transcribed x" nor a failure, so the row says what actually happened. */
export type MediaToolOutcome = { state: 'pending'; progress?: number } | { state: 'needs-model' };

/* The answer's own first key: at most a short provider prefix ("Error: ")
   before its opening brace, so a transcript that quotes this never matches. */
const OUTCOME_STATUS = /^[^{]{0,80}\{\s*"status"\s*:\s*"(transcribing|model-required)"/;
const OUTCOME_PROGRESS = /"progress"\s*:\s*([0-9]*\.?[0-9]+)/;

/** Reads a non-final outcome from a tool-end excerpt; the status is the answer's first key. */
export function mediaToolOutcome(toolName: unknown, output: unknown): MediaToolOutcome | null {
  if (mediaToolName(toolName) !== 'transcribe_media' || typeof output !== 'string') return null;
  const status = OUTCOME_STATUS.exec(output)?.[1];
  if (status === 'model-required') return { state: 'needs-model' };
  if (status !== 'transcribing') return null;
  const progress = Number(OUTCOME_PROGRESS.exec(output)?.[1]);
  return Number.isFinite(progress) && progress > 0 && progress <= 1 ? { state: 'pending', progress } : { state: 'pending' };
}

/** "Transcribing interview.mov…" → "Still transcribing interview.mov" (chip "42%"), or "Needs a transcription model for interview.mov". */
export function mediaOutcomeLabels(label: string, detail: string | undefined, outcome: MediaToolOutcome): { label: string; detail?: string } {
  const what = label.replace(/^Transcribing /, '').replace(/…$/, '').trim() || 'media';
  if (outcome.state === 'needs-model') return { label: `Needs a transcription model for ${what}`, ...(detail ? { detail } : {}) };
  const percent = outcome.progress !== undefined ? `${Math.round(outcome.progress * 100)}%` : undefined;
  const chip = [detail, percent].filter(Boolean).join(' · ');
  return { label: `Still transcribing ${what}`, ...(chip ? { detail: chip } : {}) };
}
