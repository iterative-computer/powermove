/*
 * Proxies (plan section 14, appendix D, Phase 4a): the contract between the
 * renderer's proxy controller (renderer/src/media/proxies.ts) and the native
 * engine's proxy service in main (`host.nativeEngine.proxies`, installed as
 * `PM.EngineProxies`). Main and preload import these types from here; the
 * renderer re-exports them (media/proxies-types.ts).
 *
 * The project stores intent only (section 14.5): `project.media.proxies`
 * and each asset's `alwaysPlayOriginal`. What is generated, where, and when
 * is the backend's (the machine profile and the automatic policy).
 */

/** pm-protocol `priority::ProxyFormat` (shared/engine-protocol/ProxyFormat). */
export type ProxyFormat = 'proResProxy' | 'proResLt' | 'hevc10';
/** pm-protocol `priority::ProxySize` (shared/engine-protocol/ProxySize). */
export type ProxySize = 'automatic' | 'half' | 'quarter' | 'p1080' | 'p720';
/** Settings › Project › Media › Create proxies. */
export type ProxyCreatePolicy = 'whenNeeded' | 'allVideo' | 'onlyWhenAsked';

export type ProjectProxyIntent = {
  create: ProxyCreatePolicy;
  size: ProxySize;
  format: ProxyFormat;
  /** Assets marked Always Play Original: shown from the original in every viewer mode. */
  alwaysPlayOriginal: string[];
};

/** What main needs to find and key an asset's proxy (sent with the intent). */
export type ProxyAssetDescriptor = {
  /** BLAKE3 of the original's bytes (the media store's and pm-media's digest). */
  contentHash?: string;
  kind?: string;
  name?: string;
  /** The original's path on disk, when known (linked media, or the import's source). */
  sourcePath?: string;
  /** The asset is an image sequence (proxies apply, section 14.3). */
  imageSequence?: boolean;
  alwaysPlayOriginal?: boolean;
};

export type AssetProxyState =
  | { state: 'none' }
  | { state: 'queued' }
  | { state: 'creating'; progress: number }
  | { state: 'ready'; width: number; height: number; codec: string; bytes: number }
  | { state: 'failed'; error: string }
  | { state: 'unsupported' };

export type ProxyActivity = {
  /** Whole-clip jobs for this project; null when none are queued or running. */
  creating: { done: number; total: number } | null;
  /** Why generation is paused. `lowDisk` is the disk guard (section 14.5). */
  paused: 'lowDisk' | 'playback' | 'export' | null;
};

export type ProxySnapshot = {
  project: string;
  /** By asset id; an asset missing here has no proxy (`none`). */
  assets: Record<string, AssetProxyState>;
  activity: ProxyActivity;
};

/** Settings › General › Media (this Mac) › Generated media location. */
export type GeneratedMediaLocation =
  | { kind: 'cache' }
  | { kind: 'project' }
  | { kind: 'folder'; path: string };

/** `PM.EngineProxies`: main's proxy service for this window. */
export interface EngineProxiesBridge {
  /** Measurements from the native decoder; project intent still decides generation. */
  slowDecodes?(project: string, assets: string[]): Promise<void>;
  snapshot(project: string): Promise<ProxySnapshot>;
  subscribe(listener: (snapshot: ProxySnapshot) => void): () => void;
  /** On project open and after each intent edit, its undo and redo, and each
   *  change to the project's assets. */
  setIntent(
    project: string,
    intent: ProjectProxyIntent,
    assets: Record<string, ProxyAssetDescriptor>,
    projectPath?: string
  ): Promise<void>;
  create(project: string, assets: string[]): Promise<void>;
  rebuild(project: string, assets: string[]): Promise<void>;
  remove(project: string, assets: string[]): Promise<void>;
  location(): Promise<GeneratedMediaLocation>;
  setLocation(location: GeneratedMediaLocation): Promise<GeneratedMediaLocation>;
  /** The native folder panel; null when cancelled. */
  chooseFolder(): Promise<string | null>;
  /** "Preview memory limit" in bytes (0 is Off): the engine's cached-preview lease. */
  setPreviewMemory?(bytes: number): Promise<void>;
}

/** Window event dispatched after `PM.EngineProxies` is set or cleared. */
export const ENGINE_PROXIES_CHANGED = 'powermove:engine-proxies-changed';

export const DEFAULT_PROXY_INTENT: Readonly<Omit<ProjectProxyIntent, 'alwaysPlayOriginal'>> = Object.freeze({
  create: 'whenNeeded',
  size: 'automatic',
  format: 'proResProxy'
});

export const PROXY_CREATE_OPTIONS: ReadonlyArray<{ v: ProxyCreatePolicy; label: string }> = [
  { v: 'whenNeeded', label: 'When needed' },
  { v: 'allVideo', label: 'For all video' },
  { v: 'onlyWhenAsked', label: 'Only when I ask' }
];

export const PROXY_SIZE_OPTIONS: ReadonlyArray<{ v: ProxySize; label: string }> = [
  { v: 'automatic', label: 'Automatic' },
  { v: 'half', label: 'Half' },
  { v: 'quarter', label: 'Quarter' },
  { v: 'p1080', label: '1080p' },
  { v: 'p720', label: '720p' }
];

export const PROXY_FORMAT_OPTIONS: ReadonlyArray<{ v: ProxyFormat; label: string }> = [
  { v: 'proResProxy', label: 'ProRes 422 Proxy' },
  { v: 'proResLt', label: 'ProRes 422 LT' },
  { v: 'hevc10', label: 'HEVC (smaller files)' }
];

const oneOf = <T extends string>(options: ReadonlyArray<{ v: T }>, value: unknown, fallback: T): T =>
  options.some((option) => option.v === value) ? value as T : fallback;

/** The stored `project.media.proxies` (absent fields are defaults) as a full intent. */
export function proxyIntentOf(project: { media?: { proxies?: unknown }; assets?: Record<string, unknown> } | null | undefined): ProjectProxyIntent {
  const stored = project?.media?.proxies;
  const fields = stored && typeof stored === 'object' ? stored as Record<string, unknown> : {};
  const alwaysPlayOriginal = Object.entries(project?.assets ?? {})
    .filter(([, meta]) => !!meta && typeof meta === 'object' && (meta as { alwaysPlayOriginal?: unknown }).alwaysPlayOriginal === true)
    .map(([id]) => id)
    .sort();
  return {
    create: oneOf(PROXY_CREATE_OPTIONS, fields['create'], DEFAULT_PROXY_INTENT.create),
    size: oneOf(PROXY_SIZE_OPTIONS, fields['size'], DEFAULT_PROXY_INTENT.size),
    format: oneOf(PROXY_FORMAT_OPTIONS, fields['format'], DEFAULT_PROXY_INTENT.format),
    alwaysPlayOriginal
  };
}

/** The stored form of the project-level fields: defaults are left out, and
 *  null when every field is its default (the key is then removed). */
export function storedProxyIntent(intent: Pick<ProjectProxyIntent, 'create' | 'size' | 'format'>): Record<string, string> | null {
  const stored: Record<string, string> = {};
  if (intent.create !== DEFAULT_PROXY_INTENT.create) stored['create'] = intent.create;
  if (intent.size !== DEFAULT_PROXY_INTENT.size) stored['size'] = intent.size;
  if (intent.format !== DEFAULT_PROXY_INTENT.format) stored['format'] = intent.format;
  return Object.keys(stored).length ? stored : null;
}

/** Asset kinds that can have a proxy: video, and image sequences (section 14.3). */
export function proxyEligible(meta: { kind?: unknown; imageSequence?: unknown } | null | undefined): boolean {
  return meta?.kind === 'video' || !!meta?.imageSequence;
}

export function validGeneratedMediaLocation(value: unknown): value is GeneratedMediaLocation {
  if (!value || typeof value !== 'object') return false;
  const location = value as Record<string, unknown>;
  if (location['kind'] === 'cache' || location['kind'] === 'project') return Object.keys(location).length === 1;
  return location['kind'] === 'folder' && typeof location['path'] === 'string' && location['path'].startsWith('/')
    && location['path'].length <= 4096 && !location['path'].includes('\0') && Object.keys(location).length === 2;
}
