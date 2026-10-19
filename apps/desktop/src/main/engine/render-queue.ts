/*
 * The render queue (plan section 17, "Presets and the render queue"): jobs
 * from the export sheet and agents, each with its destination; unattended
 * runs in the job engine from snapshots, without touching the live project;
 * per-job failures; persistence across relaunch for user jobs.
 *
 * User jobs are stored under `<userData>/Render Queue`: `queue.json` (the
 * jobs and whether the queue was running) and one owner-only project
 * snapshot per job. A job running when the app quit returns as pending, and
 * a queue that was running resumes once the job engine is ready. Agent
 * exports (`export_video`) appear while they run and are not persisted:
 * agent jobs never resume (section 17).
 *
 * Jobs run one at a time in the job engine's user lane. A job whose media is
 * offline waits there ("Waiting for media"). A failed job does not stop the
 * queue; the next one starts. While anything renders, the Mac does not sleep,
 * and a finished queue posts a notification when no Powermove window has
 * focus. A job the new engine cannot render yet fails with the reason (the
 * export sheet still routes such projects to the current exporter).
 */
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rm, unlink } from 'node:fs/promises';
import path from 'node:path';

import {
  formatInfo,
  migrateFormat,
  normalizeDelivery,
  type OverwritePolicy
} from '../../shared/delivery';
import {
  type CaptionSidecar,
  type RenderQueueAction,
  type RenderQueueJob,
  type RenderQueueState,
  type SheetExportOptions,
  validSheetExportOptions
} from '../../shared/engine-ipc';
import { SIDECAR_SUFFIX_PATTERN } from '../../shared/ipc';
import { sidecarPath } from '../export-sidecar';
import { atomicWrite } from '../project-files';
import type { UserExportStatus } from './agent-jobs';

/** What the queue needs from the job engine (AgentJobs). */
export interface QueueEngine {
  readonly available: boolean;
  exportQueued(project: unknown, options: SheetExportOptions, destination: string, overwrite: 'uniqueName' | 'replace' | 'fail', seedOwner?: number | null): Promise<{ jobId: string; frames: number }>;
  cancelQueued(jobId: string): Promise<void>;
}

export interface QueuePlatform {
  /** Keeps the Mac awake while jobs render; returns a release. */
  preventSleep(): () => void;
  /** Posts a completion notification (only when no window has focus). */
  notify(title: string, body: string): void;
  /** Reveals a file in Finder. */
  reveal(file: string): void;
  log?(level: 'info' | 'warn' | 'error', message: string): void;
}

type StoredJob = {
  id: string;
  name: string;
  summary: string;
  composition: string;
  options: SheetExportOptions;
  destination: string;
  overwrite: OverwritePolicy;
  sidecars: CaptionSidecar[];
  state: RenderQueueJob['state'];
  progress: number;
  error?: string;
  path?: string;
  createdAt: number;
  finishedAt?: number;
};

type Stored = { version: 1; running: boolean; jobs: StoredJob[] };

type AgentEntry = RenderQueueJob & { owner: number };

const ENGINE_POLICY: Record<OverwritePolicy, 'uniqueName' | 'replace' | 'fail'> = {
  'keep-both': 'uniqueName',
  replace: 'replace',
  stop: 'fail'
};

/** Most finished agent entries kept in the list. */
const AGENT_KEPT = 20;

export type RenderQueueOptions = {
  directory: string;
  engine: QueueEngine;
  platform: QueuePlatform;
  /** Called with the new state after every change. */
  onChange?(state: RenderQueueState): void;
  now?: () => number;
  windowOpen?(owner: number): boolean;
};

export class RenderQueue {
  private jobs: StoredJob[] = [];
  // Runtime only: webContents ids must never survive an app relaunch.
  private seedOwners = new Map<string, number>();
  private agents: AgentEntry[] = [];
  private running = false;
  /** The engine job of the queue job that is rendering. */
  private current: { id: string; engineJob: string | null } | null = null;
  private releaseSleep: (() => void) | null = null;
  private loaded: Promise<void>;
  private persisting: Promise<void> = Promise.resolve();
  private readonly now: () => number;
  /** Finished during this run of the queue (for the notification). */
  private finishedInRun = { done: 0, failed: 0 };

  constructor(private readonly options: RenderQueueOptions) {
    this.now = options.now ?? Date.now;
    this.loaded = this.load();
  }

  private get file(): string {
    return path.join(this.options.directory, 'queue.json');
  }

  private snapshotFile(id: string): string {
    return path.join(this.options.directory, `${id}.json`);
  }

  private async load(): Promise<void> {
    let stored: Stored | null = null;
    try {
      stored = JSON.parse(await readFile(this.file, 'utf8')) as Stored;
    } catch {
      stored = null;
    }
    if (!stored || stored.version !== 1 || !Array.isArray(stored.jobs)) return;
    this.jobs = stored.jobs
      .filter((j) => j && typeof j.id === 'string' && /^[a-z0-9-]{8,64}$/.test(j.id) && validSheetExportOptions(j.options))
      .map((j) => ({
        ...j,
        // A job interrupted by quitting renders again from the start.
        ...(['running', 'queued', 'waitingForMedia'].includes(j.state) ? { state: 'pending' as const, progress: 0 } : {})
      }));
    this.running = stored.running === true && this.jobs.some((j) => j.state === 'pending');
  }

  /** Resolves when the stored queue is read. */
  ready(): Promise<void> {
    return this.loaded;
  }

  private persist(): Promise<void> {
    const body: Stored = { version: 1, running: this.running, jobs: this.jobs };
    const bytes = new TextEncoder().encode(JSON.stringify(body));
    this.persisting = this.persisting
      .then(async () => {
        await mkdir(this.options.directory, { recursive: true, mode: 0o700 });
        await atomicWrite(this.file, bytes);
      })
      .catch((error: unknown) => this.options.platform.log?.('warn', `the render queue could not be saved: ${error instanceof Error ? error.message : String(error)}`));
    return this.persisting;
  }

  /** The panel's view: user jobs in order, then agent exports. */
  state(): RenderQueueState {
    const user: RenderQueueJob[] = this.jobs.map((j) => ({
      id: j.id,
      source: 'user',
      name: j.name,
      summary: j.summary,
      destination: j.destination,
      state: j.state,
      progress: j.progress,
      ...(j.error ? { error: j.error } : {}),
      ...(j.path ? { path: j.path } : {}),
      createdAt: j.createdAt,
      ...(j.finishedAt ? { finishedAt: j.finishedAt } : {})
    }));
    const agents = this.agents.map(({ owner: _owner, ...rest }) => rest);
    return { running: this.running, jobs: [...user, ...agents] };
  }

  private changed(): void {
    void this.persist();
    this.options.onChange?.(this.state());
    this.updateSleep();
  }

  /** Adds a user job. `project` is the snapshot it renders. */
  async add(input: {
    name: string;
    project: unknown;
    options: SheetExportOptions;
    destination: string;
    sidecars?: CaptionSidecar[];
    preset?: string;
    owner?: number;
  }): Promise<string> {
    await this.loaded;
    if (!validSheetExportOptions(input.options)) throw new Error('Invalid export settings.');
    if (!path.isAbsolute(input.destination)) throw new Error('A queued export needs a destination.');
    const project = input.project as Record<string, any> | null;
    if (!project || typeof project !== 'object' || typeof project['compId'] !== 'string') throw new Error('The project could not be read.');
    const id = `job-${randomUUID()}`;
    const delivery = normalizeDelivery(input.options.delivery ?? {});
    const format = migrateFormat(input.options.format);
    const w = input.options.width ?? Math.round(Number(project['w']) * input.options.scale);
    const h = input.options.height ?? Math.round(Number(project['h']) * input.options.scale);
    const fps = Number.isInteger(input.options.fps) ? String(input.options.fps) : input.options.fps.toFixed(3);
    const summary = [input.preset, formatInfo(format).label, `${w}×${h}`, format === 'still' ? null : `${fps} fps`]
      .filter(Boolean).join(' · ');
    const sidecars = (input.sidecars ?? []).filter((s) =>
      (s.extension === 'srt' || s.extension === 'vtt') && typeof s.text === 'string' && (s.suffix === undefined || SIDECAR_SUFFIX_PATTERN.test(s.suffix)));
    await mkdir(this.options.directory, { recursive: true, mode: 0o700 });
    await atomicWrite(this.snapshotFile(id), new TextEncoder().encode(JSON.stringify(project)));
    if (input.owner !== undefined) this.seedOwners.set(id, input.owner);
    this.jobs.push({
      id,
      name: input.name,
      summary,
      composition: project['compId'],
      options: { ...input.options, delivery },
      destination: input.destination,
      overwrite: delivery.overwrite,
      sidecars,
      state: 'pending',
      progress: 0,
      createdAt: this.now()
    });
    this.changed();
    return id;
  }

  /** Runs pending jobs one after another until none is left or the queue is
   *  paused. */
  async start(): Promise<void> {
    await this.loaded;
    if (!this.running) {
      this.running = true;
      this.finishedInRun = { done: 0, failed: 0 };
      this.changed();
    }
    void this.pump();
  }

  /** Stops taking new jobs; the rendering job finishes. */
  pause(): void {
    if (!this.running) return;
    this.running = false;
    this.changed();
  }

  /** Called when the job engine becomes ready: a queue that was running at
   *  quit resumes (unattended runs). */
  engineReady(): void {
    if (this.running) void this.pump();
  }

  private pumping = false;

  private async pump(): Promise<void> {
    if (this.pumping) return;
    this.pumping = true;
    try {
      while (this.running && !this.current) {
        const job = this.jobs.find((j) => j.state === 'pending');
        if (!job) {
          this.running = false;
          this.finishRun();
          this.changed();
          return;
        }
        if (!this.options.engine.available) return; // engineReady() resumes
        await this.run(job);
      }
    } finally {
      this.pumping = false;
    }
  }

  private finishRun(): void {
    const { done, failed } = this.finishedInRun;
    if (done + failed === 0) return;
    const title = failed ? 'Render queue finished with errors' : 'Render queue finished';
    const body = failed
      ? `${done} ${done === 1 ? 'export' : 'exports'} done · ${failed} failed`
      : `${done} ${done === 1 ? 'export' : 'exports'} done`;
    this.options.platform.notify(title, body);
    this.finishedInRun = { done: 0, failed: 0 };
  }

  private async run(job: StoredJob): Promise<void> {
    this.current = { id: job.id, engineJob: null };
    job.state = 'queued';
    job.progress = 0;
    delete job.error;
    delete job.path;
    this.changed();
    let project: unknown;
    try {
      project = JSON.parse(await readFile(this.snapshotFile(job.id), 'utf8'));
    } catch {
      this.end(job, 'failed', 'The project snapshot of this job is missing.');
      return;
    }
    const finished = new Promise<void>((resolve) => { this.waiters.set(job.id, resolve); });
    try {
      const owner = this.seedOwners.get(job.id);
      const seedOwner = owner !== undefined && this.options.windowOpen?.(owner) ? owner : null;
      const started = await this.options.engine.exportQueued(project, job.options, job.destination, ENGINE_POLICY[job.overwrite], seedOwner);
      if (!this.current || this.current.id !== job.id) {
        // Cancelled while it was being submitted.
        await this.options.engine.cancelQueued(started.jobId);
      } else {
        this.current.engineJob = started.jobId;
        for (const status of this.early.splice(0).filter((s) => s.jobId === started.jobId)) this.status(started.jobId, status);
      }
    } catch (error) {
      this.waiters.delete(job.id);
      this.end(job, 'failed', error instanceof Error ? error.message : String(error));
      return;
    }
    await finished;
  }

  private readonly waiters = new Map<string, () => void>();
  /** Statuses that arrived before `exportQueued` answered. */
  private early: UserExportStatus[] = [];

  private disposing = false;

  private end(job: StoredJob, state: 'done' | 'failed' | 'cancelled', error?: string, file?: string): void {
    if (this.disposing && state === 'cancelled') {
      // Quitting: the job renders again on the next launch.
      job.state = 'pending';
      job.progress = 0;
      if (this.current?.id === job.id) this.current = null;
      this.waiters.get(job.id)?.();
      this.waiters.delete(job.id);
      void this.persist();
      return;
    }
    job.state = state;
    job.finishedAt = this.now();
    if (state === 'done') {
      job.progress = 1;
      this.finishedInRun.done++;
    }
    if (state === 'failed') this.finishedInRun.failed++;
    if (error) job.error = error;
    if (file) job.path = file;
    if (this.current?.id === job.id) this.current = null;
    this.waiters.get(job.id)?.();
    this.waiters.delete(job.id);
    this.changed();
    void this.pump();
  }

  /** A queue job's status from the job engine (AgentJobs `onQueueStatus`). */
  status(engineJob: string, status: UserExportStatus): void {
    const current = this.current;
    if (!current || current.engineJob === null) {
      this.early.push(status);
      return;
    }
    if (current.engineJob !== engineJob) return;
    const job = this.jobs.find((j) => j.id === current.id);
    if (!job) return;
    switch (status.state) {
      case 'queued':
      case 'running':
      case 'waitingForMedia':
        job.state = status.state;
        job.progress = status.progress;
        if (status.waiting) job.error = status.waiting;
        else delete job.error;
        this.options.onChange?.(this.state());
        this.updateSleep();
        return;
      case 'done':
        void this.writeSidecars(job, status.path ?? job.destination).then(() => this.end(job, 'done', undefined, status.path));
        return;
      case 'cancelled':
        this.end(job, 'cancelled');
        return;
      case 'failed':
        this.end(job, 'failed', status.routed
          ? `The new engine cannot render this project yet (${status.error ?? 'unsupported'}). Export it from the export sheet.`
          : status.error ?? 'The export failed.');
    }
  }

  private async writeSidecars(job: StoredJob, file: string): Promise<void> {
    for (const sidecar of job.sidecars) {
      const target = sidecarPath(file, sidecar.extension, sidecar.suffix);
      await atomicWrite(target, new TextEncoder().encode(sidecar.text)).catch((error: unknown) =>
        this.options.platform.log?.('warn', `a caption file of ${job.name} could not be written: ${error instanceof Error ? error.message : String(error)}`));
    }
  }

  /** An agent export appeared or changed (AgentJobs `onAgentJob`). */
  agentJob(job: { jobId: string; owner: number; name: string; summary: string; status: { state: RenderQueueJob['state']; progress: number; path?: string; error?: string } }): void {
    const entry = this.agents.find((a) => a.id === job.jobId);
    const next: AgentEntry = {
      id: job.jobId,
      owner: job.owner,
      source: 'agent',
      name: job.name,
      summary: job.summary,
      destination: '',
      state: job.status.state,
      progress: job.status.progress,
      ...(job.status.error ? { error: job.status.error } : {}),
      ...(job.status.path ? { path: job.status.path, destination: job.status.path } : {}),
      createdAt: entry?.createdAt ?? this.now(),
      ...(['done', 'failed', 'cancelled'].includes(job.status.state) ? { finishedAt: this.now() } : {})
    };
    if (entry) Object.assign(entry, next);
    else this.agents.push(next);
    const finished = this.agents.filter((a) => a.finishedAt);
    if (finished.length > AGENT_KEPT) {
      const drop = new Set(finished.slice(0, finished.length - AGENT_KEPT).map((a) => a.id));
      this.agents = this.agents.filter((a) => !drop.has(a.id));
    }
    this.options.onChange?.(this.state());
    this.updateSleep();
  }

  /** A panel action. */
  async act(action: RenderQueueAction, cancelAgent?: (owner: number, jobId: string) => Promise<void>): Promise<RenderQueueState> {
    await this.loaded;
    switch (action.kind) {
      case 'start':
        await this.start();
        break;
      case 'pause':
        this.pause();
        break;
      case 'cancel': {
        const agent = this.agents.find((a) => a.id === action.id);
        if (agent) {
          await cancelAgent?.(agent.owner, agent.id);
          break;
        }
        const job = this.jobs.find((j) => j.id === action.id);
        if (!job) break;
        if (this.current?.id === job.id) {
          if (this.current.engineJob) await this.options.engine.cancelQueued(this.current.engineJob);
          else this.end(job, 'cancelled');
        } else if (job.state === 'pending') {
          job.state = 'cancelled';
          job.finishedAt = this.now();
          this.changed();
        }
        break;
      }
      case 'remove': {
        const job = this.jobs.find((j) => j.id === action.id);
        if (job && this.current?.id !== job.id) {
          this.jobs = this.jobs.filter((j) => j.id !== job.id);
          await unlink(this.snapshotFile(job.id)).catch(() => {});
          this.changed();
        }
        this.agents = this.agents.filter((a) => !(a.id === action.id && a.finishedAt));
        this.options.onChange?.(this.state());
        break;
      }
      case 'retry': {
        const job = this.jobs.find((j) => j.id === action.id);
        if (job && (job.state === 'failed' || job.state === 'cancelled')) {
          job.state = 'pending';
          job.progress = 0;
          delete job.error;
          delete job.finishedAt;
          this.changed();
          if (this.running) void this.pump();
        }
        break;
      }
      case 'move': {
        const from = this.jobs.findIndex((j) => j.id === action.id);
        if (from >= 0 && Number.isInteger(action.to)) {
          const [job] = this.jobs.splice(from, 1);
          this.jobs.splice(Math.max(0, Math.min(this.jobs.length, action.to)), 0, job!);
          this.changed();
        }
        break;
      }
      case 'reveal': {
        const file = this.jobs.find((j) => j.id === action.id)?.path ?? this.agents.find((a) => a.id === action.id)?.path;
        if (file) this.options.platform.reveal(file);
        break;
      }
      case 'clearFinished': {
        const gone = this.jobs.filter((j) => j.state === 'done' || j.state === 'cancelled');
        this.jobs = this.jobs.filter((j) => !gone.includes(j));
        for (const job of gone) await unlink(this.snapshotFile(job.id)).catch(() => {});
        this.agents = this.agents.filter((a) => !a.finishedAt);
        this.changed();
        break;
      }
    }
    return this.state();
  }

  /** Whether anything renders now (the Mac stays awake). */
  busy(): boolean {
    return !!this.current || this.agents.some((a) => a.state === 'running' || a.state === 'queued');
  }

  private updateSleep(): void {
    const busy = this.busy();
    if (busy && !this.releaseSleep) this.releaseSleep = this.options.platform.preventSleep();
    if (!busy && this.releaseSleep) {
      this.releaseSleep();
      this.releaseSleep = null;
    }
  }

  /** Stops the queue (app quit): the rendering job is cancelled and returns
   *  as pending on the next launch; `running` is kept so it resumes. */
  async dispose(): Promise<void> {
    this.disposing = true;
    const current = this.current;
    const job = current ? this.jobs.find((j) => j.id === current.id) : undefined;
    if (job) {
      job.state = 'pending';
      job.progress = 0;
    }
    await this.persist();
    if (current?.engineJob) await this.options.engine.cancelQueued(current.engineJob).catch(() => {});
    this.releaseSleep?.();
    this.releaseSleep = null;
    await this.persisting;
  }

  /** Removes the stored queue (tests). */
  async destroy(): Promise<void> {
    await rm(this.options.directory, { recursive: true, force: true });
  }
}
