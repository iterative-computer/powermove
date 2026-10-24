import { fileAssets } from '../../shared/project-assets';
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
 * With the new engine enabled, failures stay named failures. No job or
 * batch is handed to the legacy renderer.
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
  CompatFrameRequest,
  CompatSource,
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
  canRequest?(process: EngineProcess): boolean;
  ensureReady?(process: EngineProcess): Promise<void>;
  holdJobRun?(run: string): void;
  releaseJobRun?(run: string): void;
  /** A user-chosen destination as a write handle for `job` on `session`,
   *  placed under `overwrite` (the engine's default for chosen files is replace). */
  chooseOutput?(file: string, job: string, session: number, overwrite?: 'uniqueName' | 'replace' | 'fail'): string;
  /** Starts only the hidden WebM encoder window while a user snapshot opens. */
  warmExportEncoder?(): void;
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

/** How long agent work waits for a starting or restarting job engine: the
 *  longest crash backoff (30 s, crash.ts) plus a launch. A stopped engine
 *  (setting off, or one that cannot run here) fails at once. */
export const ENGINE_READY_WAIT_MS = 45_000;

/** The owner of render queue jobs (no window). */
export const QUEUE_OWNER = -1;

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
  | { status: 'queued'; jobId: string };

export type RenderStatus = { jobId: string; state: JobState; progress: number; path?: string; error?: string };

/** An export from the export sheet or the render queue, as the renderer asks
 *  for it (shared/engine-ipc.ts `SheetExportOptions`). */
export type UserExportOptions = SheetExportOptions;

/** A preview of the latest encoded frame, for the progress sheet. */
export type ExportPreviewFrame = { frame: number; width: number; height: number; jpeg: Uint8Array };

/** A user export job's status as the window sees it. `routed` remains in
 * the shared legacy wire shape until the derouter removes that field; this
 * service never emits it. */
export type UserExportStatus = RenderStatus & { routed?: boolean; preview?: ExportPreviewFrame; waiting?: string };

export type AgentFrameImage = { time: number; data: Uint8Array; mimeType: 'image/jpeg' | 'image/png'; width: number; height: number };
export type AgentFramesResult =
  | { status: 'rendered'; images: AgentFrameImage[]; revision: number; queuedMs: number };

/** How one agent frame batch may end early. With `signal`, aborting it
 *  cancels this batch only (never the run's exports) and the batch answers
 *  with the frames that finished before the cancel: always the first ones,
 *  in order (a contact sheet at its call deadline, section 20). */
export type FramesOptions = { signal?: AbortSignal };

/** The project as the agent sees it, and where the frames are. */
export type FramesPlan = { project: unknown; times: number[]; width: number; revision: number; imageFormat?: 'png' };

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
 *  tags, audio coding, start timecode and reel, a Float32 wire hint (the
 *  project's colorDepth selects working precision), and dither for outputs
 *  at 8 bits or less. */
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
  audioIndexPlayhead?: number;
  motionBlur: boolean;
  hideCaptions?: boolean;
  draft?: boolean;
  preview?: boolean;
  sequenceName?: string;
  compat?: CompatSource[];
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
    ...(input.audioIndexPlayhead !== undefined ? { audioIndexPlayhead: toTicks(input.audioIndexPlayhead) } : {}),
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
      sequenceName: format === 'png' ? input.sequenceName ?? null : null,
      // The exporting window's compatibility producers (a raster override).
      ...(Array.isArray(input.compat) && input.compat.length ? { compat: input.compat } : {})
    }
  };
}

/** `export_video`'s settings as the legacy exporter applies them (exporter.ts
 *  `run`, render-encoder.ts): the composition size at scale 1 rounded to even
 *  pixels, the composition rate, all or the work area, ProRes 4444 (straight
 *  alpha when asked) or H.264 at the High tier (16 Mbps), Rec.709 (QuickTime
 *  and Safari) tags; or a preset's or another delivery format's. Exports
 *  use the project's colorDepth despite the Float32 wire hint, and delivery
 *  formats at 8 bits or less are dithered (section 17). */
export function exportSettingsFor(projectValue: unknown, input: ExportVideoInput, compat: readonly CompatSource[] = []): ExportSettings {
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
    sequenceName: suggestedName(project, input.name),
    // The window's compatibility producers: the job asks it for those layers.
    compat: compatSources(compat)
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
    audio: o.audio, audioIndexPlayhead: o.audioIndexPlayhead, motionBlur: o.motionBlur, hideCaptions: o.hideCaptions === true, draft: o.draft === true,
    ...(Array.isArray(o.compat) ? { compat: compatSources(o.compat) } : {}),
    ...(extra.preview ? { preview: true } : {}),
    ...(extra.sequenceName ? { sequenceName: extra.sequenceName } : {})
  });
}

/** A layer object of a project (its root composition or a nested one) by id. */
export function snapshotLayer(project: Project, composition: string, layer: string): unknown {
  const root = String(project['compId'] ?? project['id'] ?? '');
  const comps = project['comps'] && typeof project['comps'] === 'object' ? project['comps'] as Record<string, { layers?: unknown }> : {};
  const lists: unknown[] = [];
  if (composition === root) lists.push(project['layers']);
  if (comps[composition]) lists.push(comps[composition]!.layers);
  for (const list of lists) {
    if (!Array.isArray(list)) continue;
    const found = list.find((l) => !!l && typeof l === 'object' && (l as Record<string, unknown>)['id'] === layer);
    if (found) return found;
  }
  return null;
}

/** The compatibility producers a window named (unknown values dropped). */
export function compatSources(value: unknown): CompatSource[] {
  if (!Array.isArray(value)) return [];
  const out: CompatSource[] = [];
  for (const s of value) if ((s === 'rasterOverride' || s === 'compatContext') && !out.includes(s)) out.push(s);
  return out;
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
  /** The project as the job's snapshot holds it (compatibility frames name its layers). */
  project?: Project;
};

type FrameSession = {
  session: number; revision: number; hash: string; generation: number;
  /** The relay version of the window's set the session holds, and its definitions generation. */
  definitions: { version: number; generation: number } | null;
};

export type AgentJobsOptions = {
  engine: AgentJobsEngine;
  /** Pins media for each export snapshot until its session closes. */
  bindMediaSession?(session: number, project: Project | null): Promise<void>;
  /** Status of an export sheet job, for its window. */
  onUserStatus?(owner: number, status: UserExportStatus): void;
  /** Status of a render queue job (`exportQueued`). */
  onQueueStatus?(jobId: string, status: UserExportStatus): void;
  /** An agent export was submitted or changed (the render queue lists them). */
  onAgentJob?(job: { jobId: string; owner: number; name: string; summary: string; status: RenderStatus }): void;
  /** A job finished with a file at `path` (export sidecars remember it). */
  onDelivered?(owner: number, path: string): void;
  /** The window a job engine session is for (`null`: no window, or the
   *  session closed): its text measures with that window's font cache
   *  seeds (engine-fonts-seeds.ts). Awaited before the session renders. */
  sessionOwner?(session: number, owner: number | null): Promise<unknown>;
  packProject?(project: Project, session: number, document: unknown): Promise<string>;
  queueBoundMs?: number;
  /** Bounded wait for a starting/restarting job engine (`ENGINE_READY_WAIT_MS`). */
  readyWaitMs?: number;
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
  /** Owners whose frame sessions a run has used. A shared owner stays open
   *  until its last run finishes. */
  private readonly runOwners = new Map<string, Set<number>>();
  private readonly finishedRuns = new Set<string>();
  private readonly preparingRuns = new Map<string, Promise<void>>();
  private readonly heldRuns = new Set<string>();
  /** The window each agent frame session works for (by job engine session). */
  private readonly frameOwners = new Map<number, number>();

  /**
   * The window a job engine session works for: an export's or an agent
   * frame batch's (the render queue's own jobs: `QUEUE_OWNER`); `null` when
   * no job or frame session of this launch has it.
   */
  ownerOf(session: number): number | null {
    for (const record of this.jobs.values()) if (record.session === session) return record.owner;
    return this.frameOwners.get(session) ?? null;
  }
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
    for (const run of this.heldRuns) this.options.engine.releaseJobRun?.(run);
    this.heldRuns.clear();
  }

  /** Whether the job engine can take work now. */
  get available(): boolean {
    return this.options.engine.canRequest?.('job') ?? this.options.engine.isReady('job');
  }

  /** Begin the separate render utility while an agent is setting up its
   *  project run. The caller does not wait: the first render still awaits
   *  readiness and receives a startup failure through the normal path. */
  async prepareRun(run: string): Promise<void> {
    this.heldRuns.add(run);
    this.options.engine.holdJobRun?.(run);
    const pending = this.preparingRuns.get(run);
    if (pending) return pending;
    const start = this.options.engine.ensureReady?.('job') ?? Promise.resolve();
    this.preparingRuns.set(run, start);
    try {
      await start;
    } catch (error) {
      this.releaseRunHold(run);
      throw error;
    } finally {
      if (this.preparingRuns.get(run) === start) this.preparingRuns.delete(run);
    }
  }

  private releaseRunHold(run: string): void {
    if (!this.heldRuns.delete(run)) return;
    this.options.engine.releaseJobRun?.(run);
  }

  /** End a committed run's warm hold and close its idle frame sessions.
   *  Accepted async exports retain their own job sessions. */
  async finishRun(run: string): Promise<void> {
    this.releaseRunHold(run);
    this.finishedRuns.add(run);
    await this.closeRunFrames(run);
  }

  private nextSaveSession = -1;

  async saveProject(projectValue: unknown, document: unknown = projectValue): Promise<string> {
    if (!this.options.packProject) throw new AgentJobError('project-save-unavailable: The host project packer is unavailable.');
    const project = asProject(projectValue);
    const session = this.nextSaveSession--;
    try {
      await this.options.bindMediaSession?.(session, { ...project, assets: fileAssets(document) });
      return await this.options.packProject(project, session, document);
    } finally { await this.options.bindMediaSession?.(session, null); }
  }

  /**
   * The layer a job's compatibility frame names, as the job's snapshot
   * holds it (JSON): the window draws that layer rather than its live one,
   * so an export (and a render queue job started later) draws the project
   * as it was exported, as today's exporter and queue did. Null when the
   * session is not a job of these or the layer is not in its snapshot.
   */
  compatLayer(session: number | null, request: CompatFrameRequest): string | null {
    if (session === null) return null;
    for (const record of this.jobs.values()) {
      if (record.session !== session || !record.project) continue;
      const layer = snapshotLayer(record.project, String(request.composition), String(request.layer));
      return layer === null ? null : JSON.stringify(layer);
    }
    return null;
  }

  /* ── export_video ──────────────────────────────────────────────────── */

  /** Runs an agent export; throws a named engine failure on refusal. */
  async exportVideo(owner: number, run: string, projectValue: unknown, input: ExportVideoInput, onStarted?: () => void, compat: readonly CompatSource[] = []): Promise<ExportVideoResult> {
    const project = asProject(projectValue);
    const settings = exportSettingsFor(project, input, compat);
    const { format, delivery } = agentFormat(input);
    // PNG sequences are folders (`png` names the frames' extension); the job
    // engine asks for a folder destination by the format.
    const extension = extensionOf(format, delivery) ?? 'png';
    const jobId = `agent-export-${randomUUID()}`;
    const session = await this.openSnapshot(project, `agent-export-${jobId}`, owner, this.windowUpdates(owner));
    const record: JobRecord = {
      jobId, owner, run, session,
      status: { job: jobId, class: 'agentExport', state: 'queued', progress: 0, path: null, error: null },
      queuedAt: this.now(), startedAt: null, onStarted,
      name: suggestedName(project, input.name),
      size: settings.size,
      fps: Number(project['fps']) || settings.frameRate.num / settings.frameRate.den,
      changed: [],
      ...(settings.delivery?.compat?.length ? { project } : {})
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
    // Frame batches carry their own wire run (one per batch), so a batch can
    // be cancelled alone; the agent run's cancel reaches every one of them.
    // No live work means no request: a retired job utility is not restarted
    // just to cancel nothing.
    const wires = [...(live ? [run] : []), ...(this.frameRuns.get(run) ?? [])];
    const counts = await Promise.all(wires.map((wire) => this.cancelWireRun(wire, run)));
    await this.finishRun(run);
    return counts.reduce((sum, count) => sum + count, 0);
  }

  private async cancelWireRun(wire: string, run: string): Promise<number> {
    try {
      const response = await this.options.engine.request('job', { type: 'cancelJobs', kind: 'run', run: wire });
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
      const preview = status.preview
        ? { frame: status.preview.frame, width: status.preview.width, height: status.preview.height, jpeg: new Uint8Array(status.preview.jpeg) }
        : undefined;
      const waiting = status.state === 'waitingForMedia' ? status.error?.message : undefined;
      const view: UserExportStatus = {
        ...this.status(record.owner, record.jobId),
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
    const job = (status as { job?: { generation?: number; state?: string } } | null)?.job;
    if (typeof job?.state === 'string') this.jobState = job.state;
    if (!job || typeof job.generation !== 'number' || job.generation === this.generation) return;
    const restarted = this.generation !== 0;
    this.generation = job.generation;
    this.frameSessions.clear();
    this.frameOwners.clear();
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
   *  engine failures are delivered by name without changing renderers. */
  async exportUser(owner: number, projectValue: unknown, options: UserExportOptions, destination: string, extra: { draft?: boolean } = {}): Promise<{ jobId: string; frames: number }> {
    return this.submitUser(owner, projectValue, { ...options, ...(extra.draft ? { draft: true } : {}) }, destination, 'replace', false, this.windowUpdates(owner));
  }

  /** The window's set as the updates a fresh job session starts with (generation 1). */
  private windowUpdates(owner: number): DefinitionsUpdate[] {
    return this.options.definitions?.(owner)?.updates(1) ?? [];
  }

  /** Starts a render queue job (unattended, from its stored snapshot) to its
   *  destination under its overwrite policy. Statuses go to `onQueueStatus`. */
  async exportQueued(projectValue: unknown, options: UserExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', definitions: DefinitionsUpdate[] = [], seedOwner: number | null = null): Promise<{ jobId: string; frames: number }> {
    return this.submitUser(QUEUE_OWNER, projectValue, options, destination, overwrite, true, definitions, seedOwner);
  }

  private async submitUser(owner: number, projectValue: unknown, options: UserExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', queue: boolean, definitions: DefinitionsUpdate[], seedOwner: number | null = null): Promise<{ jobId: string; frames: number }> {
    const project = asProject(projectValue);
    const format = migrateFormat(options.format);
    const sequenceName = format === 'png' ? path.basename(destination) : undefined;
    const settings = userExportSettings(project, options, { preview: !queue, ...(sequenceName ? { sequenceName } : {}) });
    if (!this.options.engine.chooseOutput) throw new AgentJobError('This host cannot write user exports.');
    if (settings.format.kind === 'webm' && settings.alpha === 'none') this.options.engine.warmExportEncoder?.();
    const jobId = `${queue ? 'queue' : 'user'}-export-${randomUUID()}`;
    // Queue ownership is for status delivery; seed ownership follows the
    // window that queued this snapshot while that renderer still exists.
    const session = await this.openSnapshot(project, `${queue ? 'queue' : 'user'}-export-${jobId}`, queue ? seedOwner : owner, definitions);
    const record: JobRecord = {
      jobId, owner, run: '', session, ...(queue ? { queue: true } : { user: true }),
      status: { job: jobId, class: 'userJob', state: 'queued', progress: 0, path: null, error: null, preview: null },
      queuedAt: this.now(), startedAt: null,
      name: suggestedName(project, undefined),
      size: settings.size,
      fps: options.fps,
      changed: [],
      ...(settings.delivery?.compat?.length ? { project } : {})
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

  /** Agent run -> the wire runs of its frame batches in flight. */
  private readonly frameRuns = new Map<string, Set<string>>();
  /** Agent run -> its renderFrames calls in progress (startup included). */
  private readonly frameCalls = new Map<string, number>();
  private frameBatch = 0;

  /** Renders agent frames at the agent's revision in the agent snapshot
   *  class. Every failure is named; playback is never paused. */
  async renderFrames(owner: number, run: string, plan: FramesPlan, options: FramesOptions = {}): Promise<AgentFramesResult> {
    const project = asProject(plan.project);
    if (!plan.times.length) return { status: 'rendered', images: [], revision: plan.revision, queuedMs: 0 };
    const owners = this.runOwners.get(run) ?? new Set<number>();
    owners.add(owner);
    this.runOwners.set(run, owners);
    // Count the whole call, including startup and session creation: a run
    // finishing during either must not close the session underneath it.
    this.frameCalls.set(run, (this.frameCalls.get(run) ?? 0) + 1);
    try {
      return await this.renderFrameBatch(owner, run, project, plan, options);
    } finally {
      const left = (this.frameCalls.get(run) ?? 1) - 1;
      if (left) this.frameCalls.set(run, left); else this.frameCalls.delete(run);
      if (!left && this.finishedRuns.has(run)) await this.closeRunFrames(run);
    }
  }

  private async renderFrameBatch(owner: number, run: string, project: Project, plan: FramesPlan, options: FramesOptions): Promise<AgentFramesResult> {
    const { signal } = options;
    if (signal?.aborted) return { status: 'rendered', images: [], revision: plan.revision, queuedMs: 0 };
    // Startup announces its generation and invalidates old frame sessions.
    // Wait before installing the first cached session, so it is not discarded.
    await this.options.engine.ensureReady?.('job');
    const w = Number(project['w']), h = Number(project['h']);
    // `width` is today's output width; the engine takes the longest edge.
    const maxDimension = Math.max(1, Math.min(8192, Math.round(h > w && w > 0 ? plan.width * h / w : plan.width)));
    if (signal?.aborted) return { status: 'rendered', images: [], revision: plan.revision, queuedMs: 0 };
    const frameSession = await this.frameSession(owner, project);
    const wire = `${run}#frames-${++this.frameBatch}`;
    const wires = this.frameRuns.get(run) ?? new Set<string>();
    wires.add(wire);
    this.frameRuns.set(run, wires);
    let aborted = false;
    const onAbort = () => {
      aborted = true;
      void this.cancelWireRun(wire, run);
    };
    let response: Response;
    try {
      if (signal?.aborted) onAbort();
      else signal?.addEventListener('abort', onAbort, { once: true });
      response = await this.options.engine.request('job', {
        type: 'agentFrames',
        run: wire,
        frames: plan.times.map((time) => ({ composition: project['compId'], time: toTicks(time) })),
        maxDimension,
        format: plan.imageFormat === 'png' ? { kind: 'png' } : { kind: 'jpeg', quality: 0.74 },
        motionBlur: false,
        blender: 'cachedOrPending'
      }, frameSession.session, frameSession.revision);
    } catch (error) {
      const e = remote(error);
      if (e?.code === 'unknownSession' || e?.code === 'staleRevision') this.frameSessions.delete(owner);
      throw new AgentJobError(e?.message ?? (error instanceof Error ? error.message : String(error)), e?.code ?? 'internal');
    } finally {
      signal?.removeEventListener('abort', onAbort);
      const left = this.frameRuns.get(run);
      left?.delete(wire);
      if (left && !left.size) this.frameRuns.delete(run);
    }
    if (response.type !== 'agentFrames') throw new AgentJobError('The job engine answered agent frames with something else.');
    return this.frameImages(plan.times, response.outcomes, response.queuedMs, frameSession.revision, aborted, plan.imageFormat === 'png' ? 'image/png' : 'image/jpeg');
  }

  private async closeRunFrames(run: string): Promise<void> {
    if (this.frameCalls.has(run)) return;
    const owners = this.runOwners.get(run);
    this.runOwners.delete(run);
    this.finishedRuns.delete(run);
    for (const owner of owners ?? []) {
      if ([...this.runOwners.values()].some((other) => other.has(owner))) continue;
      await this.forgetOwner(owner);
    }
  }

  /** The batch's images in request order. A batch cancelled through its
   *  signal answers with the frames before the first cancelled one; any other
   *  failure (and a cancel the caller did not ask for) is named. */
  private frameImages(times: number[], outcomes: AgentFrameOutcome[], queuedMs: number, revision: number, aborted = false, mimeType: AgentFrameImage['mimeType'] = 'image/jpeg'): AgentFramesResult {
    const images: AgentFrameImage[] = new Array(times.length);
    const cancelled = new Set<number>();
    if (outcomes.length !== times.length) throw new AgentJobError('The job engine did not render every requested frame.');
    for (const outcome of outcomes) {
      if (!Number.isInteger(outcome.index) || outcome.index < 0 || outcome.index >= times.length || images[outcome.index] || cancelled.has(outcome.index)) {
        throw new AgentJobError('The job engine returned an invalid or duplicate frame index.');
      }
      if (outcome.status === 'failed') {
        if (aborted && outcome.error.code === 'cancelled') { cancelled.add(outcome.index); continue; }
        throw new AgentJobError(outcome.error.message, outcome.error.code);
      }
      if (outcome.image.kind !== 'inline') throw new AgentJobError('The job engine answered with an image file; agent frames expect inline images.');
      images[outcome.index] = { time: times[outcome.index]!, data: outcome.image.bytes, mimeType, width: outcome.width, height: outcome.height };
    }
    if (!cancelled.size) return { status: 'rendered', images, revision, queuedMs };
    const first = Math.min(...cancelled);
    return { status: 'rendered', images: images.slice(0, first), revision, queuedMs };
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
        await this.options.bindMediaSession?.(current.session, project);
        await this.resync(current.session, `agent-frames-${owner}`, canonical, hash, revision);
        return this.currentDefinitions(owner, { ...current, revision, hash });
      }
      const generation = this.generation;
      const session = await this.openSession(`agent-frames-${owner}`, 'job', owner);
      this.frameOwners.set(session, owner);
      try {
        await this.options.bindMediaSession?.(session, project);
        await this.resync(session, `agent-frames-${owner}`, canonical, hash, 1);
        return await this.currentDefinitions(owner, { session, revision: 1, hash, generation, definitions: null });
      } catch (error) {
        await this.closeSession(session);
        throw error;
      }
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
    const session = await this.openSession(`validate-${randomUUID()}`, 'job', null);
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
    if (current) {
      this.frameOwners.delete(current.session);
      if (current.generation === this.generation) await this.closeSession(current.session);
    }
  }

  /* ── sessions ──────────────────────────────────────────────────────── */

  /** The job engine's last reported supervisor state (null: none reported yet). */
  private jobState: string | null = null;

  /** Resolves once the job engine is ready. A starting or restarting engine
   *  (crash backoff) is waited for within `readyWaitMs`; a stopped one, or
   *  one still down at the bound, is the named error. */
  private whenAvailable(): Promise<void> {
    if (this.available) return Promise.resolve();
    const fail = () => new AgentJobError('The job engine is not running.', 'internal');
    if (this.jobState !== 'starting' && this.jobState !== 'restarting') return Promise.reject(fail());
    return new Promise<void>((resolve, reject) => {
      const done = (error?: AgentJobError) => { clearTimeout(timer); off(); error ? reject(error) : resolve(); };
      const timer = setTimeout(() => done(fail()), this.options.readyWaitMs ?? ENGINE_READY_WAIT_MS);
      const off = this.options.engine.listen({ status: () => {
        if (this.available) done();
        else if (this.jobState === 'stopped') done(fail());
      } });
    });
  }

  private async openSession(document: string, role: 'job' | 'jobSnapshot', owner: number | null): Promise<number> {
    // A retired job utility starts on demand; a fatal startup or a crash
    // still down at the bound is the named error below.
    await this.options.engine.ensureReady?.('job').catch(() => undefined);
    await this.whenAvailable();
    const opened = await this.options.engine.request('job', { type: 'openSession', document, role, process: 'job', color: STANDARD_COLOR });
    if (opened.type !== 'sessionOpened') throw new AgentJobError('The job engine did not open a session.');
    if (owner !== null) {
      try {
        await this.ownSession(opened.session, owner);
      } catch (error) {
        await this.closeSession(opened.session);
        throw new AgentJobError(`The job engine could not lay text out as this window does, so the export did not start: ${error instanceof Error ? error.message : String(error)}`, 'internal');
      }
    }
    return opened.session;
  }

  /** Tells the job engine's font service whose window a session is for,
   *  once more if it did not answer: without it the session would measure
   *  text with every window's seeds, which can differ from this window's
   *  layout, so the job does not run. */
  private async ownSession(session: number, owner: number): Promise<void> {
    try {
      await this.options.sessionOwner?.(session, owner);
    } catch (error) {
      this.options.log?.('warn', `font cache seeds for session ${session}: ${error instanceof Error ? error.message : String(error)}; trying again`);
      await this.options.sessionOwner?.(session, owner);
    }
  }

  /** Forgets a closed session's window; failures only log. */
  private async sessionOwner(session: number, owner: null): Promise<void> {
    try {
      await this.options.sessionOwner?.(session, owner);
    } catch (error) {
      this.options.log?.('warn', `font cache seeds for session ${session}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  /** A frozen snapshot of the project for one export, with the definitions it renders with. */
  private async openSnapshot(project: Project, document: string, owner: number | null, definitions: DefinitionsUpdate[]): Promise<number> {
    const canonical = canonicalProject(project);
    const session = await this.openSession(document, 'jobSnapshot', owner);
    try {
      await this.options.bindMediaSession?.(session, project);
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
    this.frameOwners.delete(session);
    void this.sessionOwner(session, null);
    try {
      await this.options.bindMediaSession?.(session, null);
    } finally {
      await this.options.engine.request('job', { type: 'closeSession' }, session).catch(() => undefined);
    }
  }
}

function terminal(state: JobState): boolean {
  return state === 'done' || state === 'failed' || state === 'cancelled';
}
