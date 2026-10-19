import { describe, expect, it, vi } from 'vitest';

import { canonicalHash, canonicalProject } from '../../shared/canonical-json';
import type { EngineError, EngineProcess, JobStatus, Request, Response } from '../../shared/engine-protocol';
import { AgentJobError, AgentJobs, exportSettingsFor, suggestedName, userExportSettings, type AgentJobsEngine, type UserExportStatus } from './agent-jobs';
import { DefinitionRelay } from './definitions';
import { RemoteError } from './protocol';

const project = (extra: Record<string, unknown> = {}) => ({
  id: 'P1', name: 'Launch', compId: 'P1-root', compName: 'Launch', w: 1279, h: 720, fps: 29.97, fpsRational: [30000, 1001], dur: 4,
  work: [1, 3], layers: [], assets: {}, ...extra
});

const error = (code: EngineError['code'], message: string): EngineError => ({ code, scope: { kind: 'connection' }, message, retry: { kind: 'never' }, detail: null });

type Sent = { process: EngineProcess; body: Request; session: number | null; revision: number | null };

class FakeEngine implements AgentJobsEngine {
  sent: Sent[] = [];
  ready = true;
  private nextSession = 10;
  private listeners: Array<{ jobStatus?(s: JobStatus): void; status?(s: unknown): void }> = [];
  /** How submitJob, agentFrames answer. */
  submit: (body: Request) => Response | EngineError = (body) => ({ type: 'jobAccepted', job: (body as { job: string }).job, state: 'queued', position: 0 });
  frames: (body: Request) => Response | EngineError = body => ({ type: 'agentFrames', outcomes: body.type === 'agentFrames' ? body.frames.map((_, index) => ({ status: 'rendered' as const, index, revision: 1, width: 720, height: 405, image: { kind: 'inline' as const, bytes: new Uint8Array([1]) }, blender: 'notUsed' as const })) : [], queuedMs: 0 });
  staged: Array<{ session: number; bytes: number }> = [];
  /** How pushDefinitions answers (the job engine acknowledges each generation). */
  definitions: (body: Request) => Response | EngineError = (body) => ({ type: 'definitionsApplied', generation: (body as { generation: number }).generation, failures: [] });

  async request(process: EngineProcess, body: Request, session: number | null = null, revision: number | null = null): Promise<Response> {
    this.sent.push({ process, body, session, revision });
    const answer = ((): Response | EngineError => {
      switch (body.type) {
        case 'openSession': return { type: 'sessionOpened', session: this.nextSession++ };
        case 'resync': return { type: 'patchesApplied', revision: body.revision, hash: body.hash } as unknown as Response;
        case 'closeSession': return { type: 'ack' };
        case 'submitJob': return this.submit(body);
        case 'cancelJobs': return { type: 'jobsCancelled', jobs: ['x'] };
        case 'agentFrames': return this.frames(body);
        case 'pushDefinitions': return this.definitions(body);
        default: throw new Error(`unexpected ${body.type}`);
      }
    })();
    if ('code' in answer) throw new RemoteError(answer);
    return answer;
  }

  async stageDocument(_process: EngineProcess, session: number, bytes: Uint8Array) {
    this.staged.push({ session, bytes: bytes.byteLength });
    return { handle: 'h'.repeat(43), length: bytes.byteLength };
  }

  listen(listener: { jobStatus?(s: JobStatus): void; status?(s: unknown): void }) {
    this.listeners.push(listener);
    return () => { this.listeners = this.listeners.filter((l) => l !== listener); };
  }

  isReady(): boolean { return this.ready; }

  chosen: Array<[string, string, number]> = [];
  chooseOutput(file: string, job: string, session: number): string {
    this.chosen.push([file, job, session]);
    return 'c'.repeat(32);
  }

  status(job: string, state: JobStatus['state'], extra: Partial<JobStatus> = {}): void {
    for (const l of this.listeners) l.jobStatus?.({ job, class: 'agentExport', state, progress: state === 'done' ? 1 : 0.5, path: null, error: null, ...extra });
  }

  generation(n: number): void {
    for (const l of this.listeners) l.status?.({ interactive: { generation: 1 }, job: { generation: n } });
  }

  of(type: Request['type']): Sent[] {
    return this.sent.filter((s) => s.body.type === type);
  }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

it('binds export snapshot media before resync and releases it after success or submission failure', async () => {
  const engine = new FakeEngine();
  const bindMediaSession = vi.fn(async (_session: number, _project: unknown) => {});
  const request = engine.request.bind(engine);
  engine.request = async (...args) => {
    if (args[1].type === 'resync') expect(bindMediaSession).toHaveBeenLastCalledWith(args[2], expect.objectContaining({ id: 'P1' }));
    return request(...args);
  };
  const jobs = new AgentJobs({ engine, bindMediaSession });
  const input = { format: 'mp4', range: 'all' as const, audio: false, motionBlur: false, alpha: false, mode: 'async' as const };
  const submitted = await jobs.exportVideo(1, 'run', project(), input);
  expect(submitted.status).toBe('queued');
  engine.status((submitted as { jobId: string }).jobId, 'done', { path: '/tmp/export.mp4' });
  await tick();
  expect(bindMediaSession).toHaveBeenLastCalledWith(10, null);
  engine.submit = () => error('mediaOffline', 'offline');
  await expect(jobs.exportVideo(1, 'run', project(), input)).rejects.toMatchObject({ code: 'mediaOffline' });
  expect(bindMediaSession).toHaveBeenLastCalledWith(11, null);
  jobs.dispose();
});

it('packs a save through host services with its own media scope even when rendering is unavailable', async () => {
  const engine = new FakeEngine(); engine.ready = false;
  const bindMediaSession = vi.fn(async (_session: number, _project: unknown) => {});
  const packProject = vi.fn(async () => '/exports/project.pmv');
  const jobs = new AgentJobs({ engine, bindMediaSession, packProject });
  await expect(jobs.saveProject(project())).resolves.toBe('/exports/project.pmv');
  expect(engine.sent).toEqual([]);
  expect(bindMediaSession).toHaveBeenNthCalledWith(1, -1, expect.objectContaining({ id: 'P1' }));
  expect(bindMediaSession).toHaveBeenNthCalledWith(2, -1, null);
  packProject.mockRejectedValueOnce(new Error('mediaOffline: clip'));
  await expect(jobs.saveProject(project())).rejects.toThrow('mediaOffline');
  expect(bindMediaSession).toHaveBeenLastCalledWith(-2, null);
  jobs.dispose();
});

describe('export settings', () => {
  it('match the legacy exporter: even size, exact rate, work area, tiers, tags, dither', () => {
    const mp4 = exportSettingsFor(project(), { format: 'mp4', range: 'work', audio: true, motionBlur: false, alpha: false });
    expect(mp4).toEqual({
      format: { kind: 'h264' }, alpha: 'none', quality: 'high', size: { width: 1280, height: 720 },
      frameRate: { num: 30000, den: 1001 }, range: { start: 705_600_000, end: 3 * 705_600_000 },
      audio: true, motionBlur: false, color: { kind: 'rec709QuickTime', candidate: 'cicp1_13_1' }, precision: 'float32', dither: true, draftFromProxies: false,
      delivery: { audioCoding: null, timecode: null, hideCaptions: false, preview: false, sequenceName: null }
    });
    const prores = exportSettingsFor(project({ work: null }), { format: 'prores', range: 'work', audio: false, motionBlur: true, alpha: true });
    expect(prores.format).toEqual({ kind: 'proRes', profile: 'p4444' });
    expect(prores.alpha).toBe('straight');
    expect(prores.dither).toBe(false);
    expect(prores.range).toEqual({ start: 0, end: 4 * 705_600_000 });
    expect(() => exportSettingsFor(project(), { format: 'mp4', range: 'all', audio: true, motionBlur: true, alpha: true })).toThrow('ProRes');
    expect(() => exportSettingsFor({ name: 'x' }, { format: 'mp4', range: 'all', audio: true, motionBlur: true, alpha: false })).toThrow(AgentJobError);
  });

  it('suggest bare file names', () => {
    expect(suggestedName(project(), 'a/b:c')).toBe('a_b_c');
    expect(suggestedName(project(), undefined)).toBe('Launch');
    expect(suggestedName(project(), '..')).toBe('_');
    expect(Buffer.byteLength(suggestedName(project(), 'é'.repeat(200)))).toBeLessThanOrEqual(180);
  });
});

describe('agent exports through the job engine', () => {
  it('snapshot the project, submit in the agent export class, and answer with the committed path', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const onStarted = vi.fn();
    const result = jobs.exportVideo(7, 'run-1', project(), { name: 'Cut', format: 'mp4', range: 'all', audio: true, motionBlur: false, alpha: false }, onStarted);
    await tick(); await tick();
    const [open] = engine.of('openSession');
    expect(open!.body).toMatchObject({ role: 'jobSnapshot', process: 'job' });
    const [resync] = engine.of('resync');
    const canonical = canonicalProject(project());
    expect(resync!.body).toMatchObject({ revision: 1, encoding: 'canonicalJsonV1', source: { kind: 'inline' } });
    expect(Buffer.from((resync!.body as { hash: Uint8Array }).hash).toString('hex')).toBe(canonicalHash(canonical));
    const [submit] = engine.of('submitJob');
    const body = submit!.body as Extract<Request, { type: 'submitJob' }>;
    expect(body.class).toBe('agentExport');
    expect(body.run).toBe('run-1');
    expect(body.output).toEqual({ kind: 'agentAutomatic', suggestedName: 'Cut', extension: 'mp4' });
    expect(body.spec).toMatchObject({ kind: 'export', session: 10, composition: 'P1-root' });
    expect(onStarted).not.toHaveBeenCalled();
    engine.status(body.job, 'running');
    engine.status(body.job, 'running');
    expect(onStarted).toHaveBeenCalledTimes(1);
    engine.status(body.job, 'done', { path: '/x/Cut.mp4' });
    await expect(result).resolves.toEqual({ status: 'done', jobId: body.job, path: '/x/Cut.mp4', width: 1280, height: 720, fps: 29.97 });
    // The snapshot session closes once the job ends.
    expect(engine.of('closeSession').map((s) => s.session)).toEqual([10]);
    expect(jobs.status(7, body.job)).toEqual({ jobId: body.job, state: 'done', progress: 1, path: '/x/Cut.mp4' });
    expect(() => jobs.status(8, body.job)).toThrow('No render job');
  });

  it('returns named failures for unsupported and offline exports before and after acceptance', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const input = { format: 'mp4', range: 'all', audio: true, motionBlur: true, alpha: false } as const;
    const first = jobs.exportVideo(1, 'r', project(), input);
    await tick(); await tick();
    const job = (engine.of('submitJob')[0]!.body as { job: string }).job;
    engine.status(job, 'failed', { error: error('unsupported', 'the new engine cannot render this yet: text layer') });
    await expect(first).rejects.toMatchObject({ code: 'unsupported', message: 'the new engine cannot render this yet: text layer' });
    engine.submit = () => error('mediaOffline', 'offline');
    await expect(jobs.exportVideo(1, 'r', project(), input)).rejects.toMatchObject({ code: 'mediaOffline', message: 'offline' });
    // Real failures are errors, with the engine's sentence.
    engine.submit = (body) => ({ type: 'jobAccepted', job: (body as { job: string }).job, state: 'queued', position: 0 });
    const failing = jobs.exportVideo(1, 'r', project(), input);
    await tick(); await tick();
    const id = (engine.of('submitJob').at(-1)!.body as { job: string }).job;
    engine.status(id, 'failed', { error: error('exportFailed', 'The disk is full.') });
    await expect(failing).rejects.toThrow('The disk is full.');
  });

  it('answer queued at once in async mode and after the queue bound in wait mode', async () => {
    let now = 0;
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine, queueBoundMs: 50, now: () => now });
    const input = { format: 'prores', range: 'all', audio: false, motionBlur: false, alpha: false } as const;
    const async = await jobs.exportVideo(1, 'r', project(), { ...input, mode: 'async' });
    expect(async).toMatchObject({ status: 'queued' });
    const waiting = jobs.exportVideo(1, 'r', project(), input);
    await tick(); await tick();
    now = 100;
    await expect(waiting).resolves.toMatchObject({ status: 'queued' });
    // A running job keeps the call waiting: tool timeouts count from job start.
    const running = jobs.exportVideo(1, 'r', project(), input);
    await tick(); await tick();
    const id = (engine.of('submitJob').at(-1)!.body as { job: string }).job;
    engine.status(id, 'running');
    now = 10_000;
    await new Promise((resolve) => setTimeout(resolve, 80));
    engine.status(id, 'done', { path: '/x/a.mov' });
    await expect(running).resolves.toMatchObject({ status: 'done', path: '/x/a.mov' });
    expect(jobs.status(1, (async as { jobId: string }).jobId)).toMatchObject({ state: 'queued' });
  });

  it('stage large documents through a cache file', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const big = project({ notes: 'x'.repeat(5 * 1024 * 1024) });
    void jobs.exportVideo(1, 'r', big, { format: 'mp4', range: 'all', audio: false, motionBlur: false, alpha: false, mode: 'async' });
    await tick(); await tick(); await tick();
    expect(engine.staged).toHaveLength(1);
    expect(engine.of('resync')[0]!.body).toMatchObject({ source: { kind: 'cacheFile', handle: 'h'.repeat(43) } });
  });

  it('cancel a run\'s jobs, and fail unfinished jobs when the job engine restarts', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    engine.generation(1);
    const pending = jobs.exportVideo(1, 'run-9', project(), { format: 'mp4', range: 'all', audio: false, motionBlur: false, alpha: false });
    await tick(); await tick();
    expect(await jobs.cancelRun('run-9')).toBe(1);
    expect(engine.of('cancelJobs')[0]!.body).toEqual({ type: 'cancelJobs', kind: 'run', run: 'run-9' });
    expect(await jobs.cancelRun('other-run')).toBe(0);
    engine.generation(2);
    await expect(pending).rejects.toThrow('job engine stopped');
  });

  it('refuse when the job engine is not running', async () => {
    const engine = new FakeEngine();
    engine.ready = false;
    const jobs = new AgentJobs({ engine });
    expect(jobs.available).toBe(false);
    await expect(jobs.exportVideo(1, 'r', project(), { format: 'mp4', range: 'all', audio: false, motionBlur: false, alpha: false })).rejects.toThrow('not running');
  });
});

describe('agent frames through the job engine', () => {
  const image = (index: number) => ({ status: 'rendered' as const, index, revision: 1, width: 720, height: 405, image: { kind: 'inline' as const, bytes: new Uint8Array([0xff, 0xd8, index]) }, blender: 'notUsed' as const });

  it('render at the agent revision in one batch and reuse the window session until the project changes', async () => {
    const engine = new FakeEngine();
    engine.frames = (body) => ({ type: 'agentFrames', outcomes: (body as { frames: unknown[] }).frames.map((_, i) => image(i)), queuedMs: 3 });
    const jobs = new AgentJobs({ engine });
    const first = await jobs.renderFrames(4, 'run', { project: project(), times: [0, 0.5], width: 720, revision: 12 });
    expect(first).toMatchObject({ status: 'rendered', revision: 1, queuedMs: 3 });
    if (first.status !== 'rendered') throw new Error('expected frames');
    expect(first.images.map((i) => [i.time, i.mimeType, [...i.data]])).toEqual([[0, 'image/jpeg', [0xff, 0xd8, 0]], [0.5, 'image/jpeg', [0xff, 0xd8, 1]]]);
    const [request] = engine.of('agentFrames');
    expect(request!.body).toMatchObject({ run: 'run#frames-1', maxDimension: 720, format: { kind: 'jpeg', quality: 0.74 }, motionBlur: false, blender: 'cachedOrPending', frames: [{ composition: 'P1-root', time: 0 }, { composition: 'P1-root', time: 352_800_000 }] });
    expect(request!.revision).toBe(1);
    expect(engine.of('openSession')[0]!.body).toMatchObject({ role: 'job' });
    await jobs.renderFrames(4, 'run', { project: project(), times: [1], width: 720, revision: 12 });
    expect(engine.of('openSession')).toHaveLength(1);
    expect(engine.of('resync')).toHaveLength(1);
    await jobs.renderFrames(4, 'run', { project: project({ dur: 5 }), times: [1], width: 720, revision: 13 });
    expect(engine.of('resync').map((s) => (s.body as { revision: number }).revision)).toEqual([1, 2]);
    expect(engine.of('agentFrames').at(-1)!.revision).toBe(2);
    // Portrait compositions keep today's width: the longest edge grows.
    await jobs.renderFrames(4, 'run', { project: project({ w: 720, h: 1280 }), times: [0], width: 360, revision: 14 });
    expect(engine.of('agentFrames').at(-1)!.body).toMatchObject({ maxDimension: 640 });
    await jobs.forgetOwner(4);
    expect(engine.of('closeSession').map((s) => s.session)).toEqual([10]);
  });

  it('restores frame order and refuses partial or duplicate outcomes', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const plan = { project: project(), times: [0, 1], width: 720, revision: 1 };
    engine.frames = () => ({ type: 'agentFrames', outcomes: [image(1), image(0)], queuedMs: 0 });
    const result = await jobs.renderFrames(1, 'r', plan);
    expect(result.images.map(frame => frame.time)).toEqual([0, 1]);
    engine.frames = () => ({ type: 'agentFrames', outcomes: [image(0), image(0)], queuedMs: 0 });
    await expect(jobs.renderFrames(1, 'r', plan)).rejects.toThrow('duplicate frame index');
    engine.frames = () => ({ type: 'agentFrames', outcomes: [image(0)], queuedMs: 0 });
    await expect(jobs.renderFrames(1, 'r', plan)).rejects.toThrow('every requested frame');
    await jobs.forgetOwner(1);
    jobs.dispose();
  });

  it('cancel one batch alone through its signal and answer with the frames that finished first', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const plan = { project: project(), times: [0, 1, 2, 3], width: 720, revision: 1 };
    const cancelled = (index: number) => ({ status: 'failed' as const, index, error: error('cancelled', 'The agent run was cancelled.') });
    let answer!: (response: Response) => void;
    engine.frames = () => new Promise<Response>((resolve) => { answer = resolve; }) as never;
    const original = engine.request.bind(engine);
    engine.request = async (process, body, session, revision) => body.type === 'agentFrames'
      ? (engine.sent.push({ process, body, session: session ?? null, revision: revision ?? null }), await (engine.frames(body) as unknown as Promise<Response>))
      : original(process, body, session, revision);
    const controller = new AbortController();
    const pending = jobs.renderFrames(2, 'run-s', plan, { signal: controller.signal });
    await vi.waitFor(() => expect(engine.of('agentFrames')).toHaveLength(1));
    const wire = (engine.of('agentFrames')[0]!.body as { run: string }).run;
    expect(wire).not.toBe('run-s');
    controller.abort();
    await vi.waitFor(() => expect(engine.of('cancelJobs')).toHaveLength(1));
    // Only this batch: never the agent run (its exports keep rendering).
    expect(engine.of('cancelJobs')[0]!.body).toEqual({ type: 'cancelJobs', kind: 'run', run: wire });
    answer({ type: 'agentFrames', outcomes: [image(0), image(1), cancelled(2), cancelled(3)], queuedMs: 0 });
    const result = await pending;
    expect(result.images.map((frame) => frame.time)).toEqual([0, 1]);
    // A cancel the caller did not ask for (the run was rolled back) is named.
    engine.frames = () => ({ type: 'agentFrames', outcomes: [image(0), cancelled(1)], queuedMs: 0 });
    engine.request = original;
    await expect(jobs.renderFrames(2, 'run-s', { ...plan, times: [0, 1] })).rejects.toMatchObject({ code: 'cancelled' });
    // An already-aborted batch renders nothing.
    const done = new AbortController();
    done.abort();
    expect((await jobs.renderFrames(2, 'run-s', plan, { signal: done.signal })).images).toEqual([]);
  });

  it('cancel every frame batch of an agent run with the run, alongside its exports', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const original = engine.request.bind(engine);
    engine.request = async (process, body, session, revision) => {
      if (body.type !== 'agentFrames') return original(process, body, session, revision);
      engine.sent.push({ process, body, session: session ?? null, revision: revision ?? null });
      await gate;
      return engine.frames(body) as Response;
    };
    const first = jobs.renderFrames(3, 'run-c', { project: project(), times: [0], width: 720, revision: 1 });
    await vi.waitFor(() => expect(engine.of('agentFrames')).toHaveLength(1));
    const second = jobs.renderFrames(3, 'run-c', { project: project(), times: [1], width: 720, revision: 1 });
    await vi.waitFor(() => expect(engine.of('agentFrames')).toHaveLength(2));
    const wires = engine.of('agentFrames').map((s) => (s.body as { run: string }).run);
    expect(new Set(wires).size).toBe(2);
    expect(await jobs.cancelRun('run-c')).toBe(2);
    expect(engine.of('cancelJobs').map((s) => (s.body as { run: string }).run).sort()).toEqual([...wires].sort());
    release();
    await Promise.all([first, second]);
    // Nothing left in flight: a later cancel sends nothing.
    expect(await jobs.cancelRun('run-c')).toBe(0);
    expect(engine.of('cancelJobs')).toHaveLength(2);
  });

  it('returns named frame failures without routing to another renderer', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    engine.frames = () => ({ type: 'agentFrames', outcomes: [{ status: 'failed', index: 0, error: error('unsupported', 'text layer') }], queuedMs: 0 });
    await expect(jobs.renderFrames(1, 'r', { project: project(), times: [0], width: 720, revision: 1 })).rejects.toMatchObject({ code: 'unsupported', message: 'text layer' });
    engine.frames = () => ({ type: 'agentFrames', outcomes: [{ status: 'failed', index: 0, error: error('internal', 'GPU lost') }], queuedMs: 0 });
    await expect(jobs.renderFrames(1, 'r', { project: project(), times: [0], width: 720, revision: 1 })).rejects.toThrow('GPU lost');
    engine.frames = () => error('staleRevision', 'late');
    await expect(jobs.renderFrames(1, 'r', { project: project(), times: [0], width: 720, revision: 1 })).rejects.toThrow('late');
    // A stale session is reopened on the next call.
    engine.frames = () => ({ type: 'agentFrames', outcomes: [image(0)], queuedMs: 0 });
    await jobs.renderFrames(1, 'r', { project: project(), times: [0], width: 720, revision: 1 });
    expect(engine.of('openSession')).toHaveLength(2);
  });
});

describe('export sheet jobs through the job engine', () => {
  const options = { format: 'mp4' as const, alpha: false, fps: 60, scale: 0.5, start: 0.5, end: 2.5, audio: true, motionBlur: false, quality: 'max' as const };

  it('take the sheet\'s rate, scale, range, and tier', () => {
    expect(userExportSettings(project(), options)).toMatchObject({
      format: { kind: 'h264' }, quality: 'max', size: { width: 640, height: 360 }, frameRate: { num: 60, den: 1 },
      range: { start: 352_800_000, end: 5 * 352_800_000 }, dither: true, precision: 'float32'
    });
    // The composition's own rate keeps its exact rational.
    expect(userExportSettings(project(), { ...options, fps: 29.97 }).frameRate).toEqual({ num: 30000, den: 1001 });
    expect(userExportSettings(project(), { ...options, format: 'prores', alpha: true })).toMatchObject({ format: { kind: 'proRes', profile: 'p4444' }, alpha: 'straight', dither: false });
    for (const bad of [{ scale: 0 }, { scale: 17 }, { end: 0.5 }, { fps: 0 }, { quality: 'best' }, { scale: 13 }]) {
      expect(() => userExportSettings(project(), { ...options, ...bad } as never)).toThrow(AgentJobError);
    }
  });

  it('submit a user job to the chosen file and send its statuses to the window', async () => {
    const engine = new FakeEngine();
    const statuses: Array<[number, UserExportStatus]> = [];
    const delivered: Array<[number, string]> = [];
    // The job engine learns the window of each export session before the
    // job is submitted (its text measures with that window's seeds).
    const owners: Array<[number, number | null, number]> = [];
    const sessionOwner = async (session: number, owner: number | null) => void owners.push([session, owner, engine.of('submitJob').length]);
    const jobs = new AgentJobs({ engine, onUserStatus: (o, s) => statuses.push([o, s]), onDelivered: (o, p) => delivered.push([o, p]), sessionOwner });
    const { jobId, frames } = await jobs.exportUser(5, project(), options, '/Users/x/Cut.mp4');
    expect(owners).toEqual([[10, 5, 0]]);
    expect(frames).toBe(120);
    expect(engine.chosen).toEqual([['/Users/x/Cut.mp4', jobId, 10]]);
    const body = engine.of('submitJob')[0]!.body as Extract<Request, { type: 'submitJob' }>;
    expect(body).toMatchObject({ class: 'userJob', run: null, output: { kind: 'userChosen', destination: 'c'.repeat(32) } });
    engine.status(jobId, 'running', { class: 'userJob', progress: 0.25 });
    engine.status(jobId, 'done', { class: 'userJob', path: '/Users/x/Cut.mp4' });
    expect(statuses).toEqual([
      [5, { jobId, state: 'running', progress: 0.25 }],
      [5, { jobId, state: 'done', progress: 1, path: '/Users/x/Cut.mp4' }]
    ]);
    expect(delivered).toEqual([[5, '/Users/x/Cut.mp4']]);
    // A routed job says so; cancels are scoped to the window.
    const second = await jobs.exportUser(5, project(), options, '/Users/x/Cut 2.mp4');
    await jobs.cancelJob(6, second.jobId);
    expect(engine.of('cancelJobs')).toHaveLength(0);
    await jobs.cancelJob(5, second.jobId);
    expect(engine.of('cancelJobs')[0]!.body).toEqual({ type: 'cancelJobs', kind: 'job', job: second.jobId });
    engine.status(second.jobId, 'failed', { class: 'userJob', error: error('unsupported', 'text layers') });
    expect(statuses.at(-1)).toEqual([5, { jobId: second.jobId, state: 'failed', progress: 0.5, error: 'text layers' }]);
  });

  it('do not start an export whose window the job engine\'s font service never takes', async () => {
    const engine = new FakeEngine();
    const calls: Array<[number, number | null]> = [];
    let failures = 1;
    const sessionOwner = async (session: number, owner: number | null) => {
      calls.push([session, owner]);
      if (owner !== null && failures-- > 0) throw new Error('timed out');
    };
    const jobs = new AgentJobs({ engine, onUserStatus: () => undefined, onDelivered: () => undefined, sessionOwner });
    // One miss is tried again.
    await jobs.exportUser(5, project(), options, '/Users/x/Cut.mp4');
    expect(calls).toEqual([[10, 5], [10, 5]]);
    // Two misses: the session closes and nothing is submitted.
    failures = 2;
    const submitted = engine.of('submitJob').length;
    await expect(jobs.exportUser(5, project(), options, '/Users/x/Cut 2.mp4')).rejects.toThrow(/could not lay text out as this window does/);
    expect(engine.of('submitJob')).toHaveLength(submitted);
    expect(engine.of('closeSession').length).toBeGreaterThan(0);
  });
});

describe('Phase 3b delivery through the job engine', () => {
  const sheet = (extra: Record<string, unknown> = {}) => ({
    format: 'hevc', alpha: false, fps: 29.97, scale: 1, start: 0, end: 2, audio: true, motionBlur: false, quality: 'max' as const,
    delivery: { container: 'mov' as const, highDepth: true, audioCoding: 'float' as const, timecode: '01:00:00;00', reel: 'A001' },
    ...extra
  });

  it('builds every delivery option into the settings', () => {
    const s = userExportSettings(project(), sheet());
    expect(s).toMatchObject({
      format: { kind: 'hevc', tenBit: true, container: 'mov' },
      quality: 'max', size: { width: 1280, height: 720 }, frameRate: { num: 30000, den: 1001 },
      color: { kind: 'rec709QuickTime', candidate: 'cicp1_13_1' }, dither: false,
      delivery: { audioCoding: 'pcmFloat', timecode: { start: '01:00:00;00', dropFrame: true, reel: 'A001' }, hideCaptions: false, preview: false, sequenceName: null }
    });
    // PNG sequences: odd sizes, image tags, 16-bit undithered, no audio, frames named after the folder.
    const png = userExportSettings(project(), sheet({ format: 'png', width: 641, height: 361, alpha: true, delivery: { highDepth: true, imageColor: 'rec709' } }), { sequenceName: 'Frames' });
    expect(png).toMatchObject({
      format: { kind: 'pngSequence', sixteenBit: true }, alpha: 'straight', size: { width: 641, height: 361 },
      color: { kind: 'rec709Image' }, audio: false, dither: false, delivery: { sequenceName: 'Frames', audioCoding: null, timecode: null }
    });
    // Today's ids still work; timecode is MOV only.
    const mp4 = userExportSettings(project(), sheet({ format: 'mp4', delivery: { timecode: '01:00:00:00', audioCoding: 'pcm24' } }));
    expect(mp4).toMatchObject({ format: { kind: 'h264' }, dither: true, delivery: { timecode: null, audioCoding: null } });
    expect(() => userExportSettings(project(), sheet({ format: 'hevc', alpha: true }))).toThrow('transparency');
    expect(() => userExportSettings(project(), sheet({ format: 'json' }))).toThrow(AgentJobError);
  });

  it('streams previews and offline waits to the window, marks draft sessions, and closes them', async () => {
    const engine = new FakeEngine();
    const drafts: Array<[number, boolean]> = [];
    (engine as any).setDraftSession = (session: number, draft: boolean) => drafts.push([session, draft]);
    const statuses: Array<[number, UserExportStatus]> = [];
    const jobs = new AgentJobs({ engine, onUserStatus: (owner, s) => statuses.push([owner, s]) });
    const { jobId } = await jobs.exportUser(5, project(), sheet({ format: 'prores-hq' }), '/Users/x/Cut.mov', { draft: true });
    const body = engine.of('submitJob')[0]!.body as Extract<Request, { type: 'submitJob' }>;
    expect((body.spec as any).settings).toMatchObject({ draftFromProxies: true, delivery: { preview: true } });
    expect(drafts).toEqual([[10, true]]);
    engine.status(jobId, 'waitingForMedia', { class: 'userJob', progress: 0, error: error('mediaOffline', '1 clip is offline: a.mov.') });
    engine.status(jobId, 'running', { class: 'userJob', progress: 0.5, preview: { frame: 12, width: 640, height: 360, jpeg: new Uint8Array([0xff, 0xd8]) } });
    expect(statuses[0]![1]).toMatchObject({ state: 'waitingForMedia', waiting: '1 clip is offline: a.mov.' });
    expect(statuses[1]![1]).toMatchObject({ state: 'running', preview: { frame: 12, width: 640, height: 360 } });
    expect([...statuses[1]![1].preview!.jpeg]).toEqual([0xff, 0xd8]);
    engine.status(jobId, 'done', { class: 'userJob', path: '/Users/x/Cut.mov' });
    expect(drafts).toEqual([[10, true], [10, false]]);
  });

  it('keeps queue font seeds scoped to the starting renderer', async () => {
    const engine = new FakeEngine();
    const sessionOwner = vi.fn(async () => {});
    const jobs = new AgentJobs({ engine, sessionOwner });
    await jobs.exportQueued(project(), sheet({ format: 'webm-vp9' }), '/Users/x/Queued.webm', 'uniqueName', [], 42);
    expect(sessionOwner).toHaveBeenCalledWith(10, 42);
  });

  it('runs queue jobs without a window and reports to the queue', async () => {
    const engine = new FakeEngine();
    const queued: Array<[string, UserExportStatus]> = [];
    const windows = vi.fn();
    const jobs = new AgentJobs({ engine, onQueueStatus: (id, s) => queued.push([id, s]), onUserStatus: windows });
    const { jobId } = await jobs.exportQueued(project(), sheet({ format: 'webm-vp9' }), '/Users/x/Web.webm', 'uniqueName');
    expect(jobId.startsWith('queue-export-')).toBe(true);
    expect(engine.chosen).toEqual([['/Users/x/Web.webm', jobId, 10]]);
    const body = engine.of('submitJob')[0]!.body as Extract<Request, { type: 'submitJob' }>;
    expect((body.spec as any).settings).toMatchObject({ format: { kind: 'webm', codec: 'vp9' }, delivery: { preview: false } });
    engine.status(jobId, 'done', { class: 'userJob', path: '/Users/x/Web (2).webm' });
    expect(queued).toEqual([[jobId, { jobId, state: 'done', progress: 1, path: '/Users/x/Web (2).webm' }]]);
    expect(windows).not.toHaveBeenCalled();
    await jobs.cancelQueued(jobId);
  });

  it('exports agent presets and formats, PNG sequences as folders, and lists them for the queue', async () => {
    const engine = new FakeEngine();
    const listed: string[] = [];
    const jobs = new AgentJobs({ engine, onAgentJob: (job) => listed.push(`${job.name}: ${job.summary} ${job.status.state}`) });
    void jobs.exportVideo(7, 'run-1', project(), { name: 'Frames', format: '', preset: 'png', range: 'all', audio: true, motionBlur: false, alpha: true, mode: 'async' });
    void jobs.exportVideo(7, 'run-1', project(), { format: 'prores-hq', preset: 'master-alpha', range: 'all', audio: true, motionBlur: false, alpha: false, mode: 'async' });
    await tick(); await tick(); await tick();
    const [png, hq] = engine.of('submitJob').map((s) => s.body as Extract<Request, { type: 'submitJob' }>);
    expect(png!.output).toEqual({ kind: 'agentAutomatic', suggestedName: 'Frames', extension: 'png' });
    expect((png!.spec as any).settings).toMatchObject({ format: { kind: 'pngSequence', sixteenBit: false }, alpha: 'straight', audio: false, color: { kind: 'srgbImage' }, delivery: { sequenceName: 'Frames' } });
    expect(hq!.output).toMatchObject({ extension: 'mov' });
    expect((hq!.spec as any).settings).toMatchObject({ format: { kind: 'proRes', profile: 'hq' }, alpha: 'none', delivery: { audioCoding: 'pcm24' } });
    expect(listed).toEqual(['Frames: PNG sequence · 1280×720 queued', 'Launch: QuickTime · ProRes 422 HQ · 1280×720 queued']);
    await expect(jobs.exportVideo(7, 'run-1', project(), { format: 'h264', preset: 'bogus', range: 'all', audio: true, motionBlur: false, alpha: false })).rejects.toThrow('preset');
  });
});

describe('definitions in the job engine (contract 10)', () => {
  const snapshot = (id: string, source = 'void main(){}') => ({
    id, kind: 'effect', version: 0, origin: 'local', renderer: null,
    glsl: { source, bodyLineOffset: 0, bodyUtf16Len: source.length, bodyUtf8Len: source.length },
    params: [], flags: { passes: 1, keepOrig: false, backdrop: false, rawShader: true, viewportSafe: false, viewportPadding: [], clamp: false }
  }) as any;

  it('push the window\'s set to an export snapshot and wait for the acknowledgment before submitting', async () => {
    const engine = new FakeEngine();
    const relay = new DefinitionRelay();
    relay.update(7, { generation: 1, full: true, upserts: [snapshot('acme.glow'), snapshot('blur')], removals: [] });
    // A removed registration stays missing for the job (skipped, as the window skips it).
    relay.update(7, { generation: 2, full: false, upserts: [], removals: [{ kind: 'effect', id: 'blur' }] });
    let acknowledge!: () => void;
    const acknowledged = new Promise<void>((resolve) => { acknowledge = resolve; });
    let pushes = 0;
    const answers = engine.definitions;
    engine.definitions = (body) => { pushes++; return answers(body); };
    const original = engine.request.bind(engine);
    engine.request = async (process, body, session, revision) => {
      if (body.type === 'pushDefinitions' && pushes === 0) await acknowledged;
      return original(process, body, session, revision);
    };
    const jobs = new AgentJobs({ engine, definitions: (owner) => relay.current(owner) });
    const result = jobs.exportVideo(7, 'run-d', project(), { name: 'Cut', format: 'mp4', range: 'all', audio: true, motionBlur: false, alpha: false });
    await tick(); await tick();
    // Not submitted while the definitions are unacknowledged.
    expect(engine.of('submitJob')).toHaveLength(0);
    acknowledge();
    await tick(); await tick();
    const order = engine.sent.filter((s) => s.session === 10 || s.body.type === 'submitJob').map((s) => s.body.type);
    expect(order).toEqual(['resync', 'pushDefinitions', 'pushDefinitions', 'submitJob']);
    const [full, missing] = engine.of('pushDefinitions').map((s) => s.body as any);
    expect(full).toMatchObject({ generation: 1, full: true, removals: [] });
    expect(full.upserts.map((d: any) => d.id)).toEqual(['acme.glow']);
    expect(missing).toEqual({ type: 'pushDefinitions', generation: 2, full: false, upserts: [], removals: [{ kind: 'effect', id: 'blur' }] });
    engine.status((engine.of('submitJob')[0]!.body as any).job, 'done', { path: '/x/Cut.mp4' });
    await expect(result).resolves.toMatchObject({ status: 'done' });
  });

  it('fail the export instead of rendering without the window\'s definitions', async () => {
    const engine = new FakeEngine();
    engine.definitions = () => error('invalidMessage', 'bad definition');
    const relay = new DefinitionRelay();
    relay.update(7, { generation: 1, full: true, upserts: [snapshot('acme.glow')], removals: [] });
    const jobs = new AgentJobs({ engine, definitions: (owner) => relay.current(owner) });
    await expect(jobs.exportVideo(7, 'run-d', project(), { format: 'mp4', range: 'all', audio: true, motionBlur: false, alpha: false }))
      .rejects.toThrow('refused the window\'s definitions: bad definition');
    expect(engine.of('submitJob')).toHaveLength(0);
    expect(engine.of('closeSession').map((s) => s.session)).toEqual([10]);
  });

  it('bring the window\'s frame session up to date after every publication, never with a stale set', async () => {
    const engine = new FakeEngine();
    const relay = new DefinitionRelay();
    relay.update(3, { generation: 1, full: true, upserts: [snapshot('blur')], removals: [] });
    const jobs = new AgentJobs({ engine, definitions: (owner) => relay.current(owner) });
    const plan = { project: project(), times: [0], width: 640, revision: 1 };
    await jobs.renderFrames(3, 'run-f', plan);
    let pushes = engine.of('pushDefinitions').map((s) => s.body as any);
    expect(pushes.map((p) => [p.generation, p.full])).toEqual([[1, true]]);
    // The batch was asked for after the acknowledgment.
    const types = engine.sent.map((s) => s.body.type);
    expect(types.indexOf('pushDefinitions')).toBeLessThan(types.indexOf('agentFrames'));
    // Nothing changed: no push.
    await jobs.renderFrames(3, 'run-f', plan);
    expect(engine.of('pushDefinitions')).toHaveLength(1);
    // An agent publishes an effect: the next batch gets the whole set at
    // the session's next generation first.
    relay.update(3, { generation: 2, full: false, upserts: [snapshot('acme.new')], removals: [] });
    await jobs.renderFrames(3, 'run-f', plan);
    pushes = engine.of('pushDefinitions').map((s) => s.body as any);
    expect(pushes.map((p) => [p.generation, p.full])).toEqual([[1, true], [2, true]]);
    expect(pushes[1].upserts.map((d: any) => d.id).sort()).toEqual(['acme.new', 'blur']);
    const lastFrames = engine.of('agentFrames').at(-1)!;
    expect(engine.sent.indexOf(lastFrames)).toBeGreaterThan(engine.sent.indexOf(engine.of('pushDefinitions').at(-1)!));
    // Concurrent batches after another change push once, in order.
    relay.update(3, { generation: 3, full: false, upserts: [], removals: [{ kind: 'effect', id: 'acme.new' }] });
    await Promise.all([jobs.renderFrames(3, 'run-f', plan), jobs.renderFrames(3, 'run-g', plan)]);
    pushes = engine.of('pushDefinitions').map((s) => s.body as any);
    expect(pushes.map((p) => [p.generation, p.full])).toEqual([[1, true], [2, true], [3, true], [4, false]]);
    expect(pushes[3].removals).toEqual([{ kind: 'effect', id: 'acme.new' }]);
  });

  it('run queue jobs with the set captured when they were queued', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const captured = [{ generation: 1, full: true, upserts: [snapshot('acme.fork')], removals: [] }];
    await jobs.exportQueued(project(), { format: 'mp4', fps: 30, scale: 1, start: 0, end: 1, quality: 'high', audio: true, motionBlur: false, alpha: false } as any, '/x/out.mp4', 'replace', captured as any);
    const [push] = engine.of('pushDefinitions');
    expect(push!.body).toMatchObject({ generation: 1, full: true, upserts: [{ id: 'acme.fork' }] });
    expect(engine.sent.indexOf(push!)).toBeLessThan(engine.sent.indexOf(engine.of('submitJob')[0]!));
  });

  it('compile definitions on a scratch session and give back their failures (validate_effect)', async () => {
    const engine = new FakeEngine();
    const failure = { code: 'shaderCompile', scope: { kind: 'definition', definition: 'acme.glow' }, message: 'no', retry: { kind: 'afterUserAction' }, detail: null };
    engine.definitions = (body) => ({ type: 'definitionsApplied', generation: (body as any).generation, failures: [failure] } as any);
    const jobs = new AgentJobs({ engine });
    await expect(jobs.compileDefinitions([snapshot('acme.glow')])).resolves.toEqual([failure]);
    expect(engine.of('openSession')[0]!.body).toMatchObject({ role: 'job' });
    expect(engine.of('pushDefinitions')[0]!.body).toMatchObject({ generation: 1, full: true, upserts: [{ id: 'acme.glow' }], removals: [] });
    expect(engine.of('closeSession').map((s) => s.session)).toEqual([10]);
  });
});
