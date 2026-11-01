import { createHash, randomUUID } from 'node:crypto';
import { readFile, statfs } from 'node:fs/promises';
import path from 'node:path';
import type { IpcMain, IpcMainInvokeEvent } from 'electron';
import { HASH_HEX, validStorageKey } from '../../shared/media-store';
import { TR_IPC, type TRCommand, type TRPayload, type TRStatus, type ProxyOf } from '../../shared/transition';
import { hashFile } from '../media-store/hasher';
import { MediaStore, type MediaStoreOptions } from '../media-store/store';
import { writeJSON } from './json';
import { watchRendererDocument } from '../renderer-recovery';

const RESERVE = 4 * 1024 ** 3;
const digest = (value: unknown) => createHash('sha256').update(JSON.stringify(value)).digest('hex');
type Receipt = { payload: TRPayload; hash: string; conflict: boolean };
type Manifest = {
  version: 1; state: TRStatus['state']; inventory: string[]; completed: string[];
  records: Record<string, unknown>; payloads: Record<string, Receipt>;
  localStorageComplete: boolean; secretsComplete: boolean;
};
export class TransitionMedia {
  readonly store: MediaStore;
  private manifest: Manifest = { version: 1, state: 'pending', inventory: [], completed: [], records: {}, payloads: {}, localStorageComplete: false, secretsComplete: false };
  private generation = 0;
  private references: Record<string, ProxyOf[]> = Object.create(null);
  private pass: { id: string; owner: number; generation: number; seen: Set<string>; payloads: Set<string> } | undefined;
  private uploads = new Map<string, { owner: number; payload: TRPayload; pass?: string }>();
  private queue: Promise<unknown> = Promise.resolve();
  private initialized: Promise<void> | undefined;
  constructor(private options: MediaStoreOptions & { freeBytes?: () => Promise<number>; secrets?: () => Promise<void>; references?: () => Promise<Record<string, ProxyOf[]>> }) { this.store = new MediaStore(options); }
  private get file() { return path.join(this.store.root, 'migration.json'); }
  initialize(): Promise<void> {
    return this.initialized ??= (async () => {
      await this.store.init();
      try {
        const value = JSON.parse(await readFile(this.file, 'utf8'));
        if (value.version !== 1 || !Array.isArray(value.inventory) || !Array.isArray(value.completed) || !value.records || !value.payloads) throw new Error('Invalid transition checkpoint.');
        this.manifest = value;
      } catch (error: any) { if (error.code !== 'ENOENT') throw error; }
      // Every launch needs a fresh delta pass. Receipts remain resumable.
      this.manifest.state = 'pending';
      this.manifest.localStorageComplete = false;
      this.manifest.secretsComplete = false;
      await this.persist();
    })();
  }
  private persist() { this.options.assertWritable?.(); return writeJSON(this.file, this.manifest); }
  status(): TRStatus {
    const mediaComplete = this.manifest.state === 'complete';
    return { state: this.manifest.state, done: this.manifest.completed.length, total: this.manifest.inventory.length,
      mediaComplete, localStorageComplete: this.manifest.localStorageComplete, secretsComplete: this.manifest.secretsComplete,
      cutoverReady: mediaComplete && this.manifest.localStorageComplete && this.manifest.secretsComplete };
  }
  request(owner: number, command: TRCommand): Promise<any> {
    const task = this.queue.then(async () => {
      await this.initialize();
      try { return await this.perform(owner, command); }
      catch (error) {
        if (command.action !== 'status' && command.action !== 'abort') {
          this.manifest.state = 'paused'; await this.persist();
        }
        throw error;
      }
    });
    this.queue = task.catch(() => undefined);
    return task;
  }
  private checkPass(owner: number, id: string | undefined) {
    if (!this.pass || this.pass.id !== id || this.pass.owner !== owner) throw new Error('The transition pass is no longer owned by this window.');
    return this.pass;
  }
  private inventory(value: unknown): string[] {
    if (!Array.isArray(value) || value.some(x => typeof x !== 'string' || x.length > 4096) || new Set(value).size !== value.length) throw new Error('Invalid browser inventory.');
    return [...value].sort();
  }
  private async space(size: number) {
    const available = this.options.freeBytes ? await this.options.freeBytes() : await statfs(this.store.root).then(s => s.bavail * s.bsize);
    if (available < size + RESERVE) throw new Error('Transition preparation paused: free more space on this disk, then retry.');
  }
  private async perform(owner: number, command: TRCommand): Promise<any> {
    if (command.action !== 'status' && command.action !== 'abort') this.options.assertWritable?.();
    switch (command.action) {
      case 'status': return this.status();
      case 'secrets': {
        this.manifest.secretsComplete = false; await this.persist();
        if (!this.options.secrets) throw new Error('Sign-in transfer is unavailable.');
        await this.options.secrets(); this.manifest.secretsComplete = true; await this.persist(); return this.status();
      }
      case 'start': {
        if (this.pass && this.pass.owner !== owner) throw new Error('Another window is preparing the transition.');
        const inventory = this.inventory(command.inventory);
        for (const [token, job] of this.uploads) if (job.owner === owner && job.pass) {
          await this.store.abortStream(owner, token); this.uploads.delete(token);
        }
        await this.space(0);
        this.references = await this.options.references?.() ?? Object.create(null);
        this.pass = { id: randomUUID(), owner, generation: this.generation, seen: new Set(), payloads: new Set() };
        this.manifest.inventory = inventory; this.manifest.completed = []; this.manifest.state = 'copying';
        await this.persist(); return this.pass.id;
      }
      case 'localStorage': {
        if (!command.values || typeof command.values !== 'object' || Array.isArray(command.values)
          || Object.values(command.values).some(v => !v || typeof v.raw !== 'string')) throw new Error('Invalid localStorage export.');
        const value = { version: 1, values: command.values };
        // Immutable logical snapshots retain unknown keys and deletions across passes.
        await writeJSON(path.join(this.store.root, 'local-storage', `${digest(value)}.json`), value);
        this.manifest.localStorageComplete = true; await this.persist(); return;
      }
      case 'reuse': {
        if (!command.payload || !HASH_HEX.test(command.hash)) throw new Error('Invalid resume request.');
        const pass = this.checkPass(owner, command.pass);
        const id = digest([command.payload, command.hash]);
        const receipt = this.manifest.payloads[id];
        if (!receipt || receipt.conflict) return null;
        const record = await this.store.record(receipt.payload.key);
        if (record?.hash !== receipt.hash) return null;
        const object = await hashFile(this.options.executor, this.store.objectPath(receipt.hash), { settleMs: 0 });
        if (object.hash !== receipt.hash) throw new Error('A migrated object is damaged.');
        const bindings = this.references[receipt.payload.key] ?? [];
        if (bindings.length) {
          Object.assign(receipt, { classification: 'proxy-of', proxyOf: receipt.payload.proxyOf ? [receipt.payload.proxyOf, ...bindings] : bindings });
          await this.store.publishRecord({ ...record, source: 'rendition' });
          await this.persist();
        }
        pass.payloads.add(id); return id;
      }
      case 'begin': {
        const p = command.payload;
        if (!p || !validStorageKey(p.key)) throw new Error('Invalid transition payload.');
        if (command.pass) this.checkPass(owner, command.pass);
        if (!Number.isSafeInteger(p.size) || p.size < 0) throw new Error('Invalid media size.');
        await this.space(p.size);
        if (!command.pass) this.generation++;
        this.manifest.state = 'copying'; await this.persist();
        const token = await this.store.beginStream(owner, { key: null, size: p.size, type: p.type, revision: p.revision, origin: command.pass ? 'indexeddb' : 'import' });
        this.uploads.set(token, { owner, payload: p, pass: command.pass }); return token;
      }
      case 'chunk': return this.store.chunk(owner, command.token, command.offset, command.data);
      case 'abort': {
        if (this.uploads.get(command.token)?.owner !== owner) throw new Error('Unknown transition upload.');
        this.uploads.delete(command.token); return this.store.abortStream(owner, command.token);
      }
      case 'finish': {
        const job = this.uploads.get(command.token);
        if (!job || job.owner !== owner || !HASH_HEX.test(command.hash)) throw new Error('Invalid transition upload.');
        if (job.pass) this.checkPass(owner, job.pass);
        try {
          const record = await this.store.finishStream(owner, command.token, { blake3: command.hash });
          const prior = await this.store.record(job.payload.key);
          const conflict = !!job.pass && !!prior && prior.hash !== record.hash;
          const bindings = this.references[job.payload.key] ?? [];
          const receipt = { payload: job.payload, hash: record.hash, conflict,
            classification: job.payload.proxyOf || bindings.length ? 'proxy-of' : job.payload.key.endsWith(':poster') ? 'poster' : 'unclassified',
            proxyOf: job.payload.proxyOf ? [job.payload.proxyOf, ...bindings] : bindings };
          if (receipt.classification === 'proxy-of') record.source = 'rendition';
          const id = digest([job.payload, record.hash]);
          // Checkpoint BEFORE the key, so a crash cannot lose provenance.
          this.manifest.payloads[id] = receipt; await this.persist();
          if (!conflict) await this.store.publishRecord({ ...record, key: job.payload.key });
          this.pass?.payloads.add(id);
          await this.persist(); return id;
        } finally { this.uploads.delete(command.token); }
      }
      case 'record': {
        const pass = this.checkPass(owner, command.pass);
        if (!this.manifest.inventory.includes(command.locator) || !Array.isArray(command.payloads)
          || command.payloads.some(id => !pass.payloads.has(id) || this.manifest.payloads[id]?.conflict)) throw new Error('A browser record is incomplete or conflicts with the media store.');
        const value = { locator: command.locator, metadata: command.metadata, payloads: command.payloads };
        await writeJSON(path.join(this.store.root, 'records', `${digest(value)}.json`), value);
        this.manifest.records[command.locator] = value;
        pass.seen.add(command.locator); this.manifest.completed = [...pass.seen]; await this.persist(); return;
      }
      case 'complete': {
        const pass = this.checkPass(owner, command.pass);
        if (pass.generation !== this.generation) throw new Error('New imports require a fresh delta pass.');
        if (JSON.stringify(this.inventory(command.inventory)) !== JSON.stringify(this.manifest.inventory)
          || this.manifest.inventory.some(id => !pass.seen.has(id)) || [...this.uploads.values()].some(x => x.pass === pass.id)) throw new Error('Browser media changed; another delta pass is required.');
        for (const id of pass.payloads) {
          const receipt = this.manifest.payloads[id]!;
          const object = await hashFile(this.options.executor, this.store.objectPath(receipt.hash), { settleMs: 0 });
          const mapping = await this.store.record(receipt.payload.key);
          if (object.hash !== receipt.hash || mapping?.hash !== receipt.hash || receipt.conflict) throw new Error('Transition verification failed.');
        }
        const completed = { inventory: this.manifest.inventory, records: this.manifest.records, payloads: this.manifest.payloads };
        await writeJSON(path.join(this.store.root, 'passes', `${digest(completed)}.json`), completed);
        this.manifest.state = 'complete'; await this.persist(); this.pass = undefined; return this.status();
      }
      default: throw new Error('Unknown transition command.');
    }
  }
  release(owner: number): Promise<void> {
    const task = this.queue.then(async () => {
      await this.store.abandon(owner);
      for (const [token, job] of this.uploads) if (job.owner === owner) this.uploads.delete(token);
      if (this.pass?.owner === owner) { this.pass = undefined; this.manifest.state = 'paused'; await this.persist(); }
    });
    this.queue = task.catch(() => undefined);
    return task;
  }
}
export function registerTransition(ipc: Pick<IpcMain, 'handle'>, service: TransitionMedia, trusted: (event: IpcMainInvokeEvent) => boolean): void {
  const watched = new WeakSet<object>();
  ipc.handle(TR_IPC, (event, command: TRCommand) => {
    if (!trusted(event) || !command || typeof command !== 'object') throw new Error('Untrusted transition request.');
    if (!watched.has(event.sender)) {
      watched.add(event.sender);
      watchRendererDocument(event.sender, () => {
        watched.delete(event.sender);
        void service.release(event.sender.id).catch(() => {});
      });
    }
    return service.request(event.sender.id, command);
  });
}
