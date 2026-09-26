import { expect, test } from 'bun:test';
import { eq, sql } from 'drizzle-orm';
import { createApp } from '../src/app';
import { claimGcCandidates, runGc, sweepClaimed } from '../src/gc';
import { objectLeases, objects, publishers, releaseObjects, releases, repos } from '../src/db/schema';
import { encodeCommit, encodeLoose, encodeTree, hashObject, sha256Hex } from '@powermove/registry/git';
import { REGISTRY_LIMITS } from '@powermove/registry/limits';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';
const blob = (text: string) => new TextEncoder().encode(text);
async function object(type: 'blob' | 'tree' | 'commit', body: Uint8Array) {
  return { sha: await hashObject(type, body), bytes: await encodeLoose(type, body), type, body };
}
type Obj = Awaited<ReturnType<typeof object>>;
function form(parts: {
  sha: string;
  bytes: Uint8Array;
  contentType?: string;
}[]) {
  const f = new FormData();
  for (const p of parts) {
    f.append(
      p.sha,
      new Blob([new Uint8Array(p.bytes)], { type: p.contentType ?? 'application/x-git-loose-object' }),
      'object',
    );
  }
  return f;
}
test('object upload validates blob, tree, commit, duplicates and invalid parts', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data }), s = await seedSession(data, env);
    const call = (parts: {
      sha: string;
      bytes: Uint8Array;
      contentType?: string;
    }[]) => app.request('/v1/objects', { method: 'POST', headers: s.headers, body: form(parts) }, env);
    const b1 = await object('blob', blob('first')), b2 = await object('blob', blob('second'));
    const tree = await object(
      'tree',
      encodeTree([{ mode: '100644', name: 'a.txt', sha: b1.sha }, { mode: '100644', name: 'b.txt', sha: b2.sha }]),
    );
    const id = { name: 'Jude', email: 'jude@example.com', time: 1, tz: '+0000' };
    const commit = await object(
      'commit',
      encodeCommit({ tree: tree.sha, parents: [], author: id, committer: id, message: 'test' }),
    );
    const all = [b1, b2, tree, commit];
    let r = await call(all);
    expect(r.status).toBe(200);
    expect(await r.json() as any).toEqual({ stored: all.map((x) => x.sha), present: [], rejected: [] });
    r = await call(all);
    expect(r.status).toBe(200);
    expect(await r.json() as any).toEqual({ stored: [], present: all.map((x) => x.sha), rejected: [] });
    r = await call([{ sha: 'a'.repeat(40), bytes: b1.bytes }]);
    expect((await r.json() as any).rejected).toEqual([{ sha: 'a'.repeat(40), code: 'hash_mismatch' }]);
    r = await call([{ sha: b1.sha, bytes: blob('not zlib') }]);
    expect((await r.json() as any).rejected).toEqual([{ sha: b1.sha, code: 'bad_object' }]);
    const large = await object('blob', new Uint8Array(REGISTRY_LIMITS.fileBytes + 100));
    r = await call([large]);
    expect((await r.json() as any).rejected).toEqual([{ sha: large.sha, code: 'too_large' }]);
    r = await call([{ sha: b1.sha, bytes: b1.bytes, contentType: 'text/plain' }]);
    expect((await r.json() as any).rejected).toEqual([{ sha: b1.sha, code: 'bad_object' }]);
    r = await call(Array.from({ length: 65 }, () => b1));
    expect(r.status).toBe(413);
    expect(await r.json() as any).toEqual({ error: 'too_large', limit: 'envelope' });
  }));
test('write once, relative presence, lease expiry and visible origin', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      app = createApp({ data: () => data }),
      a = await seedSession(data, env, 'a@example.com'),
      b = await seedSession(data, env, 'b@example.com');
    const x = await object('blob', blob('private'));
    const request = (headers: Record<string, string>, path: string, body: unknown) =>
      app.request(path, {
        method: 'POST',
        headers: { ...headers, 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }, env);
    let r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect((await r.json() as any).stored).toEqual([x.sha]);
    r = await request(b.headers, '/v1/objects/missing', { shas: [x.sha, x.sha] });
    expect(await r.json() as any).toEqual({ missing: [x.sha] });
    const [p] = await data.db.insert(publishers).values({ handle: 'author', userId: a.id }).returning();
    const [repo] = await data.db.insert(repos).values({ ownerId: p.id, slug: 'demo' }).returning();
    const [release] = await data.db.insert(releases).values({
      repoId: repo.id,
      version: '1.0.0',
      commitSha: x.sha,
      treeSha: x.sha,
      tarKey: 'tar',
      tarSha256: 'a'.repeat(64),
      manifest: {},
      apiVersion: 1,
      fileCount: 1,
      sizeBytes: 1,
    }).returning();
    await data.db.insert(releaseObjects).values({ releaseId: release.id, sha: x.sha });
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.userId, a.id));
    r = await request(a.headers, '/v1/objects/missing', { shas: [x.sha], repoId: repo.id });
    expect(await r.json() as any).toEqual({ missing: [] });
    r = await request(b.headers, '/v1/objects/missing', { shas: [x.sha], originReleaseId: release.id });
    expect(await r.json() as any).toEqual({ missing: [] });
    await data.db.update(repos).set({ moderation: 'hidden' }).where(eq(repos.id, repo.id));
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.userId, b.id));
    r = await request(b.headers, '/v1/objects/missing', { shas: [x.sha], originReleaseId: release.id });
    expect(await r.json() as any).toEqual({ missing: [x.sha] });
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.userId, a.id));
    r = await request(a.headers, '/v1/objects/missing', { shas: [x.sha] });
    expect(await r.json() as any).toEqual({ missing: [x.sha] });
    // A mismatched stored tuple must never replace the bucket key.
    await data.db.update(objects).set({ sha256: 'f'.repeat(64) }).where(eq(objects.sha, x.sha));
    r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect((await r.json() as any).rejected).toEqual([{ sha: x.sha, code: 'object_conflict' }]);
    expect(Array.from(new Uint8Array(await (await env.OBJECTS.get(`objects/${x.sha}`))!.arrayBuffer()))).toEqual(
      Array.from(x.bytes),
    );
  }));
test('conditional put checks existing R2 metadata', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      app = createApp({ data: () => data }),
      a = await seedSession(data, env),
      x = await object('blob', blob('bucket first'));
    await env.OBJECTS.put(`objects/${x.sha}`, x.bytes, {
      customMetadata: { type: 'blob', size: String(x.body.length), sha256: 'f'.repeat(64) },
    });
    const r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect((await r.json() as any).rejected).toEqual([{ sha: x.sha, code: 'object_conflict' }]);
    expect((await env.OBJECTS.head(`objects/${x.sha}`))?.customMetadata?.sha256).toBe('f'.repeat(64));
  }));
test('GC claim rescue and sequential deletion and re-upload', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      app = createApp({ data: () => data }),
      a = await seedSession(data, env),
      x = await object('blob', blob('gc'));
    await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    await data.db.update(objects).set({ lastSeenAt: new Date(0) }).where(eq(objects.sha, x.sha));
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.sha, x.sha));
    const claimed = await claimGcCandidates(data);
    expect(claimed).toEqual([x.sha]);
    // A lease renewed before the recheck rescues a claimed row.
    await data.db.update(objectLeases).set({ expiresAt: new Date(Date.now() + 86400000) }).where(
      eq(objectLeases.sha, x.sha),
    );
    const missing = await app.request('/v1/objects/missing', {
      method: 'POST',
      headers: { ...a.headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ shas: [x.sha] }),
    }, env);
    expect(await missing.json() as any).toEqual({ missing: [] });
    await sweepClaimed(data, env, claimed);
    expect((await data.db.select().from(objects).where(eq(objects.sha, x.sha)))[0]?.gcState).toBe('live');
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.sha, x.sha));
    await data.db.update(objects).set({ lastSeenAt: new Date(0) }).where(eq(objects.sha, x.sha));
    await runGc(data, env);
    expect(await data.db.select().from(objects).where(eq(objects.sha, x.sha))).toHaveLength(0);
    expect(await env.OBJECTS.head(`objects/${x.sha}`)).toBeNull();
    const again = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect((await again.json() as any).stored).toEqual([x.sha]);
  }));
test('orphan quota and upload rate limit', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      app = createApp({ data: () => data }),
      a = await seedSession(data, env),
      x = await object('blob', blob('quota'));
    await data.db.insert(objects).values({
      sha: 'f'.repeat(40),
      type: 'blob',
      size: 256 * 1024 * 1024,
      sha256: 'f'.repeat(64),
    });
    await data.db.insert(objectLeases).values({
      userId: a.id,
      sha: 'f'.repeat(40),
      expiresAt: new Date(Date.now() + 10000),
    });
    let r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect(r.status).toBe(429);
    expect((await r.json() as any).error).toBe('quota_exceeded');
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.userId, a.id));
    r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect(r.status).toBe(200);
    env.RL_UPLOAD = { limit: async () => ({ success: false }) } as RateLimit;
    r = await app.request('/v1/objects', { method: 'POST', headers: a.headers, body: form([x]) }, env);
    expect(r.status).toBe(429);
    expect((await r.json() as any).error).toBe('rate_limited');
  }));
test('upload response does not disclose another user object and stores canonical SHA-256', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const alice = await seedSession(data, env, 'alice@example.com');
    const bob = await seedSession(data, env, 'bob@example.com');
    const item = await object('blob', blob('shared byte sequence'));
    const upload = (headers: Record<string, string>) =>
      app.request('/v1/objects', { method: 'POST', headers, body: form([item]) }, env);
    expect((await upload(alice.headers)).status).toBe(200);
    const other = await upload(bob.headers);
    expect(await other.json()).toMatchObject({ stored: [item.sha], present: [] });
    const [row] = await data.db.select().from(objects).where(eq(objects.sha, item.sha));
    const header = blob(`blob ${item.body.length}\0`);
    const canonical = new Uint8Array(header.length + item.body.length);
    canonical.set(header);
    canonical.set(item.body, header.length);
    expect(row.sha256).toBe(await sha256Hex(canonical));
  }));
test('yanked origin and based-on releases do not grant presence', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const alice = await seedSession(data, env, 'alice@example.com');
    const bob = await seedSession(data, env, 'bob@example.com');
    const item = await object('blob', blob('yanked only'));
    await app.request('/v1/objects', { method: 'POST', headers: alice.headers, body: form([item]) }, env);
    const [owner] = await data.db.insert(publishers).values({ handle: 'alice', userId: alice.id }).returning();
    const [repo] = await data.db.insert(repos).values({ ownerId: owner.id, slug: 'demo' }).returning();
    const [release] = await data.db.insert(releases).values({
      repoId: repo.id,
      version: '1.0.0',
      commitSha: item.sha,
      treeSha: item.sha,
      tarKey: 'tar',
      tarSha256: 'a'.repeat(64),
      manifest: {},
      apiVersion: 1,
      fileCount: 1,
      sizeBytes: 1,
      yankedAt: new Date(),
    }).returning();
    await data.db.insert(releaseObjects).values({ releaseId: release.id, sha: item.sha });
    for (const context of [{ originReleaseId: release.id }, { basedOnReleaseId: release.id }]) {
      const response = await app.request('/v1/objects/missing', {
        method: 'POST',
        headers: { ...bob.headers, 'Content-Type': 'application/json' },
        body: JSON.stringify({ shas: [item.sha], ...context }),
      }, env);
      expect(await response.json() as any).toEqual({ missing: [item.sha] });
    }
  }));

test('deleting objects are absent and an interrupted GC sweep resumes', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const actor = await seedSession(data, env);
    const item = await object('blob', blob('deleting'));
    await app.request('/v1/objects', { method: 'POST', headers: actor.headers, body: form([item]) }, env);
    await data.db.update(objects).set({ gcState: 'deleting' }).where(eq(objects.sha, item.sha));
    const missing = await app.request('/v1/objects/missing', {
      method: 'POST',
      headers: { ...actor.headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ shas: [item.sha] }),
    }, env);
    expect(await missing.json() as any).toEqual({ missing: [item.sha] });
    await data.db.update(objectLeases).set({ expiresAt: new Date(0) }).where(eq(objectLeases.sha, item.sha));
    await runGc(data, env);
    expect(await data.db.select().from(objects).where(eq(objects.sha, item.sha))).toHaveLength(0);
    expect(await env.OBJECTS.head(`objects/${item.sha}`)).toBeNull();
  }));
