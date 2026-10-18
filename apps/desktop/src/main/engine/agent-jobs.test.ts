import { describe, expect, it, vi } from 'vitest';

import { canonicalHash, canonicalProject } from '../../shared/canonical-json';
import type { EngineError, EngineProcess, JobStatus, Request, Response } from '../../shared/engine-protocol';
import { AgentJobError, AgentJobs, exportSettingsFor, suggestedName, userExportSettings, type AgentJobsEngine, type UserExportStatus } from './agent-jobs';
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
  frames: (body: Request) => Response | EngineError = () => ({ type: 'agentFrames', outcomes: [], queuedMs: 0 });
  staged: Array<{ session: number; bytes: number }> = [];

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
  await jobs.exportVideo(1, 'run', project(), input);
  expect(bindMediaSession).toHaveBeenLastCalledWith(11, null);
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

  it('route unsupported projects to the legacy exporter, before and after acceptance', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    const input = { format: 'mp4', range: 'all', audio: true, motionBlur: true, alpha: false } as const;
    const first = jobs.exportVideo(1, 'r', project(), input);
    await tick(); await tick();
    const job = (engine.of('submitJob')[0]!.body as { job: string }).job;
    engine.status(job, 'failed', { error: error('unsupported', 'the new engine cannot render this yet: text layer') });
    await expect(first).resolves.toEqual({ status: 'routed', reason: 'the new engine cannot render this yet: text layer' });
    engine.submit = () => error('mediaOffline', 'offline');
    await expect(jobs.exportVideo(1, 'r', project(), input)).resolves.toEqual({ status: 'routed', reason: 'offline' });
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
    expect(request!.body).toMatchObject({ run: 'run', maxDimension: 720, format: { kind: 'jpeg', quality: 0.74 }, motionBlur: false, blender: 'cachedOrPending', frames: [{ composition: 'P1-root', time: 0 }, { composition: 'P1-root', time: 352_800_000 }] });
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

  it('route compositions the engine cannot draw, and fail other frame errors', async () => {
    const engine = new FakeEngine();
    const jobs = new AgentJobs({ engine });
    engine.frames = () => ({ type: 'agentFrames', outcomes: [{ status: 'failed', index: 0, error: error('unsupported', 'text layer') }], queuedMs: 0 });
    await expect(jobs.renderFrames(1, 'r', { project: project(), times: [0], width: 720, revision: 1 })).resolves.toEqual({ status: 'routed', reason: 'text layer' });
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
    const jobs = new AgentJobs({ engine, onUserStatus: (o, s) => statuses.push([o, s]), onDelivered: (o, p) => delivered.push([o, p]) });
    const { jobId, frames } = await jobs.exportUser(5, project(), options, '/Users/x/Cut.mp4');
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
    expect(statuses.at(-1)).toEqual([5, { jobId: second.jobId, state: 'failed', progress: 0.5, error: 'text layers', routed: true }]);
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
