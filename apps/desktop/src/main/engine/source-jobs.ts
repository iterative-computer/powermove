/** Private source jobs. Source frame sessions stay frozen for bounded reuse; rendition jobs remain isolated. */
import { randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { canonicalHash, canonicalProject, canonicalStringify } from '../../shared/canonical-json';
import { ENGINE_IPC, type EngineBrowserRenditionRequest, type EngineMediaJobRequest, type EngineSourceFilmstripRequest, type EngineSourcePrepareRequest } from '../../shared/engine-ipc';
import type { JobSpec, JobStatus } from '../../shared/engine-protocol';
import { STANDARD_COLOR, type AgentJobsEngine } from './agent-jobs';
import { SourceSessionPool, sourceInputKey } from './source-session';

const MAX_BYTES = 512 * 1024 * 1024;
const TICKS = 705_600_000;
const validId = (v: unknown): v is string => typeof v === 'string' && v.length > 0 && new TextEncoder().encode(v).length <= 256;
export function validMediaJob(value: unknown): value is EngineMediaJobRequest {
  if (!value || typeof value !== 'object') return false;
  const v = value as EngineMediaJobRequest;
  return typeof v.project === 'string' && v.project.length > 0 && v.project.length <= MAX_BYTES
    && Number.isSafeInteger(v.revision) && v.revision >= 0 && validId(v.asset) && validId(v.requestId);
}
export function validSourceFilmstrip(value: unknown): value is EngineSourceFilmstripRequest {
  if (!validMediaJob(value)) return false;
  const v = value as EngineSourceFilmstripRequest;
  return Number.isInteger(v.width) && v.width > 0 && v.width <= 8192
    && Number.isInteger(v.height) && v.height > 0 && v.height <= 8192
    && Array.isArray(v.times) && v.times.length > 0 && v.times.length <= 64
    && v.times.every(t => Number.isFinite(t) && t >= 0 && Number.isSafeInteger(Math.round(t * TICKS)))
    && v.width * v.height * 4 <= 16 * 1024 * 1024 && v.width * v.height * 4 * v.times.length <= MAX_BYTES
    && ['stretch', 'contain', 'cover'].includes(v.fit) && ['display', 'interpreted'].includes(v.color);
}
export function validSourcePrepare(value: unknown): value is EngineSourcePrepareRequest {
  if (!validMediaJob(value)) return false;
  const v = value as EngineSourcePrepareRequest;
  return validSourceFilmstrip({ ...v, times: [v.time] });
}
export function validBrowserRendition(value: unknown): value is EngineBrowserRenditionRequest {
  return validMediaJob(value) && ['png', 'webm'].includes((value as EngineBrowserRenditionRequest).format);
}

type Active = { job: string; source: boolean; cancelled: boolean; cancel(): void };
type Options = {
  engine: AgentJobsEngine;
  directory: string;
  releaseOutput?(handle: string): void;
  snapshotMedia?(project: Record<string, any>): Promise<Record<string, any>>;
  bindMediaSession?(session: number, project: Record<string, any> | null): Promise<void>;
};

export class SourceJobs {
  private readonly active = new Map<number, Map<string, Active>>();
  private readonly sessions: SourceSessionPool;
  private readonly warming = new Map<number, Map<string, { key: string; done: Promise<void> }>>();
  private disposed = false;
  constructor(private readonly options: Options) {
    this.sessions = new SourceSessionPool(options);
  }

  cancel(owner: number, request: string): void {
    const active = this.active.get(owner)?.get(request);
    if (!active) return;
    active.cancelled = true; active.cancel();
    void this.options.engine.request('job', { type: 'cancelJobs', kind: 'job', job: active.job }).catch(() => {});
  }

  forget(owner: number): void {
    for (const key of this.active.get(owner)?.keys() ?? []) this.cancel(owner, key);
    this.sessions.forget(owner);
    this.warming.delete(owner);
  }

  releasePreparedSources(owner: number): void {
    for (const [key, active] of this.active.get(owner) ?? []) if (active.source) this.cancel(owner, key);
    this.sessions.forget(owner);
    this.warming.delete(owner);
  }

  dispose(): void { this.disposed = true; for (const owner of this.active.keys()) this.forget(owner); this.sessions.dispose(); }

  /** Opens the frozen source and performs one actual decode/draw/readback. */
  prepare(owner: number, request: EngineSourcePrepareRequest): Promise<void> {
    if (this.disposed) return Promise.reject(new Error('The job engine is not running.'));
    const key = sourceInputKey(request);
    const existing = this.warming.get(owner)?.get(request.asset);
    if (existing?.key === key) return existing.done;
    const done = this.run(owner, request, session => ({ kind: 'sourceFilmstrip', session, asset: request.asset,
      times: [Math.round(request.time * TICKS)], outputSize: { width: request.width, height: request.height },
      fit: request.fit, color: request.color }), 'rgba', true, true).then(() => {});
    let entries = this.warming.get(owner);
    if (!entries) this.warming.set(owner, entries = new Map());
    const entry = { key, done };
    entries.set(request.asset, entry);
    void done.finally(() => {
      if (this.warming.get(owner)?.get(request.asset) === entry) {
        this.warming.get(owner)?.delete(request.asset);
        if (!this.warming.get(owner)?.size) this.warming.delete(owner);
      }
    }).catch(() => {});
    return done;
  }

  async run(owner: number, request: EngineMediaJobRequest, spec: (session: number) => JobSpec, extension: string,
    reuseSource = false, warm = false): Promise<Uint8Array> {
    if (this.disposed) throw new Error('The job engine is not running.');
    let requests = this.active.get(owner);
    if (!requests) this.active.set(owner, requests = new Map());
    if (requests.has(request.requestId)) throw new Error('A media request with this id is already running.');
    if (requests.size >= 4) throw new Error('Too many source jobs are running in this window.');
    const active: Active = { job: `media-${randomUUID()}`, source: reuseSource, cancelled: false, cancel: () => {} };
    requests.set(request.requestId, active);
    const check = () => { if (active.cancelled || this.disposed) throw new Error('The source job was cancelled.'); };
    let session: number | null = null, directory: string | null = null;
    let outputHandle: string | null = null;
    let submittedJob = false;
    let releaseSource: (() => void) | null = null;
    let reused = false;
    let sourcePhases: Record<string, number> = {};
    const started = performance.now();
    let readyAt = started, submittedAt = started, completedAt = started;
    let resultReadMs = 0;
    let finishedSuccessfully = false;
    let off = () => {};
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      if (reuseSource) {
        if (!warm) {
          const warming = this.warming.get(owner)?.get(request.asset);
          if (warming?.key === sourceInputKey(request)) await warming.done.catch(() => {});
        }
        check();
        const acquired = await this.sessions.acquire(owner, request);
        session = acquired.session;
        releaseSource = acquired.release;
        reused = acquired.reused;
        sourcePhases = acquired.phases;
        if (warm && acquired.warmed) { finishedSuccessfully = true; return new Uint8Array(0); }
      } else {
      const parsed = JSON.parse(request.project);
      let project = parsed.proj ?? parsed;
      if (!project.assets?.[request.asset]) throw new Error('The source is not in the frozen project.');
      // Older projects can still name a mutable storage key. Pin its current
      // object before waiting for a cold job engine, as queued exports do.
      project = await this.options.snapshotMedia?.(project) ?? project;
      check();
      if (!this.options.engine.isReady('job')) {
        // The job utility starts on demand and retires when idle.
        void this.options.engine.ensureReady?.('job').catch(() => {});
        try {
          await new Promise<void>((resolve, reject) => {
            active.cancel = () => reject(new Error('The source job was cancelled.'));
            off = this.options.engine.listen({ status: () => { if (this.options.engine.isReady('job')) resolve(); } });
            timer = setTimeout(() => reject(new Error('The job engine did not become ready.')), 45_000);
            if (this.options.engine.isReady('job')) resolve();
          });
        } finally { off(); off = () => {}; if (timer) clearTimeout(timer); timer = null; }
      }
      check();
      // Sessions isolate concurrent snapshots; the document id keeps host SVG
      // decoder requests associated with the window that owns those bytes.
      const document = validId(project.id) ? project.id : `media-${randomUUID()}`;
      const opened = await this.options.engine.request('job', { type: 'openSession', document, role: 'jobSnapshot', process: 'job', color: STANDARD_COLOR });
      if (opened.type !== 'sessionOpened') throw new Error('The job engine did not open a source session.');
      session = opened.session;
      check();
      await this.options.bindMediaSession?.(session, project);
      check();
      const canonical = canonicalProject(project), text = canonicalStringify(canonical);
      if (text === undefined) throw new Error('The source project could not be serialized.');
      const body = new TextEncoder().encode(text);
      const hash = Uint8Array.from(canonicalHash(canonical).match(/../g)!.map(v => parseInt(v, 16)));
      const source = body.byteLength <= 4 * 1024 * 1024 ? { kind: 'inline' as const, body }
        : { kind: 'cacheFile' as const, ...await this.options.engine.stageDocument('job', session, body) };
      await this.options.engine.request('job', { type: 'resync', document, revision: request.revision, encoding: 'canonicalJsonV1', source, hash }, session);
      }
      check();
      readyAt = performance.now();
      await mkdir(this.options.directory, { recursive: true, mode: 0o700 });
      directory = await mkdtemp(path.join(this.options.directory, 'source-'));
      const destination = path.join(directory, `output.${extension}`);
      if (!this.options.engine.chooseOutput) throw new Error('This host cannot receive source job output.');
      const handle = this.options.engine.chooseOutput(destination, active.job, session, 'fail');
      outputHandle = handle;
      const finished = new Promise<JobStatus>((resolve, reject) => {
        active.cancel = () => reject(new Error('The source job was cancelled.'));
        off = this.options.engine.listen({
          jobStatus: status => {
            if (status.job !== active.job) return;
            if (status.state === 'done') resolve(status);
            else if (status.state === 'failed' || status.state === 'cancelled') reject(new Error(status.error?.message ?? 'The source job was cancelled.'));
          },
          status: () => { if (!this.options.engine.isReady('job')) reject(new Error('The job engine stopped.')); }
        });
        timer = setTimeout(() => { this.cancel(owner, request.requestId); }, 60 * 60 * 1000);
      });
      // Attach before submission: even an immediate failure has a consumer.
      finished.catch(() => {});
      submittedJob = true;
      await this.options.engine.request('job', { type: 'submitJob', job: active.job, class: 'userJob', run: null,
        spec: spec(session), output: { kind: 'userChosen', destination: handle } }, null, request.revision);
      submittedAt = performance.now();
      await finished;
      completedAt = performance.now();
      check();
      const size = (await stat(destination)).size;
      if (size < 1 || size >= 0xffffffff) throw new Error('The source result is empty or exceeds the web archive size limit.');
      if (warm && size !== (request as EngineSourcePrepareRequest).width * (request as EngineSourcePrepareRequest).height * 4)
        throw new Error('The engine returned an incomplete source preparation.');
      if (reuseSource) this.sessions.markWarm(owner, request.asset, session);
      // A prewarm needs the native decode/readback, but not a second host read.
      if (warm) { finishedSuccessfully = true; return new Uint8Array(0); }
      const readAt = performance.now();
      const result = await readFile(destination);
      resultReadMs = performance.now() - readAt;
      finishedSuccessfully = true;
      return new Uint8Array(result.buffer, result.byteOffset, result.byteLength);
    } finally {
      off(); if (timer) clearTimeout(timer);
      if (warm && !finishedSuccessfully && session !== null) this.sessions.invalidateSource(owner, request.asset, session);
      // Stop the native worker before removing its destination or its media binding.
      if (submittedJob) await this.options.engine.request('job', { type: 'cancelJobs', kind: 'job', job: active.job }).catch(() => {});
      try {
        if (outputHandle) this.options.releaseOutput?.(outputHandle);
      } finally {
        try {
          if (releaseSource) releaseSource();
          else if (session !== null) {
            await this.options.engine.request('job', { type: 'closeSession' }, session).catch(() => {});
            await this.options.bindMediaSession?.(session, null);
          }
        } finally {
          if (directory) await rm(directory, { force: true, recursive: true });
          requests.delete(request.requestId); if (!requests.size) this.active.delete(owner);
          if (process.env.PM_SOURCE_JOB_TIMINGS === '1') console.info('source-job-timing', JSON.stringify({
            kind: reuseSource ? (warm ? 'prepare' : 'filmstrip') : 'rendition', reused, ok: finishedSuccessfully,
            ...sourcePhases,
            prepareMs: +(readyAt - started).toFixed(2), submitMs: +(submittedAt - readyAt).toFixed(2),
            jobStatusMs: +(completedAt - submittedAt).toFixed(2), resultReadMs: +resultReadMs.toFixed(2),
            returnMs: +(performance.now() - completedAt).toFixed(2),
            totalMs: +(performance.now() - started).toFixed(2)
          }));
        }
      }
    }
  }
}

export function installSourceJobsIpc(ipc: Pick<IpcMain, 'handle' | 'removeHandler'>, jobs: SourceJobs, trusted: (event: IpcMainInvokeEvent) => boolean): () => void {
  const authorize = (event: IpcMainInvokeEvent) => { if (!trusted(event)) throw new Error('Unauthorized IPC sender'); };
  ipc.handle(ENGINE_IPC.sourceFilmstrip, async (event, value: unknown) => {
    authorize(event);
    if (!validSourceFilmstrip(value)) throw new Error('A filmstrip needs a source, bounded dimensions and one to 64 source times.');
    const data = await jobs.run(event.sender.id, value, session => ({ kind: 'sourceFilmstrip', session, asset: value.asset,
      times: value.times.map(t => Math.round(t * TICKS)), outputSize: { width: value.width, height: value.height }, fit: value.fit, color: value.color }), 'rgba', true);
    const frameBytes = value.width * value.height * 4;
    if (data.byteLength !== frameBytes * value.times.length) throw new Error('The engine returned an incomplete filmstrip.');
    return { width: value.width, height: value.height, count: value.times.length, data };
  });
  ipc.handle(ENGINE_IPC.prepareSource, async (event, value: unknown) => {
    authorize(event);
    if (!validSourcePrepare(value)) throw new Error('A source preparation needs a source, bounded dimensions and a source time.');
    await jobs.prepare(event.sender.id, value);
  });
  ipc.handle(ENGINE_IPC.releasePreparedSources, event => { authorize(event); jobs.releasePreparedSources(event.sender.id); });
  ipc.handle(ENGINE_IPC.browserRendition, async (event, value: unknown) => {
    authorize(event);
    if (!validBrowserRendition(value)) throw new Error('A browser rendition needs a source and png or webm format.');
    const data = await jobs.run(event.sender.id, value, session => ({ kind: 'browserRendition', session, asset: value.asset, format: value.format }), value.format);
    return { data, type: value.format === 'png' ? 'image/png' : 'video/webm' };
  });
  ipc.handle(ENGINE_IPC.cancelMediaJob, (event, value: unknown) => { authorize(event); if (!validId(value)) throw new Error('Invalid source request id.'); jobs.cancel(event.sender.id, value); });
  return () => { for (const channel of [ENGINE_IPC.prepareSource, ENGINE_IPC.releasePreparedSources, ENGINE_IPC.sourceFilmstrip, ENGINE_IPC.browserRendition, ENGINE_IPC.cancelMediaJob]) ipc.removeHandler(channel); };
}
