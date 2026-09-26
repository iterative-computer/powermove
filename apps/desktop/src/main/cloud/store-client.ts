/*
 * Typed wrappers over the cloud client for the Store's read routes and the
 * installs list (wire contract: Store reads + Installs). Every JSON body is
 * parsed with its wire schema (`client.request`); the tar is read as bytes
 * under a cap and verified by the installer, never trusted as is.
 */
import { REGISTRY_LIMITS } from '@powermove/registry/limits';
import { ApiError, Installs, Store } from '@powermove/registry/wire';
import type { z } from 'zod';

import type { CloudClient } from './client';

/** The most compressed bytes a release tar may be: its inflated cap plus tar headers. */
export const TAR_BYTES_CAP = REGISTRY_LIMITS.treeBytes + REGISTRY_LIMITS.files * 1024 + 64 * 1024;

export type BrowseResult = z.infer<typeof Store.Browse.Res>;
export type ExtensionsResult = z.infer<typeof Store.Extensions.Res>;
export type DetailResult = z.infer<typeof Store.Detail.Res>;
export type ReleaseByIdResult = z.infer<typeof Store.ReleaseById.Res>;
export type TreeResult = z.infer<typeof Store.TreeById.Res>;
export type CompareResult = z.infer<typeof Store.Compare.Res>;
export type VersionsResult = z.infer<typeof Store.Versions.Res>;
export type VersionsItem = VersionsResult['items'][number];
export type InstallsResult = z.infer<typeof Installs.List.Res>;

export interface ExtensionsQuery {
  category?: z.infer<typeof Store.Extensions.Req.shape.query.shape.category>;
  q?: string;
  sort?: 'new' | 'installs' | 'name';
  cursor?: string;
  limit?: number;
}

export interface StoreClient {
  browse(): Promise<BrowseResult>;
  extensions(query: ExtensionsQuery): Promise<ExtensionsResult>;
  detail(handle: string, slug: string): Promise<DetailResult>;
  release(releaseId: string): Promise<ReleaseByIdResult>;
  tree(releaseId: string): Promise<TreeResult>;
  /** The release's `.tar.gz`, read under a size cap. */
  tar(releaseId: string): Promise<Uint8Array>;
  /** One source file as text (≤ 2 MiB). */
  file(handle: string, slug: string, version: string, path: string): Promise<string>;
  compare(base: string, head: string): Promise<CompareResult>;
  versions(items: Array<{ repoId: string; releaseId: string }>): Promise<VersionsResult>;
  listInstalls(): Promise<InstallsResult>;
  addInstall(repoId: string, releaseId: string): Promise<void>;
  deleteInstall(repoId: string): Promise<void>;
}

/** Read a body into memory, refusing anything past `max` bytes. */
export async function readCapped(response: Response, max: number): Promise<Uint8Array> {
  const declared = Number(response.headers.get('Content-Length'));
  if (Number.isFinite(declared) && declared > max) throw new ApiError({ error: 'too_large', limit: 'envelope' });
  if (!response.body) return new Uint8Array(await response.arrayBuffer());
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > max) {
      await reader.cancel().catch(() => undefined);
      throw new ApiError({ error: 'too_large', limit: 'envelope' });
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  return bytes;
}

/** `path` as URL segments: each one encoded, slashes kept. */
function encodePath(path: string): string {
  return path.split('/').map(encodeURIComponent).join('/');
}

export function createStoreClient(client: () => CloudClient): StoreClient {
  return {
    browse: () => client().request(Store.Browse.Res, (api) => api.v1.store.browse.$get()),

    extensions(query) {
      const q: Record<string, string> = {};
      if (query.category) q['category'] = query.category;
      if (query.q) q['q'] = query.q;
      if (query.sort) q['sort'] = query.sort;
      if (query.cursor) q['cursor'] = query.cursor;
      if (query.limit) q['limit'] = String(query.limit);
      const args = { query: q };
      return client().request(Store.Extensions.Res, (api) => api.v1.store.extensions.$get(args));
    },

    detail: (handle, slug) =>
      client().request(Store.Detail.Res, (api) => api.v1.store.x[':handle'][':slug'].$get({ param: { handle, slug } })),

    release: (releaseId) =>
      client().request(Store.ReleaseById.Res, (api) => api.v1.store.releases[':releaseId'].$get({ param: { releaseId } })),

    tree: (releaseId) =>
      client().request(Store.TreeById.Res, (api) => api.v1.store.releases[':releaseId'].tree.$get({ param: { releaseId } })),

    async tar(releaseId) {
      const response = await client().raw((api) => api.v1.store.releases[':releaseId'].tar.$get({ param: { releaseId } }));
      return readCapped(response, TAR_BYTES_CAP);
    },

    async file(handle, slug, version, path) {
      const current = client();
      /* A wildcard route: the typed client cannot address it, so the URL is
         built here from validated segments on the bound origin. */
      const url = `${current.origin}/v1/store/x/${encodeURIComponent(handle)}/${encodeURIComponent(slug)}/r/${encodeURIComponent(version)}/file/${encodePath(path)}`;
      const response = await current.raw((_api, fetch) => fetch(url));
      const bytes = await readCapped(response, REGISTRY_LIMITS.fileBytes);
      return new TextDecoder('utf-8').decode(bytes);
    },

    compare: (base, head) => {
      const args = { query: { base, head } };
      return client().request(Store.Compare.Res, (api) => api.v1.store.compare.$get(args));
    },

    versions: (items) => {
      const args = { json: { items } };
      return client().request(Store.Versions.Res, (api) => api.v1.store.versions.$post(args));
    },

    listInstalls: () => client().request(Installs.List.Res, (api) => api.v1.installs.$get()),

    addInstall: (repoId, releaseId) => {
      const args = { json: { repoId, releaseId } };
      return client().request(null, (api) => api.v1.installs.$post(args));
    },

    deleteInstall: (repoId) =>
      client().request(null, (api) => api.v1.installs[':repoId'].$delete({ param: { repoId } }))
  };
}
