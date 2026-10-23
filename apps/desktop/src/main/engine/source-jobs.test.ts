import { afterEach, describe, expect, it, vi } from 'vitest';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import type { JobSpec, JobStatus, Request, Response } from '../../shared/engine-protocol';
import { SourceJobs, validBrowserRendition, validSourceFilmstrip } from './source-jobs';
import type { AgentJobsEngine } from './agent-jobs';

const roots: string[] = [];
afterEach(async () => { for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true }); });
const request = () => ({ project: JSON.stringify({ id: 'p', w: 100, h: 100, fps: 30, dur: 3, layers: [], assets: { clip: { id: 'clip', kind: 'video', contentHash: 'frozen' } } }), revision: 7, requestId: 'request', asset: 'clip' });

async function fixture(mode: 'done' | 'failed' | 'wait' = 'done') {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-source-job-test-')); roots.push(root);
  let destination = '';
  const status = new Set<(status: JobStatus) => void>();
  const sent: Array<{ body: Request; session?: number | null; revision?: number | null }> = [];
  const engine: AgentJobsEngine = {
    isReady: () => true,
    stageDocument: vi.fn(),
    chooseOutput: vi.fn((file) => { destination = file; return 'a'.repeat(32); }),
    listen: listener => { if (listener.jobStatus) status.add(listener.jobStatus); return () => { if (listener.jobStatus) status.delete(listener.jobStatus); }; },
    request: vi.fn(async (_process, body, session, revision) => {
      sent.push({ body, session, revision });
      if (body.type === 'openSession') return { type: 'sessionOpened', session: 5 } as Response;
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
  const jobs = new SourceJobs({ engine, directory: root, bindMediaSession: bind });
  const spec = (session: number): JobSpec => ({ kind: 'browserRendition', session, asset: 'clip', format: 'webm' });
  return { root, engine, jobs, spec, bind, sent, status };
}

describe('source job lifecycle', () => {
  it('pins old storage-key media before a cold-engine wait can admit a replacement', async () => {
    const f = await fixture();
    let keyHash = 'original';
    f.engine.isReady = vi.fn(() => { keyHash = 'replacement'; return true; });
    const jobs = new SourceJobs({ engine: f.engine, directory: f.root, bindMediaSession: f.bind,
      snapshotMedia: async project => ({ ...project, assets: { clip: { ...project.assets.clip, contentHash: keyHash } } }) });
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
    expect(await readdir(f.root)).toEqual([]);
    f.jobs.dispose();
  });

  it('cleans scratch and bindings on conversion failure', async () => {
    const f = await fixture('failed');
    await expect(f.jobs.run(1, request(), f.spec, 'webm')).rejects.toThrow('conversion failed');
    expect(await readdir(f.root)).toEqual([]);
    expect(f.bind.mock.calls.at(-1)).toEqual([5, null]);
    expect(f.status.size).toBe(0);
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
});
