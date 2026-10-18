/*
 * Agent renders through the job engine (plan sections 17 and 20; contracts
 * 12 and 14), behind "Use New Engine".
 *
 * `export_video` becomes an agent export job: main opens a job snapshot
 * session on the job engine with the project as the agent sees it, submits
 * the export in the agent export class (its own lane, ahead of user jobs)
 * with the host's automatic destination, and answers with the real path
 * once host services have moved the verified file into place (unique name,
 * created `open(wx, 0600)`, empty and unverified files removed;
 * outputs.ts). `mode: "async"` answers `{status: "queued", jobId}` at once,
 * and `wait` does too when the job has waited in the queue longer than the
 * queue bound; `get_render_status` reads the job afterwards. Cancelling or
 * rolling back the agent run cancels its jobs.
 *
 * `render_frames` (and the media tools' contact sheets) become agent frame
 * batches in the agent snapshot class, carrying the agent's revision
 * (read-your-writes). They run in the job engine, sliced while the
 * interactive engine plays, so they never pause or drop the user's playback.
 *
 * Definitions (contract 10): every job session main opens receives the
 * owning window's winning effect, transition, and layer registrations
 * (main's relay, `definitions.ts`) and the job engine acknowledges them
 * before the job or batch is submitted, so the job renders that window's
 * effects, forks and overrides by id included. A window's frame session
 * gets the current set again before a batch whenever the window pushed
 * since. Render queue jobs carry the set captured when they were queued.
 *
 * Capability routing (section 26, Phase 2): a project the engine cannot
 * render yet (`unsupported`), or whose media the host media store cannot
 * hand it (`mediaOffline`), comes back as `routed`, and the caller runs the
 * legacy exporter or capture instead.
 */
import { randomUUID } from 'node:crypto';
import path from 'node:path';

import { canonicalHash, canonicalProject, canonicalStringify } from '../../shared/canonical-json';
import {
  BUILT_IN_PRESETS,
  extensionOf,
  formatInfo,
  type DeliveryFormat,
  type DeliveryOptions,
  type ExportPreset,
  isDropFrameTimecode,
  isMov,
  isRenderFormat,
  migrateFormat,
  normalizeDelivery,
  outputDepth,
  supportsAlpha,
  wireAlpha,
  wireAudioCoding,
  wireColor,
  wireFormat
} from '../../shared/delivery';
import type {
  AgentFrameOutcome,
  DefinitionsApplied,
  DefinitionsUpdate,
  EngineError,
  EngineProcess,
  ExportSettings,
  JobState,
  JobStatus,
  ProjectColor,
  Request,
  Response
} from '../../shared/engine-protocol';
import { type SheetExportOptions, validSheetExportOptions } from '../../shared/engine-ipc';
import { resolveFrameRate, TICKS_PER_SECOND } from '../../shared/frame-rate';
import type { WindowDefinitions } from './definitions';
import { RemoteError } from './protocol';

/** What the agent jobs need from the engine host (NativeEngine). */
export interface AgentJobsEngine {
  request(process: EngineProcess, body: Request, session?: number | null, revision?: number | null): Promise<Response>;
  stageDocument(process: EngineProcess, session: number, bytes: Uint8Array): Promise<{ handle: string; length: number }>;
  listen(listener: { jobStatus?(status: JobStatus): void; status?(status: unknown): void }): () => void;
  isReady(process: EngineProcess): boolean;
  /** A user-chosen destination as a write handle for `job` on `session`,
   *  placed under `overwrite` (the engine's default for chosen files is replace). */
  chooseOutput?(file: string, job: string, session: number, overwrite?: 'uniqueName' | 'replace' | 'fail'): string;
  /** Marks a job session as a draft render (offline media from proxies). */
  setDraftSession?(session: number, draft: boolean): void;
}

/** Decision 1: Standard color, linear blending off (pm_protocol `ProjectColor::DEFAULT`). */
export const STANDARD_COLOR: ProjectColor = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };

/** In `wait` mode, a job queued longer than this answers `queued` (contract 12 invariant 8). */
export const DEFAULT_QUEUE_BOUND_MS = 60_000;
/** Resync bodies up to this size travel inline; larger ones through a staged cache file. */
export const INLINE_DOCUMENT_BYTES = 4 * 1024 * 1024;
/** Finished jobs kept for `get_render_status`. */
const FINISHED_KEPT = 64;

/** The owner of render queue jobs (no window). */
export const QUEUE_OWNER = -1;

/** Error codes that send work to the legacy engine instead of failing it. */
const ROUTED_CODES = new Set<EngineError['code']>(['unsupported', 'mediaOffline']);

export type ExportVideoInput = {
  name?: string;
  /** `mp4` (H.264) and `prores` (ProRes 4444) as before, or any delivery
   *  format id (`prores-hq`, `hevc`, `webm-vp9`, `png`, …). */
  format: string;
  /** A built-in preset (`master`, `master-alpha`, `web-h264`, `web-hevc`,
   *  `png`); an explicit format overrides its format. */
  preset?: string;
  range: 'all' | 'work';
  audio: boolean;
  motionBlur: boolean;
  alpha: boolean;
  mode?: 'wait' | 'async';
};

export type ExportVideoResult =
  | { status: 'done'; jobId: string; path: string; width: number; height: number; fps: number }
  | { status: 'queued'; jobId: string }
  | { status: 'routed'; reason: string };

export type RenderStatus = { jobId: string; state: JobState; progress: number; path?: string; error?: string };

/** An export from the export sheet or the render queue, as the renderer asks
 *  for it (shared/engine-ipc.ts `SheetExportOptions`). */
export type UserExportOptions = SheetExportOptions;

/** A preview of the latest encoded frame, for the progress sheet. */
export type ExportPreviewFrame = { frame: number; width: number; height: number; jpeg: Uint8Array };

/** A user export job's status as the window sees it; `routed` when the
 *  engine cannot render the project and the current exporter should. */
export type UserExportStatus = RenderStatus & { routed?: boolean; preview?: ExportPreviewFrame; waiting?: string };

export type AgentFrameImage = { time: number; data: Uint8Array; mimeType: 'image/jpeg' | 'image/png'; width: number; height: number };
export type AgentFramesResult =
  | { status: 'rendered'; images: AgentFrameImage[]; revision: number; queuedMs: number }
  | { status: 'routed'; reason: string };

/** The project as the agent sees it, and where the frames are. */
export type FramesPlan = { project: unknown; times: number[]; width: number; revision: number };

/** Agent-facing failure: the engine's one-sentence message. */
export class AgentJobError extends Error {
  override name = 'AgentJobError';
  constructor(message: string, readonly code: EngineError['code'] | 'internal' = 'internal') {
    super(message);
  }
}

type Project = Record<string, any>;

function asProject(value: unknown): Project {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new AgentJobError('The project could not be read.');
  const project = value as Project;
  if (typeof project['compId'] !== 'string' || !project['compId']) throw new AgentJobError('The project has no open composition.');
  return project;
}

const evenSize = (value: number): number => Math.max(2, Math.round(value / 2) * 2);
const toTicks = (seconds: number): number => Math.round(seconds * TICKS_PER_SECOND);

/** Agent preset names (`export_video` `preset`). */
export const AGENT_PRESETS: Readonly<Record<string, string>> = {
  master: 'builtin:master',
  'master-alpha': 'builtin:master-alpha',
  'web-h264': 'builtin:web-h264',
  'web-hevc': 'builtin:web-hevc',
  png: 'builtin:png'
};

/** An agent preset's built-in preset, or undefined for an unknown name. */
export function agentPreset(name: string): ExportPreset | undefined {
  const id = Object.prototype.hasOwnProperty.call(AGENT_PRESETS, name) ? AGENT_PRESETS[name] : undefined;
  return id ? BUILT_IN_PRESETS.find((p) => p.id === id) : undefined;
}

/** The format an agent export writes: the preset's, unless a format is given. */
function agentFormat(input: ExportVideoInput): { format: DeliveryFormat; delivery: DeliveryOptions } {
  const preset = input.preset ? agentPreset(input.preset) : undefined;
  if (input.preset && !preset) throw new AgentJobError(`Unknown export preset ${input.preset}.`);
  const delivery = normalizeDelivery(preset?.options.delivery ?? {});
  const format = input.format ? migrateFormat(input.format, 'h264') : preset?.options.format ?? 'h264';
  if (!isRenderFormat(format)) throw new AgentJobError('export_video writes video or image files; use save_project for a project file.');
  return { format, delivery };
}

/** The job engine's settings for a format and the sheet's delivery options:
 *  tags, audio coding, start timecode and reel, precision (exports render at
 *  32-bit, amendment 8), and dither (on for 8-bit outputs, section 17). */
export function wireSettings(input: {
  format: DeliveryFormat;
  delivery: DeliveryOptions;
  alpha: boolean;
  quality: 'draft' | 'high' | 'max';
  size: { width: number; height: number };
  rate: { num: number; den: number };
  start: number;
  end: number;
  audio: boolean;
  motionBlur: boolean;
  hideCaptions?: boolean;
  draft?: boolean;
  preview?: boolean;
  sequenceName?: string;
}): ExportSettings {
  const { format, delivery } = input;
  const image = format === 'png' || format === 'still';
  const coding = input.audio && !image ? wireAudioCoding(format, delivery) : null;
  const timecode = isMov(format, delivery) && delivery.timecode
    ? { start: delivery.timecode, dropFrame: isDropFrameTimecode(delivery.timecode), reel: delivery.reel || null }
    : null;
  return {
    format: wireFormat(format, delivery),
    alpha: wireAlpha(format, input.alpha, delivery),
    quality: input.quality,
    size: input.size,
    frameRate: { num: input.rate.num, den: input.rate.den },
    range: { start: toTicks(Math.max(0, input.start)), end: toTicks(input.end) },
    audio: input.audio && !image,
    motionBlur: input.motionBlur,
    color: wireColor(format, delivery),
    precision: 'float32',
    dither: outputDepth(format, delivery) <= 8,
    draftFromProxies: input.draft === true,
    delivery: {
      audioCoding: coding,
      timecode,
      hideCaptions: input.hideCaptions === true,
      preview: input.preview === true,
      sequenceName: format === 'png' ? input.sequenceName ?? null : null
    }
  };
}

/** `export_video`'s settings as the legacy exporter applies them (exporter.ts
 *  `run`, render-encoder.ts): the composition size at scale 1 rounded to even
 *  pixels, the composition rate, all or the work area, ProRes 4444 (straight
 *  alpha when asked) or H.264 at the High tier (16 Mbps), Rec.709 (QuickTime
 *  and Safari) tags; or a preset's or another delivery format's. Exports
 *  render at 32-bit precision (amendment 8 asks for it whenever a source
 *  exceeds 10 bits; agent exports do not inspect sources first), and 8-bit
 *  outputs are dithered (section 17). */
export function exportSettingsFor(projectValue: unknown, input: ExportVideoInput): ExportSettings {
  const project = asProject(projectValue);
  const rate = resolveFrameRate(project['fps'], project['fpsRational']);
  if (!rate) throw new AgentJobError('The project has no valid frame rate.');
  const width = Number(project['w']), height = Number(project['h']), dur = Number(project['dur']);
  if (!(width >= 2 && height >= 2 && dur > 0)) throw new AgentJobError('The project has no size or duration.');
  const work = project['work'];
  const [start, end] = input.range === 'work' && Array.isArray(work) && Number(work[1]) > Number(work[0])
    ? [Number(work[0]), Number(work[1])]
    : [0, dur];
  // `input.alpha` already holds the preset's default (exportVideoInput).
  const { format, delivery } = agentFormat(input);
  const alpha = input.alpha;
  if (alpha && !supportsAlpha(format, delivery)) {
    throw new AgentJobError('Transparent video requires ProRes 4444, HEVC (8-bit), WebM, or PNG.');
  }
  return wireSettings({
    format, delivery, alpha, quality: 'high',
    size: { width: evenSize(width), height: evenSize(height) },
    rate, start, end, audio: input.audio, motionBlur: input.motionBlur,
    sequenceName: suggestedName(project, input.name)
  });
}

/** The export sheet's and render queue's settings: the chosen format, size
 *  (scale or exact pixels; PNG takes odd sizes, video even), rate, range,
 *  quality tier, alpha, and delivery options. */
export function userExportSettings(projectValue: unknown, options: UserExportOptions, extra: { sequenceName?: string; preview?: boolean } = {}): ExportSettings {
  const project = asProject(projectValue);
  const o = options;
  if (!validSheetExportOptions(o)) throw new AgentJobError('Invalid export settings.');
  const format = migrateFormat(o.format);
  if (!isRenderFormat(format)) throw new AgentJobError('Code and project exports are not render jobs.');
  const delivery = normalizeDelivery(o.delivery ?? {});
  const rate = resolveFrameRate(o.fps, Number(project['fps']) === o.fps ? project['fpsRational'] : undefined);
  if (!rate) throw new AgentJobError('Invalid export frame rate.');
  const image = format === 'png' || format === 'still';
  const round = (v: number) => image ? Math.max(1, Math.round(v)) : evenSize(v);
  const size = o.width && o.height
    ? { width: round(o.width), height: round(o.height) }
    : { width: round(Number(project['w']) * o.scale), height: round(Number(project['h']) * o.scale) };
  const limit = image ? 16384 : 8192;
  if (size.width > limit || size.height > limit) throw new AgentJobError(`The export is larger than ${limit} pixels.`);
  if (o.alpha && !supportsAlpha(format, delivery)) throw new AgentJobError('This format cannot carry transparency.');
  return wireSettings({
    format, delivery, alpha: o.alpha, quality: o.quality, size, rate,
    start: o.start, end: format === 'still' ? Math.max(o.end, o.start + 1 / (rate.num / rate.den)) : o.end,
    audio: o.audio, motionBlur: o.motionBlur, hideCaptions: o.hideCaptions === true, draft: o.draft === true,
    ...(extra.preview ? { preview: true } : {}),
    ...(extra.sequenceName ? { sequenceName: extra.sequenceName } : {})
  });
}

/** The file name the agent asked for, or the project's, as a bare name part. */
export function suggestedName(project: Project, name: string | undefined): string {
  const raw = (name ?? project['name'] ?? 'Powermove').toString();
  let safe = raw.replace(/[\\/\x00-\x1f:]/g, '_').trim().replace(/^\.+$/, '_') || 'Powermove';
  while (Buffer.byteLength(safe) > 180) safe = Array.from(safe).slice(0, -1).join('');
  return safe;
}

const hexBytes = (hex: string): Uint8Array => new Uint8Array(Buffer.from(hex, 'hex'));

function errorText(error: EngineError | null | undefined): string {
  return error?.message || 'The export failed.';
}

function remote(error: unknown): EngineError | null {
  return error instanceof RemoteError ? error.error : null;
}

type JobRecord = {
  jobId: string;
  /** An export sheet job: its statuses go to the window. */
  user?: boolean;
  /** A render queue job: its statuses go to the queue. */
  queue?: boolean;
  owner: number;
  run: string;
  session: number;
  status: JobStatus;
  /** When the job entered the queue (ms), and when it started running. */
  queuedAt: number;
  startedAt: number | null;
  /** Runs once when the job starts running (`export_video`'s timeout). */
  onStarted?: (() => void) | undefined;
  name: string;
  size: { width: number; height: number };
  fps: number;
  changed: Array<() => void>;
};

type FrameSession = {
  session: number; revision: number; hash: string; generation: number;
  /** The relay version of the window's set the session holds, and its definitions generation. */
  definitions: { version: number; generation: number } | null;
};

export type AgentJobsOptions = {
  engine: AgentJobsEngine;
  /** Status of an export sheet job, for its window. */
  onUserStatus?(owner: number, status: UserExportStatus): void;
  /** Status of a render queue job (`exportQueued`). */
  onQueueStatus?(jobId: string, status: UserExportStatus): void;
  /** An agent export was submitted or changed (the render queue lists them). */
  onAgentJob?(job: { jobId: string; owner: number; name: string; summary: string; status: RenderStatus }): void;
  /** A job finished with a file at `path` (export sidecars remember it). */
  onDelivered?(owner: number, path: string): void;
  queueBoundMs?: number;
  now?: () => number;
  log?: (level: 'info' | 'warn' | 'error', message: string) => void;
  /** A window's current definition set (main's relay); null when it pushed none. */
  definitions?(owner: number): WindowDefinitions | null;
};

export class AgentJobs {
  private readonly jobs = new Map<string, JobRecord>();
  private readonly finished: string[] = [];
  /** One live-following job session per window for its agent frames. */
  private readonly frameSessions = new Map<number, Promise<FrameSession>>();
  /** Bumped when the job engine restarts: every session it had is gone. */
  private generation = 0;
  private readonly unlisten: () => void;
  private readonly queueBoundMs: number;
  private readonly now: () => number;

  constructor(private readonly options: AgentJobsOptions) {
    this.queueBoundMs = options.queueBoundMs ?? DEFAULT_QUEUE_BOUND_MS;
    this.now = options.now ?? Date.now;
    this.unlisten = options.engine.listen({
      jobStatus: (status) => this.onStatus(status),
      status: (status) => this.onEngineStatus(status)
    });
  }

  dispose(): void {
    this.unlisten();
  }

  /** Whether the job engine can take work now. */
  get available(): boolean {
    return this.options.engine.isReady('job');
  }

  /* ── export_video ──────────────────────────────────────────────────── */

  /** Runs an agent export. Resolves `routed` when the engine cannot render
   *  the project (the caller runs the legacy exporter); throws on failure. */
  async exportVideo(owner: number, run: string, projectValue: unknown, input: ExportVideoInput, onStarted?: () => void): Promise<ExportVideoResult> {
    const project = asProject(projectValue);
    const settings = exportSettingsFor(project, input);
    const { format, delivery } = agentFormat(input);
    // PNG sequences are folders (`png` names the frames' extension); the job
    // engine asks for a folder destination by the format.
    const extension = extensionOf(format, delivery) ?? 'png';
    const jobId = `agent-export-${randomUUID()}`;
    const session = await this.openSnapshot(project, `agent-export-${jobId}`, this.windowUpdates(owner));
    const record: JobRecord = {
      jobId, owner, run, session,
      status: { job: jobId, class: 'agentExport', state: 'queued', progress: 0, path: null, error: null },
      queuedAt: this.now(), startedAt: null, onStarted,
      name: suggestedName(project, input.name),
      size: settings.size,
      fps: Number(project['fps']) || settings.frameRate.num / settings.frameRate.den,
      changed: []
    };
    this.jobs.set(jobId, record);
    try {
      await this.options.engine.request('job', {
        type: 'submitJob',
        job: jobId,
        class: 'agentExport',
        run,
        spec: { kind: 'export', session, composition: project['compId'], settings },
        output: { kind: 'agentAutomatic', suggestedName: record.name, extension }
      });
    } catch (error) {
      this.jobs.delete(jobId);
      await this.closeSession(session);
      const e = remote(error);
      if (e && ROUTED_CODES.has(e.code)) return { status: 'routed', reason: e.message };
      throw new AgentJobError(e?.message ?? (error instanceof Error ? error.message : String(error)), e?.code ?? 'internal');
    }
    this.announceAgent(record, `${formatInfo(format).label} · ${settings.size.width}×${settings.size.height}`);
    return this.reply(record, input.mode ?? 'wait');
  }

  /** contract 12's `agent_export_reply`, waiting while it says to. */
  private async reply(record: JobRecord, mode: 'wait' | 'async'): Promise<ExportVideoResult> {
    for (;;) {
      const { status } = record;
      switch (status.state) {
        case 'done':
          if (!status.path) throw new AgentJobError('The export finished without a path.', 'internal');
          return { status: 'done', jobId: record.jobId, path: status.path, width: record.size.width, height: record.size.height, fps: record.fps };
        case 'failed':
          if (status.error && ROUTED_CODES.has(status.error.code)) return { status: 'routed', reason: status.error.message };
          throw new AgentJobError(errorText(status.error), status.error?.code ?? 'exportFailed');
        case 'cancelled':
          throw new AgentJobError('Export cancelled.', 'cancelled');
        default:
          break;
      }
      if (mode === 'async') return { status: 'queued', jobId: record.jobId };
      const waiting = status.state === 'queued' || status.state === 'waitingForMedia';
      if (waiting && this.now() - record.queuedAt > this.queueBoundMs) return { status: 'queued', jobId: record.jobId };
      await new Promise<void>((resolve) => {
        record.changed.push(resolve);
        // Re-check the queue bound while nothing changes.
        if (waiting) {
          const timer = setTimeout(resolve, Math.max(1, this.queueBoundMs - (this.now() - record.queuedAt) + 1));
          timer.unref?.();
        }
      });
    }
  }

  /** `get_render_status` for a job of this window's agents. */
  status(owner: number, jobId: string): RenderStatus {
    const record = this.jobs.get(jobId);
    if (!record || record.owner !== owner) throw new AgentJobError('No render job with that id belongs to this project window.');
    const { status } = record;
    return {
      jobId,
      state: status.state,
      progress: Math.max(0, Math.min(1, status.progress)),
      ...(status.state === 'done' && status.path ? { path: status.path } : {}),
      ...(status.state === 'failed' ? { error: errorText(status.error) } : {})
    };
  }

  /** Cancels every job of an agent run (the run was cancelled or rolled back). */
  async cancelRun(run: string): Promise<number> {
    const live = [...this.jobs.values()].some((r) => r.run === run && !terminal(r.status.state));
    const frames = this.frameRuns.has(run);
    if (!live && !frames) return 0;
    try {
      const response = await this.options.engine.request('job', { type: 'cancelJobs', kind: 'run', run });
      return response.type === 'jobsCancelled' ? response.jobs.length : 0;
    } catch (error) {
      this.options.log?.('warn', `cancelling the jobs of agent run ${run} failed: ${error instanceof Error ? error.message : String(error)}`);
      return 0;
    }
  }

  /** Agent jobs the render queue lists, with their summaries. */
  private readonly agentSummaries = new Map<string, string>();

  private announceAgent(record: JobRecord, summary: string): void {
    this.agentSummaries.set(record.jobId, summary);
    this.options.onAgentJob?.({ jobId: record.jobId, owner: record.owner, name: record.name, summary, status: this.status(record.owner, record.jobId) });
  }

  private onStatus(status: JobStatus): void {
    const record = this.jobs.get(status.job);
    if (!record) return;
    if (status.state === 'running' && record.startedAt === null) {
      record.startedAt = this.now();
      try {
        record.onStarted?.();
      } catch (error) {
        this.options.log?.('warn', `an export start callback failed: ${error instanceof Error ? error.message : String(error)}`);
      }
    }
    record.status = status;
    if (status.state === 'done' && status.path && !record.queue) this.options.onDelivered?.(record.owner, status.path);
    if (record.user || record.queue) {
      const routed = status.state === 'failed' && !!status.error && ROUTED_CODES.has(status.error.code);
      const preview = status.preview
        ? { frame: status.preview.frame, width: status.preview.width, height: status.preview.height, jpeg: new Uint8Array(status.preview.jpeg) }
        : undefined;
      const waiting = status.state === 'waitingForMedia' ? status.error?.message : undefined;
      const view: UserExportStatus = {
        ...this.status(record.owner, record.jobId),
        ...(routed ? { routed: true } : {}),
        ...(preview ? { preview } : {}),
        ...(waiting ? { waiting } : {})
      };
      if (record.queue) this.options.onQueueStatus?.(record.jobId, view);
      else this.options.onUserStatus?.(record.owner, view);
    } else if (this.agentSummaries.has(record.jobId)) {
      this.options.onAgentJob?.({ jobId: record.jobId, owner: record.owner, name: record.name, summary: this.agentSummaries.get(record.jobId)!, status: this.status(record.owner, record.jobId) });
      if (terminal(status.state)) this.agentSummaries.delete(record.jobId);
    }
    if (terminal(status.state)) {
      this.options.engine.setDraftSession?.(record.session, false);
      void this.closeSession(record.session);
      this.finished.push(record.jobId);
      while (this.finished.length > FINISHED_KEPT) this.jobs.delete(this.finished.shift()!);
    }
    const waiters = record.changed.splice(0);
    for (const wake of waiters) wake();
  }

  /** The job engine restarted: its sessions and unfinished jobs are gone
   *  (agent jobs never resume, section 17). */
  private onEngineStatus(status: unknown): void {
    const job = (status as { job?: { generation?: number } } | null)?.job;
    if (!job || typeof job.generation !== 'number' || job.generation === this.generation) return;
    const restarted = this.generation !== 0;
    this.generation = job.generation;
    this.frameSessions.clear();
    if (!restarted) return;
    for (const record of this.jobs.values()) {
      if (terminal(record.status.state)) continue;
      this.onStatus({
        ...record.status,
        state: 'failed',
        error: { code: 'crashAttributed', scope: { kind: 'job', job: record.jobId }, message: 'The job engine stopped during the export; run it again.', retry: { kind: 'never' }, detail: null }
      });
    }
  }

  /* ── export sheet ──────────────────────────────────────────────────── */

  /** Starts an export the user asked for in the export sheet, to the file
   *  (or, for PNG sequences, folder) they chose, replaced if it exists (the
   *  save panel confirmed that). Statuses go to `onUserStatus` with previews;
   *  an `unsupported` or `mediaOffline` failure arrives as `routed`, and the
   *  window runs the current exporter to the same file. */
  async exportUser(owner: number, projectValue: unknown, options: UserExportOptions, destination: string, extra: { draft?: boolean } = {}): Promise<{ jobId: string; frames: number }> {
    return this.submitUser(owner, projectValue, { ...options, ...(extra.draft ? { draft: true } : {}) }, destination, 'replace', false, this.windowUpdates(owner));
  }

  /** The window's set as the updates a fresh job session starts with (generation 1). */
  private windowUpdates(owner: number): DefinitionsUpdate[] {
    return this.options.definitions?.(owner)?.updates(1) ?? [];
  }

  /** Starts a render queue job (unattended, from its stored snapshot) to its
   *  destination under its overwrite policy. Statuses go to `onQueueStatus`. */
  async exportQueued(projectValue: unknown, options: UserExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', definitions: DefinitionsUpdate[] = []): Promise<{ jobId: string; frames: number }> {
    return this.submitUser(QUEUE_OWNER, projectValue, options, destination, overwrite, true, definitions);
  }

  private async submitUser(owner: number, projectValue: unknown, options: UserExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', queue: boolean, definitions: DefinitionsUpdate[]): Promise<{ jobId: string; frames: number }> {
    const project = asProject(projectValue);
    const format = migrateFormat(options.format);
    const sequenceName = format === 'png' ? path.basename(destination) : undefined;
    const settings = userExportSettings(project, options, { preview: !queue, ...(sequenceName ? { sequenceName } : {}) });
    if (!this.options.engine.chooseOutput) throw new AgentJobError('This host cannot write user exports.');
    const jobId = `${queue ? 'queue' : 'user'}-export-${randomUUID()}`;
    const session = await this.openSnapshot(project, `${queue ? 'queue' : 'user'}-export-${jobId}`, definitions);
    const record: JobRecord = {
      jobId, owner, run: '', session, ...(queue ? { queue: true } : { user: true }),
      status: { job: jobId, class: 'userJob', state: 'queued', progress: 0, path: null, error: null, preview: null },
      queuedAt: this.now(), startedAt: null,
      name: suggestedName(project, undefined),
      size: settings.size,
      fps: options.fps,
      changed: []
    };
    this.jobs.set(jobId, record);
    try {
      if (settings.draftFromProxies) this.options.engine.setDraftSession?.(session, true);
      const handle = this.options.engine.chooseOutput(destination, jobId, session, overwrite);
      await this.options.engine.request('job', {
        type: 'submitJob',
        job: jobId,
        class: 'userJob',
        run: null,
        spec: { kind: 'export', session, composition: project['compId'], settings },
        output: { kind: 'userChosen', destination: handle }
      });
    } catch (error) {
      this.jobs.delete(jobId);
      await this.closeSession(session);
      const e = remote(error);
      throw new AgentJobError(e?.message ?? (error instanceof Error ? error.message : String(error)), e?.code ?? 'internal');
    }
    const frames = format === 'still' ? 1 : Math.max(1, Math.round((options.end - options.start) * settings.frameRate.num / settings.frameRate.den));
    return { jobId, frames };
  }

  /** Cancels a render queue job. */
  async cancelQueued(jobId: string): Promise<void> {
    await this.cancelJob(QUEUE_OWNER, jobId);
  }

  /** Cancels one job of this window. */
  async cancelJob(owner: number, jobId: string): Promise<void> {
    const record = this.jobs.get(jobId);
    if (!record || record.owner !== owner || terminal(record.status.state)) return;
    await this.options.engine.request('job', { type: 'cancelJobs', kind: 'job', job: jobId }).catch(() => undefined);
  }

  /* ── render_frames ─────────────────────────────────────────────────── */

  private readonly frameRuns = new Map<string, number>();

  /** Renders agent frames at the agent's revision in the agent snapshot
   *  class. `routed` when the engine cannot draw the composition. */
  async renderFrames(owner: number, run: string, plan: FramesPlan): Promise<AgentFramesResult> {
    const project = asProject(plan.project);
    if (!plan.times.length) return { status: 'rendered', images: [], revision: plan.revision, queuedMs: 0 };
    const w = Number(project['w']), h = Number(project['h']);
    // `width` is today's output width; the engine takes the longest edge.
    const maxDimension = Math.max(1, Math.min(8192, Math.round(h > w && w > 0 ? plan.width * h / w : plan.width)));
    const frameSession = await this.frameSession(owner, project);
    this.frameRuns.set(run, (this.frameRuns.get(run) ?? 0) + 1);
    let response: Response;
    try {
      response = await this.options.engine.request('job', {
        type: 'agentFrames',
        run,
        frames: plan.times.map((time) => ({ composition: project['compId'], time: toTicks(time) })),
        maxDimension,
        format: { kind: 'jpeg', quality: 0.74 },
        motionBlur: false,
        blender: 'cachedOrPending'
      }, frameSession.session, frameSession.revision);
    } catch (error) {
      const e = remote(error);
      if (e?.code === 'unknownSession' || e?.code === 'staleRevision') this.frameSessions.delete(owner);
      if (e && ROUTED_CODES.has(e.code)) return { status: 'routed', reason: e.message };
      throw new AgentJobError(e?.message ?? (error instanceof Error ? error.message : String(error)), e?.code ?? 'internal');
    } finally {
      const left = (this.frameRuns.get(run) ?? 1) - 1;
      if (left) this.frameRuns.set(run, left); else this.frameRuns.delete(run);
    }
    if (response.type !== 'agentFrames') throw new AgentJobError('The job engine answered agent frames with something else.');
    return this.frameImages(plan.times, response.outcomes, response.queuedMs, frameSession.revision);
  }

  private frameImages(times: number[], outcomes: AgentFrameOutcome[], queuedMs: number, revision: number): AgentFramesResult {
    const images: AgentFrameImage[] = [];
    for (const outcome of outcomes) {
      if (outcome.status === 'failed') {
        if (ROUTED_CODES.has(outcome.error.code)) return { status: 'routed', reason: outcome.error.message };
        throw new AgentJobError(outcome.error.message, outcome.error.code);
      }
      if (outcome.image.kind !== 'inline') throw new AgentJobError('The job engine answered with an image file; agent frames expect inline images.');
      images.push({ time: times[outcome.index] ?? 0, data: outcome.image.bytes, mimeType: 'image/jpeg', width: outcome.width, height: outcome.height });
    }
    return { status: 'rendered', images, revision, queuedMs };
  }

  /** The window's frame session, resynced when the project changed. */
  private frameSession(owner: number, project: Project): Promise<FrameSession> {
    const canonical = canonicalProject(project);
    const hash = canonicalHash(canonical);
    const previous = this.frameSessions.get(owner);
    const next = (async (): Promise<FrameSession> => {
      const current = await previous?.catch(() => null);
      if (current && current.generation === this.generation && current.hash === hash) return this.currentDefinitions(owner, current);
      if (current && current.generation === this.generation) {
        const revision = current.revision + 1;
        await this.resync(current.session, `agent-frames-${owner}`, canonical, hash, revision);
        return this.currentDefinitions(owner, { ...current, revision, hash });
      }
      const generation = this.generation;
      const session = await this.openSession(`agent-frames-${owner}`, 'job');
      await this.resync(session, `agent-frames-${owner}`, canonical, hash, 1);
      return this.currentDefinitions(owner, { session, revision: 1, hash, generation, definitions: null });
    })();
    this.frameSessions.set(owner, next);
    next.catch(() => { if (this.frameSessions.get(owner) === next) this.frameSessions.delete(owner); });
    return next;
  }

  /**
   * Brings a frame session to the window's current definitions: when the
   * window pushed since the session last got them (a registration, a
   * publication, a dispose, a reload), the whole set again at the next
   * generation, acknowledged before the batch is asked for. Runs in the
   * window's frame-session chain, so pushes never race.
   */
  private async currentDefinitions(owner: number, frames: FrameSession): Promise<FrameSession> {
    const window = this.options.definitions?.(owner);
    if (!window || frames.definitions?.version === window.version) return frames;
    const from = (frames.definitions?.generation ?? 0) + 1;
    const updates = window.updates(from);
    await this.pushDefinitions(frames.session, updates);
    return { ...frames, definitions: { version: window.version, generation: updates.at(-1)!.generation } };
  }

  /** Sends `updates` to a job session in order; each is acknowledged
   *  (`definitionsApplied`) before the next, and before the caller goes on. */
  private async pushDefinitions(session: number, updates: DefinitionsUpdate[]): Promise<void> {
    for (const update of updates) {
      let answer: Response;
      try {
        answer = await this.options.engine.request('job', { type: 'pushDefinitions', ...update }, session);
      } catch (error) {
        const e = remote(error);
        throw new AgentJobError(`The job engine refused the window's definitions: ${e?.message ?? (error instanceof Error ? error.message : String(error))}`, e?.code ?? 'internal');
      }
      if (answer.type !== 'definitionsApplied' || answer.generation !== update.generation) {
        throw new AgentJobError('The job engine did not acknowledge the definitions.');
      }
    }
  }

  /**
   * Compiles definitions through the job engine's GLSL pipeline (and their
   * GPU pipelines) without registering them anywhere: a scratch session
   * receives them and its acknowledgment carries their failures
   * (`validate_effect`).
   */
  async compileDefinitions(snapshots: DefinitionsUpdate['upserts']): Promise<DefinitionsApplied['failures']> {
    const session = await this.openSession(`validate-${randomUUID()}`, 'job');
    try {
      let answer: Response;
      try {
        answer = await this.options.engine.request('job', { type: 'pushDefinitions', generation: 1, full: true, upserts: snapshots, removals: [] }, session);
      } catch (error) {
        const e = remote(error);
        throw new AgentJobError(e?.message ?? (error instanceof Error ? error.message : String(error)), e?.code ?? 'internal');
      }
      if (answer.type !== 'definitionsApplied') throw new AgentJobError('The job engine did not compile the definition.');
      return answer.failures;
    } finally {
      await this.closeSession(session);
    }
  }

  /** The window closed: its frame session goes. */
  async forgetOwner(owner: number): Promise<void> {
    const pending = this.frameSessions.get(owner);
    this.frameSessions.delete(owner);
    const current = await pending?.catch(() => null);
    if (current && current.generation === this.generation) await this.closeSession(current.session);
  }

  /* ── sessions ──────────────────────────────────────────────────────── */

  private async openSession(document: string, role: 'job' | 'jobSnapshot'): Promise<number> {
    if (!this.available) throw new AgentJobError('The job engine is not running.', 'internal');
    const opened = await this.options.engine.request('job', { type: 'openSession', document, role, process: 'job', color: STANDARD_COLOR });
    if (opened.type !== 'sessionOpened') throw new AgentJobError('The job engine did not open a session.');
    return opened.session;
  }

  /** A frozen snapshot of the project for one export, with the definitions it renders with. */
  private async openSnapshot(project: Project, document: string, definitions: DefinitionsUpdate[]): Promise<number> {
    const canonical = canonicalProject(project);
    const session = await this.openSession(document, 'jobSnapshot');
    try {
      await this.resync(session, document, canonical, canonicalHash(canonical), 1);
      await this.pushDefinitions(session, definitions);
    } catch (error) {
      await this.closeSession(session);
      throw error;
    }
    return session;
  }

  private async resync(session: number, document: string, canonical: unknown, hash: string, revision: number): Promise<void> {
    const text = canonicalStringify(canonical);
    if (text === undefined) throw new AgentJobError('The project could not be serialized.');
    const body = new TextEncoder().encode(text);
    const source = body.byteLength <= INLINE_DOCUMENT_BYTES
      ? { kind: 'inline' as const, body }
      : { kind: 'cacheFile' as const, ...(await this.options.engine.stageDocument('job', session, body)) };
    try {
      await this.options.engine.request('job', { type: 'resync', document, revision, encoding: 'canonicalJsonV1', source, hash: hexBytes(hash) }, session);
    } catch (error) {
      const e = remote(error);
      throw new AgentJobError(`The job engine could not read the project: ${e?.message ?? (error instanceof Error ? error.message : String(error))}`, e?.code ?? 'internal');
    }
  }

  private async closeSession(session: number): Promise<void> {
    await this.options.engine.request('job', { type: 'closeSession' }, session).catch(() => undefined);
  }
}

function terminal(state: JobState): boolean {
  return state === 'done' || state === 'failed' || state === 'cancelled';
}
