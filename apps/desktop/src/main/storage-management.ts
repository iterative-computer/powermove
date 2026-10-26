import { lstat, readdir, rm } from 'node:fs/promises';
import path from 'node:path';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { BACKUP_BUDGETS, DEFAULT_STORAGE_SETTINGS, STORAGE_IPC,
  type StorageCategory, type StorageCleanupResult, type StorageSettings, type StorageStatus } from '../shared/storage-management';
import { planBackupRetention } from './backup-retention';

const DAY = 24 * 60 * 60 * 1000;
export const CACHE_BUDGET_BYTES = 256 * 1024 ** 2;
type Tree = { bytes: number; touched: number; ino: number; ctime: number };
type Snapshot = Tree & { file: string };

/** Symlinks include shared AI resources outside this profile. Never traverse them. */
export async function measureStorageTree(file: string): Promise<Tree> {
  try {
    const info = await lstat(file);
    if (info.isSymbolicLink()) return { bytes: 0, touched: 0, ino: info.ino, ctime: info.ctimeMs };
    const result = { bytes: info.isFile() ? info.size : 0, touched: info.mtimeMs, ino: info.ino, ctime: info.ctimeMs };
    if (info.isDirectory()) {
      for (const name of await readdir(file)) {
        const child = await measureStorageTree(path.join(file, name));
        result.bytes += child.bytes;
        result.touched = Math.max(result.touched, child.touched);
      }
    }
    return result;
  } catch (error: any) {
    if (error.code === 'ENOENT') return { bytes: 0, touched: 0, ino: 0, ctime: 0 };
    throw error;
  }
}

/** Old whole-profile snapshots are separate from current projects and their
 * save generations. Always retain the two latest snapshots in each family. */
export async function planSnapshotRetention(root: string, manual: boolean, now = Date.now()): Promise<Snapshot[]> {
  const groups: string[][] = [[], []];
  try {
    if (!(await lstat(root)).isDirectory()) return [];
    const entries = await readdir(root, { withFileTypes: true });
    groups[0] = entries.filter(entry => entry.isDirectory() && /^store-backup-[A-Za-z0-9_-]+$/.test(entry.name))
      .map(entry => path.join(root, entry.name));
    const restarts = path.join(root, 'restart-backups');
    try {
      if ((await lstat(restarts)).isDirectory()) groups[1] = (await readdir(restarts, { withFileTypes: true }))
        .filter(entry => entry.isDirectory()).map(entry => path.join(restarts, entry.name));
    } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  } catch (error: any) { if (error.code === 'ENOENT') return []; throw error; }
  const remove: Snapshot[] = [];
  for (const group of groups) {
    const snapshots: Snapshot[] = [];
    for (const file of group) {
      if ((await lstat(file)).isDirectory()) snapshots.push({ file, ...await measureStorageTree(file) });
    }
    snapshots.sort((a, b) => b.touched - a.touched || b.file.localeCompare(a.file));
    remove.push(...snapshots.slice(2).filter(item => item.touched < now - (manual ? 7 : 30) * DAY));
  }
  return remove;
}

export async function applySnapshotRetention(root: string, snapshots: Snapshot[]): Promise<number> {
  let freed = 0;
  if (!(await lstat(root)).isDirectory()) return freed;
  for (const item of snapshots) {
    const relative = path.relative(root, item.file).split(path.sep);
    if (!((relative.length === 1 && /^store-backup-[A-Za-z0-9_-]+$/.test(relative[0]!))
      || (relative.length === 2 && relative[0] === 'restart-backups' && relative[1] !== '..'))) continue;
    try {
      if (!(await lstat(path.dirname(item.file))).isDirectory() || !(await lstat(item.file)).isDirectory()) continue;
      const current = await measureStorageTree(item.file);
      if (current.ino !== item.ino || current.ctime !== item.ctime || current.touched !== item.touched || current.bytes !== item.bytes) continue;
      await rm(item.file, { recursive: true });
      freed += current.bytes;
    } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
  }
  return freed;
}

export function storageSettings(raw: unknown): StorageSettings {
  const value = raw as Partial<StorageSettings> | null;
  return { automatic: typeof value?.automatic === 'boolean' ? value.automatic : DEFAULT_STORAGE_SETTINGS.automatic,
    backupBudgetGiB: BACKUP_BUDGETS.includes(value?.backupBudgetGiB as any) ? value!.backupBudgetGiB! : DEFAULT_STORAGE_SETTINGS.backupBudgetGiB };
}

const categoryFor = (name: string): [string, string] => {
  if (['backups', 'Backups', 'Recovery', 'restart-backups'].includes(name) || name.startsWith('store-backup-')) return ['backups', 'Backups'];
  if (['Media', 'IndexedDB', 'Media Store', 'Remote Projects', 'Remote Uploads'].includes(name)) return ['media', 'Imported media'];
  if (name === 'Projects') return ['projects', 'Saved projects and history'];
  if (['AI Workspaces', 'Agent Workspaces', 'Agent Change History', 'Agent State', 'Agent Tasks'].includes(name)) return ['agent', 'AI workspaces and history'];
  if (['codex-runtime', 'claude-runtime', 'runtimes', 'models'].includes(name)) return ['tools', 'AI tools and conversations'];
  if (['Cache', 'Code Cache', 'GPUCache', 'DawnGraphiteCache', 'DawnWebGPUCache', 'Attachment Cache', 'Media Tool Cache'].includes(name)) return ['cache', 'Temporary caches'];
  return ['app', 'Projects, settings and other app data'];
};

export interface StorageManagementDeps {
  root: string;
  settings(): unknown;
  saveSettings(value: StorageSettings): Promise<void>;
  cleanupBackups(budget: number): Promise<number>;
  clearCaches(): Promise<void>;
  cacheBytes(): Promise<number>;
  reveal(): Promise<void>;
  isTrustedSender(event: IpcMainInvokeEvent): boolean;
  cleanupWorkspaces?(manual: boolean): Promise<number>;
  cleanupMedia?(): Promise<number>;
  workspaceCacheBytes?(): Promise<number>;
  unusedMediaBytes?(): Promise<number>;
}

export class StorageManagement {
  private queue: Promise<unknown> = Promise.resolve();
  constructor(private readonly deps: StorageManagementDeps) {}
  private serial<T>(action: () => Promise<T>): Promise<T> {
    const next = this.queue.then(action);
    this.queue = next.then(() => undefined, () => undefined);
    return next;
  }
  /** First-launch upgrades share the maintenance queue. */
  exclusive<T>(action: () => Promise<T>): Promise<T> { return this.serial(action); }
  async status(): Promise<StorageStatus> {
    const settings = storageSettings(this.deps.settings());
    const categories = new Map<string, StorageCategory>();
    if (!(await lstat(this.deps.root)).isDirectory()) throw new Error('The storage folder is unavailable.');
    for (const entry of await readdir(this.deps.root)) {
      const [id, label] = categoryFor(entry);
      const category = categories.get(id) ?? { id, label, bytes: 0, reclaimable: 0 };
      const bytes = (await measureStorageTree(path.join(this.deps.root, entry))).bytes;
      category.bytes += bytes;
      categories.set(id, category);
    }
    const backups = await planBackupRetention(path.join(this.deps.root, 'backups'), settings.backupBudgetGiB * 1024 ** 3);
    const snapshots = await planSnapshotRetention(this.deps.root, true);
    const backupCategory = categories.get('backups');
    if (backupCategory) backupCategory.reclaimable = backups.reclaimableBytes + snapshots.reduce((sum, item) => sum + item.bytes, 0);
    const agentCategory = categories.get('agent');
    if (agentCategory) agentCategory.reclaimable = await this.deps.workspaceCacheBytes?.() ?? 0;
    const mediaCategory = categories.get('media');
    if (mediaCategory) mediaCategory.reclaimable = await this.deps.unusedMediaBytes?.() ?? 0;
    const cacheCategory = categories.get('cache');
    if (cacheCategory) {
      // Chromium's APIs clear HTTP and compiled-code caches, never browser storage.
      const code = (await measureStorageTree(path.join(this.deps.root, 'Code Cache'))).bytes;
      cacheCategory.reclaimable = Math.min(cacheCategory.bytes, await this.deps.cacheBytes() + code);
    }
    const values = [...categories.values()].sort((a, b) => b.bytes - a.bytes);
    return { settings, categories: values, totalBytes: values.reduce((sum, item) => sum + item.bytes, 0),
      reclaimableBytes: values.reduce((sum, item) => sum + item.reclaimable, 0),
      backupBudgetExceeded: backups.retainedBytes > settings.backupBudgetGiB * 1024 ** 3 };
  }
  configure(raw: unknown): Promise<StorageStatus> {
    return this.serial(async () => {
      const value = raw as StorageSettings | null;
      if (typeof value?.automatic !== 'boolean' || !BACKUP_BUDGETS.includes(value?.backupBudgetGiB as any)) throw new Error('Invalid storage settings.');
      await this.deps.saveSettings({ automatic: value.automatic, backupBudgetGiB: value.backupBudgetGiB });
      return this.status();
    });
  }
  cleanup(): Promise<StorageCleanupResult> {
    return this.serial(async () => {
      const before = await this.status();
      const warnings: string[] = [];
      // A busy save aborts cleanup before any recovery data is touched.
      await this.deps.cleanupBackups(before.settings.backupBudgetGiB * 1024 ** 3);
      const recovery = this.deps.root;
      try { await applySnapshotRetention(recovery, await planSnapshotRetention(recovery, true)); }
      catch { warnings.push('Some old recovery snapshots could not be removed.'); }
      try { await this.deps.clearCaches(); }
      catch { warnings.push('Some temporary caches could not be cleared.'); }
      try { await this.deps.cleanupWorkspaces?.(true); await this.deps.cleanupMedia?.(); }
      catch { warnings.push('Some temporary AI or media files could not be removed.'); }
      const status = await this.status();
      return { freedBytes: Math.max(0, before.totalBytes - status.totalBytes), warnings, status };
    });
  }
  /** Startup and daily maintenance use a longer grace period than Clean Up. */
  maintain(): Promise<void> {
    return this.serial(async () => {
      const settings = storageSettings(this.deps.settings());
      if (!settings.automatic) return;
      await this.deps.cleanupBackups(settings.backupBudgetGiB * 1024 ** 3);
      const recovery = this.deps.root;
      await applySnapshotRetention(recovery, await planSnapshotRetention(recovery, false));
      await this.deps.cleanupWorkspaces?.(false);
      await this.deps.cleanupMedia?.();
      const codeCache = await measureStorageTree(path.join(this.deps.root, 'Code Cache'));
      if (await this.deps.cacheBytes() + codeCache.bytes > CACHE_BUDGET_BYTES) await this.deps.clearCaches();
    });
  }
}

export function installStorageManagement(ipc: Pick<IpcMain, 'handle'>, deps: StorageManagementDeps): { service: StorageManagement; dispose(): void } {
  const service = new StorageManagement(deps);
  const guard = (event: IpcMainInvokeEvent) => { if (!deps.isTrustedSender(event)) throw new Error('Unauthorized storage request'); };
  ipc.handle(STORAGE_IPC.status, (event) => { guard(event); return service.status(); });
  ipc.handle(STORAGE_IPC.configure, (event, raw: unknown) => { guard(event); return service.configure(raw); });
  ipc.handle(STORAGE_IPC.cleanup, (event) => { guard(event); return service.cleanup(); });
  ipc.handle(STORAGE_IPC.reveal, (event) => { guard(event); return deps.reveal(); });
  const maintain = () => { void service.maintain().catch(() => console.warn('[storage] Maintenance will retry later.')); };
  const startup = setTimeout(maintain, 60_000);
  const daily = setInterval(maintain, DAY);
  startup.unref(); daily.unref();
  return { service, dispose: () => { clearTimeout(startup); clearInterval(daily); } };
}
