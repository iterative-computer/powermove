import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import type { JobSpec, JobStatus, Request, Response } from '../../shared/engine-protocol';
import { ENGINE_IPC } from '../../shared/engine-ipc';
import { SourceJobs, installSourceJobsIpc, validBrowserRendition, validSourceFilmstrip, validSourcePrepare } from './source-jobs';
import { SourceSessionPool } from './source-session';
import type { AgentJobsEngine } from './agent-jobs';

const roots: string[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true }); });
const request = () => ({ project: JSON.stringify({ id: 'p', w: 100, h: 100, fps: 30, dur: 3, layers: [], assets: { clip: { id: 'clip', kind: 'video', contentHash: 'frozen' } } }), revision: 7, requestId: 'request', asset: 'clip' });

async function fixture(mode: 'done' | 'failed' | 'wait' = 'done') {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-source-job-test-')); roots.push(root);
  let destination = '';
  const heldOutputs = new Set<string>();
  let nextOutput = 0;
  const status = new Set<(status: JobStatus) => void>();
  const statusListeners = new Set<(status: unknown) => void>();
  let ready = true;
  let nextSession = 5;
  const sent: Array<{ body: Request; session?: number | null; revision?: number | null }> = [];
  const engine: AgentJobsEngine = {
    isReady: () => ready,
    stageDocument: vi.fn(),
    chooseOutput: vi.fn((file) => { destination = file; const handle = String.fromCharCode(97 + nextOutput++).repeat(32); heldOutputs.add(handle); return handle; }),
    listen: listener => { if (listener.jobStatus) status.add(listener.jobStatus); if (listener.status) statusListeners.add(listener.status);
      return () => { if (listener.jobStatus) status.delete(listener.jobStatus); if (listener.status) statusListeners.delete(listener.status); }; },
    request: vi.fn(async (_process, body, session, revision) => {
      sent.push({ body, session, revision });
      if (body.type === 'openSession') return { type: 'sessionOpened', session: nextSession++ } as Response;
      if (body.type === 'submitJob') {
        await writeFile(destination, new Uint8Array([1, 2, 3, 4]));
        if (mode !== 'wait') queueMicrotask(() => {
          for (const receive of status) receive({ job: body.job, state: mode, error: mode === 'failed' ? { message: 'conversion failed' } : null } as JobStatus);
        });
      }
      return { type: 'ack' } as Response;
    })
  };
  const bind = vi.fn(async (_session: number, _project: Record<string, any> | null) => {});
  const jobs = new SourceJobs({ engine, directory: root, bindMediaSession: bind, releaseOutput: handle => { heldOutputs.delete(handle); } });
  const spec = (session: number): JobSpec => ({ kind: 'browserRendition', session, asset: 'clip', format: 'webm' });
  return { root, engine, jobs, spec, bind, sent, status, heldOutputs,
    setReady(value: boolean) { ready = value; for (const listener of statusListeners) listener({}); } };
}

describe('source job lifecycle', () => {
  it('pins old storage-key media before a cold-engine wait can admit a replacement', async () => {
    const f = await fixture();
    let keyHash = 'original';
    let snapshotted = false;
    f.engine.isReady = vi.fn(() => { if (snapshotted) keyHash = 'replacement'; return true; });
    const jobs = new SourceJobs({ engine: f.engine, directory: f.root, bindMediaSession: f.bind,
      snapshotMedia: async project => {
        const frozen = { ...project, assets: { clip: { ...project.assets.clip, contentHash: keyHash } } };
        snapshotted = true;
        return frozen;
      } });
    const value = request();
    const project = JSON.parse(value.project); delete project.assets.clip.contentHash; project.assets.clip.storageKey = 'mutable-key';
    await jobs.run(1, { ...value, project: JSON.stringify(project) }, f.spec, 'webm');
    expect(f.bind.mock.calls[0]![1]).toMatchObject({ assets: { clip: { contentHash: 'original' } } });
    expect(keyHash).toBe('replacement');
    jobs.dispose(); f.jobs.dispose();
  });
  it('binds frozen originals before resync, receives output and removes session and scratch files', async () => {
    const f = await fixture();
    expect(await f.jobs.run(1, request(), f.spec, 'webm')).toEqual(new Uint8Array([1, 2, 3, 4]));
    expect(f.bind.mock.calls[0]).toEqual([5, expect.objectContaining({ assets: { clip: expect.objectContaining({ contentHash: 'frozen' }) } })]);
    const submit = f.sent.find(s => s.body.type === 'submitJob')!;
    expect(submit.revision).toBe(7);
    expect(submit.body).toMatchObject({ output: { kind: 'userChosen', destination: 'a'.repeat(32) }, spec: { kind: 'browserRendition', session: 5 } });
    expect(f.sent.at(-1)?.body.type).toBe('closeSession');
    expect(f.bind.mock.calls.at(-1)).toEqual([5, null]);
    expect(f.status.size).toBe(0);
    expect(f.heldOutputs.size).toBe(0);
    expect(await readdir(f.root)).toEqual([]);
    f.jobs.dispose();
  });

  it('cleans scratch and bindings on conversion failure', async () => {
    const f = await fixture('failed');
    await expect(f.jobs.run(1, request(), f.spec, 'webm')).rejects.toThrow('conversion failed');
    expect(await readdir(f.root)).toEqual([]);
    expect(f.bind.mock.calls.at(-1)).toEqual([5, null]);
    expect(f.status.size).toBe(0);
    expect(f.heldOutputs.size).toBe(0);
    f.jobs.dispose();
  });

  it('scopes cancellation to the owner and removes failed output', async () => {
    const f = await fixture('wait');
    const result = f.jobs.run(1, request(), f.spec, 'webm');
    const rejected = expect(result).rejects.toThrow('cancelled');
    await vi.waitFor(() => expect(f.sent.some(s => s.body.type === 'submitJob')).toBe(true));
    f.jobs.cancel(2, 'request');
    expect(f.sent.filter(s => s.body.type === 'cancelJobs')).toHaveLength(0);
    f.jobs.cancel(1, 'request');
    await rejected;
    expect(await readdir(f.root)).toEqual([]);
    expect(f.status.size).toBe(0);
    expect(f.heldOutputs.size).toBe(0);
    f.jobs.dispose();
  });

  it('warms a real frame and reuses its frozen session for subsequent strips', async () => {
    const f = await fixture();
    const prepared = { ...request(), width: 1, height: 1, time: .25, fit: 'stretch' as const, color: 'display' as const };
    expect(validSourcePrepare(prepared)).toBe(true);
    await f.jobs.prepare(1, prepared);
    const warm = f.sent.find(s => s.body.type === 'submitJob')!;
    expect(warm.body).toMatchObject({ spec: { kind: 'sourceFilmstrip', session: 5, times: [176400000] } });
    await f.jobs.prepare(1, { ...prepared, requestId: 'already-warm' });
    expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(1);
    expect(await f.jobs.run(1, { ...request(), requestId: 'strip' }, session => ({ kind: 'sourceFilmstrip', session,
      asset: 'clip', times: [0], outputSize: { width: 1, height: 1 }, fit: 'stretch', color: 'display' }), 'rgba', true))
      .toEqual(new Uint8Array([1, 2, 3, 4]));
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(1);
    expect(f.sent.filter(s => s.body.type === 'resync')).toHaveLength(1);
    expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(2);
    expect(f.heldOutputs.size).toBe(0);
    expect(f.bind.mock.calls.filter(call => call[1] !== null)).toHaveLength(1);
    f.jobs.forget(1);
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'closeSession')).toHaveLength(1));
    expect(f.bind.mock.calls.at(-1)).toEqual([5, null]);
    expect(await readdir(f.root)).toEqual([]);
    f.jobs.dispose();
  });

  it('advances the same source session across poster and unrelated project edits', async () => {
    const f = await fixture();
    const before = JSON.parse(request().project);
    before.layers = [{ id: 'unrelated' }];
    before.assets.other = { id: 'other', contentHash: 'other' };
    before.assets.clip.poster = false;
    const first = { ...request(), project: JSON.stringify(before), width: 1, height: 1,
      time: 0, fit: 'stretch' as const, color: 'display' as const };
    await f.jobs.prepare(1, first);
    const after = structuredClone(before);
    after.layers = [{ id: 'changed' }];
    after.assets.other.contentHash = 'new-other';
    after.assets.clip.poster = true;
    await f.jobs.run(1, { ...request(), project: JSON.stringify(after), revision: 8, requestId: 'strip' },
      session => ({ kind: 'sourceFilmstrip', session, asset: 'clip', times: [0], outputSize: { width: 1, height: 1 },
        fit: 'stretch', color: 'display' }), 'rgba', true);
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(1);
    expect(f.sent.filter(s => s.body.type === 'resync').map(s => (s.body as Extract<Request, { type: 'resync' }>).revision)).toEqual([7, 8]);
    expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(2);
    const bound = f.bind.mock.calls.find(call => call[1] !== null)![1]!;
    expect(Object.keys(bound.assets)).toEqual(['clip']);
    expect(bound.assets.clip.poster).toBeUndefined();
    f.jobs.dispose();
  });

  it('joins an in-flight warm across the poster revision, then advances before submitting', async () => {
    const f = await fixture('wait');
    const before = JSON.parse(request().project);
    const first = { ...request(), project: JSON.stringify(before), width: 1, height: 1,
      time: 0, fit: 'stretch' as const, color: 'display' as const };
    const warm = f.jobs.prepare(1, first);
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(1));
    before.assets.clip.poster = true;
    const strip = f.jobs.run(1, { ...request(), project: JSON.stringify(before), revision: 8, requestId: 'strip' },
      session => ({ kind: 'sourceFilmstrip', session, asset: 'clip', times: [0], outputSize: { width: 1, height: 1 },
        fit: 'stretch', color: 'display' }), 'rgba', true);
    await new Promise(resolve => setTimeout(resolve, 20));
    expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(1);
    const firstJob = (f.sent.find(s => s.body.type === 'submitJob')!.body as Extract<Request, { type: 'submitJob' }>).job;
    for (const receive of f.status) receive({ job: firstJob, state: 'done' } as JobStatus);
    await warm;
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(2));
    const secondJob = (f.sent.filter(s => s.body.type === 'submitJob')[1]!.body as Extract<Request, { type: 'submitJob' }>).job;
    for (const receive of f.status) receive({ job: secondJob, state: 'done' } as JobStatus);
    await strip;
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(1);
    expect(f.sent.filter(s => s.body.type === 'resync').map(s => (s.body as Extract<Request, { type: 'resync' }>).revision)).toEqual([7, 8]);
    f.jobs.dispose();
  });

  it('replaces the session if a mutable source key resolves to different frozen bytes', async () => {
    const f = await fixture();
    let hash = 'original';
    const jobs = new SourceJobs({ engine: f.engine, directory: f.root, bindMediaSession: f.bind,
      snapshotMedia: async project => ({ ...project, assets: { clip: { ...project.assets.clip, contentHash: hash } } }) });
    const input = JSON.parse(request().project);
    delete input.assets.clip.contentHash;
    input.assets.clip.storageKey = 'mutable-key';
    const value = { ...request(), project: JSON.stringify(input) };
    await jobs.prepare(1, { ...value, width: 1, height: 1, time: 0, fit: 'stretch', color: 'display' });
    hash = 'replacement';
    await jobs.run(1, { ...value, requestId: 'strip' }, session => ({ kind: 'sourceFilmstrip', session,
      asset: 'clip', times: [0], outputSize: { width: 1, height: 1 }, fit: 'stretch', color: 'display' }), 'rgba', true);
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(2);
    expect(f.bind.mock.calls.filter(call => call[1] !== null).map(call => call[1]?.assets.clip.contentHash)).toEqual(['original', 'replacement']);
    jobs.dispose(); f.jobs.dispose();
  });

  it('invalidates a prepared session after a job-engine restart', async () => {
    const f = await fixture();
    const prepared = { ...request(), width: 1, height: 1, time: 0, fit: 'stretch' as const, color: 'display' as const };
    await f.jobs.prepare(1, prepared);
    f.setReady(false);
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'closeSession')).toHaveLength(1));
    f.setReady(true);
    await f.jobs.prepare(1, { ...prepared, requestId: 'again' });
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(2);
    expect(f.bind.mock.calls.filter(call => call[1] !== null)).toHaveLength(2);
    f.jobs.dispose();
  });

  it.each(['forget', 'dispose'] as const)('does not resurrect a source after %s during media snapshot', async action => {
    const f = await fixture();
    let resolveSnapshot!: () => void;
    const snapshotGate = new Promise<void>(resolve => { resolveSnapshot = resolve; });
    const snapshotMedia = vi.fn(async project => { await snapshotGate; return project; });
    const pool = new SourceSessionPool({ engine: f.engine, snapshotMedia, bindMediaSession: f.bind });
    const pending = pool.acquire(1, request());
    const rejected = expect(pending).rejects.toThrow('invalidated');
    await vi.waitFor(() => expect(snapshotMedia).toHaveBeenCalledOnce());
    if (action === 'forget') pool.forget(1);
    else pool.dispose();
    resolveSnapshot();
    await rejected;
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(0);
    expect(f.bind).not.toHaveBeenCalled();
    pool.dispose(); f.jobs.dispose();
  });

  it('does not open a stale source after a job-engine restart during media snapshot', async () => {
    const f = await fixture();
    let resolveSnapshot!: () => void;
    const snapshotGate = new Promise<void>(resolve => { resolveSnapshot = resolve; });
    let hold = false;
    const snapshotMedia = vi.fn(async project => { if (hold) await snapshotGate; return project; });
    const pool = new SourceSessionPool({ engine: f.engine, snapshotMedia, bindMediaSession: f.bind });
    const first = await pool.acquire(1, request());
    first.release();
    hold = true;
    const changed = JSON.parse(request().project);
    changed.assets.clip.contentHash = 'replacement';
    const pending = pool.acquire(1, { ...request(), project: JSON.stringify(changed), revision: 8 });
    const rejected = expect(pending).rejects.toThrow('invalidated');
    await vi.waitFor(() => expect(snapshotMedia).toHaveBeenCalledTimes(2));
    f.setReady(false);
    resolveSnapshot();
    await rejected;
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(1);
    expect(f.bind.mock.calls.filter(call => call[1] !== null)).toHaveLength(1);
    await vi.waitFor(() => expect(f.bind.mock.calls).toContainEqual([first.session, null]));
    pool.dispose(); f.jobs.dispose();
  });

  it('rejects the first snapshot if its ready engine restarts before any session opens', async () => {
    const f = await fixture();
    let resolveSnapshot!: () => void;
    const snapshotGate = new Promise<void>(resolve => { resolveSnapshot = resolve; });
    const snapshotMedia = vi.fn(async project => { await snapshotGate; return project; });
    const pool = new SourceSessionPool({ engine: f.engine, snapshotMedia, bindMediaSession: f.bind });
    const pending = pool.acquire(1, request());
    const rejected = expect(pending).rejects.toThrow('invalidated');
    await vi.waitFor(() => expect(snapshotMedia).toHaveBeenCalledOnce());
    f.setReady(false);
    f.setReady(true);
    resolveSnapshot();
    await rejected;
    expect(f.sent.filter(s => s.body.type === 'openSession')).toHaveLength(0);
    expect(f.bind).not.toHaveBeenCalled();
    pool.dispose(); f.jobs.dispose();
  });

  it('serializes higher revisions and never resyncs a session backward', async () => {
    const f = await fixture();
    const pool = new SourceSessionPool({ engine: f.engine, bindMediaSession: f.bind });
    const first = await pool.acquire(1, request());
    first.release();
    const [eight, nine] = await Promise.all([
      pool.acquire(1, { ...request(), revision: 8 }),
      pool.acquire(1, { ...request(), revision: 9 })
    ]);
    expect([eight.session, nine.session]).toEqual([first.session, first.session]);
    expect(f.sent.filter(s => s.body.type === 'resync').map(s => (s.body as Extract<Request, { type: 'resync' }>).revision))
      .toEqual([7, 8, 9]);
    eight.release(); nine.release();
    const older = await pool.acquire(1, request());
    expect(older.session).not.toBe(first.session);
    expect(f.sent.filter(s => s.body.type === 'resync').map(s => (s.body as Extract<Request, { type: 'resync' }>).revision))
      .toEqual([7, 8, 9, 7]);
    older.release();
    pool.dispose(); f.jobs.dispose();
  });

  it('keeps an in-flight old revision bound until its own job finishes', async () => {
    const f = await fixture('wait');
    const old = f.jobs.run(1, request(), f.spec, 'webm', true);
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(1));
    const changed = JSON.parse(request().project);
    changed.assets.clip.contentHash = 'replacement';
    const next = f.jobs.run(1, { ...request(), project: JSON.stringify(changed), revision: 8, requestId: 'next' }, f.spec, 'webm', true);
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(2));
    expect(f.bind.mock.calls).toEqual([
      [5, expect.objectContaining({ id: 'p' })],
      [6, expect.objectContaining({ id: 'p' })]
    ]);
    const submissions = f.sent.filter(s => s.body.type === 'submitJob');
    for (const receive of f.status) receive({ job: (submissions[0]!.body as Extract<Request, { type: 'submitJob' }>).job, state: 'done' } as JobStatus);
    await old;
    await vi.waitFor(() => expect(f.bind.mock.calls).toContainEqual([5, null]));
    expect(f.bind.mock.calls).not.toContainEqual([6, null]);
    for (const receive of f.status) receive({ job: (submissions[1]!.body as Extract<Request, { type: 'submitJob' }>).job, state: 'done' } as JobStatus);
    await next;
    f.jobs.dispose();
    await vi.waitFor(() => expect(f.bind.mock.calls).toContainEqual([6, null]));
  });

  it('cancels a pending warm job and releases its frozen binding', async () => {
    const f = await fixture('wait');
    const pending = f.jobs.prepare(1, { ...request(), width: 1, height: 1, time: 0, fit: 'stretch', color: 'display' });
    const rejected = expect(pending).rejects.toThrow('cancelled');
    await vi.waitFor(() => expect(f.sent.some(s => s.body.type === 'submitJob')).toBe(true));
    f.jobs.cancel(1, 'request');
    await rejected;
    await vi.waitFor(() => expect(f.bind.mock.calls).toContainEqual([5, null]));
    expect(await readdir(f.root)).toEqual([]);
    f.jobs.dispose();
  });

  it('releases source work on project reset while a frozen rendition continues', async () => {
    const f = await fixture('wait');
    const source = f.jobs.run(1, request(), session => ({ kind: 'sourceFilmstrip', session, asset: 'clip',
      times: [0], outputSize: { width: 1, height: 1 }, fit: 'stretch', color: 'display' }), 'rgba', true);
    const sourceRejected = expect(source).rejects.toThrow('cancelled');
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(1));
    const rendition = f.jobs.run(1, { ...request(), requestId: 'rendition' }, f.spec, 'webm');
    await vi.waitFor(() => expect(f.sent.filter(s => s.body.type === 'submitJob')).toHaveLength(2));
    const submissions = f.sent.filter(s => s.body.type === 'submitJob');
    const sourceJob = (submissions[0]!.body as Extract<Request, { type: 'submitJob' }>).job;
    const renditionJob = (submissions[1]!.body as Extract<Request, { type: 'submitJob' }>).job;
    f.jobs.releasePreparedSources(1);
    await sourceRejected;
    expect(f.sent.some(s => s.body.type === 'cancelJobs' && s.body.kind === 'job' && s.body.job === sourceJob)).toBe(true);
    expect(f.sent.some(s => s.body.type === 'cancelJobs' && s.body.kind === 'job' && s.body.job === renditionJob)).toBe(false);
    for (const receive of f.status) receive({ job: renditionJob, state: 'done' } as JobStatus);
    expect(await rendition).toEqual(new Uint8Array([1, 2, 3, 4]));
    expect(f.heldOutputs.size).toBe(0);
    f.jobs.dispose();
  });
});

it('bounds media jobs before allocating output and accepts non-monotonic source times', () => {
  const strip = { ...request(), width: 320, height: 180, times: [2, 0], fit: 'contain', color: 'display' };
  expect(validSourceFilmstrip(strip)).toBe(true);
  for (const invalid of [{ times: [] }, { times: Array(65).fill(0) }, { times: [-1] }, { width: 0 }, { width: 8192, height: 8192 }, { fit: 'native' }, { color: 'project' }]) {
    expect(validSourceFilmstrip({ ...strip, ...invalid })).toBe(false);
  }
  expect(validBrowserRendition({ ...request(), format: 'webm' })).toBe(true);
  expect(validBrowserRendition({ ...request(), format: 'shell' })).toBe(false);
  expect(validSourcePrepare({ ...strip, time: -1 })).toBe(false);
});

it('returns one packed, bounded RGBA result across the filmstrip IPC', async () => {
  const f = await fixture();
  const handlers = new Map<string, (event: IpcMainInvokeEvent, value: unknown) => Promise<unknown>>();
  const ipc = { handle: (channel: string, handler: (event: IpcMainInvokeEvent, value: unknown) => Promise<unknown>) => {
    handlers.set(channel, handler);
  }, removeHandler: (channel: string) => { handlers.delete(channel); } } as unknown as Pick<IpcMain, 'handle' | 'removeHandler'>;
  const remove = installSourceJobsIpc(ipc, f.jobs, () => true);
  const data = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
  const run = vi.spyOn(f.jobs, 'run').mockResolvedValue(data);
  try {
    const reply = await handlers.get(ENGINE_IPC.sourceFilmstrip)!({ sender: { id: 1 } } as IpcMainInvokeEvent,
      { ...request(), width: 1, height: 1, times: [0, 1], fit: 'stretch', color: 'display' });
    expect(reply).toEqual({ width: 1, height: 1, count: 2, data });
    expect((reply as { data: Uint8Array }).data).toBe(data);
    expect(run).toHaveBeenCalledOnce();
  } finally { remove(); f.jobs.dispose(); }
  expect(handlers.size).toBe(0);
});
