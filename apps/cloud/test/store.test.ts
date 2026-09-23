import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { type ListingDto, Store } from '@powermove/registry/wire';
import { createApp } from '../src/app';
import { neonData } from '../src/db/client';
import { extensions, featured, releases, repos } from '../src/db/schema';
import { withData } from './db';
import { makeEnv } from './env';
import { files, publisher, publishRequest, uploadTree } from './publish-fixture';
const enc = new TextEncoder();
const post = (body: unknown, headers: Record<string, string> = {}) => ({
  method: 'POST',
  headers: { ...headers, 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
});
const json = async (response: Response) => response.json() as Promise<any>;
test('browse returns a clean internal error without a database binding', async () => {
  const response = await createApp({ data: (env) => neonData(env.DATABASE_URL) }).request(
    '/v1/store/browse',
    {},
    { APP_ORIGIN: 'https://cloud.trypowermove.com' } as CloudflareBindings,
  );
  expect(response.status).toBe(500);
  expect(await json(response)).toEqual({ error: 'internal' });
});
test('browse, search, category, cursor, sort and fork lineage', () =>
  withData(async (data) => {
    const env = makeEnv(data), alice = await publisher(data, env, 'alice'), bob = await publisher(data, env, 'bob');
    const built = await uploadTree(data, env, alice, files('demo'));
    const first = await json(
      await publishRequest(data, env, alice, 'demo', built.commitSha, {
        listing: { name: 'Aurora Glow', tagline: 'Color bloom', category: 'effects' },
      }),
    );
    const secondBuilt = await uploadTree(data, env, alice, files('spark'));
    const second = await json(
      await publishRequest(data, env, alice, 'spark', secondBuilt.commitSha, {
        listing: { name: 'Spark Trail', tagline: 'Bright glow', category: 'effects' },
      }),
    );
    const forkBuilt = await uploadTree(data, env, bob, [
      {
        path: 'manifest.json',
        bytes: enc.encode(
          JSON.stringify({ id: 'fork', name: 'Fork', version: '1.0.0', apiVersion: 1, forkedFrom: 'alice/demo@1.0.0' }),
        ),
      },
      { path: 'index.ts', bytes: enc.encode('export default 2') },
    ], { parents: [built.commitSha] });
    const fork = await json(
      await publishRequest(data, env, bob, 'fork', forkBuilt.commitSha, {
        originReleaseId: first.release.id,
        listing: { name: 'Aurora Fork', tagline: 'A remix', category: 'tools' },
      }),
    );
    await data.db.insert(featured).values([{ repoId: second.repo.repoId, section: 'featured', position: 1 }, {
      repoId: first.repo.repoId,
      section: 'featured',
      position: 0,
    }, { repoId: fork.repo.repoId, section: 'picks', position: 0 }]);
    const app = createApp({ data: () => data });
    const browse = Store.Browse.Res.parse(await json(await app.request('/v1/store/browse', {}, env)));
    expect(browse.sections.map((s) => s.id)).toEqual(['featured', 'picks', 'new', 'effects', 'tools']);
    expect(browse.sections[0]?.items.map((i) => i.repoId)).toEqual([first.repo.repoId, second.repo.repoId]);
    expect(browse.sections[1]?.items[0]?.forkedFrom?.repoId).toBe(first.repo.repoId);
    const word = await json(await app.request('/v1/store/extensions?q=glow', {}, env));
    expect(word.items.map((x: ListingDto) => x.repoId).sort()).toEqual([first.repo.repoId, second.repo.repoId].sort());
    const prefix = await json(await app.request('/v1/store/extensions?q=Aur', {}, env));
    expect(prefix.items.map((x: ListingDto) => x.repoId).sort()).toEqual([first.repo.repoId, fork.repo.repoId].sort());
    for (const literal of ['%25', '_', '%5C']) {
      expect((await json(await app.request(`/v1/store/extensions?q=${literal}`, {}, env))).items).toHaveLength(0);
    }
    const category = await json(await app.request('/v1/store/extensions?category=tools', {}, env));
    expect(category.items.map((x: ListingDto) => x.repoId)).toEqual([fork.repo.repoId]);
    for (const sort of ['new', 'installs', 'name']) {
      const full = await json(await app.request(`/v1/store/extensions?sort=${sort}`, {}, env));
      const seen: string[] = [];
      let cursor: string | null = null;
      do {
        const page: any = await json(
          await app.request(`/v1/store/extensions?sort=${sort}&limit=1${cursor ? `&cursor=${cursor}` : ''}`, {}, env),
        );
        seen.push(...page.items.map((x: ListingDto) => x.repoId));
        cursor = page.nextCursor;
      } while (cursor);
      expect(seen).toEqual(full.items.map((x: ListingDto) => x.repoId));
      expect(new Set(seen).size).toBe(seen.length);
    }
  }));
test('release and detail routes follow lifecycle and yanked releases fall back', () =>
  withData(async (data) => {
    const env = makeEnv(data), alice = await publisher(data, env, 'alice'), bob = await publisher(data, env, 'bob');
    const built = await uploadTree(data, env, alice, files('demo'));
    const first = await json(await publishRequest(data, env, alice, 'demo', built.commitSha));
    const nextBuilt = await uploadTree(
      data,
      env,
      alice,
      files('demo', '2.0.0', [{ path: 'extra.txt', bytes: enc.encode('new') }]),
      { parents: [built.commitSha] },
    );
    const next = await json(await publishRequest(data, env, alice, 'demo', nextBuilt.commitSha, { version: '2.0.0' }));
    const app = createApp({ data: () => data });
    const repoId = first.repo.repoId, releaseId = next.release.id;
    const changed = await json(
      await app.request(`/v1/store/compare?base=${first.release.id}&head=${releaseId}`, {}, env),
    );
    expect(changed.files).toContainEqual({ path: 'extra.txt', status: 'added' });
    expect(changed.files).toContainEqual({ path: 'manifest.json', status: 'modified' });
    const paths = [
      '/v1/store/x/alice/demo/r/2.0.0',
      '/v1/store/x/alice/demo/r/2.0.0/tree',
      '/v1/store/x/alice/demo/r/2.0.0/tar',
      '/v1/store/x/alice/demo/r/2.0.0/file/index.ts',
      `/v1/store/releases/${releaseId}`,
      `/v1/store/releases/${releaseId}/tree`,
      `/v1/store/releases/${releaseId}/tar`,
      `/v1/store/compare?base=${first.release.id}&head=${releaseId}`,
    ];
    const check = async (expected: {
      browse: boolean;
      detail: number;
      content: number;
      state: string;
      latest: string | null;
    }, headers: Record<string, string> = {}) => {
      const browse = await json(await app.request('/v1/store/browse', { headers }, env));
      expect(browse.sections.find((s: any) => s.id === 'new').items.some((i: any) => i.repoId === repoId)).toBe(
        expected.browse,
      );
      expect((await app.request('/v1/store/x/alice/demo', { headers }, env)).status).toBe(expected.detail);
      for (const path of paths) {
        expect((await app.request(path, { headers }, env)).status).toBe(expected.content);
      }
      const versions = await json(
        await app.request('/v1/store/versions', post({ items: [{ repoId, releaseId }] }, headers), env),
      );
      expect(versions.items[0].state).toBe(expected.state);
      expect(versions.items[0].ownerPublisherId).toBe(alice.publisher.id);
      expect(versions.items[0].latest?.releaseId ?? null).toBe(expected.latest);
    };
    await check({ browse: true, detail: 200, content: 200, state: 'ok', latest: releaseId });
    await data.db.update(repos).set({ visibility: 'unlisted' }).where(eq(repos.id, repoId));
    await check({ browse: false, detail: 200, content: 200, state: 'ok', latest: releaseId });
    await data.db.update(repos).set({ visibility: 'public', moderation: 'hidden' }).where(eq(repos.id, repoId));
    await check({ browse: false, detail: 404, content: 404, state: 'hidden', latest: null }, bob.headers);
    await check({ browse: false, detail: 200, content: 200, state: 'hidden', latest: null }, alice.headers);
    expect(
      (await app.request('/v1/store/x/alice/demo/r/2.0.0/tar', { headers: alice.headers }, env)).headers.get(
        'Cache-Control',
      ),
    ).toBe('private, no-store');
    expect(
      (await app.request('/v1/store/x/alice/demo/r/2.0.0/file/index.ts', { headers: alice.headers }, env)).headers.get(
        'Cache-Control',
      ),
    ).toBe('private, no-store');
    await data.db.update(repos).set({ moderation: 'removed' }).where(eq(repos.id, repoId));
    await check({ browse: false, detail: 410, content: 410, state: 'removed', latest: null });
    await data.db.update(repos).set({ moderation: 'none', tombstonedAt: new Date() }).where(eq(repos.id, repoId));
    await check({ browse: false, detail: 410, content: 410, state: 'tombstoned', latest: null });
    await data.db.update(repos).set({ tombstonedAt: null }).where(eq(repos.id, repoId));
    await data.db.update(releases).set({ yankedAt: new Date() }).where(eq(releases.id, releaseId));
    await data.db.update(extensions).set({ latestReleaseId: first.release.id }).where(eq(extensions.repoId, repoId));
    expect((await app.request('/v1/store/x/alice/demo', {}, env)).status).toBe(200);
    for (const path of paths) {
      expect((await app.request(path, {}, env)).status).toBe(410);
    }
    const versions = await json(
      await app.request(
        '/v1/store/versions',
        post({ items: [{ repoId, releaseId }, { repoId: crypto.randomUUID(), releaseId }] }),
        env,
      ),
    );
    expect(versions.items[0]).toMatchObject({
      state: 'ok',
      current: { yanked: true },
      latest: { releaseId: first.release.id },
    });
    expect(versions.items[1]).toMatchObject({
      state: 'removed',
      ownerPublisherId: null,
      latest: null,
      handle: null,
      slug: null,
    });
    await data.db.update(releases).set({ yankedAt: new Date() }).where(eq(releases.id, first.release.id));
    await data.db.update(extensions).set({ latestReleaseId: null }).where(eq(extensions.repoId, repoId));
    const onlyYanked = await json(
      await app.request('/v1/store/versions', post({ items: [{ repoId, releaseId }] }), env),
    );
    expect(onlyYanked.items[0].latest).toBeNull();
  }));
test('file headers, comparison, path rejection and install count lifecycle', () =>
  withData(async (data) => {
    const env = makeEnv(data), alice = await publisher(data, env, 'alice');
    const built = await uploadTree(data, env, alice, files('demo'));
    const published = await json(await publishRequest(data, env, alice, 'demo', built.commitSha));
    const app = createApp({ data: () => data }), repoId = published.repo.repoId, releaseId = published.release.id;
    const path = '/v1/store/x/alice/demo/r/1.0.0/file/index.ts';
    const response = await app.request(path, {}, env);
    expect(response.status).toBe(200);
    expect(response.headers.get('Content-Type')).toBe('text/plain; charset=utf-8');
    expect(response.headers.get('X-Content-Type-Options')).toBe('nosniff');
    expect(response.headers.get('Content-Security-Policy')).toBe('sandbox');
    expect(response.headers.get('Cache-Control')).toBe('public, max-age=300');
    expect(response.headers.get('ETag')).toBeTruthy();
    expect(await response.text()).toBe('export default 1\n');
    const tar = await app.request('/v1/store/x/alice/demo/r/1.0.0/tar', {}, env);
    expect(tar.headers.get('Content-Type')).toBe('application/gzip');
    expect(tar.headers.get('X-Tar-Sha256')).toBe(published.release.tarSha256);
    expect(tar.headers.get('X-Tree-Sha')).toBe(published.release.treeSha);
    expect(tar.headers.get('Cache-Control')).toBe('public, max-age=300');
    expect(tar.headers.get('ETag')).toBe(`\"${published.release.tarSha256}\"`);
    for (const suffix of ['../index.ts', '%2e%2e/index.ts', '%2findex.ts']) {
      const status = (await app.request(path.replace('index.ts', suffix), {}, env)).status;
      expect([400, 404]).toContain(status);
    }
    const tree = await json(await app.request('/v1/store/x/alice/demo/r/1.0.0/tree', {}, env));
    expect(tree.files).toEqual(built.snap.files);
    const compared = await json(await app.request(`/v1/store/compare?base=${releaseId}&head=${releaseId}`, {}, env));
    expect(compared.counts).toEqual({ added: 0, removed: 0, modified: 0 });
    expect((await app.request('/v1/installs', {}, env)).status).toBe(401);
    expect((await app.request('/v1/installs', post({ repoId, releaseId }), env)).status).toBe(401);
    expect((await app.request(`/v1/installs/${repoId}`, { method: 'DELETE' }, env)).status).toBe(401);
    expect((await app.request('/v1/installs', post({ repoId, releaseId }, alice.headers), env)).status).toBe(204);
    expect((await app.request('/v1/installs', post({ repoId, releaseId }, alice.headers), env)).status).toBe(204);
    let listed = await json(await app.request('/v1/installs', { headers: alice.headers }, env));
    expect(listed.items).toHaveLength(1);
    expect((await data.db.select().from(extensions))[0]?.installCount).toBe(1);
    expect((await app.request(`/v1/installs/${repoId}`, { method: 'DELETE', headers: alice.headers }, env)).status)
      .toBe(204);
    listed = await json(await app.request('/v1/installs', { headers: alice.headers }, env));
    expect(listed.items).toHaveLength(0);
    expect((await data.db.select().from(extensions))[0]?.installCount).toBe(0);
    await app.request('/v1/installs', post({ repoId, releaseId }, alice.headers), env);
    const settings = await app.request('/v1/me/settings', {
      method: 'PATCH',
      headers: { ...alice.headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ rememberInstalls: false }),
    }, env);
    expect(settings.status).toBe(200);
    expect((await data.db.select().from(extensions))[0]?.installCount).toBe(0);
    expect((await app.request('/v1/installs', post({ repoId, releaseId }, alice.headers), env)).status).toBe(204);
    expect((await json(await app.request('/v1/installs', { headers: alice.headers }, env))).items).toHaveLength(0);
  }));
