import { mkdir, mkdtemp, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { blake3Hex } from '../../shared/blake3';
import type { JobStatus, Request } from '../../shared/engine-protocol';
import type { ProjectProxyIntent, ProxyAssetDescriptor, ProxySnapshot } from '../../shared/proxies';
import { ProtocolError } from './protocol';
import { ASSESSMENTS_FILE, DELETIONS_FILE, EngineProxyService, LOCATIONS_FILE, PROJECT_ROOT, type ProxyServiceOptions, type ProxySource, type ProxySourceScope } from './proxies';

const H1 = '1'.repeat(64), H2 = '2'.repeat(64), H3 = '3'.repeat(64);
const bytes = (hash: string) => Uint8Array.from(Buffer.from(hash, 'hex'));
const JOB = { process: 'job', session: 0 } as const;
const INTERACTIVE = { process: 'interactive', session: 4 } as const;

let temp: string;
beforeEach(async () => { temp = await mkdtemp(path.join(os.tmpdir(), 'pm-proxies-test-')); });
afterEach(async () => { await rm(temp, { recursive: true, force: true }); });

type Harness = {
  service: EngineProxyService;
  cache: string;
  requests: Request[];
  resolves: { asset: string; scope: ProxySourceScope }[];
  sources: Map<string, ProxySource | null>;
  snapshots: { owner: number; snapshot: ProxySnapshot }[];
  last(owner?: number): ProxySnapshot;
  submitted(): Extract<Request, { type: 'submitJob' }>[];
  logs: string[];
  /** Every file `hashFile` read, in order. */
  hashed: string[];
};

function harness(over: Partial<ProxyServiceOptions> & { fail?: (r: Request) => Error | null } = {}): Harness {
  const cache = path.join(temp, 'cache');
  const requests: Request[] = [], resolves: Harness['resolves'] = [], snapshots: Harness['snapshots'] = [], logs: string[] = [], hashed: string[] = [];
  const sources = new Map<string, ProxySource | null>();
  const service = new EngineProxyService({
    cache,
    request: async (r) => { requests.push(r); const e = over.fail?.(r); if (e) throw e; return {}; },
    resolve: async (asset, scope) => { resolves.push({ asset, scope }); return sources.get(`${scope.project}/${asset}`) ?? null; },
    changed: (owner, snapshot) => { snapshots.push({ owner, snapshot }); },
    log: (_level, m) => { logs.push(m); },
    hashFile: async (file) => { hashed.push(file); return blake3Hex(await readFile(file)); },
    now: () => 1_000,
    ...over
  });
  return {
    service, cache, requests, resolves, sources, snapshots, logs, hashed,
    last: (owner = 1) => snapshots.filter((s) => s.owner === owner).at(-1)!.snapshot,
    submitted: () => requests.filter((r): r is Extract<Request, { type: 'submitJob' }> => r.type === 'submitJob')
  };
}

const intent = (over: Partial<ProjectProxyIntent> = {}): ProjectProxyIntent =>
  ({ create: 'onlyWhenAsked', size: 'automatic', format: 'proResProxy', alwaysPlayOriginal: [], ...over });
const video = (contentHash?: string, over: ProxyAssetDescriptor = {}): ProxyAssetDescriptor =>
  ({ kind: 'video', name: 'Clip', ...(contentHash ? { contentHash } : {}), ...over });

/** What pm-proxy's ProxyStore::commit leaves behind. */
async function storeProxy(root: string, hash: string, over: { version?: number; format?: string; size?: string; alpha?: boolean; depth?: number; bytes?: number; key?: string; mediaBytes?: number; blake3?: string } = {}) {
  const key = over.key ?? (hash.slice(0, 8) + (over.format ?? 'proResProxy') + (over.size ?? 'automatic')).padEnd(64, '0').replace(/[^0-9a-f]/g, 'a').slice(0, 64);
  const dir = path.join(root, key.slice(0, 2));
  await mkdir(dir, { recursive: true });
  const size = over.bytes ?? 16;
  const file = path.join(dir, `${key}.mov`);
  const media = Buffer.alloc(over.mediaBytes ?? size, 7);
  await writeFile(file, media);
  await writeFile(path.join(dir, `${key}.json`), JSON.stringify({
    source_hash: hash,
    recipe: { version: over.version ?? 2, requested_format: over.format ?? 'proResProxy', requested_size: over.size ?? 'automatic', width: 960, height: 540, source_depth: over.depth ?? 10, source_has_alpha: over.alpha ?? false },
    byte_size: size, blake3: over.blake3 ?? blake3Hex(media)
  }));
  return { file, record: path.join(dir, `${key}.json`) };
}

const status = (job: string, state: JobStatus['state'], over: Partial<JobStatus> = {}): JobStatus =>
  ({ job, class: 'generation', state, progress: 0, path: null, error: null, ...over });
const exists = (file: string) => stat(file).then(() => true, () => false);

describe('EngineProxyService', () => {
  it('queues a whole-clip job by alias, grants it to the job lane only, and finds the proxy after relaunch', async () => {
    const h = harness();
    h.sources.set('p1/a1', { path: '/media/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a1: video(H1), au: { kind: 'audio' } });
    expect(h.last().assets).toEqual({ a1: { state: 'none' }, au: { state: 'unsupported' } });

    await h.service.create(1, 'p1', ['a1']);
    const [submit] = h.submitted();
    expect(submit).toMatchObject({ class: 'generation', spec: { kind: 'generateProxy', format: 'proResProxy', size: 'automatic' } });
    const alias = (submit!.spec as { asset: string }).asset;
    expect(alias).not.toBe('a1');
    expect(h.last().assets['a1']).toEqual({ state: 'queued' });
    expect(h.last().activity.creating).toEqual({ done: 0, total: 1 });
    expect(h.resolves).toEqual([{ asset: 'a1', scope: { owner: 1, project: 'p1' } }]);

    // Capability guard: aliases are generation-lane grants, not media the interactive engine can name.
    expect(await h.service.resolveAlias(alias, INTERACTIVE)).toBeNull();
    expect(await h.service.resolveAlias(alias)).toBeNull();
    expect(await h.service.resolveAlias('a1', JOB)).toBeNull();
    expect(await h.service.resolveAlias(alias, JOB)).toEqual({ path: '/media/a.mov', contentHash: bytes(H1) });
    const root = h.service.generatedDirectory(JOB);
    expect(root).toBe(path.join(h.cache, 'Generated Media'));
    expect(h.service.generatedDirectory(INTERACTIVE)).toBe(root);

    h.service.status(status(submit!.job, 'running', { progress: 0.25 }));
    expect(h.last().assets['a1']).toEqual({ state: 'creating', progress: 0.25 });
    const { file } = await storeProxy(root, H1);
    h.service.status(status(submit!.job, 'done', { progress: 1, path: file }));
    await h.service.settled();
    expect(h.last().assets['a1']).toEqual({ state: 'ready', width: 960, height: 540, codec: 'ProRes 422 Proxy', bytes: 16 });
    expect(h.last().activity.creating).toBeNull();
    await h.service.dispose();

    // The engine reads the same roots file (POWERMOVE_PROXY_LOCATIONS).
    expect(JSON.parse(await readFile(path.join(h.cache, LOCATIONS_FILE), 'utf8'))).toEqual({ location: { kind: 'cache' }, roots: [root] });
    const relaunched = harness();
    expect((await relaunched.service.snapshot(1, 'p1')).assets).toEqual({});
    await relaunched.service.setIntent(7, 'p1', intent(), { moved: video(H1) });
    expect(relaunched.last(7).assets['moved']).toMatchObject({ state: 'ready' });
    await relaunched.service.dispose();
  });

  it('uses the recipe: format and size select the record, alpha or >10-bit reads as 4444', async () => {
    const h = harness();
    const root = path.join(h.cache, 'Generated Media');
    await storeProxy(root, H1, { format: 'proResLt', size: 'half', alpha: true });
    await storeProxy(root, H2, { format: 'hevc10', size: 'p720' });
    await h.service.setIntent(1, 'p1', intent(), { a1: video(H1), a2: video(H2) });
    expect(h.last().assets).toEqual({ a1: { state: 'none' }, a2: { state: 'none' } });
    await h.service.setIntent(1, 'p1', intent({ format: 'proResLt', size: 'half' }), { a1: video(H1), a2: video(H2) });
    expect(h.last().assets['a1']).toMatchObject({ state: 'ready', codec: 'ProRes 4444' });
    await h.service.setIntent(1, 'p1', intent({ format: 'hevc10', size: 'p720' }), { a1: video(H1), a2: video(H2) });
    expect(h.last().assets['a2']).toMatchObject({ state: 'ready', codec: 'HEVC 10-bit' });
    await h.service.dispose();
  });

  it('only recipe version 2 is ready; Delete still removes this source\'s old-version files', async () => {
    const h = harness();
    const root = path.join(h.cache, 'Generated Media');
    const v1 = await storeProxy(root, H1, { version: 1, key: 'a'.repeat(64) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.setIntent(2, 'p2', intent(), { b: video(H1) });
    expect(h.last(1).assets['a']).toEqual({ state: 'none' });
    expect(await h.service.resolveDraft('a', { project: 'p1' })).toBeNull();
    const v2 = await storeProxy(root, H1, { key: 'c'.repeat(64) });
    expect((await h.service.snapshot(1, 'p1')).assets['a']).toMatchObject({ state: 'ready' });
    expect(h.hashed).toEqual([v2.file]); // old-version files are never playable, so never hashed
    // p2 plays the version 2 file, so Delete in p1 keeps it; the stale version 1 file goes.
    await h.service.remove(1, 'p1', ['a']);
    expect(await exists(v1.file)).toBe(false);
    expect(await exists(v2.file)).toBe(true);
    await h.service.dispose();
  });

  it('runs concurrent create, remove, create, rebuild in call order, with terminal events in between', async () => {
    let inFlight = 0, overlapped = false;
    const h = harness({
      resolve: async () => {
        overlapped ||= ++inFlight > 1;
        await new Promise((r) => setTimeout(r, 3));
        inFlight--;
        return { path: '/m/a.mov', contentHash: bytes(H1) };
      }
    });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    const ops = [
      h.service.create(1, 'p1', ['a']),
      h.service.remove(1, 'p1', ['a']),
      h.service.create(1, 'p1', ['a']),
      h.service.create(1, 'p1', ['a']),
      h.service.rebuild(1, 'p1', ['a'])
    ];
    await ops[0];
    const first = h.submitted()[0]!.job;
    h.service.status(status(first, 'cancelled'));
    await Promise.all(ops);
    expect(overlapped).toBe(false);
    const jobs = h.submitted().map((r) => r.job);
    expect(jobs).toHaveLength(3);
    // The first job had already ended when Delete ran, so only the second needed cancelling.
    expect(h.requests.filter((r) => r.type === 'cancelJobs').map((r) => (r as { job: string }).job)).toEqual([jobs[1]]);
    expect(h.last().assets['a']).toEqual({ state: 'queued' });
    expect(JSON.parse(await readFile(path.join(h.cache, DELETIONS_FILE), 'utf8')).deleted).toEqual({});
    // The rebuilt job's late predecessor finishing does not touch the live job.
    h.service.status(status(jobs[1]!, 'failed', { error: { message: 'boom' } as JobStatus['error'] }));
    await h.service.settled();
    expect(h.last().assets['a']).toEqual({ state: 'queued' });
    h.service.status(status(jobs[2]!, 'running', { progress: 0.5 }));
    expect(h.last().assets['a']).toEqual({ state: 'creating', progress: 0.5 });
    await h.service.dispose();
  });

  it('serializes duplicate creates, and allVideo makes one job per original', async () => {
    const h = harness();
    h.sources.set('p1/a1', { path: '/m/a.mov', contentHash: bytes(H1) });
    h.sources.set('p1/a2', { path: '/m/b.mov', contentHash: bytes(H2) });
    await h.service.setIntent(1, 'p1', intent(), { a1: video(H1), a2: video(H2) });
    await Promise.all([h.service.create(1, 'p1', ['a1', 'a1']), h.service.create(1, 'p1', ['a1'])]);
    expect(h.submitted()).toHaveLength(1);
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a1: video(H1), a2: video(H2) });
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a1: video(H1), a2: video(H2) });
    expect(h.submitted().map((r) => r.spec)).toHaveLength(2);
    expect(h.last().activity.creating).toEqual({ done: 0, total: 2 });
    h.service.status(status(h.submitted()[0]!.job, 'done', { progress: 1 }));
    await h.service.settled();
    expect(h.last().activity.creating).toEqual({ done: 1, total: 2 });
    await h.service.dispose();
  });

  it('shares one job between projects that use the same original', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    h.sources.set('p2/b', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.setIntent(2, 'p2', intent(), { b: video(H1) });
    await h.service.create(1, 'p1', ['a']);
    await h.service.create(2, 'p2', ['b']);
    expect(h.submitted()).toHaveLength(1);
    h.service.status(status(h.submitted()[0]!.job, 'running', { progress: 0.5 }));
    expect(h.last(2).assets['b']).toEqual({ state: 'creating', progress: 0.5 });
    await h.service.dispose();
  });

  it('deletes only generated files, keeps the deletion across relaunch, and Create or a recipe change ends it', async () => {
    const h = harness();
    const root = path.join(h.cache, 'Generated Media');
    const original = path.join(temp, 'original.mov');
    await writeFile(original, 'original bytes');
    h.sources.set('p1/a1', { path: original, contentHash: bytes(H1) });
    const stored = await storeProxy(root, H1);
    const assets = { a1: video(H1) };
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), assets);
    expect(h.submitted()).toHaveLength(0);
    expect(h.last().assets['a1']).toMatchObject({ state: 'ready' });

    await h.service.remove(1, 'p1', ['a1']);
    expect(await exists(stored.file)).toBe(false);
    expect(await exists(stored.record)).toBe(false);
    expect(await readFile(original, 'utf8')).toBe('original bytes');
    expect(h.last().assets['a1']).toEqual({ state: 'none' });
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), assets);
    expect(h.submitted()).toHaveLength(0);
    await h.service.dispose();

    const relaunched = harness();
    relaunched.sources.set('p1/a1', { path: original, contentHash: bytes(H1) });
    await relaunched.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), assets);
    expect(relaunched.submitted()).toHaveLength(0);
    expect(JSON.parse(await readFile(path.join(relaunched.cache, DELETIONS_FILE), 'utf8')).deleted.p1.a1).toMatchObject({ hash: H1, format: 'proResProxy', size: 'automatic' });
    // A recipe change ends the deletion; changing back regenerates under allVideo.
    await relaunched.service.setIntent(1, 'p1', intent({ create: 'allVideo', size: 'half' }), assets);
    expect(relaunched.submitted()).toHaveLength(1);
    await relaunched.service.dispose();

    const explicit = harness();
    explicit.sources.set('p1/a1', { path: original, contentHash: bytes(H1) });
    await explicit.service.setIntent(1, 'p1', intent(), assets);
    await explicit.service.remove(1, 'p1', ['a1']);
    await explicit.service.create(1, 'p1', ['a1']);
    expect(explicit.submitted()).toHaveLength(1);
    expect(JSON.parse(await readFile(path.join(explicit.cache, DELETIONS_FILE), 'utf8')).deleted).toEqual({});
    await explicit.service.dispose();
  });

  it('delete keeps a file another open project plays; Rebuild replaces it', async () => {
    const h = harness();
    const root = path.join(h.cache, 'Generated Media');
    const stored = await storeProxy(root, H1);
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.setIntent(2, 'p2', intent(), { b: video(H1) });
    await h.service.remove(1, 'p1', ['a']);
    expect(await exists(stored.file)).toBe(true);
    expect(h.last(1).assets['a']).toMatchObject({ state: 'ready' }); // the same bytes are still there
    await h.service.rebuild(1, 'p1', ['a']);
    expect(await exists(stored.file)).toBe(false);
    expect(h.submitted()).toHaveLength(1);
    await h.service.dispose();
  });

  it('rebuild during a running job cancels it and regenerates; the late output of the old job stays', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.create(1, 'p1', ['a']);
    const old = h.submitted()[0]!;
    h.service.status(status(old.job, 'running', { progress: 0.9 }));
    await h.service.rebuild(1, 'p1', ['a']);
    expect(h.requests.filter((r) => r.type === 'cancelJobs')).toEqual([{ type: 'cancelJobs', kind: 'job', job: old.job }]);
    expect(h.submitted()).toHaveLength(2);
    const fresh = h.submitted()[1]!;
    // The old alias can no longer be granted.
    await expect(h.service.resolveAlias((old.spec as { asset: string }).asset, JOB)).rejects.toBeInstanceOf(ProtocolError);
    // The old job finished before the cancel landed: its file is the new job's key; keep it.
    const { file } = await storeProxy(path.join(h.cache, 'Generated Media'), H1);
    h.service.status(status(old.job, 'done', { progress: 1, path: file }));
    await h.service.settled();
    expect(await exists(file)).toBe(true);
    expect(h.last().assets['a']).toEqual({ state: 'queued' });
    h.service.status(status(fresh.job, 'done', { progress: 1, path: file }));
    await h.service.settled();
    expect(h.last().assets['a']).toMatchObject({ state: 'ready' });
    await h.service.dispose();
  });

  it('delete during a running job removes the output it finishes anyway, and the policy does not restart it', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1) });
    const job = h.submitted()[0]!.job;
    h.service.status(status(job, 'running', { progress: 0.5 }));
    await h.service.remove(1, 'p1', ['a']);
    expect(h.last().assets['a']).toEqual({ state: 'none' });
    h.service.status(status(job, 'running', { progress: 0.8 }));
    expect(h.last().assets['a']).toEqual({ state: 'none' });
    const late = await storeProxy(path.join(h.cache, 'Generated Media'), H1);
    h.service.status(status(job, 'done', { progress: 1, path: late.file }));
    await h.service.settled();
    expect(await exists(late.file)).toBe(false);
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1) });
    expect(h.submitted()).toHaveLength(1);
    expect(h.last().assets['a']).toEqual({ state: 'none' });
    await h.service.dispose();
  });

  it('reports paused jobs with their real reason and keeps progress', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.create(1, 'p1', ['a']);
    const job = h.submitted()[0]!.job;
    h.service.status(status(job, 'running', { progress: 0.4 }));
    const error = (m: string) => ({ code: 'budgetExceeded', scope: { kind: 'job', job }, message: m, retry: 'never', detail: null }) as unknown as JobStatus['error'];
    h.service.status(status(job, 'paused', { progress: 0.4, error: error('Proxies paused · Low disk space') }));
    expect(h.last().activity.paused).toBe('lowDisk');
    expect(h.last().assets['a']).toEqual({ state: 'creating', progress: 0.4 });
    h.service.playback(true);
    h.service.status(status(job, 'paused', { progress: 0.4, error: error('Proxies paused · Playback or export in progress') }));
    expect(h.last().activity.paused).toBe('playback');
    h.service.playback(false);
    h.service.status(status('export-1', 'running', { class: 'userJob' }));
    h.service.status(status(job, 'paused', { progress: 0.4, error: error('Proxies paused · Playback or export in progress') }));
    expect(h.last().activity.paused).toBe('export');
    h.service.status(status(job, 'running', { progress: 0.5 }));
    expect(h.last().activity.paused).toBeNull();
    await h.service.dispose();
  });

  it('keeps a failure named, does not retry it automatically, and retries on Create', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1) });
    const job = h.submitted()[0]!.job;
    h.service.status(status(job, 'failed', { error: { code: 'decodeFailed', message: 'Could not create proxy: unsupported codec' } as JobStatus['error'] }));
    await h.service.settled();
    expect(h.last().assets['a']).toEqual({ state: 'failed', error: 'Could not create proxy: unsupported codec' });
    await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1) });
    expect(h.submitted()).toHaveLength(1);
    await h.service.create(1, 'p1', ['a']);
    expect(h.submitted()).toHaveLength(2);
    // A job engine crash: nothing in flight will ever report again.
    h.service.jobsLost();
    expect(h.last().assets['a']).toEqual({ state: 'failed', error: 'The engine stopped while creating this proxy. Create it again.' });
    await h.service.create(1, 'p1', ['a']);
    expect(h.submitted()).toHaveLength(3);
    await h.service.dispose();
  });

  it('fails a submit the engine rejects, with a message', async () => {
    const h = harness({ fail: (r) => r.type === 'submitJob' ? new Error('the job engine is not running') : null });
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await expect(h.service.create(1, 'p1', ['a'])).rejects.toThrow("Couldn't start creating the proxy: the job engine is not running");
    expect(h.last().assets['a']).toEqual({ state: 'failed', error: "Couldn't start creating the proxy: the job engine is not running" });
    await expect(h.service.resolveAlias((h.submitted()[0]!.spec as { asset: string }).asset, JOB)).resolves.toBeNull();
    await h.service.dispose();
  });

  describe('source guards', () => {
    it('resolves through the project, refuses changed or offline originals, and skips them quietly when automatic', async () => {
      const h = harness();
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H2) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), off: video(H3) });
      await expect(h.service.create(1, 'p1', ['a'])).rejects.toThrow('The original for Clip changed');
      await expect(h.service.create(1, 'p1', ['off'])).rejects.toThrow('The original for Clip is offline');
      await expect(h.service.create(1, 'p1', ['missing'])).rejects.toThrow('not in the project');
      await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1), off: video(H3) });
      expect(h.submitted()).toHaveLength(0);
      // Another project binding the same id never stands in (the resolver is asked for p1 only).
      expect(h.resolves.every((r) => r.scope.project === 'p1')).toBe(true);
      await h.service.dispose();
    });

    it('re-resolves at grant time, honours a relink, rejects changed bytes, and uses the frozen source after the window closed', async () => {
      const h = harness();
      h.sources.set('p1/a', { path: '/old/a.mov', contentHash: bytes(H1) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), b: video(H2) });
      h.sources.set('p1/b', { path: '/m/b.mov', contentHash: bytes(H2) });
      await h.service.create(1, 'p1', ['a', 'b']);
      const [a, b] = h.submitted().map((r) => (r.spec as { asset: string }).asset);
      h.sources.set('p1/a', { path: '/relinked/a.mov', contentHash: bytes(H1) });
      expect(await h.service.resolveAlias(a!, JOB)).toEqual({ path: '/relinked/a.mov', contentHash: bytes(H1) });
      h.sources.set('p1/a', { path: '/relinked/a.mov', contentHash: bytes(H3) });
      await expect(h.service.resolveAlias(a!, JOB)).rejects.toThrow('The original changed after the proxy was queued');
      h.service.release(1);
      h.sources.delete('p1/b');
      expect(await h.service.resolveAlias(b!, JOB)).toEqual({ path: '/m/b.mov', contentHash: bytes(H2) });
      // Statuses for a closed window are harmless.
      expect(() => h.service.status(status(h.submitted()[1]!.job, 'running', { progress: 0.3 }))).not.toThrow();
      await h.service.dispose();
    });

    it('freezes the store root per job when the location changes, and requires a saved project for Next to the project', async () => {
      const h = harness();
      const folderA = path.join(temp, 'A'), folderB = path.join(temp, 'B');
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      h.sources.set('p1/b', { path: '/m/b.mov', contentHash: bytes(H2) });
      expect(await h.service.setLocation({ kind: 'folder', path: folderA })).toEqual({ kind: 'folder', path: folderA });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), b: video(H2) });
      await h.service.create(1, 'p1', ['a']);
      await h.service.setLocation({ kind: 'folder', path: folderB });
      await h.service.create(1, 'p1', ['b']);
      const [a, b] = h.submitted().map((r) => (r.spec as { asset: string }).asset);
      await h.service.resolveAlias(a!, JOB);
      expect(h.service.generatedDirectory(JOB)).toBe(folderA);
      await h.service.resolveAlias(b!, JOB);
      expect(h.service.generatedDirectory(JOB)).toBe(folderB);
      h.service.status(status(h.submitted()[0]!.job, 'done', { progress: 1 }));
      expect(h.service.generatedDirectory(JOB)).toBe(folderB); // a late status does not reset the running job's store
      const roots = JSON.parse(await readFile(h.service.locationsFile, 'utf8')).roots;
      expect(roots).toEqual([path.join(h.cache, 'Generated Media'), folderA, folderB]);

      await h.service.setLocation({ kind: 'project' });
      h.sources.set('p1/c', { path: '/m/c.mov', contentHash: bytes(H3) });
      await h.service.setIntent(1, 'p1', intent(), { c: video(H3) }, 'relative/project.pmv');
      await expect(h.service.create(1, 'p1', ['c'])).rejects.toThrow('Save this project');
      const project = path.join(temp, 'Projects', 'Film.pmv');
      await h.service.setIntent(1, 'p1', intent(), { c: video(H3) }, project);
      await h.service.create(1, 'p1', ['c']);
      expect(await exists(path.join(temp, 'Projects', PROJECT_ROOT))).toBe(true);
      await expect(h.service.setLocation({ kind: 'folder', path: 'relative' } as never)).rejects.toThrow('Choose a generated media location');
      await h.service.dispose();
    });

    it('resolves draft proxies for one project only', async () => {
      const h = harness();
      const root = path.join(h.cache, 'Generated Media');
      const p1 = await storeProxy(root, H1);
      const p2 = await storeProxy(root, H2, { size: 'half' });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      await h.service.setIntent(2, 'p2', intent({ size: 'half' }), { a: video(H2) });
      expect((await h.service.resolveDraft('a', { project: 'p1' }))?.path).toBe(p1.file);
      expect((await h.service.resolveDraft('a', { project: 'p2' }))?.path).toBe(p2.file);
      expect(await h.service.resolveDraft('a', { project: 'p3' })).toBeNull();
      // A job snapshot's frozen hash wins over whatever windows show now (any recipe as fallback).
      expect((await h.service.resolveDraft('a', { project: 'p1', contentHash: H2 }))?.path).toBe(p2.file);
      expect(await h.service.resolveDraft('a', undefined as never)).toBeNull();
      await h.service.dispose();
    });
  });

  describe('damaged state', () => {
    it('fails by name and never overwrites a damaged locations file', async () => {
      const cache = path.join(temp, 'cache');
      await mkdir(cache, { recursive: true });
      await writeFile(path.join(cache, LOCATIONS_FILE), '{"roots": [');
      const h = harness();
      await expect(h.service.location()).rejects.toThrow(`Powermove's generated-media locations file is damaged (${path.join(cache, LOCATIONS_FILE)}`);
      await expect(h.service.setLocation({ kind: 'cache' })).rejects.toThrow('Move that file out of the folder to reset it');
      await expect(h.service.snapshot(1, 'p1')).rejects.toThrow('damaged');
      expect(await readFile(path.join(cache, LOCATIONS_FILE), 'utf8')).toBe('{"roots": [');
      await h.service.dispose();
    });

    it('rejects invalid shapes in the state files', async () => {
      const cache = path.join(temp, 'cache');
      await mkdir(cache, { recursive: true });
      await writeFile(path.join(cache, LOCATIONS_FILE), JSON.stringify({ location: { kind: 'folder', path: 'relative' }, roots: [] }));
      await expect(harness().service.location()).rejects.toThrow('the location is invalid');
      await writeFile(path.join(cache, LOCATIONS_FILE), JSON.stringify({ location: { kind: 'cache' }, roots: ['/ok'] }));
      await writeFile(path.join(cache, DELETIONS_FILE), JSON.stringify({ version: 1, deleted: { p1: { a: { hash: 'nope', format: 'proResProxy', size: 'half', at: 1 } } } }));
      await expect(harness().service.location()).rejects.toThrow(`deleted-proxy list file is damaged (${path.join(cache, DELETIONS_FILE)}: invalid asset entry)`);
    });

    it('shows a damaged record for this source as a failure, ignores unreadable records, and never plays a size mismatch', async () => {
      const h = harness();
      const root = path.join(h.cache, 'Generated Media');
      await storeProxy(root, H1, { bytes: 16, mediaBytes: 9 });
      await mkdir(path.join(root, 'ff'), { recursive: true });
      await writeFile(path.join(root, 'ff', 'f'.repeat(64) + '.json'), '{not json');
      const bad = path.join(root, 'ee', 'e'.repeat(64) + '.json');
      await mkdir(path.dirname(bad), { recursive: true });
      await writeFile(bad, JSON.stringify({ source_hash: H2, recipe: { requested_format: 'proResProxy', requested_size: 'automatic' } }));
      await writeFile(bad.replace('.json', '.mov'), 'x');
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), b: video(H2), c: video(H3) });
      const assets = h.last().assets;
      expect(assets['a']).toEqual({ state: 'failed', error: 'The proxy file is damaged (the file is 9 bytes but its record says 16). Rebuild the proxy.' });
      expect(assets['b']).toEqual({ state: 'failed', error: 'The proxy file is damaged (its recipe version is invalid). Rebuild the proxy.' });
      expect(assets['c']).toEqual({ state: 'none' });
      expect(h.logs.some((l) => l.includes('f'.repeat(64)))).toBe(true);
      expect(await h.service.resolveDraft('a', { project: 'p1' })).toBeNull();
      // Rebuild removes the damaged files and regenerates.
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      await h.service.rebuild(1, 'p1', ['a']);
      expect(await readdir(path.join(root, (await readdir(root)).find((d) => d !== 'ff' && d !== 'ee')!))).toEqual([]);
      expect(h.submitted()).toHaveLength(1);
      await h.service.dispose();
    });

    it('never plays or drafts a same-size corrupt proxy, names it, hashes each file identity once, and Rebuild fixes it', async () => {
      const h = harness();
      const root = path.join(h.cache, 'Generated Media');
      const good = await storeProxy(root, H1);
      const unrelated = await storeProxy(root, H3);
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      expect(h.last().assets['a']).toMatchObject({ state: 'ready' });
      // Only sources an open project uses are hashed, each file once while it is unchanged.
      await h.service.snapshot(1, 'p1');
      await h.service.snapshot(1, 'p1');
      expect((await h.service.resolveDraft('a', { project: 'p1' }))?.path).toBe(good.file);
      expect(h.hashed).toEqual([good.file]);
      expect(h.hashed).not.toContain(unrelated.file);

      // Replaced by a corrupt file of the same size (new inode): rehashed, named, never ready or draft.
      const swap = `${good.file}.swap`;
      await writeFile(swap, Buffer.alloc(16, 8));
      await rename(swap, good.file);
      const damaged = { state: 'failed', error: 'The proxy file is damaged (its contents do not match its checksum). Rebuild the proxy.' };
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toEqual(damaged);
      expect(await h.service.resolveDraft('a', { project: 'p1' })).toBeNull();
      expect(h.hashed).toEqual([good.file, good.file]);
      expect(h.logs.some((l) => l.includes('does not match its checksum'))).toBe(true);
      // The verdict is cached for that identity too.
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toEqual(damaged);
      expect(h.hashed).toHaveLength(2);

      // A record that now expects other bytes is checked against them, not the cached verdict.
      const record = JSON.parse(await readFile(good.record, 'utf8'));
      await writeFile(swap, Buffer.alloc(16, 7));
      await rename(swap, good.file);
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toMatchObject({ state: 'ready' });
      await writeFile(good.record, JSON.stringify({ ...record, blake3: 'd'.repeat(64) }));
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toEqual(damaged);
      await writeFile(good.record, JSON.stringify(record));
      expect(h.hashed).toHaveLength(4);
      // Rewritten in place with the right bytes (same inode): rehashed and ready again.
      await writeFile(good.file, Buffer.alloc(16, 7));
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toMatchObject({ state: 'ready' });
      expect(h.hashed).toHaveLength(5);
      // And corrupted in place at the same size.
      await writeFile(good.file, Buffer.alloc(16, 9));
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toEqual(damaged);
      expect(h.hashed).toHaveLength(6);

      // Automatic creation regenerates over it (pm-proxy generate removes a corrupt store entry); Rebuild removes it now.
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      await h.service.rebuild(1, 'p1', ['a']);
      expect(await exists(good.file)).toBe(false);
      expect(await exists(good.record)).toBe(false);
      expect(h.submitted()).toHaveLength(1);
      await h.service.dispose();
    });

    it('requires every current recipe of a source to verify: draft falls back past a corrupt one only to a verified one', async () => {
      const h = harness();
      const root = path.join(h.cache, 'Generated Media');
      const corrupt = await storeProxy(root, H1, { blake3: 'c'.repeat(64) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      expect(h.last().assets['a']).toMatchObject({ state: 'failed' });
      expect(await h.service.resolveDraft('a', { project: 'p1' })).toBeNull();
      const half = await storeProxy(root, H1, { size: 'half' });
      expect((await h.service.resolveDraft('a', { project: 'p1' }))?.path).toBe(half.file);
      expect(h.hashed.filter((f) => f === corrupt.file)).toHaveLength(1);
      // Automatic creation does not count the corrupt file as ready.
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1) });
      expect(h.submitted()).toHaveLength(1);
      await h.service.dispose();
    });

    it('verifies sources no open project names before trusting them (frozen draft hash, descriptor without a hash)', async () => {
      const h = harness();
      const root = path.join(h.cache, 'Generated Media');
      const frozen = await storeProxy(root, H2);
      await storeProxy(root, H3, { blake3: 'c'.repeat(64) });
      const unhashed = await storeProxy(root, H1);
      await h.service.setIntent(1, 'p1', intent(), { a: video(), b: video() });
      expect(h.hashed).toEqual([]);
      // An export job's frozen hash: verified on demand, a corrupt one is never drafted.
      expect((await h.service.resolveDraft('a', { project: 'p1', contentHash: H2 }))?.path).toBe(frozen.file);
      expect(await h.service.resolveDraft('a', { project: 'p1', contentHash: H3 })).toBeNull();
      // The resolved original's verified proxy counts as existing: no duplicate job.
      h.sources.set('p1/b', { path: '/m/b.mov', contentHash: bytes(H1) });
      await h.service.create(1, 'p1', ['b']);
      expect(h.submitted()).toHaveLength(0);
      expect(h.hashed).toContain(unhashed.file);
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H3) });
      await h.service.create(1, 'p1', ['a']);
      expect(h.submitted()).toHaveLength(1);
      await h.service.dispose();
    });

    it('a proxy rewritten while it is hashed is hashed again, never trusted on the stale digest', async () => {
      let raced = false;
      const h = harness({
        hashFile: async (file) => {
          const digest = blake3Hex(await readFile(file));
          if (!raced) { raced = true; await writeFile(file, Buffer.alloc(16, 8)); }
          return digest;
        }
      });
      await storeProxy(path.join(h.cache, 'Generated Media'), H1);
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      expect(h.last().assets['a']).toMatchObject({ state: 'failed', error: expect.stringContaining('do not match its checksum') });
      await h.service.dispose();
    });

    it('job statuses during a slow hash report the last verified scan, never an unverified one', async () => {
      let release: () => void = () => {};
      const gate = new Promise<void>((r) => { release = r; });
      const h = harness({
        hashFile: async (file) => {
          if (path.basename(file) === `${'c'.repeat(64)}.mov`) await gate;
          return blake3Hex(await readFile(file));
        }
      });
      const root = path.join(h.cache, 'Generated Media');
      await storeProxy(root, H1);
      h.sources.set('p1/b', { path: '/m/b.mov', contentHash: bytes(H2) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), b: video(H2) });
      await h.service.create(1, 'p1', ['b']);
      expect(h.last().assets['a']).toMatchObject({ state: 'ready' });
      // A new clip whose proxy is still being hashed comes first in scan order.
      await storeProxy(root, H3, { key: 'c'.repeat(64) });
      const slow = h.service.setIntent(1, 'p1', intent(), { c: video(H3), a: video(H1), b: video(H2) });
      await new Promise((r) => setTimeout(r, 20));
      h.service.status(status(h.submitted()[0]!.job, 'running', { progress: 0.5 }));
      expect(h.last().assets['a']).toMatchObject({ state: 'ready' });
      release();
      await slow;
      expect(h.last().assets['c']).toMatchObject({ state: 'ready' });
      await h.service.dispose();
    });

    it('a hash that cannot be read is a named failure and retried; media gone while hashing is a miss', async () => {
      let mode: 'eio' | 'gone' | 'ok' = 'eio';
      const h = harness({
        hashFile: async (file) => {
          if (mode === 'eio') throw Object.assign(new Error('EIO: i/o error, read'), { code: 'EIO' });
          if (mode === 'gone') { await rm(file); throw Object.assign(new Error('ENOENT: no such file'), { code: 'ENOENT' }); }
          return blake3Hex(await readFile(file));
        }
      });
      const root = path.join(h.cache, 'Generated Media');
      const stored = await storeProxy(root, H1);
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      expect(h.last().assets['a']).toEqual({ state: 'failed', error: "Powermove couldn't check the proxy file (EIO: i/o error, read). Rebuild the proxy." });
      expect(await h.service.resolveDraft('a', { project: 'p1' })).toBeNull();
      mode = 'ok';
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toMatchObject({ state: 'ready' });
      await writeFile(stored.file, Buffer.alloc(16, 7)); // new identity
      mode = 'gone';
      expect((await h.service.snapshot(1, 'p1')).assets['a']).toEqual({ state: 'none' });
      await h.service.dispose();
    });
  });

  describe('when needed (measured decodes)', () => {
    it('measures in the background one at a time, creates only slow originals, and caches by content hash', async () => {
      const calls: string[] = [];
      let active = 0, overlap = false;
      const assess = async (asset: string, source: ProxySource, scope: ProxySourceScope) => {
        calls.push(`${scope.project}/${asset}:${source.path}`);
        overlap ||= ++active > 1;
        await new Promise((r) => setTimeout(r, 5));
        active--;
        return asset === 'slow';
      };
      const h = harness({ assess, machine: 'm1' });
      h.sources.set('p1/slow', { path: '/m/slow.mov', contentHash: bytes(H1) });
      h.sources.set('p1/fast', { path: '/m/fast.mov', contentHash: bytes(H2) });
      h.sources.set('p1/keep', { path: '/m/keep.mov', contentHash: bytes(H3) });
      const assets = { slow: video(H1), fast: video(H2), keep: video(H3, { alwaysPlayOriginal: true }), audio: { kind: 'audio', contentHash: '4'.repeat(64) } };
      await h.service.setIntent(1, 'p1', intent({ create: 'whenNeeded' }), assets);
      expect(h.submitted()).toHaveLength(0); // setIntent did not wait for measurement
      await h.service.settled();
      expect(calls).toEqual(['p1/slow:/m/slow.mov', 'p1/fast:/m/fast.mov']);
      expect(overlap).toBe(false);
      expect(h.submitted()).toHaveLength(1);
      expect(h.last().assets['slow']).toEqual({ state: 'queued' });
      expect(JSON.parse(await readFile(path.join(h.cache, ASSESSMENTS_FILE), 'utf8'))).toEqual({ version: 1, machine: 'm1', results: { [H1]: { needsProxy: true, at: 1000 }, [H2]: { needsProxy: false, at: 1000 } } });
      await h.service.dispose();

      // Relaunch: cached results apply on setIntent, nothing is measured again.
      calls.length = 0;
      const again = harness({ assess, machine: 'm1' });
      again.sources.set('p1/slow', { path: '/m/slow.mov', contentHash: bytes(H1) });
      await again.service.setIntent(1, 'p1', intent({ create: 'whenNeeded' }), { slow: video(H1), fast: video(H2) });
      expect(again.submitted()).toHaveLength(1);
      await again.service.settled();
      expect(calls).toEqual([]);
      await again.service.dispose();

      // Another Mac's measurements do not count.
      const other = harness({ assess, machine: 'm2' });
      other.sources.set('p1/fast', { path: '/m/fast.mov', contentHash: bytes(H2) });
      await other.service.setIntent(1, 'p1', intent({ create: 'whenNeeded' }), { fast: video(H2) });
      await other.service.settled();
      expect(calls).toEqual(['p1/fast:/m/fast.mov']);
      await other.service.dispose();
    });

    it('never measures for allVideo or onlyWhenAsked, or deleted proxies, and keeps a failed measurement named', async () => {
      const calls: string[] = [];
      const h = harness({ assess: async (asset) => { calls.push(asset); if (asset === 'broken') throw new Error('the decoder could not open the file'); return true; } });
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      h.sources.set('p1/broken', { path: '/m/b.mov', contentHash: bytes(H2) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1), broken: video(H2, { name: 'Broken' }) });
      await h.service.setIntent(1, 'p1', intent({ create: 'allVideo' }), { a: video(H1), broken: video(H2, { name: 'Broken' }) });
      await h.service.settled();
      expect(calls).toEqual([]);
      await h.service.remove(1, 'p1', ['a', 'broken']);
      await h.service.setIntent(1, 'p1', intent({ create: 'whenNeeded' }), { a: video(H1), broken: video(H2, { name: 'Broken' }) });
      await h.service.settled();
      expect(calls).toEqual([]);
      const fresh = harness({ assess: async (asset) => { calls.push(asset); throw new Error('the decoder could not open the file'); } });
      fresh.sources.set('p2/x', { path: '/m/x.mov', contentHash: bytes(H3) });
      await fresh.service.setIntent(1, 'p2', intent({ create: 'whenNeeded' }), { x: video(H3, { name: 'Broken' }) });
      await fresh.service.settled();
      expect(fresh.service.assessmentError(H3)).toBe("Couldn't measure how Broken decodes on this Mac: the decoder could not open the file");
      expect(fresh.logs.some((l) => l.includes("Couldn't measure how Broken"))).toBe(true);
      await fresh.service.setIntent(1, 'p2', intent({ create: 'whenNeeded' }), { x: video(H3, { name: 'Broken' }) });
      await fresh.service.settled();
      expect(calls.filter((c) => c === 'x')).toHaveLength(1);
      expect(fresh.submitted()).toHaveLength(0);
      await fresh.service.dispose();
      await h.service.dispose();
    });

    it('playback measurements (needed) create proxies under When needed only', async () => {
      const h = harness();
      h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
      await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
      await h.service.needed(1, 'p1', ['a']);
      expect(h.submitted()).toHaveLength(0);
      await h.service.setIntent(1, 'p1', intent({ create: 'whenNeeded' }), { a: video(H1) });
      expect(h.submitted()).toHaveLength(1);
      await h.service.dispose();
    });
  });

  it('stops cleanly: cancels jobs in flight and refuses later calls', async () => {
    const h = harness();
    h.sources.set('p1/a', { path: '/m/a.mov', contentHash: bytes(H1) });
    await h.service.setIntent(1, 'p1', intent(), { a: video(H1) });
    await h.service.create(1, 'p1', ['a']);
    await h.service.dispose();
    expect(h.requests.at(-1)).toEqual({ type: 'cancelJobs', kind: 'job', job: h.submitted()[0]!.job });
    await expect(h.service.snapshot(1, 'p1')).rejects.toThrow('stopped');
    expect(await readdir(h.cache)).not.toContain(expect.stringMatching(/\.tmp$/));
  });
});
