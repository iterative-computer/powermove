import { randomBytes } from 'node:crypto';
import * as fs from 'node:fs/promises';
import path from 'node:path';

import { BrowserWindow, shell } from 'electron';

import {
  EXTENSION_ID,
  EXTENSION_VERSION,
  EXTENSIONS_STORE_KEY,
  EXT_IPC,
  MANIFEST_LIMITS,
  needsTrust,
  parseManifest,
  type ExtensionCreateRequest,
  type ExtensionHealth,
  type ExtensionHealthReport,
  type ExtensionIdRequest,
  type ExtensionManifest,
  type ExtensionRecord,
  type ExtensionSetEnabledRequest,
  type ExtensionSourceFile,
  type ExtensionVarDecl,
  type ExtensionsChangedEvent,
  type TrustLevel
} from '../../shared/extensions';
import { missingKeys, resolveVars, type VarsResolution } from '../env/resolve';
import type { Store } from '../storage';
import { compileExtension } from './compiler';
import { scanExtensionDirs, type DiscoveredExtension } from './discovery';

const SOURCE_FILE_BYTES = 64 * 1024;

type Compiler = typeof compileExtension;
type Scanner = typeof scanExtensionDirs;

export interface ExtensionRegistryOptions {
  store: Store;
  userDir: string;
  buildDir: string;
  builtinIds: string[];
  resourcesDir: string;
  /** Project extension roots, in increasing override priority. */
  projectDirs?: string[];
  /** Test seams; production callers should leave these unset. */
  compile?: Compiler;
  scan?: Scanner;
  broadcast?: (event: ExtensionsChangedEvent) => void;
  revealPath?: (fullPath: string) => void;
  now?: () => number;
  /**
   * Resolves a user extension's declared values. Without one (the
   * `powermove serve` host, where values are not available yet) nothing is
   * set, so required values keep the extension in "Needs setup".
   */
  resolveVars?: (id: string, decls: ExtensionVarDecl[]) => Promise<VarsResolution>;
  /**
   * Who wrote a user or project extension (design §2): made here, from the
   * Store, or a Store install the user trusted. Without one (the
   * `powermove serve` host, which installs nothing from the Store) every
   * folder is local. A resolver that fails reads as `store`: an unreadable
   * provenance file must not promote someone else's code.
   */
  trustFor?: (id: string, scope: 'user' | 'project') => TrustLevel | Promise<TrustLevel>;
}

export interface ExtensionRegistry {
  readonly buildDir: string;
  readonly userDir: string;
  list(): ExtensionRecord[];
  refresh(ids?: string[]): Promise<void>;
  setEnabled(request: ExtensionSetEnabledRequest): Promise<ExtensionRecord[]>;
  remove(request: ExtensionIdRequest): Promise<ExtensionRecord[]>;
  reload(request: ExtensionIdRequest): Promise<ExtensionRecord[]>;
  create(request: ExtensionCreateRequest): Promise<ExtensionRecord[]>;
  reveal(request: ExtensionIdRequest): Promise<void>;
  readSource(request: ExtensionIdRequest): Promise<ExtensionSourceFile[]>;
  reportHealth(report: ExtensionHealthReport): void;
  emitChanged(event: ExtensionsChangedEvent): void;
}

interface CachedBuild {
  signature: string;
  result: Awaited<ReturnType<Compiler>>;
}

interface ParsedBuiltin {
  id: string;
  scope: 'builtin';
  dir: string;
  manifest: ExtensionManifest | null;
  error?: string;
}

export function createExtensionRegistry(options: ExtensionRegistryOptions): ExtensionRegistry {
  const records = new Map<string, ExtensionRecord>();
  const buildCache = new Map<string, CachedBuild>();
  const enabledState = readEnabledState(options.store.get
    ? options.store.get(EXTENSIONS_STORE_KEY) : options.store.snapshot()[EXTENSIONS_STORE_KEY]);
  const compiler = options.compile ?? compileExtension;
  const scanner = options.scan ?? scanExtensionDirs;
  const now = options.now ?? Date.now;
  const resolveFor = async (id: string, decls: ExtensionVarDecl[]): Promise<VarsResolution> => {
    if (!options.resolveVars) return resolveVars(decls, new Map());
    try {
      return await options.resolveVars(id, decls);
    } catch (error) {
      // Unreadable values behave as unset: the extension waits for setup.
      console.error(`[extensions] could not read values for ${id}`, error);
      return resolveVars(decls, new Map());
    }
  };
  const trustOf = async (id: string, scope: 'user' | 'project'): Promise<TrustLevel> => {
    if (!options.trustFor) return 'local';
    try {
      return await options.trustFor(id, scope);
    } catch (error) {
      console.error(`[extensions] could not tell where ${id} came from`, error);
      return 'store';
    }
  };
  let refreshQueue = Promise.resolve();

  const enabledFor = (id: string): boolean => enabledState.get(id) ?? true;

  const emitChanged = (event: ExtensionsChangedEvent): void => {
    if (options.broadcast) {
      options.broadcast(event);
      return;
    }
    for (const window of BrowserWindow.getAllWindows()) {
      try {
        if (!window.isDestroyed() && !window.webContents.isDestroyed()) {
          window.webContents.send(EXT_IPC.changed, event);
        }
      } catch {
        // A closing renderer must never make extension maintenance fail.
      }
    }
  };

  async function performRefresh(requestedIds?: string[]): Promise<void> {
    const filter = requestedIds === undefined ? null : new Set(requestedIds.map(validateId));
    await fs.mkdir(options.userDir, { recursive: true });
    await fs.mkdir(options.buildDir, { recursive: true });

    const builtins = await readBuiltins(options.resourcesDir, options.builtinIds);
    const shippedBuiltins = new Map(
      builtins.flatMap((builtin) => builtin.manifest === null ? [] : [[builtin.id, builtin.manifest] as const])
    );
    const scanDirs: Array<{ dir: string; scope: 'user' | 'project' }> = [
      { dir: options.userDir, scope: 'user' },
      ...(options.projectDirs ?? []).map((dir) => ({ dir, scope: 'project' as const }))
    ];
    const discovered = await scanner(scanDirs);

    // Later scopes deliberately replace earlier ones: builtin < user < project.
    const candidates = new Map<string, ParsedBuiltin | DiscoveredExtension>();
    for (const builtin of builtins) candidates.set(builtin.id, builtin);
    for (const extension of discovered) candidates.set(extension.id, extension);

    if (filter === null) {
      for (const id of [...records.keys()]) {
        if (!candidates.has(id)) records.delete(id);
      }
    } else {
      for (const id of filter) {
        if (!candidates.has(id)) records.delete(id);
      }
    }

    for (const [id, candidate] of candidates) {
      if (filter !== null && !filter.has(id)) continue;
      let enabled = enabledFor(id);
      const updatedAt = now();
      if (candidate.scope === 'builtin') {
        const health = healthForBuiltin(enabled, candidate);
        records.set(id, {
          trust: 'builtin',
          id,
          scope: 'builtin',
          manifest: candidate.manifest,
          dir: candidate.dir,
          enabled,
          bundleUrl: null,
          bundleHash: null,
          health,
          updatedAt
        });
        continue;
      }

      const trust = await trustOf(id, candidate.scope);
      const put = (record: ExtensionRecord): void => void records.set(id, { ...record, trust });
      enabled = enabledFor(id);

      if (candidate.manifest === null) {
        put({
          id,
          scope: candidate.scope,
          manifest: null,
          dir: candidate.dir,
          enabled,
          bundleUrl: null,
          bundleHash: null,
          health: enabled
            ? { state: 'manifest-error', error: truncateError(candidate.error ?? 'Invalid manifest') }
            : { state: 'disabled' },
          updatedAt
        });
        continue;
      }

      const update = staleForkUpdate(candidate, shippedBuiltins);

      let signature: string;
      try {
        signature = await directorySignature(candidate.dir);
      } catch (error) {
        put(buildErrorRecord(candidate, candidate.manifest, enabledFor(id), errorText(error), updatedAt, update));
        continue;
      }

      let result: Awaited<ReturnType<Compiler>>;
      const cached = buildCache.get(candidate.dir);
      if (cached?.signature === signature) {
        result = cached.result;
      } else {
        try {
          result = await compiler({
            dir: candidate.dir,
            entry: candidate.manifest.entry ?? 'index.ts',
            outDir: options.buildDir
          });
        } catch (error) {
          result = { ok: false, error: truncateError(errorText(error)) };
        }
        buildCache.set(candidate.dir, { signature, result });
      }

      // Compilation can yield while a runtime health report disables this ID.
      enabled = enabledFor(id);
      if (!result.ok) {
        put(buildErrorRecord(candidate, candidate.manifest, enabled, result.error, updatedAt, update));
        continue;
      }

      /* Someone else's code that asks for full access waits for the user to
         trust it: no bundle URL, so the loader never imports it. */
      if (enabled && needsTrust({ trust, manifest: candidate.manifest })) {
        put({
          id,
          scope: candidate.scope,
          manifest: candidate.manifest,
          ...(update === undefined ? {} : { update }),
          dir: candidate.dir,
          enabled,
          bundleUrl: null,
          bundleHash: null,
          health: { state: 'needs-trust' },
          updatedAt
        });
        continue;
      }

      /* Values gate activation: a user extension missing a required value
         (or holding one this Mac cannot decrypt) gets no bundle URL, so the
         loader never imports it. */
      const decls = candidate.manifest.vars ?? [];
      if (candidate.scope === 'user' && decls.length > 0 && enabled) {
        const resolution = await resolveFor(id, decls);
        enabled = enabledFor(id);
        if (enabled && resolution.status === 'needs-setup') {
          put({
            id,
            scope: candidate.scope,
            manifest: candidate.manifest,
            ...(update === undefined ? {} : { update }),
            dir: candidate.dir,
            enabled,
            bundleUrl: null,
            bundleHash: null,
            health: { state: 'needs-setup', missing: missingKeys(resolution) },
            updatedAt
          });
          continue;
        }
      }

      put({
        id,
        scope: candidate.scope,
        manifest: candidate.manifest,
        ...(update === undefined ? {} : { update }),
        dir: candidate.dir,
        enabled,
        bundleUrl: `app://powermove/ext/${id}/bundle.js?v=${result.hash}`,
        bundleHash: result.hash,
        health: enabled ? { state: 'ok' } : { state: 'disabled' },
        updatedAt
      });
    }
  }

  const registry: ExtensionRegistry = {
    buildDir: options.buildDir,
    userDir: options.userDir,

    list() {
      const builtinOrder = new Map(options.builtinIds.map((id, index) => [id, index]));
      return [...records.values()]
        .sort((left, right) => {
          const scopeDifference = scopeRank(left.scope) - scopeRank(right.scope);
          if (scopeDifference !== 0) return scopeDifference;
          if (left.scope === 'builtin' && right.scope === 'builtin') {
            return (builtinOrder.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
              (builtinOrder.get(right.id) ?? Number.MAX_SAFE_INTEGER);
          }
          return left.id.localeCompare(right.id);
        })
        .map(cloneRecord);
    },

    refresh(ids) {
      if (ids !== undefined) ids.forEach(validateId);
      const run = refreshQueue.then(() => performRefresh(ids));
      refreshQueue = run.catch(() => undefined);
      return run;
    },

    async setEnabled(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension enable request.');
      const { id, enabled } = request;
      validateId(id);
      if (typeof enabled !== 'boolean') throw new Error('Invalid extension enabled state.');
      const record = requireRecord(records, id);
      enabledState.set(id, enabled);
      persistEnabledState(options.store, enabledState);
      if (enabled) {
        // Re-evaluate the underlying manifest/build result so enabling a
        // previously disabled broken extension restores its specific error.
        await registry.refresh([id]);
      } else {
        record.enabled = false;
        record.health = { state: 'disabled' };
        record.updatedAt = now();
      }
      emitChanged({ ids: [id], reason: enabled ? 'enable' : 'disable' });
      return registry.list();
    },

    async remove(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension remove request.');
      const { id } = request;
      validateId(id);
      const record = requireRecord(records, id);
      if (record.scope !== 'user') throw new Error('Only user extensions can be removed.');

      // Validate every deletion target before deleting either one, avoiding a
      // partially removed extension if the build target was replaced by a link.
      const sourceDirectory = await validateContainedExistingDirectory(options.userDir, record.dir, true);
      const buildDirectory = await containedChildIfPresent(options.buildDir, id);
      await fs.rm(sourceDirectory, { recursive: true });
      if (buildDirectory !== null) await fs.rm(buildDirectory, { recursive: true });
      buildCache.delete(record.dir);
      records.delete(id);
      enabledState.delete(id);
      persistEnabledState(options.store, enabledState);

      // Removing an override may expose a builtin or project candidate.
      await registry.refresh([id]);
      emitChanged({ ids: [id], reason: 'remove' });
      return registry.list();
    },

    async reload(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension reload request.');
      const { id } = request;
      validateId(id);
      requireRecord(records, id);
      const record = records.get(id);
      if (record) buildCache.delete(record.dir);
      await registry.refresh([id]);
      emitChanged({ ids: [id], reason: 'reload' });
      return registry.list();
    },

    async create(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension create request.');
      const parsed = parseManifest(request.manifest);
      if (!parsed.ok) throw new Error(parsed.error);
      const id = validateId(parsed.manifest.id);
      const files = validateCreateFiles(request.files, parsed.manifest.entry ?? 'index.ts');

      await fs.mkdir(options.userDir, { recursive: true });
      const realUserDir = await fs.realpath(options.userDir);
      const destination = path.join(realUserDir, id);
      await assertMissing(destination, 'Extension already exists.');

      const temporary = path.join(realUserDir, `.${id}.tmp-${randomBytes(8).toString('hex')}`);
      await fs.mkdir(temporary);
      try {
        for (const [relativePath, text] of files) {
          const destinationFile = path.join(temporary, ...relativePath.split('/'));
          await fs.mkdir(path.dirname(destinationFile), { recursive: true });
          await fs.writeFile(destinationFile, text, 'utf8');
        }
        await fs.writeFile(
          path.join(temporary, 'manifest.json'),
          `${JSON.stringify(parsed.manifest, null, 2)}\n`,
          'utf8'
        );
        await fs.rename(temporary, destination);
      } catch (error) {
        await fs.rm(temporary, { recursive: true, force: true }).catch(() => undefined);
        throw error;
      }

      await registry.refresh([id]);
      emitChanged({ ids: [id], reason: 'create' });
      return registry.list();
    },

    async reveal(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension reveal request.');
      const { id } = request;
      validateId(id);
      const record = requireRecord(records, id);
      await validateRecordDirectory(record, options);
      (options.revealPath ?? shell.showItemInFolder)(record.dir);
    },

    async readSource(request) {
      if (!request || typeof request !== 'object') throw new Error('Invalid extension source request.');
      const { id } = request;
      validateId(id);
      const record = requireRecord(records, id);
      const directory = await validateRecordDirectory(record, options);
      return collectTextSource(directory);
    },

    reportHealth(report) {
      if (!report || typeof report !== 'object') throw new Error('Invalid extension health report.');
      const id = validateId(report.id);
      const health = validateReportedHealth(report.health);
      const record = requireRecord(records, id);
      // Nothing of an extension waiting for setup or trust ever ran; a report is stale.
      if (record.health.state === 'needs-setup' || record.health.state === 'needs-trust') return;
      if (record.enabled && health.state === 'runtime-error') {
        enabledState.set(id, false);
        persistEnabledState(options.store, enabledState);
        record.enabled = false;
        record.health = health;
      } else if (record.enabled) {
        record.health = health;
      } // Ignore late health reports from a runtime already disabled.

      record.updatedAt = now();
      /* 'health' refreshes UI lists without triggering a loader reload — a
         reload here would re-report health and loop forever. */
      emitChanged({ ids: [id], reason: 'health' });
    },

    emitChanged
  };

  return registry;
}

function readEnabledState(raw: unknown): Map<string, boolean> {
  const result = new Map<string, boolean>();
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return result;
  for (const [id, enabled] of Object.entries(raw)) {
    if (EXTENSION_ID.test(id) && typeof enabled === 'boolean') result.set(id, enabled);
  }
  return result;
}

function persistEnabledState(store: Store, state: ReadonlyMap<string, boolean>): void {
  store.set(EXTENSIONS_STORE_KEY, Object.fromEntries([...state].sort(([a], [b]) => a.localeCompare(b))));
}

function validateId(id: string): string {
  if (typeof id !== 'string' || !EXTENSION_ID.test(id)) throw new Error('Invalid extension id.');
  return id;
}

function requireRecord(records: ReadonlyMap<string, ExtensionRecord>, id: string): ExtensionRecord {
  const record = records.get(id);
  if (!record) throw new Error(`Unknown extension: ${id}`);
  return record;
}

function truncateError(error: string): string {
  return error.slice(0, MANIFEST_LIMITS.errorChars);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function healthForBuiltin(enabled: boolean, candidate: ParsedBuiltin): ExtensionHealth {
  if (!enabled) return { state: 'disabled' };
  return candidate.manifest
    ? { state: 'ok' }
    : { state: 'manifest-error', error: truncateError(candidate.error ?? 'Invalid manifest') };
}

function buildErrorRecord(
  candidate: DiscoveredExtension,
  manifest: ExtensionManifest,
  enabled: boolean,
  error: string,
  updatedAt: number,
  update?: ExtensionRecord['update']
): ExtensionRecord {
  return {
    id: candidate.id,
    scope: candidate.scope,
    manifest,
    ...(update === undefined ? {} : { update }),
    dir: candidate.dir,
    enabled,
    bundleUrl: null,
    bundleHash: null,
    health: enabled ? { state: 'build-error', error: truncateError(error) } : { state: 'disabled' },
    updatedAt
  };
}

function staleForkUpdate(
  candidate: DiscoveredExtension,
  shippedBuiltins: ReadonlyMap<string, ExtensionManifest>
): ExtensionRecord['update'] {
  if (candidate.scope !== 'user' || candidate.manifest === null) return undefined;
  const forkedFrom = candidate.manifest.forkedFrom;
  if (!forkedFrom) return undefined;

  const separator = forkedFrom.indexOf('@');
  if (separator <= 0 || separator !== forkedFrom.lastIndexOf('@')) return undefined;
  const builtinId = forkedFrom.slice(0, separator);
  const base = forkedFrom.slice(separator + 1);
  if (!EXTENSION_ID.test(builtinId) || !EXTENSION_VERSION.test(base)) return undefined;

  const shipped = shippedBuiltins.get(builtinId);
  if (!shipped || !candidate.manifest.replaces?.includes(builtinId) || shipped.version === base) return undefined;
  return { forkedFrom: builtinId, base, current: shipped.version };
}

async function readBuiltins(resourcesDir: string, ids: readonly string[]): Promise<ParsedBuiltin[]> {
  const result: ParsedBuiltin[] = [];
  for (const rawId of ids) {
    if (!EXTENSION_ID.test(rawId)) continue;
    const dir = path.join(resourcesDir, rawId);
    try {
      const raw: unknown = JSON.parse(await fs.readFile(path.join(dir, 'manifest.json'), 'utf8'));
      const parsed = parseManifest(raw);
      if (!parsed.ok) {
        result.push({ id: rawId, scope: 'builtin', dir, manifest: null, error: parsed.error });
      } else if (parsed.manifest.id !== rawId) {
        result.push({
          id: rawId,
          scope: 'builtin',
          dir,
          manifest: null,
          error: 'Directory name does not match manifest id.'
        });
      } else {
        result.push({ id: rawId, scope: 'builtin', dir, manifest: parsed.manifest });
      }
    } catch (error) {
      result.push({ id: rawId, scope: 'builtin', dir, manifest: null, error: errorText(error) });
    }
  }
  return result;
}

async function directorySignature(root: string): Promise<string> {
  const parts: string[] = [];
  let files = 0;
  async function walk(directory: string, prefix: string): Promise<void> {
    const entries = await fs.readdir(directory, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (entry.name.startsWith('.')) continue;
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const fullPath = path.join(directory, entry.name);
      const metadata = await fs.lstat(fullPath);
      if (metadata.isSymbolicLink()) {
        parts.push(`l:${relative}:${metadata.mtimeMs}`);
      } else if (metadata.isDirectory()) {
        parts.push(`d:${relative}:${metadata.mtimeMs}`);
        await walk(fullPath, relative);
      } else if (metadata.isFile()) {
        files += 1;
        if (files > MANIFEST_LIMITS.sourceFiles) throw new Error('Extension has too many source files.');
        parts.push(`f:${relative}:${metadata.size}:${metadata.mtimeMs}`);
      }
    }
  }
  await walk(root, '');
  return parts.join('\n');
}

function scopeRank(scope: ExtensionRecord['scope']): number {
  return scope === 'builtin' ? 0 : scope === 'user' ? 1 : 2;
}

function cloneRecord(record: ExtensionRecord): ExtensionRecord {
  return structuredClone(record);
}

function validateCreateFiles(files: unknown, entry: string): Array<[string, string]> {
  if (!files || typeof files !== 'object' || Array.isArray(files)) {
    throw new Error('Extension files must be an object.');
  }
  const entries = Object.entries(files);
  // manifest.json is generated below and counts toward discovery's total.
  if (entries.length + 1 > MANIFEST_LIMITS.sourceFiles) throw new Error('Extension has too many source files.');
  let bytes = 0;
  let hasEntry = false;
  for (const [relativePath, value] of entries) {
    validateSourcePath(relativePath);
    if (relativePath === 'manifest.json') throw new Error('manifest.json is generated from the manifest.');
    if (typeof value !== 'string' || value.includes('\0')) throw new Error(`Extension file is not text: ${relativePath}`);
    bytes += Buffer.byteLength(value, 'utf8');
    if (bytes > MANIFEST_LIMITS.sourceBytes) throw new Error('Extension source is too large.');
    if (relativePath === entry) hasEntry = true;
  }
  if (!hasEntry) throw new Error(`Extension entry file is missing: ${entry}`);
  return entries as Array<[string, string]>;
}

function validateSourcePath(relativePath: string): void {
  if (
    relativePath.length === 0 ||
    relativePath.length > 200 ||
    relativePath.includes('\\') ||
    relativePath.includes('\0') ||
    path.isAbsolute(relativePath)
  ) {
    throw new Error(`Invalid extension file path: ${relativePath}`);
  }
  const segments = relativePath.split('/');
  if (segments.some((segment) => segment === '' || segment === '.' || segment === '..')) {
    throw new Error(`Invalid extension file path: ${relativePath}`);
  }
}

async function assertMissing(candidate: string, message: string): Promise<void> {
  try {
    await fs.lstat(candidate);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw error;
  }
  throw new Error(message);
}

function isContained(realRoot: string, realCandidate: string): boolean {
  const relative = path.relative(realRoot, realCandidate);
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

async function validateContainedExistingDirectory(
  root: string,
  candidate: string,
  requireImmediateChild = false
): Promise<string> {
  const rootReal = await fs.realpath(root);
  const candidateMetadata = await fs.lstat(candidate);
  if (candidateMetadata.isSymbolicLink() || !candidateMetadata.isDirectory()) {
    throw new Error('Extension path is not a regular directory.');
  }
  const candidateReal = await fs.realpath(candidate);
  if (!isContained(rootReal, candidateReal)) throw new Error('Extension path escapes its root.');
  if (requireImmediateChild && path.dirname(candidateReal) !== rootReal) {
    throw new Error('Extension path is not an immediate child of its root.');
  }
  return candidateReal;
}

async function containedChildIfPresent(root: string, id: string): Promise<string | null> {
  await fs.mkdir(root, { recursive: true });
  const rootReal = await fs.realpath(root);
  const candidate = path.join(rootReal, validateId(id));
  try {
    const metadata = await fs.lstat(candidate);
    if (metadata.isSymbolicLink()) throw new Error('Extension build path is a symbolic link.');
    const candidateReal = await fs.realpath(candidate);
    if (!isContained(rootReal, candidateReal) || path.dirname(candidateReal) !== rootReal) {
      throw new Error('Extension build path escapes its root.');
    }
    return candidateReal;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null;
    throw error;
  }
}

async function validateRecordDirectory(
  record: ExtensionRecord,
  options: Pick<ExtensionRegistryOptions, 'userDir' | 'resourcesDir' | 'projectDirs'>
): Promise<string> {
  if (record.scope === 'user') return validateContainedExistingDirectory(options.userDir, record.dir, true);
  if (record.scope === 'builtin') return validateContainedExistingDirectory(options.resourcesDir, record.dir, true);
  for (const root of options.projectDirs ?? []) {
    try {
      return await validateContainedExistingDirectory(root, record.dir, true);
    } catch {
      // Try the next configured project root.
    }
  }
  throw new Error('Project extension path escapes its root.');
}

async function collectTextSource(root: string): Promise<ExtensionSourceFile[]> {
  const result: ExtensionSourceFile[] = [];
  let visitedFiles = 0;
  async function walk(directory: string, prefix: string): Promise<void> {
    if (visitedFiles >= MANIFEST_LIMITS.sourceFiles) return;
    const entries = await fs.readdir(directory, { withFileTypes: true });
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (entry.name.startsWith('.')) continue;
      if (visitedFiles >= MANIFEST_LIMITS.sourceFiles) return;
      const relativePath = prefix ? `${prefix}/${entry.name}` : entry.name;
      const fullPath = path.join(directory, entry.name);
      const metadata = await fs.lstat(fullPath);
      if (metadata.isSymbolicLink()) continue;
      if (metadata.isDirectory()) {
        await walk(fullPath, relativePath);
      } else if (metadata.isFile()) {
        visitedFiles += 1;
        if (metadata.size > SOURCE_FILE_BYTES) continue;
        const contents = await fs.readFile(fullPath);
        if (contents.includes(0)) continue;
        result.push({ path: relativePath, text: contents.toString('utf8') });
      }
    }
  }
  await walk(root, '');
  return result;
}

function validateReportedHealth(health: unknown): ExtensionHealthReport['health'] {
  if (!health || typeof health !== 'object' || Array.isArray(health)) {
    throw new Error('Invalid extension health.');
  }
  const value = health as Record<string, unknown>;
  if (value.state === 'ok' && Object.keys(value).length === 1) return { state: 'ok' };
  if (
    (value.state === 'activation-error' || value.state === 'runtime-error') &&
    typeof value.error === 'string' &&
    value.error.length > 0 &&
    value.error.length <= MANIFEST_LIMITS.errorChars &&
    Object.keys(value).length === 2
  ) {
    return { state: value.state, error: value.error };
  }
  throw new Error('Invalid extension health.');
}
