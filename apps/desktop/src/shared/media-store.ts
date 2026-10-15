/*
 * The host media store's shared vocabulary (plan section 21): IPC channel
 * names, the bridge the preload exposes, and the document fields linked
 * media adds (section 6: asset `storage`, `link`; project `media`).
 */

/** BLAKE3 digests are 64 lowercase hex digits. */
export const HASH_HEX = /^[a-f0-9]{64}$/;

/** Renderer storage keys: `media:v2:<size>:<sha256>`, poster and preview keys
 *  under it, or a bare asset id from projects that predate content keys. */
const STORAGE_KEY = /^[A-Za-z0-9:._-]{1,300}$/;
export const validStorageKey = (key: unknown): key is string => typeof key === 'string' && STORAGE_KEY.test(key);

export const MEDIA_STORE_IPC = {
  config: 'media-store:config',
  record: 'media-store:record',
  classify: 'media-store:classify',
  putFile: 'media-store:put-file',
  beginPut: 'media-store:begin-put',
  putChunk: 'media-store:put-chunk',
  finishPut: 'media-store:finish-put',
  abortPut: 'media-store:abort-put',
  putProjectRange: 'media-store:put-project-range',
  link: 'media-store:link',
  open: 'media-store:open',
  read: 'media-store:read',
  close: 'media-store:close',
  relink: 'media-store:relink',
  bind: 'media-store:bind',
  removeKey: 'media-store:remove-key',
  convertStill: 'media-store:convert-still',
  convertVideo: 'media-store:convert-video',
  convertUpload: 'media-store:convert-upload'
} as const;

/** Where a store record's bytes came from. */
export type MediaRecordSource = 'import' | 'stream' | 'indexeddb' | 'project' | 'rendition';

export type MediaStoreRecord = {
  key: string;
  /** BLAKE3 of the object, 64 hex digits. */
  hash: string;
  size: number;
  type: string;
  /** PMV4 storage revision (the hash unless the writer named one). */
  revision: string;
  at: number;
  source?: MediaRecordSource;
};

/** Asset `storage` (section 6). */
export type AssetStorage = 'copied' | 'linked';

/** Asset `link` (section 6): where a linked original is and what it is. */
export type MediaLink = {
  path: string;
  /** BLAKE3 of the original. Relinking is by this hash only. */
  hash: string;
  size: number;
  mtimeMs?: number;
  /** Volume and file numbers when linked; a hint for finding moved files. */
  dev?: number;
  ino?: number;
};

/** Project `media` (section 6; Settings › Project › Media). */
export type ProjectMediaSettings = {
  /** When importing: Copy into project (default, decision 2) or Link to files. */
  whenImporting?: 'copy' | 'link';
};

/** Why an import is copied even when the project links files. */
export type CopyReason = 'agentWorkspace' | 'cameraMedia' | 'notAFile';

export type ClassifyResult = { storage: AssetStorage; reason?: CopyReason };

export type MediaStoreConfig = {
  /** "Use New Engine" (read once at launch). The store is the source of
   *  bytes only while it is on. */
  enabled: boolean;
};

/** A read session opened by `open`. */
export type MediaReadOpen =
  | { ok: true; token: string; size: number; type: string; hash: string; path?: string; relinked?: MediaLink }
  | { ok: false; status: 'missing' | 'offline' | 'changed' };

export type MediaPutFileResult = MediaStoreRecord & { link?: MediaLink };

/** What a kept original becomes for the page's viewer. */
export type ConversionKind = 'still' | 'video';

/** A rendition main made; the page reads it with `media.readPlaybackProxy`.
 *  `missing` means main has neither the stored object nor the linked file,
 *  so the page sends the bytes it has (`convertUpload`). */
export type MediaConversion =
  | { ok: true; token: string; size: number; type: 'image/png' | 'video/webm' }
  | { ok: false; error: string; missing?: true };

export type MediaBinding = { key?: string; hash?: string; link?: MediaLink };

/** `window.powermove.mediaStore` (preload). */
export interface MediaStoreBridge {
  configSync(): MediaStoreConfig;
  record(key: string): Promise<MediaStoreRecord | null>;
  classify(path: string): Promise<ClassifyResult>;
  /** Copies a file into the store under `key`. */
  putFile(request: { path: string; key: string | null; type?: string; revision?: string | null }): Promise<MediaStoreRecord>;
  beginPut(request: { key: string | null; type: string; size: number; revision?: string | null; origin?: MediaRecordSource }): Promise<string>;
  putChunk(token: string, offset: number, data: Uint8Array): Promise<void>;
  /** Commits an upload when the bytes on disk match the sender's BLAKE3,
   *  or (for project ranges) the file's recorded SHA-256. */
  finishPut(token: string, expected: { blake3?: string; sha256?: string }): Promise<MediaStoreRecord & { sha256: string }>;
  /** Main copies `length` bytes (at most 64 MiB) at `offset` of the project
   *  file this window opened (`readToken`) into upload `token` at `at`. */
  putProjectRange?(token: string, readToken: string, offset: number, length: number, at: number): Promise<void>;
  abortPut(token: string): Promise<void>;
  /** Hashes a file in place for Link to files. */
  link(path: string): Promise<MediaLink>;
  open(request: { key?: string; link?: MediaLink; hash?: string }): Promise<MediaReadOpen>;
  read(token: string, offset: number, length: number): Promise<Uint8Array>;
  close(token: string): Promise<void>;
  /** Looks for a moved or renamed linked original by content hash. */
  relink(request: { link: MediaLink; searchRoots?: string[] }): Promise<MediaLink | null>;
  /** The open project's media in this window, for the engine's handles. */
  bind(projectId: string, assets: Record<string, MediaBinding>): Promise<void>;
  removeKey(key: string): Promise<boolean>;
  /** A PNG for the page's viewer from a stored (`hash`) or linked TIFF or
   *  HEIF original; read it with `media.readPlaybackProxy`. */
  convertStill?(request: { hash?: string; link?: MediaLink; name: string }): Promise<MediaConversion>;
  /** A playback proxy (VP9 WebM) of a stored or linked video original the
   *  viewer cannot decode. */
  convertVideo?(request: { hash?: string; link?: MediaLink; name: string }): Promise<MediaConversion>;
  /** Converts an upload (`beginPut` with no key, then `putChunk`) without
   *  storing it, when main lacks the original; the upload is consumed. */
  convertUpload?(token: string, request: { kind: ConversionKind; name: string; blake3: string }): Promise<MediaConversion>;
}

/** Shown when a legacy-engine build opens a project stamped for the new engine. */
export const REQUIRES_NATIVE_MESSAGE = 'This project uses the new engine. Turn on Use New Engine to open it.';
