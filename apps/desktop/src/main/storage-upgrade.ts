import { randomUUID } from 'node:crypto';
import { constants } from 'node:fs';
import { open } from 'node:fs/promises';
import path from 'node:path';
import type { IpcMain, IpcMainInvokeEvent, WebContents } from 'electron';
import { STORAGE_IPC, type StorageUpgradeProgress, type StorageUpgradeRequest, type StorageUpgradeResult, type StorageUpgradeStage } from '../shared/storage-management';
import { atomicWrite } from './project-files';
import { watchRendererDocument } from './renderer-recovery';

export const STORAGE_UPGRADE_FILE = 'storage-upgrade-v1.json';
const STAGES: StorageUpgradeStage[] = ['recovery', 'browser', 'sources', 'cleanup'];
type Checkpoint = { steps: StorageUpgradeStage[]; beforeBytes: number; freedBytes?: number; notified?: boolean };
type Journal = { version: 1; origins: { [origin: string]: Checkpoint } };
export interface StorageUpgradeDeps {
  root: string;
  measure(): Promise<number>;
  exclusive<T>(action: () => Promise<T>): Promise<T>;
  recovery(): Promise<void>;
  sources(): Promise<void>;
  cleanup(): Promise<void>;
}

/** Completion belongs to this migration, not the app's release number. Browser
 * storage belongs to an origin; packaged and development profiles can share
 * files but must each finish their own IndexedDB pass. */
export class StorageUpgrade {
  private journal: Journal = { version: 1, origins: {} };
  private ready: Promise<void>;
  private claim?: { owner: object; origin: string; token: string };
  private running = false;
  private loadError?: unknown;
  constructor(private readonly deps: StorageUpgradeDeps) { this.ready = this.load().catch(error => { this.loadError = error; }); }
  private async load(): Promise<void> {
    let handle;
    try {
      handle = await open(path.join(this.deps.root, STORAGE_UPGRADE_FILE), constants.O_RDONLY | constants.O_NOFOLLOW);
      const value = JSON.parse(await handle.readFile('utf8')) as Journal;
      if (value.version !== 1 || !value.origins || typeof value.origins !== 'object' || Array.isArray(value.origins)) throw new Error('Invalid storage upgrade checkpoint.');
      for (const item of Object.values(value.origins)) {
        if (!item || !Array.isArray(item.steps) || item.steps.some((step, index) => step !== STAGES[index])
          || !Number.isFinite(item.beforeBytes) || item.beforeBytes < 0
          || item.freedBytes !== undefined && (!Number.isFinite(item.freedBytes) || item.freedBytes < 0)
          || item.notified !== undefined && typeof item.notified !== 'boolean') throw new Error('Invalid storage upgrade checkpoint.');
      }
      this.journal = value;
    } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
    finally { await handle?.close(); }
  }
  private persist(journal: Journal): Promise<void> {
    return atomicWrite(path.join(this.deps.root, STORAGE_UPGRADE_FILE), Buffer.from(JSON.stringify(journal)));
  }
  private async save(origin: string, checkpoint: Checkpoint): Promise<void> {
    const next: Journal = { version: 1, origins: { ...this.journal.origins, [origin]: checkpoint } };
    await this.persist(next);
    this.journal = next;
  }
  release(owner: object): void { if (this.claim?.owner === owner) this.claim = undefined; }
  async request(origin: string, owner: object, input: StorageUpgradeRequest,
    progress: (value: StorageUpgradeProgress) => void = () => {}): Promise<StorageUpgradeResult> {
    if (!input || !['start', 'browserComplete', 'notified'].includes(input.action)) throw new Error('Invalid storage upgrade request.');
    if (this.running || this.claim && (this.claim.owner !== owner || this.claim.origin !== origin)) return { state: 'busy' };
    if (input.action !== 'start' && (!this.claim || this.claim.owner !== owner || input.token !== this.claim.token)) throw new Error('Unknown storage upgrade session.');
    this.running = true;
    try {
      await this.ready;
      if (this.loadError) throw this.loadError;
      let checkpoint = this.journal.origins[origin];
      if (checkpoint?.steps.length === STAGES.length && checkpoint.notified) return { state: 'complete', notify: false };
      if (!this.claim) this.claim = { owner, origin, token: randomUUID() };
      const claim = this.claim;
      const current = () => { if (this.claim !== claim) throw new Error('Storage upgrade interrupted.'); };
      return await this.deps.exclusive(async () => {
        current();
        if (!checkpoint) {
          checkpoint = { steps: [], beforeBytes: await this.deps.measure() };
          await this.save(origin, checkpoint);
        }
        if (input.action === 'notified') {
          if (checkpoint.steps.length !== STAGES.length) throw new Error('Storage upgrade is not complete.');
          await this.save(origin, { ...checkpoint, notified: true });
          this.release(owner);
          return { state: 'complete', notify: false };
        }
        for (const stage of STAGES) {
          if (checkpoint.steps.includes(stage)) continue;
          current();
          progress({ stage, progress: STAGES.indexOf(stage) / STAGES.length });
          if (stage === 'browser') {
            if (input.action !== 'browserComplete') return { state: 'browser', token: claim.token };
          } else await this.deps[stage]();
          current();
          const next: Checkpoint = { ...checkpoint, steps: [...checkpoint.steps, stage] };
          // Checkpoint only after each durable step. Repeating a step after a
          // crash is safe: every operation verifies existing content again.
          if (next.steps.length === STAGES.length) next.freedBytes = Math.max(0, next.beforeBytes - await this.deps.measure());
          await this.save(origin, next); checkpoint = next;
        }
        return { state: 'complete', token: claim.token, notify: !checkpoint.notified, freedBytes: checkpoint.freedBytes ?? 0 };
      });
    } catch {
      this.release(owner);
      return { state: 'paused' };
    } finally { this.running = false; }
  }
}

/** Yield while inspecting history so the editor remains usable. No project
 * content is modified; only already-owned media records can become links. */
export async function visitSourceReferences(value: unknown, visit: (key: string, source: string) => Promise<void>): Promise<void> {
  const stack: unknown[] = [value], seen = new WeakSet<object>(), visited = new Set<string>();
  let count = 0;
  while (stack.length) {
    const item = stack.pop();
    if (!item || typeof item !== 'object' || seen.has(item)) continue;
    seen.add(item);
    const record = item as { [key: string]: unknown };
    if (typeof record.sourcePath === 'string' && typeof record.id === 'string') {
      const key = typeof record.storageKey === 'string' ? record.storageKey : record.id;
      const pair = JSON.stringify([key, record.sourcePath]);
      if (!visited.has(pair)) { visited.add(pair); await visit(key, record.sourcePath); }
    }
    for (const child of Object.values(record)) if (child && typeof child === 'object') stack.push(child);
    if (++count % 1000 === 0) await new Promise<void>(resolve => setImmediate(resolve));
  }
}

export function installStorageUpgrade(ipc: Pick<IpcMain, 'handle'>, upgrade: StorageUpgrade,
  trusted: (event: IpcMainInvokeEvent) => boolean): void {
  const owners = new WeakSet<WebContents>();
  ipc.handle(STORAGE_IPC.upgrade, (event, input: StorageUpgradeRequest) => {
    if (!trusted(event)) throw new Error('Unauthorized storage request.');
    const url = new URL(event.senderFrame?.url || event.sender.getURL());
    const origin = url.protocol === 'app:' ? `${url.protocol}//${url.host}` : url.origin;
    if (!owners.has(event.sender)) {
      owners.add(event.sender);
      watchRendererDocument(event.sender, () => { upgrade.release(event.sender); owners.delete(event.sender); });
    }
    return upgrade.request(origin, event.sender, input, value => {
      if (!event.sender.isDestroyed()) event.sender.send(STORAGE_IPC.upgradeProgress, value);
    });
  });
}
