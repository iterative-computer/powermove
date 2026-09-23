/*
 * Extension contract shared by main and renderer. FROZEN for apiVersion 1; apiVersion 2 adds `vars`.
 *
 * An extension is a directory containing `manifest.json` and an entry module.
 * Built-ins ship inside the app bundle; user/project extensions live on disk and
 * are compiled by main (esbuild + svelte) and served over app://powermove/ext/.
 */

export const EXTENSION_API_VERSION = 2 as const;
export const EXTENSION_API_VERSION_VARS = 2 as const;

export const EXTENSION_ID = /^[a-z0-9][a-z0-9-]{1,63}$/;
export const EXTENSION_VERSION = /^\d{1,6}\.\d{1,6}\.\d{1,6}$/;
export const EXTENSION_VAR_KEY = /^[A-Z][A-Z0-9_]{1,63}$/;
const EXTENSION_HANDLE = /^[a-z0-9][a-z0-9-]{1,38}$/;

export const CONTRIBUTION_KINDS = [
  'panels',
  'inspector',
  'media',
  'commands',
  'keybindings',
  'effects',
  'transitions',
  'layers',
  'themes',
  'palette',
  'menus',
  'status',
  'hooks'
] as const;
export type ContributionKind = (typeof CONTRIBUTION_KINDS)[number];

export type ExtensionAuthor = 'powermove' | 'user' | 'agent';
export type ExtensionScope = 'builtin' | 'user' | 'project';

export interface ExtensionManifest {
  id: string; // EXTENSION_ID
  name: string; // ≤ 80 chars
  version: string; // EXTENSION_VERSION
  apiVersion: number; // must be ≤ EXTENSION_API_VERSION
  description?: string; // ≤ 400 chars
  entry?: string; // relative path, default "index.ts"; .ts/.js/.mjs
  contributes?: ContributionKind[];
  replaces?: string[]; // extension ids this one supersedes while enabled
  dependsOn?: string[]; // extension ids that must be enabled and load first
  forkedFrom?: string; // "<id>@<version>" or "<handle>/<id>@<version>"
  vars?: ExtensionVarDecl[];
  author?: ExtensionAuthor;
  /** Stable, specific feature identifiers; broad contribution kinds are not features. */
  features?: string[];
  /** Built-in release metadata: custom extensions whose functionality is now included. */
  integrates?: string[];
}

export interface ExtensionVarDecl {
  key: string;
  label: string;
  secret?: boolean;
  required?: boolean;
  hint?: string;
}

export const MANIFEST_LIMITS = {
  nameChars: 80,
  descriptionChars: 400,
  listItems: 32,
  forkedFromChars: 160,
  sourceFiles: 400,
  sourceBytes: 8 * 1024 * 1024,
  bundleBytes: 16 * 1024 * 1024,
  errorChars: 4_000
} as const;

/** Result of validating an unknown value as a manifest. */
export type ManifestParse = { ok: true; manifest: ExtensionManifest } | { ok: false; error: string };

export function parseManifest(raw: unknown): ManifestParse {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return { ok: false, error: 'manifest must be an object' };
  const m = raw as Record<string, unknown>;
  const str = (k: string, max: number, required: boolean): string | undefined | null => {
    const v = m[k];
    if (v === undefined || v === null) return required ? null : undefined;
    if (typeof v !== 'string' || v.length === 0 || v.length > max) return null;
    return v;
  };
  const list = (k: string): string[] | undefined | null => {
    const v = m[k];
    if (v === undefined) return undefined;
    if (!Array.isArray(v) || v.length > MANIFEST_LIMITS.listItems) return null;
    if (!v.every((x) => typeof x === 'string' && EXTENSION_ID.test(x))) return null;
    return v as string[];
  };

  const id = str('id', 64, true);
  if (id === null || id === undefined || !EXTENSION_ID.test(id)) return { ok: false, error: 'invalid "id" (a-z, 0-9, -; 2–64 chars)' };
  const name = str('name', MANIFEST_LIMITS.nameChars, true);
  if (!name) return { ok: false, error: 'invalid "name"' };
  const version = str('version', 20, true);
  if (!version || !EXTENSION_VERSION.test(version)) return { ok: false, error: 'invalid "version" (x.y.z)' };
  const apiVersion = m.apiVersion;
  if (typeof apiVersion !== 'number' || !Number.isInteger(apiVersion) || apiVersion < 1) return { ok: false, error: 'invalid "apiVersion"' };
  if (apiVersion > EXTENSION_API_VERSION) return { ok: false, error: `apiVersion ${apiVersion} is newer than this app (${EXTENSION_API_VERSION})` };
  const description = str('description', MANIFEST_LIMITS.descriptionChars, false);
  if (description === null) return { ok: false, error: 'invalid "description"' };
  const entry = str('entry', 200, false);
  if (entry === null) return { ok: false, error: 'invalid "entry"' };
  if (entry !== undefined && !isSafeEntry(entry)) return { ok: false, error: 'invalid "entry" path' };
  const contributes = m.contributes;
  if (contributes !== undefined) {
    if (!Array.isArray(contributes) || !contributes.every((c) => (CONTRIBUTION_KINDS as readonly string[]).includes(String(c))))
      return { ok: false, error: 'invalid "contributes"' };
  }
  const replaces = list('replaces');
  if (replaces === null) return { ok: false, error: 'invalid "replaces"' };
  const dependsOn = list('dependsOn');
  if (dependsOn === null) return { ok: false, error: 'invalid "dependsOn"' };
  if (replaces?.includes(id) || dependsOn?.includes(id)) return { ok: false, error: 'extension cannot reference itself' };
  const features = list('features');
  if (features === null) return { ok: false, error: 'invalid "features"' };
  const integrates = list('integrates');
  if (integrates === null) return { ok: false, error: 'invalid "integrates"' };
  const forkedFrom = str('forkedFrom', MANIFEST_LIMITS.forkedFromChars, false);
  // Frozen for apiVersion 1: any string up to the cap is accepted here. The
  // two forms (`parseForkedFrom`) are enforced where lineage matters: the
  // built-in rebase tool and the Store's publish path.
  if (forkedFrom === null) return { ok: false, error: 'invalid "forkedFrom"' };
  const vars = m.vars;
  if (vars !== undefined) {
    if (apiVersion < EXTENSION_API_VERSION_VARS) return { ok: false, error: '"vars" requires apiVersion 2' };
    if (!Array.isArray(vars) || vars.length > MANIFEST_LIMITS.listItems || !vars.every((v) =>
      v && typeof v === 'object' && !Array.isArray(v) &&
      typeof v.key === 'string' && EXTENSION_VAR_KEY.test(v.key) &&
      typeof v.label === 'string' && v.label.length >= 1 && v.label.length <= 80 &&
      (v.hint === undefined || (typeof v.hint === 'string' && v.hint.length <= 200)) &&
      (v.secret === undefined || typeof v.secret === 'boolean') &&
      (v.required === undefined || typeof v.required === 'boolean')
    )) return { ok: false, error: 'invalid "vars"' };
    const keys = new Set<string>();
    for (const v of vars as ExtensionVarDecl[]) {
      if (keys.has(v.key)) return { ok: false, error: `duplicate var key "${v.key}"` };
      keys.add(v.key);
    }
  }
  const author = m.author;
  if (author !== undefined && author !== 'powermove' && author !== 'user' && author !== 'agent') return { ok: false, error: 'invalid "author"' };

  const manifest: ExtensionManifest = { id, name, version, apiVersion };
  if (description) manifest.description = description;
  if (entry) manifest.entry = entry;
  if (contributes) manifest.contributes = contributes as ContributionKind[];
  if (replaces) manifest.replaces = replaces;
  if (dependsOn) manifest.dependsOn = dependsOn;
  if (forkedFrom) manifest.forkedFrom = forkedFrom;
  if (vars) manifest.vars = vars as ExtensionVarDecl[];
  if (author) manifest.author = author as ExtensionAuthor;
  if (features) manifest.features = [...new Set(features)];
  if (integrates) manifest.integrates = [...new Set(integrates)];
  return { ok: true, manifest };
}

export function parseForkedFrom(s: string):
  | { kind: 'builtin'; id: string; version: string }
  | { kind: 'store'; handle: string; id: string; version: string }
  | null {
  const at = s.lastIndexOf('@');
  if (at < 0) return null;
  const version = s.slice(at + 1);
  if (!EXTENSION_VERSION.test(version)) return null;
  const origin = s.slice(0, at);
  const slash = origin.indexOf('/');
  if (slash < 0) return EXTENSION_ID.test(origin) ? { kind: 'builtin', id: origin, version } : null;
  const handle = origin.slice(0, slash);
  const id = origin.slice(slash + 1);
  return EXTENSION_HANDLE.test(handle) && EXTENSION_ID.test(id) ? { kind: 'store', handle, id, version } : null;
}

export function isSafeEntry(entry: string): boolean {
  if (entry.includes('\\') || entry.startsWith('/') || entry.includes('..') || entry.includes('\0')) return false;
  return /\.(ts|js|mjs)$/.test(entry);
}

/* ── runtime records (main → renderer) ───────────────────── */

export type ExtensionHealth =
  | { state: 'ok' }
  | { state: 'disabled' } // by user
  | { state: 'build-error'; error: string }
  | { state: 'manifest-error'; error: string }
  | { state: 'activation-error'; error: string }
  | { state: 'runtime-error'; error: string }
  | { state: 'needs-update'; error: string }
  | { state: 'replaced'; by: string };

export interface ExtensionRecord {
  id: string;
  scope: ExtensionScope;
  manifest: ExtensionManifest | null; // null when manifest failed to parse
  update?: { forkedFrom: string; base: string; current: string };
  dir: string; // absolute path on disk (built-ins: resource path)
  enabled: boolean; // user intent (persisted)
  /** Module URL to import, or null when there is nothing loadable (build/manifest error, builtin handled in-bundle). */
  bundleUrl: string | null;
  bundleHash: string | null;
  health: ExtensionHealth;
  updatedAt: number;
}

/* ── IPC ─────────────────────────────────────────────────── */

export const EXT_IPC = {
  list: 'ext:list',
  setEnabled: 'ext:set-enabled',
  remove: 'ext:remove',
  reload: 'ext:reload',
  create: 'ext:create',
  reveal: 'ext:reveal',
  readSource: 'ext:read-source',
  reportHealth: 'ext:report-health', // renderer → main (activation/runtime failures)
  changed: 'ext:changed' // main → renderer
} as const;

export interface ExtensionSetEnabledRequest {
  id: string;
  enabled: boolean;
}
export interface ExtensionIdRequest {
  id: string;
}
export interface ExtensionCreateRequest {
  manifest: ExtensionManifest;
  files: Record<string, string>; // relative path → text; entry must be included
}
export interface ExtensionSourceFile {
  path: string;
  text: string;
}
export interface ExtensionHealthReport {
  id: string;
  health: Extract<ExtensionHealth, { state: 'activation-error' | 'runtime-error' | 'ok' }>;
}
export interface ExtensionsChangedEvent {
  /** ids whose record changed; renderer re-fetches list and reloads these. */
  ids: string[];
  reason: 'watch' | 'enable' | 'disable' | 'remove' | 'create' | 'reload' | 'boot' | 'health';
}

/** Store key that holds user enable/disable intent: Record<id, boolean>. */
export const EXTENSIONS_STORE_KEY = 'extensions' as const;

export interface ExtensionsBridge {
  list(): Promise<ExtensionRecord[]>;
  setEnabled(req: ExtensionSetEnabledRequest): Promise<ExtensionRecord[]>;
  remove(req: ExtensionIdRequest): Promise<ExtensionRecord[]>;
  reload(req: ExtensionIdRequest): Promise<ExtensionRecord[]>;
  create(req: ExtensionCreateRequest): Promise<ExtensionRecord[]>;
  reveal(req: ExtensionIdRequest): Promise<void>;
  readSource(req: ExtensionIdRequest): Promise<ExtensionSourceFile[]>;
  reportHealth(req: ExtensionHealthReport): void;
  onChanged(cb: (e: ExtensionsChangedEvent) => void): () => void;
}
