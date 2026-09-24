/*
 * Installing, updating and removing store extensions (store plan §2.2, §2.6;
 * P0 decisions Q7, Q8).
 *
 * Install writes nothing into the tree: the release tar lands in
 * `extensions/<id>/` byte-for-byte, after its SHA-256 and git tree hash match
 * what the registry says the release is. Where the folder came from lives in
 * the provenance file.
 *
 * Every write is staged under `extensions/.staging/` and moved into place
 * with one `rename`, so a failed install or update leaves the previous folder
 * untouched. Staging lives inside `extensions/` because `rename` cannot cross
 * volumes; discovery and the watcher ignore dot-entries, so nothing staged is
 * ever loaded.
 *
 *   .staging/<id>          the tree being installed or swapped in
 *   .trash/<id>-<ts>       the old folder during an update's swap
 *   .updates/<id>          a newer release beside a folder the user changed
 *                          (Store 1.0 has no merge; the agent merges it)
 */
import { createHash } from 'node:crypto';
import * as fs from 'node:fs/promises';
import path from 'node:path';

import { REGISTRY_LIMITS } from '@powermove/registry/limits';
import { snapshotDir } from '@powermove/registry/node';
import { isExcludedPath, snapshot, type SnapshotInput } from '@powermove/registry/snapshot';
import { readTarGz } from '@powermove/registry/tar';
import { ApiError, type MeDto } from '@powermove/registry/wire';

import { EXTENSION_ID, parseManifest, type ExtensionManifest, type ExtensionRecord, type ExtensionsChangedEvent } from '../../shared/extensions';
import type { StoreInstallResult, StoreLocalErrorCode, StoreUpdateResult, StoreUpdates, StoreUpdateState } from '../../shared/store-ipc';
import type { ProvenanceOrigin, ProvenanceRecord, ProvenanceStore } from './provenance';
import type { ReleaseByIdResult, StoreClient, VersionsItem } from './store-client';
import { RESERVED_STORE_IDS } from './reserved-slugs';
import { STORE_MARKER } from './trust';

/** A failure decided on this Mac; `detail` is written for the person installing. */
export class StoreLocalError extends Error {
  constructor(public readonly code: StoreLocalErrorCode, public readonly detail: string) {
    super(detail);
    this.name = 'StoreLocalError';
  }
}

export interface StoreInstallerRegistry {
  readonly userDir: string;
  list(): ExtensionRecord[];
  refresh(ids?: string[]): Promise<void>;
  emitChanged(event: ExtensionsChangedEvent): void;
}

export interface StoreInstallerOptions {
  registry: StoreInstallerRegistry;
  provenance: ProvenanceStore;
  store: StoreClient;
  /** The signed-in account for the current registry (cached), or null. */
  me(): MeDto | null;
  signedIn(): boolean;
  /** main → renderers: `store:updates-changed`. */
  notifyUpdates?(updates: StoreUpdates): void;
  /** The built-in ids, from the built-in registry list. Defaults to the registry's built-in records. */
  builtinIds?(): readonly string[];
  /** Whether the extension still has a values file on this Mac. */
  hasValues?(localId: string): Promise<boolean>;
  log?(message: string, error?: unknown): void;
  now?(): number;
  /** Test seam: the rename used for moving folders into place. */
  rename?(from: string, to: string): Promise<void>;
}

export interface StoreInstaller {
  installRelease(request: { repoId: string; releaseId: string }): Promise<StoreInstallResult>;
  updateRelease(localId: string): Promise<StoreUpdateResult>;
  /**
   * `remove` deletes the folder through the `ext:remove` path (which asks
   * about values); it resolves false when nothing was removed.
   */
  uninstall(localId: string, remove: (localId: string) => Promise<boolean>): Promise<{ removed: boolean }>;
  checkUpdates(): Promise<StoreUpdates>;
  /** The last update-check results, per local id. */
  updates(): StoreUpdates;
  isModified(localId: string): Promise<boolean>;
  /** The folder's snapshot tree, or null when it can't be snapshotted. */
  localTree(localId: string): Promise<string | null>;
  /** Boot check after `delayMs`, then every `everyMs`. Returns a stop function. */
  startUpdateChecks(options?: { delayMs?: number; everyMs?: number }): () => void;
}

export const UPDATE_CHECK_DELAY_MS = 5_000;
export const UPDATE_CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const VERSIONS_BATCH = 200;

interface VerifiedRelease {
  release: ReleaseByIdResult;
  files: SnapshotInput[];
  manifest: ExtensionManifest;
  ownerPublisherId: string;
}

const sha256 = (bytes: Uint8Array): string => createHash('sha256').update(bytes).digest('hex');

function integrity(detail = 'The download doesn’t match the release on the store, so it wasn’t installed. Try again later.'): StoreLocalError {
  return new StoreLocalError('integrity', detail);
}

async function exists(target: string): Promise<boolean> {
  try {
    await fs.lstat(target);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}

/** Write `files` under `dir`, refusing any path that would land outside it. */
async function writeTree(dir: string, files: SnapshotInput[]): Promise<void> {
  await fs.rm(dir, { recursive: true, force: true });
  await fs.mkdir(dir, { recursive: true });
  const root = path.resolve(dir);
  for (const file of files) {
    const target = path.resolve(root, ...file.path.split('/'));
    if (!target.startsWith(`${root}${path.sep}`)) throw integrity();
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, file.bytes, { flag: 'wx', mode: 0o644 });
  }
}

function coordinateOf(release: ReleaseByIdResult): string {
  return `${release.handle}/${release.slug}`;
}

function originOf(verified: VerifiedRelease): ProvenanceOrigin {
  const { release } = verified;
  return {
    repoId: release.repoId,
    releaseId: release.id,
    coordinate: coordinateOf(release),
    version: release.version,
    treeSha: release.treeSha,
    commitSha: release.commitSha,
    ownerPublisherId: verified.ownerPublisherId
  };
}

function stateFor(item: VersionsItem): StoreUpdateState {
  return {
    state: item.state,
    latest: item.latest ? { releaseId: item.latest.releaseId, version: item.latest.version } : null,
    currentYanked: item.current.yanked,
    ownerPublisherId: item.ownerPublisherId
  };
}

export function createStoreInstaller(options: StoreInstallerOptions): StoreInstaller {
  const { registry, provenance, store } = options;
  const userDir = registry.userDir;
  const stagingRoot = path.join(userDir, '.staging');
  const trashRoot = path.join(userDir, '.trash');
  const updatesRoot = path.join(userDir, '.updates');
  const rename = options.rename ?? ((from: string, to: string) => fs.rename(from, to));
  const now = options.now ?? Date.now;
  const log = options.log ?? ((message: string, error?: unknown) => console.warn(`[store] ${message}`, error instanceof Error ? error.message : ''));
  let updates: StoreUpdates = {};

  /* One install, update or uninstall at a time: they share staging and the
     provenance file, and two clicks must not race a folder swap. */
  let queue: Promise<unknown> = Promise.resolve();
  const serial = <T>(task: () => Promise<T>): Promise<T> => {
    const run = queue.then(task);
    queue = run.catch(() => undefined);
    return run;
  };

  const folderFor = (localId: string): string => {
    if (!EXTENSION_ID.test(localId)) throw new StoreLocalError('not_installed', 'That extension isn’t on this Mac.');
    return path.join(userDir, localId);
  };

  const builtinIds = (): Set<string> => new Set(options.builtinIds
    ? options.builtinIds()
    : registry.list().filter((record) => record.scope === 'builtin').map((record) => record.id));

  function publishUpdates(next: StoreUpdates): void {
    updates = next;
    options.notifyUpdates?.(structuredClone(updates));
  }

  async function rememberInstall(repoId: string, releaseId: string): Promise<void> {
    if (!options.signedIn() || !options.me()?.settings.rememberInstalls) return;
    try {
      await store.addInstall(repoId, releaseId);
    } catch (error) {
      log('could not remember the install on this account', error);
    }
  }

  async function forgetInstall(repoId: string): Promise<void> {
    if (!options.signedIn()) return;
    try {
      await store.deleteInstall(repoId);
    } catch (error) {
      log('could not remove the install from this account', error);
    }
  }

  /** Download one release and prove it is the release the registry describes. */
  async function fetchVerified(repoId: string, releaseId: string): Promise<VerifiedRelease> {
    const release = await store.release(releaseId);
    if (release.id !== releaseId || release.repoId !== repoId) throw integrity();
    const bytes = await store.tar(releaseId);
    if (sha256(bytes) !== release.tarSha256) throw integrity();
    let files: SnapshotInput[];
    try {
      files = await readTarGz(bytes, REGISTRY_LIMITS);
    } catch {
      throw integrity();
    }
    /* The tree hash covers every file but dot-entries; a tar carrying one
       would write bytes nothing verified. The registry never builds one. */
    if (files.some((file) => isExcludedPath(file.path))) throw integrity();
    let treeSha: string;
    try {
      treeSha = (await snapshot(files)).treeSha;
    } catch {
      throw integrity();
    }
    if (treeSha !== release.treeSha) throw integrity();

    const manifestFile = files.find((file) => file.path === 'manifest.json');
    if (!manifestFile) throw integrity('This release has no manifest, so it can’t be installed.');
    let raw: unknown;
    try {
      raw = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(manifestFile.bytes));
    } catch {
      throw integrity('This release’s manifest can’t be read, so it wasn’t installed.');
    }
    const parsed = parseManifest(raw);
    if (!parsed.ok) throw integrity(`This release’s manifest is invalid: ${parsed.error}`);
    const manifest = parsed.manifest;
    /* The local id names the folder: it must be a plain id and agree with
       what the registry recorded at publish. */
    if (!EXTENSION_ID.test(manifest.id) || manifest.id !== release.manifest.id) throw integrity();

    const [owner] = (await store.versions([{ repoId, releaseId }])).items;
    if (!owner || owner.repoId !== repoId || !owner.ownerPublisherId) throw new ApiError({ error: 'not_found' });
    return { release, files, manifest, ownerPublisherId: owner.ownerPublisherId };
  }

  /** Write a verified tree to `.staging/<name>`, removing it again on failure. */
  async function stage(name: string, files: SnapshotInput[]): Promise<string> {
    await fs.mkdir(stagingRoot, { recursive: true });
    const staging = path.join(stagingRoot, name);
    try {
      await writeTree(staging, files);
    } catch (error) {
      await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
      if (error instanceof StoreLocalError) throw error;
      throw new StoreLocalError('local', 'Powermove couldn’t write the extension to disk. Check that there’s free space and try again.');
    }
    return staging;
  }

  async function refreshRecord(localId: string, reason: ExtensionsChangedEvent['reason']): Promise<ExtensionRecord | undefined> {
    await registry.refresh([localId]);
    registry.emitChanged({ ids: [localId], reason });
    return registry.list().find((record) => record.id === localId);
  }

  async function localTree(localId: string): Promise<string | null> {
    try {
      return (await snapshotDir(folderFor(localId))).treeSha;
    } catch {
      return null;
    }
  }

  async function isModified(localId: string): Promise<boolean> {
    const record = await provenance.get(localId);
    const origin = record?.origin;
    if (!origin) return false;
    // Unreadable, emptied or no longer snapshot-able: it is not what was installed.
    return (await localTree(localId)) !== origin.treeSha;
  }

  function installRelease(request: { repoId: string; releaseId: string }): Promise<StoreInstallResult> {
    return serial(async () => {
      const verified = await fetchVerified(request.repoId, request.releaseId);
      const { manifest } = verified;
      const id = manifest.id;
      if (RESERVED_STORE_IDS.has(id) || builtinIds().has(id)) {
        throw new ApiError({ error: 'id_collision', detail: `“${id}” is a reserved extension id.` });
      }
      const folder = folderFor(id);
      if (await exists(folder)) {
        const existing = registry.list().find((record) => record.id === id && record.scope === 'user');
        const name = existing?.manifest?.name ?? id;
        throw new ApiError({
          error: 'id_collision',
          detail: `“${name}” is already on this Mac with the id “${id}”. Remove it from your Library to install this one.`
        });
      }

      const staging = await stage(id, verified.files);
      await fs.writeFile(path.join(staging, STORE_MARKER), JSON.stringify({ repoId: request.repoId, releaseId: request.releaseId }));
      /* Provenance lands before the folder does: a watcher refresh between the
         two must already see someone else's code, never a "local" folder. */
      const origin = originOf(verified);
      const previous = await provenance.get(id);
      await provenance.update(id, () => ({
        localId: id,
        envKey: origin.repoId,
        origin,
        upstream: { releaseId: origin.releaseId, treeSha: origin.treeSha }
      }));
      try {
        await rename(staging, folder);
      } catch {
        await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
        await provenance.update(id, () => previous).catch((error: unknown) => log(`could not restore provenance for ${id}`, error));
        throw new StoreLocalError('local', 'Powermove couldn’t move the extension into place. Try again.');
      }
      await fs.rm(path.join(updatesRoot, id), { recursive: true, force: true }).catch(() => undefined);

      const record = await refreshRecord(id, 'create');
      await rememberInstall(origin.repoId, origin.releaseId);

      const next = { ...updates };
      delete next[id];
      publishUpdates(next);

      const replaced = (manifest.replaces ?? []).map((other) =>
        registry.list().find((candidate) => candidate.id === other)?.manifest?.name ?? other);
      return {
        localId: id,
        needsSetup: record?.health.state === 'needs-setup',
        needsTrust: record?.health.state === 'needs-trust',
        ...(replaced.length ? { warning: `While it’s on, ${manifest.name} replaces ${replaced.join(', ')}.` } : {})
      };
    });
  }

  async function latestFor(record: ProvenanceRecord & { origin: ProvenanceOrigin }): Promise<{ releaseId: string; version: string }> {
    const [item] = (await store.versions([{ repoId: record.origin.repoId, releaseId: record.origin.releaseId }])).items;
    if (item) publishUpdates({ ...updates, [record.localId]: stateFor(item) });
    if (!item || item.state !== 'ok' || !item.latest || item.latest.releaseId === record.origin.releaseId) {
      throw new StoreLocalError('no_update', 'There’s no newer version to update to.');
    }
    return item.latest;
  }

  function updateRelease(localId: string): Promise<StoreUpdateResult> {
    return serial(async () => {
      const folder = folderFor(localId);
      const record = await provenance.get(localId);
      const origin = record?.origin;
      if (!record || !origin || !(await exists(folder))) {
        throw new StoreLocalError('not_installed', 'That extension isn’t installed from the store on this Mac.');
      }
      const latest = await latestFor({ ...record, origin });
      const verified = await fetchVerified(origin.repoId, latest.releaseId);
      if (verified.manifest.id !== localId) {
        throw integrity('The new version changes the extension’s id, so it can’t replace this one.');
      }
      const modified = await isModified(localId);
      const version = verified.release.version;

      if (modified) {
        /* Store 1.0: no merge. The new tree goes beside the folder for the
           agent; the folder itself is not touched. */
        const staging = await stage(`${localId}-update`, verified.files);
        const target = path.join(updatesRoot, localId);
        try {
          await fs.mkdir(updatesRoot, { recursive: true });
          await fs.rm(target, { recursive: true, force: true });
          await rename(staging, target);
        } catch {
          await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
          throw new StoreLocalError('local', 'Powermove couldn’t save the new version beside your folder. Try again.');
        }
        await provenance.update(localId, (current) => current && ({
          ...current,
          pendingUpdate: { releaseId: verified.release.id, version, path: target }
        }));
        registry.emitChanged({ ids: [localId], reason: 'health' });
        publishUpdates({ ...updates });
        return { kind: 'staged-for-merge', localId, version, path: target };
      }

      const staging = await stage(localId, verified.files);
      await fs.writeFile(path.join(staging, STORE_MARKER), JSON.stringify({ repoId: origin.repoId, releaseId: verified.release.id }));
      await fs.mkdir(trashRoot, { recursive: true });
      const trash = path.join(trashRoot, `${localId}-${now()}`);
      try {
        await rename(folder, trash);
      } catch {
        await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
        throw new StoreLocalError('local', 'Powermove couldn’t replace the extension’s folder. Close anything using it and try again.');
      }
      try {
        await rename(staging, folder);
      } catch {
        // Put the old folder back exactly where it was.
        await rename(trash, folder).catch((error: unknown) => log(`could not restore ${localId} from ${trash}`, error));
        await fs.rm(staging, { recursive: true, force: true }).catch(() => undefined);
        throw new StoreLocalError('local', 'Powermove couldn’t move the new version into place, so the current one was kept. Try again.');
      }
      await fs.rm(trash, { recursive: true, force: true }).catch((error: unknown) => log(`could not delete ${trash}`, error));

      const nextOrigin = originOf(verified);
      await provenance.update(localId, (current) => {
        const base = current ?? { localId, envKey: nextOrigin.repoId };
        const { pendingUpdate: _dropped, trusted, ...rest } = base;
        /* Trust covers what the user agreed to: an update that declares more
           asks again ("Needs full access"). */
        const keep = trusted && (verified.manifest.permissions ?? []).every((permission) => trusted.permissions.includes(permission));
        return { ...rest, ...(keep ? { trusted } : {}), origin: nextOrigin, upstream: { releaseId: nextOrigin.releaseId, treeSha: nextOrigin.treeSha } };
      });
      await fs.rm(path.join(updatesRoot, localId), { recursive: true, force: true }).catch(() => undefined);
      await refreshRecord(localId, 'reload');
      await rememberInstall(nextOrigin.repoId, nextOrigin.releaseId);
      const next = { ...updates };
      const known = next[localId];
      if (known) next[localId] = { ...known, currentYanked: false };
      publishUpdates(next);
      return { kind: 'updated', localId, version };
    });
  }

  function uninstall(localId: string, remove: (localId: string) => Promise<boolean>): Promise<{ removed: boolean }> {
    return serial(async () => {
      const folder = folderFor(localId);
      const record = await provenance.get(localId);
      let removed = false;
      if (await exists(folder)) {
        removed = await remove(localId);
        if (!removed) return { removed: false };
      }
      /* Store installs name their values by repo, so a reinstall finds kept
         values without the record. A folder made here keeps its key while
         values it names are kept, so they are not orphaned. */
      const keepKey = !record?.origin && (await options.hasValues?.(localId).catch(() => false)) === true;
      await provenance.update(localId, (current) => (current && keepKey ? { localId, envKey: current.envKey } : null));
      await fs.rm(path.join(updatesRoot, localId), { recursive: true, force: true }).catch(() => undefined);
      if (record?.origin) await forgetInstall(record.origin.repoId);
      if (localId in updates) {
        const next = { ...updates };
        delete next[localId];
        publishUpdates(next);
      }
      return { removed: removed || record !== null };
    });
  }

  async function checkUpdates(): Promise<StoreUpdates> {
    const records = Object.values(await provenance.read()).filter(
      (record): record is ProvenanceRecord & { origin: ProvenanceOrigin } => !!record.origin
    );
    const next: StoreUpdates = {};
    for (let start = 0; start < records.length; start += VERSIONS_BATCH) {
      const batch = records.slice(start, start + VERSIONS_BATCH);
      const { items } = await store.versions(batch.map((record) => ({ repoId: record.origin.repoId, releaseId: record.origin.releaseId })));
      /* Answers come back in request order; match by repo as well so a
         reordered answer can never pair one extension with another's state. */
      batch.forEach((record, index) => {
        const item = items[index]?.repoId === record.origin.repoId
          ? items[index]
          : items.find((candidate) => candidate.repoId === record.origin.repoId);
        if (item) next[record.localId] = stateFor(item);
      });
    }
    publishUpdates(next);
    return structuredClone(next);
  }

  return {
    installRelease,
    updateRelease,
    uninstall,
    checkUpdates,
    updates: () => structuredClone(updates),
    isModified,
    localTree,
    startUpdateChecks({ delayMs = UPDATE_CHECK_DELAY_MS, everyMs = UPDATE_CHECK_EVERY_MS } = {}) {
      const run = (): void => {
        void checkUpdates().catch((error: unknown) => log('update check failed', error));
      };
      let interval: ReturnType<typeof setInterval> | undefined;
      const boot = setTimeout(() => {
        run();
        interval = setInterval(run, everyMs);
      }, delayMs);
      return () => {
        clearTimeout(boot);
        if (interval) clearInterval(interval);
      };
    }
  };
}
