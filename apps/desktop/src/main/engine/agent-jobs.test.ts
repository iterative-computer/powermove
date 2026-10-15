import { describe, expect, it } from 'vitest';

import { canonicalHash, canonicalProject } from '../../shared/canonical-json';
import type { EngineError, EngineProcess, JobStatus, Request, Response } from '../../shared/engine-protocol';
import { AgentJobError, AgentJobs, exportSettingsFor, suggestedName, type AgentJobsEngine } from './agent-jobs';
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

describe('export settings', () => {
  it('match the legacy exporter: even size, exact rate, work area, tiers, tags, dither', () => {
    const mp4 = exportSettingsFor(project(), { format: 'mp4', range: 'work', audio: true, motionBlur: false, alpha: false });
    expect(mp4).toEqual({
      format: { kind: 'h264' }, alpha: 'none', quality: 'high', size: { width: 1280, height: 720 },
      frameRate: { num: 30000, den: 1001 }, range: { start: 705_600_000, end: 3 * 705_600_000 },
      audio: true, motionBlur: false, color: { kind: 'rec709QuickTime', candidate: 'cicp1131' }, precision: 'float32', dither: true, draftFromProxies: false
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
    const result = jobs.exportVideo(7, 'run-1', project(), { name: 'Cut', format: 'mp4', range: 'all', audio: true, motionBlur: false, alpha: false });
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
    engine.status(body.job, 'running');
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
