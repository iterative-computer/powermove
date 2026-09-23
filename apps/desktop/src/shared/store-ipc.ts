/*
 * Store IPC: browsing the registry, and installing, updating and removing
 * store extensions on this Mac (store plan §2.2, §2.6, §6).
 *
 * Every channel is renderer-reachable and therefore hostile input: main
 * validates each payload with zod and checks the sender
 * (src/main/cloud/store-ipc.ts). Registry and install failures come back as
 * `{ ok: false, error }` results so the Store can show them in place;
 * malformed payloads and untrusted senders throw.
 */
import type {
  ApiErrorBody,
  Category,
  CompareDto,
  ExtensionDetailDto,
  ListingDto,
  ReleaseDto,
  TreeDto,
  VarDecl
} from '@powermove/registry/wire';
import type { ExtensionHealth } from '@powermove/registry/manifest';
import type { z } from 'zod';

export const STORE_IPC = {
  browse: 'store:browse',
  extensions: 'store:extensions',
  detail: 'store:detail',
  release: 'store:release',
  tree: 'store:tree',
  file: 'store:file',
  compare: 'store:compare',
  install: 'store:install',
  update: 'store:update',
  uninstall: 'store:uninstall',
  library: 'store:library',
  checkUpdates: 'store:check-updates',
  /** main → renderer: the update-check results changed (`StoreUpdates`). */
  updatesChanged: 'store:updates-changed'
} as const;

export type StoreCategory = z.infer<typeof Category>;
export type StoreSort = 'new' | 'installs' | 'name';

export interface BrowseSectionDto {
  id: 'featured' | 'picks' | 'new' | StoreCategory;
  title: string;
  items: ListingDto[];
}
export interface BrowseDto { sections: BrowseSectionDto[] }
export interface ExtensionsPageDto { items: ListingDto[]; nextCursor: string | null }
export type ReleaseWithCoordinateDto = ReleaseDto & { handle: string; slug: string };

/**
 * Errors made on this Mac rather than by the registry:
 *   integrity          the download did not match the release's hashes, or its manifest
 *   builtin_collision  the extension's id is a built-in's
 *   local              the file system refused (disk full, permissions)
 *   no_update          there is nothing newer to update to
 *   not_installed      no store install with that id is on this Mac
 * `id_collision` (a folder with that id exists) reuses the registry's code.
 */
export type StoreLocalErrorCode = 'integrity' | 'builtin_collision' | 'local' | 'no_update' | 'not_installed';
export type StoreErrorBody = ApiErrorBody | { error: StoreLocalErrorCode; detail: string };
export type StoreResult<T> = { ok: true; value: T } | { ok: false; error: StoreErrorBody };

export interface StoreInstallResult {
  localId: string;
  /** Set when the manifest `replaces` other extensions: names them. */
  warning?: string;
  /** Required values are missing, so it stays off until set up. */
  needsSetup: boolean;
}

export type StoreUpdateResult =
  | { kind: 'updated'; localId: string; version: string }
  /** The folder was changed since install: the new version is beside it for the agent to merge. */
  | { kind: 'staged-for-merge'; localId: string; version: string; path: string };

/** One update-check answer, per local id. */
export interface StoreUpdateState {
  state: 'ok' | 'hidden' | 'removed' | 'tombstoned';
  latest: { releaseId: string; version: string } | null;
  currentYanked: boolean;
  ownerPublisherId: string | null;
}
export type StoreUpdates = Record<string, StoreUpdateState>;

export type LibraryGroup = 'store' | 'yours' | 'builtin';
export type LibraryMaker = { handle: string } | { builtin: true } | { you: true };

/** An extension on this Mac, as the Library shows it. */
export interface LibraryItemDto {
  localId: string;
  name: string;
  version: string;
  category: StoreCategory;
  contributes: string[];
  vars: VarDecl[];
  health: ExtensionHealth;
  enabled: boolean;
  description: string | null;
  group: LibraryGroup;
  maker: LibraryMaker;
  origin?: { coordinate: string; version: string; repoId: string; releaseId: string };
  /** The manifest's `forkedFrom` (`id@version` for a built-in, `handle/slug@version` for the store). */
  forkedFrom?: string;
  published?: { coordinate: string | null; version: string; releaseId: string };
  update?: { version: string; releaseId: string; modified: boolean; state: 'available' | 'staged-for-merge' } | null;
  /** The origin was removed or deleted from the store; the files stay. */
  removed?: boolean;
  /** The folder no longer matches what was installed. */
  modified: boolean;
}

export interface StoreExtensionsRequest { category?: StoreCategory; q?: string; cursor?: string; sort?: StoreSort }
export interface StoreCoordinateRequest { handle: string; slug: string }
export interface StoreReleaseRequest { releaseId: string }
export interface StoreFileRequest { handle: string; slug: string; version: string; path: string }
export interface StoreCompareRequest { base: string; head: string }
export interface StoreInstallRequest { repoId: string; releaseId: string }
export interface StoreLocalRequest { localId: string }

/** Request and response types per invoke channel. */
export interface StoreChannels {
  'store:browse': { req: void; res: StoreResult<BrowseDto> };
  'store:extensions': { req: StoreExtensionsRequest; res: StoreResult<ExtensionsPageDto> };
  'store:detail': { req: StoreCoordinateRequest; res: StoreResult<ExtensionDetailDto> };
  'store:release': { req: StoreReleaseRequest; res: StoreResult<ReleaseWithCoordinateDto> };
  'store:tree': { req: StoreReleaseRequest; res: StoreResult<TreeDto> };
  'store:file': { req: StoreFileRequest; res: StoreResult<string> };
  'store:compare': { req: StoreCompareRequest; res: StoreResult<CompareDto> };
  'store:install': { req: StoreInstallRequest; res: StoreResult<StoreInstallResult> };
  'store:update': { req: StoreLocalRequest; res: StoreResult<StoreUpdateResult> };
  /** `removed: false` means nothing was there to remove. */
  'store:uninstall': { req: StoreLocalRequest; res: StoreResult<{ removed: boolean }> };
  'store:library': { req: void; res: LibraryItemDto[] };
  'store:check-updates': { req: void; res: StoreResult<StoreUpdates> };
}

export type StoreChannel = keyof StoreChannels;

export interface StoreBridge {
  browse(): Promise<StoreChannels['store:browse']['res']>;
  extensions(req: StoreExtensionsRequest): Promise<StoreChannels['store:extensions']['res']>;
  detail(req: StoreCoordinateRequest): Promise<StoreChannels['store:detail']['res']>;
  release(req: StoreReleaseRequest): Promise<StoreChannels['store:release']['res']>;
  tree(req: StoreReleaseRequest): Promise<StoreChannels['store:tree']['res']>;
  file(req: StoreFileRequest): Promise<StoreChannels['store:file']['res']>;
  compare(req: StoreCompareRequest): Promise<StoreChannels['store:compare']['res']>;
  install(req: StoreInstallRequest): Promise<StoreChannels['store:install']['res']>;
  update(req: StoreLocalRequest): Promise<StoreChannels['store:update']['res']>;
  uninstall(req: StoreLocalRequest): Promise<StoreChannels['store:uninstall']['res']>;
  library(): Promise<LibraryItemDto[]>;
  checkUpdates(): Promise<StoreChannels['store:check-updates']['res']>;
  onUpdatesChanged(cb: (updates: StoreUpdates) => void): () => void;
}

/** The longest search query sent to the registry. */
export const STORE_QUERY_MAX = 120;
