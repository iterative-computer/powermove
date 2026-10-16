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
 * Capability routing (section 26, Phase 2): a project the engine cannot
 * render yet (`unsupported`), or whose media the host media store cannot
 * hand it (`mediaOffline`), comes back as `routed`, and the caller runs the
 * legacy exporter or capture instead.
 */
import { randomUUID } from 'node:crypto';

import { canonicalHash, canonicalProject, canonicalStringify } from '../../shared/canonical-json';
import type {
  AgentFrameOutcome,
  EngineError,
  EngineProcess,
  ExportSettings,
  JobState,
  JobStatus,
  ProjectColor,
  Request,
  Response
} from '../../shared/engine-protocol';
import { resolveFrameRate, TICKS_PER_SECOND } from '../../shared/frame-rate';
import { RemoteError } from './protocol';

/** What the agent jobs need from the engine host (NativeEngine). */
export interface AgentJobsEngine {
  request(process: EngineProcess, body: Request, session?: number | null, revision?: number | null): Promise<Response>;
  stageDocument(process: EngineProcess, session: number, bytes: Uint8Array): Promise<{ handle: string; length: number }>;
  listen(listener: { jobStatus?(status: JobStatus): void; status?(status: unknown): void }): () => void;
  isReady(process: EngineProcess): boolean;
  /** A user-chosen destination as a write handle for `job` on `session`. */
  chooseOutput?(file: string, job: string, session: number): string;
}

/** Decision 1: Standard color, linear blending off (pm_protocol `ProjectColor::DEFAULT`). */
export const STANDARD_COLOR: ProjectColor = { mode: 'standard', linearBlending: false, precision: 'float16', blendDomain: 'hdrSafe' };

/** In `wait` mode, a job queued longer than this answers `queued` (contract 12 invariant 8). */
export const DEFAULT_QUEUE_BOUND_MS = 60_000;
/** Resync bodies up to this size travel inline; larger ones through a staged cache file. */
export const INLINE_DOCUMENT_BYTES = 4 * 1024 * 1024;
/** Finished jobs kept for `get_render_status`. */
const FINISHED_KEPT = 64;

/** Error codes that send work to the legacy engine instead of failing it. */
const ROUTED_CODES = new Set<EngineError['code']>(['unsupported', 'mediaOffline']);

export type ExportVideoInput = {
  name?: string;
  format: 'mp4' | 'prores';
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

/** An export from the export sheet (exporter.ts), as the renderer asks for it. */
export type UserExportOptions = {
  format: 'mp4' | 'prores';
  alpha: boolean;
  /** Export frame rate (may differ from the composition's). */
  fps: number;
  /** Output scale (0.01 to 16): size `round(w × scale / 2) × 2`. */
  scale: number;
  /** Composition seconds. */
  start: number;
  end: number;
  audio: boolean;
  motionBlur: boolean;
  quality: 'draft' | 'high' | 'max';
};

/** A user export job's status as the window sees it; `routed` when the
 *  engine cannot render the project and the current exporter should. */
export type UserExportStatus = RenderStatus & { routed?: boolean };

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

/** `export_video`'s settings as the legacy exporter applies them (exporter.ts
 *  `run`, render-encoder.ts): the composition size at scale 1 rounded to even
 *  pixels, the composition rate, all or the work area, ProRes 4444 (straight
 *  alpha when asked) or H.264 at the High tier (16 Mbps), Rec.709 (QuickTime
 *  and Safari) tags. Exports render at 32-bit precision (amendment 8 asks for
 *  it whenever a source exceeds 10 bits; agent exports do not inspect
 *  sources first), and H.264's 8-bit output is dithered (section 17). */
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
  const prores = input.format === 'prores';
  if (input.alpha && !prores) throw new AgentJobError('Transparent video requires ProRes.');
  return {
    format: prores ? { kind: 'proRes', profile: 'p4444' } : { kind: 'h264' },
    alpha: prores && input.alpha ? 'straight' : 'none',
    quality: 'high',
    size: { width: evenSize(width), height: evenSize(height) },
    frameRate: { num: rate.num, den: rate.den },
    range: { start: toTicks(Math.max(0, start)), end: toTicks(end) },
    audio: input.audio,
    motionBlur: input.motionBlur,
    color: { kind: 'rec709QuickTime', candidate: 'cicp1_13_1' },
    precision: 'float32',
    dither: !prores,
    draftFromProxies: false
  };
}

/** The export sheet's settings: today's native exporter (exporter.ts `run`,
 *  render-encoder.ts) at the chosen rate, scale, range, quality tier, and
 *  alpha, with the same tags, precision, and dither rules as agent exports. */
export function userExportSettings(projectValue: unknown, options: UserExportOptions): ExportSettings {
  const project = asProject(projectValue);
  const o = options;
  const valid = (o.format === 'mp4' || o.format === 'prores') && Number.isFinite(o.fps) && o.fps > 0 && o.fps <= 240 &&
    Number.isFinite(o.scale) && o.scale >= 0.01 && o.scale <= 16 && Number.isFinite(o.start) && Number.isFinite(o.end) &&
    o.start >= 0 && o.end > o.start && ['draft', 'high', 'max'].includes(o.quality) &&
    typeof o.audio === 'boolean' && typeof o.alpha === 'boolean' && typeof o.motionBlur === 'boolean';
  if (!valid) throw new AgentJobError('Invalid export settings.');
  const base = exportSettingsFor(project, { format: o.format, range: 'all', audio: o.audio, motionBlur: o.motionBlur, alpha: o.alpha && o.format === 'prores' });
  const rate = resolveFrameRate(o.fps, Number(project['fps']) === o.fps ? project['fpsRational'] : undefined);
  if (!rate) throw new AgentJobError('Invalid export frame rate.');
  const size = { width: evenSize(Number(project['w']) * o.scale), height: evenSize(Number(project['h']) * o.scale) };
  if (size.width > 8192 || size.height > 8192) throw new AgentJobError('The export is larger than 8192 pixels.');
  return { ...base, quality: o.quality, size, frameRate: { num: rate.num, den: rate.den }, range: { start: toTicks(o.start), end: toTicks(o.end) } };
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

type FrameSession = { session: number; revision: number; hash: string; generation: number };

export type AgentJobsOptions = {
  engine: AgentJobsEngine;
  /** Status of an export sheet job, for its window. */
  onUserStatus?(owner: number, status: UserExportStatus): void;
  /** A job finished with a file at `path` (export sidecars remember it). */
  onDelivered?(owner: number, path: string): void;
  queueBoundMs?: number;
  now?: () => number;
  log?: (level: 'info' | 'warn' | 'error', message: string) => void;
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
    const extension = input.format === 'prores' ? 'mov' : 'mp4';
    const jobId = `agent-export-${randomUUID()}`;
    const session = await this.openSnapshot(project, `agent-export-${jobId}`);
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
    if (status.state === 'done' && status.path) this.options.onDelivered?.(record.owner, status.path);
    if (record.user) {
      const routed = status.state === 'failed' && !!status.error && ROUTED_CODES.has(status.error.code);
      this.options.onUserStatus?.(record.owner, { ...this.status(record.owner, record.jobId), ...(routed ? { routed: true } : {}) });
    }
    if (terminal(status.state)) {
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
   *  they chose (replaced if it exists: the save panel confirmed that).
   *  Statuses go to `onUserStatus`; an `unsupported` or `mediaOffline`
   *  failure arrives as `routed`, and the window runs the current exporter
   *  to the same file. */
  async exportUser(owner: number, projectValue: unknown, options: UserExportOptions, destination: string): Promise<{ jobId: string; frames: number }> {
    const project = asProject(projectValue);
    const settings = userExportSettings(project, options);
    if (!this.options.engine.chooseOutput) throw new AgentJobError('This host cannot write user exports.');
    const jobId = `user-export-${randomUUID()}`;
    const session = await this.openSnapshot(project, `user-export-${jobId}`);
    const record: JobRecord = {
      jobId, owner, run: '', session, user: true,
      status: { job: jobId, class: 'userJob', state: 'queued', progress: 0, path: null, error: null },
      queuedAt: this.now(), startedAt: null,
      name: suggestedName(project, undefined),
      size: settings.size,
      fps: options.fps,
      changed: []
    };
    this.jobs.set(jobId, record);
    try {
      const handle = this.options.engine.chooseOutput(destination, jobId, session);
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
    const frames = Math.max(1, Math.round((options.end - options.start) * settings.frameRate.num / settings.frameRate.den));
    return { jobId, frames };
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
      if (current && current.generation === this.generation && current.hash === hash) return current;
      if (current && current.generation === this.generation) {
        const revision = current.revision + 1;
        await this.resync(current.session, `agent-frames-${owner}`, canonical, hash, revision);
        return { ...current, revision, hash };
      }
      const generation = this.generation;
      const session = await this.openSession(`agent-frames-${owner}`, 'job');
      await this.resync(session, `agent-frames-${owner}`, canonical, hash, 1);
      return { session, revision: 1, hash, generation };
    })();
    this.frameSessions.set(owner, next);
    next.catch(() => { if (this.frameSessions.get(owner) === next) this.frameSessions.delete(owner); });
    return next;
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

  /** A frozen snapshot of the project for one export. */
  private async openSnapshot(project: Project, document: string): Promise<number> {
    const canonical = canonicalProject(project);
    const session = await this.openSession(document, 'jobSnapshot');
    try {
      await this.resync(session, document, canonical, canonicalHash(canonical), 1);
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
