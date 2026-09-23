import { expect, test } from 'bun:test';
import { z } from 'zod';
import * as W from '../src/wire';
const id = '123e4567-e89b-42d3-a456-426614174000', sha = 'a'.repeat(40), sha256 = 'b'.repeat(64), date = '2026-09-22T10:00:00.000Z';
const owner = { id, handle: 'my-handle', tombstoned: false };
const listing = { repoId: id, owner, slug: 'my-extension', name: 'Name', tagline: 'Tag', category: 'effects', iconUrl: null, visibility: 'public', latest: { id, version: '1.0.0', publishedAt: date, apiVersion: 2, yankedAt: null }, installCount: 0, forkCount: 0, licence: 'MIT', forkedFrom: null, createdAt: date, updatedAt: date };
const manifest = { id: 'my-extension', name: 'Name', version: '1.0.0', apiVersion: 2, contributes: [], vars: [], forkedFrom: null, description: null };
const release = { id, repoId: id, version: '1.0.0', commitSha: sha, treeSha: sha, tarSha256: sha256, apiVersion: 2, fileCount: 1, sizeBytes: 100, notes: null, publishedAt: date, yankedAt: null, basedOnReleaseId: null, manifest };
const tree = { treeSha: sha, files: [{ path: 'index.ts', sha, size: 100 }] };
const me = { user: { id, name: null, email: 'a@example.com', image: null }, publisher: owner, settings: { rememberInstalls: true } };
const session = { token: 'abc', expiresAt: date };
const compare = { baseReleaseId: id, headReleaseId: id, files: [{ path: 'index.ts', status: 'added' }], counts: { added: 1, removed: 0, modified: 0 } };
const install = { repoId: id, releaseId: id, handle: 'my-handle', slug: 'my-extension', installedAt: date };
const samples: [string, z.ZodType, unknown][] = [
  ['Category', W.Category, 'effects'], ['Visibility', W.Visibility, 'public'], ['Moderation', W.Moderation, 'none'], ['Handle', W.Handle, 'my-handle'], ['Slug', W.Slug, 'my-extension'], ['Version', W.Version, '1.0.0'], ['Sha1', W.Sha1, sha], ['Sha256', W.Sha256, sha256], ['Uuid', W.Uuid, id], ['IsoDate', W.IsoDate, date], ['VarDecl', W.VarDecl, { key: 'MY_KEY', label: 'Key' }],
  ['PublisherDto', W.PublisherDto, owner], ['LineageDto', W.LineageDto, { repoId: id, handle: 'my-handle', slug: 'my-extension', releaseId: id, version: '1.0.0' }], ['ReleaseSummaryDto', W.ReleaseSummaryDto, listing.latest], ['ListingDto', W.ListingDto, listing], ['ManifestSummaryDto', W.ManifestSummaryDto, manifest], ['ReleaseDto', W.ReleaseDto, release], ['ExtensionDetailDto', W.ExtensionDetailDto, { ...listing, about: null, releases: [release], moderation: 'none' }], ['TreeFileDto', W.TreeFileDto, tree.files[0]], ['TreeDto', W.TreeDto, tree], ['CompareStatus', W.CompareStatus, 'added'], ['CompareDto', W.CompareDto, compare], ['InstallDto', W.InstallDto, install], ['MeDto', W.MeDto, me], ['SessionDto', W.SessionDto, session]
];
for (const [name, schema, sample] of samples) test(name, () => { expect(schema.safeParse(sample).success).toBe(true); expect(schema.safeParse(null).success).toBe(false); });
test('error codes and status are exhaustive', () => {
  expect(Object.keys(W.API_ERROR_STATUS).sort()).toEqual([...W.API_ERROR_CODES].sort());
  for (const code of [...W.API_ERROR_CODES]) expect(W.API_ERROR_STATUS[code]).toBeGreaterThanOrEqual(400);
  expect(W.ApiErrorBody.safeParse({ error: 'head_moved', head: sha }).success).toBe(true);
  expect(W.ApiErrorBody.safeParse({ error: 'head_moved' }).success).toBe(false);
  expect(W.ApiErrorBody.safeParse({ error: 'client_too_old', minimum: '1.2.0' }).success).toBe(true);
  expect(W.ApiErrorBody.safeParse({ error: 'bogus' }).success).toBe(false);
  expect(new W.ApiError({ error: 'not_found' }).status).toBe(404);
});
const e = { params: {}, query: {}, body: {} }, coordinate = { handle: 'my-handle', slug: 'my-extension' }, releaseCoordinate = { ...coordinate, version: '1.0.0' };
const req: Record<string, unknown> = {
  'Auth.DesktopStart': { ...e, query: { provider: 'google', state: 'a'.repeat(16), challenge: 'abc_-123' } }, 'Auth.DesktopDone': { ...e, query: { state: 'abc' } }, 'Auth.DesktopExchange': { ...e, body: { state: 'abc', token: 'tok', verifier: 'ver' } }, 'Auth.EmailSend': { ...e, body: { email: 'a@example.com' } }, 'Auth.EmailVerify': { ...e, body: { email: 'a@example.com', otp: '123456' } },
  'Me.SetHandle': { ...e, body: { handle: 'my-handle' } }, 'Me.Settings': { ...e, body: { rememberInstalls: false } },
  'Objects.Missing': { ...e, body: { shas: [sha] } }, 'Objects.Upload': { ...e, body: new FormData() },
  'Store.Extensions': { ...e, query: { category: 'effects', sort: 'new', limit: '50' } }, 'Store.Detail': { ...e, params: coordinate }, 'Store.Release': { ...e, params: releaseCoordinate }, 'Store.Tree': { ...e, params: releaseCoordinate }, 'Store.Tar': { ...e, params: releaseCoordinate }, 'Store.File': { ...e, params: { ...releaseCoordinate, path: 'index.ts' } }, 'Store.ReleaseById': { ...e, params: { releaseId: id } }, 'Store.TreeById': { ...e, params: { releaseId: id } }, 'Store.TarById': { ...e, params: { releaseId: id } }, 'Store.Compare': { ...e, query: { base: id, head: id } }, 'Store.Versions': { ...e, body: { items: [{ repoId: id, releaseId: id }] } }, 'Store.Icon': { ...e, params: { key: 'icon' } }, 'Store.Report': { ...e, params: coordinate, body: { reason: 'Reason' } },
  'Installs.Add': { ...e, body: { repoId: id, releaseId: id } }, 'Installs.Delete': { ...e, params: { repoId: id } },
  'Publish.PutRelease': { ...e, params: coordinate, body: { version: '1.0.0', commitSha: sha, listing: { name: 'Name', tagline: 'Tag', category: 'effects' }, waivers: [] } }, 'Publish.Yank': { ...e, params: releaseCoordinate }, 'Publish.PatchRepo': { ...e, params: coordinate, body: { visibility: 'public' } }, 'Publish.DeleteRepo': { ...e, params: coordinate },
  'Admin.Moderate': { ...e, params: { repoId: id }, body: { action: 'hide', reason: 'Reason' } }
};
const res: Record<string, unknown> = {
  'Auth.DesktopStart': '<html></html>', 'Auth.DesktopDone': '<html></html>', 'Auth.DesktopExchange': session, 'Auth.EmailSend': { ok: true }, 'Auth.EmailVerify': session, 'Auth.SignOut': { ok: true },
  'Me.Get': me, 'Me.SetHandle': { publisher: owner }, 'Me.Settings': { settings: { rememberInstalls: false } }, 'Me.Delete': undefined, 'Me.Repos': { items: [listing] }, 'Objects.Missing': { missing: [sha] }, 'Objects.Upload': { stored: [sha], present: [], rejected: [] },
  'Store.Browse': { sections: [{ id: 'featured', title: 'Featured', items: [listing] }] }, 'Store.Extensions': { items: [listing], nextCursor: null }, 'Store.Detail': { ...listing, about: null, releases: [release], moderation: 'none' }, 'Store.Release': release, 'Store.Tree': tree, 'Store.Tar': new Uint8Array(), 'Store.File': 'source', 'Store.ReleaseById': { ...release, handle: 'my-handle', slug: 'my-extension' }, 'Store.TreeById': tree, 'Store.TarById': new Uint8Array(), 'Store.Compare': compare,
  'Store.Versions': { items: [{ repoId: id, state: 'ok', ownerPublisherId: id, current: { yanked: false }, latest: { releaseId: id, version: '1.0.0', treeSha: sha, tarSha256: sha256, apiVersion: 2 }, handle: 'my-handle', slug: 'my-extension' }] }, 'Store.Icon': new Uint8Array(), 'Store.Report': undefined,
  'Installs.List': { items: [install] }, 'Installs.Add': undefined, 'Installs.Delete': undefined, 'Publish.PutRelease': { repo: listing, release }, 'Publish.Yank': { repo: listing, release }, 'Publish.PatchRepo': listing, 'Publish.DeleteRepo': undefined, 'Admin.Moderate': listing
};
for (const [groupName, group] of Object.entries({ Auth: W.Auth, Me: W.Me, Objects: W.Objects, Store: W.Store, Installs: W.Installs, Publish: W.Publish, Admin: W.Admin })) for (const [routeName, schemas] of Object.entries(group)) {
  const name = `${groupName}.${routeName}`;
  test(name, () => {
    const pair = schemas as { Req: z.ZodType; Res: z.ZodType };
    expect(pair.Req.safeParse(req[name] ?? e).success).toBe(true);
    expect(pair.Req.safeParse(null).success).toBe(false);
    expect(pair.Res.safeParse(res[name]).success).toBe(true);
    expect(pair.Res.safeParse(null).success).toBe(false);
  });
}
test('every API error variant parses with its required fields', () => {
  const fields: Partial<Record<W.ApiErrorCode, object>> = {
    head_moved: { head: sha }, object_conflict: { sha }, gone: { reason: 'yanked' },
    too_large: { limit: 'envelope' }, object_missing: { shas: [sha] },
    scan_blocked: { findings: [{ path: 'index.ts', line: 1, kind: 'github_token' }] },
    limit_exceeded: { code: 'file_too_large' }, client_too_old: { minimum: '1.2.0' }
  };
  for (const error of [...W.API_ERROR_CODES]) {
    expect(W.ApiErrorBody.safeParse({ error, ...fields[error] }).success).toBe(true);
  }
});
