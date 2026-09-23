/*
 * Provenance: one record per local extension folder, at
 * `<userData>/extensions-provenance.json` (store plan §2.2). Install writes
 * nothing into the extension tree, so where a folder came from and where it
 * was published live here, keyed by the folder's local id.
 *
 * `envKey` is minted here; the Store (cloud/install.ts) writes `origin`,
 * `upstream` and `pendingUpdate` through `update`. Fields a newer build wrote
 * are preserved untouched.
 */
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

import { EXTENSION_ID } from '../../shared/extensions';

export interface ProvenanceOrigin {
  repoId: string;
  releaseId: string;
  coordinate: string;
  version: string;
  treeSha: string;
  commitSha: string;
  ownerPublisherId: string;
}

export interface ProvenancePublished {
  repoId: string;
  releaseId: string;
  version: string;
  ownerPublisherId: string;
}

export interface ProvenanceUpstream {
  releaseId: string;
  treeSha: string;
}

/** A newer release staged beside a folder the user changed (Store 1.0 has no merge). */
export interface ProvenancePendingUpdate {
  releaseId: string;
  version: string;
  /** `extensions/.updates/<id>`. */
  path: string;
}

export interface ProvenanceRecord {
  localId: string;
  /** Names the values file: `origin.repoId` for store installs, `local:<uuid>` otherwise. */
  envKey: string;
  /** What was installed. Immutable. */
  origin?: ProvenanceOrigin;
  /** Where this folder was last published, if its owner is the current account. */
  published?: ProvenancePublished;
  /** Merge base for the next update from origin. */
  upstream?: ProvenanceUpstream;
  /** Set by a modified update; cleared by the next install, update or uninstall. */
  pendingUpdate?: ProvenancePendingUpdate;
}

export type ProvenanceFile = Record<string, ProvenanceRecord>;

export const PROVENANCE_FILE = 'extensions-provenance.json';

export interface ProvenanceStore {
  readonly file: string;
  read(): Promise<ProvenanceFile>;
  get(localId: string): Promise<ProvenanceRecord | null>;
  /** The folder's env key, created (`local:<uuid>`) and persisted on first use. */
  ensureEnvKey(localId: string): Promise<string>;
  remove(localId: string): Promise<void>;
  /**
   * Read-modify-write one record in order with every other write. `mutate`
   * gets the current record (null when there is none or it is unusable) and
   * returns the next one; null deletes it.
   */
  update(localId: string, mutate: (current: ProvenanceRecord | null) => ProvenanceRecord | null): Promise<ProvenanceRecord | null>;
}

export function createProvenanceStore(userData: string, options: { uuid?: () => string } = {}): ProvenanceStore {
  const file = path.join(userData, PROVENANCE_FILE);
  const uuid = options.uuid ?? randomUUID;
  /* Every read-modify-write runs in order, so two quick `set`s from the
     renderer cannot both mint a key and lose one. */
  let queue: Promise<unknown> = Promise.resolve();
  const serial = <T>(task: () => Promise<T>): Promise<T> => {
    const run = queue.then(task);
    queue = run.catch(() => undefined);
    return run;
  };

  async function load(): Promise<ProvenanceFile> {
    let text: string;
    try {
      text = await readFile(file, 'utf8');
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return {};
      throw error;
    }
    let raw: unknown;
    try {
      raw = JSON.parse(text);
    } catch {
      // Never overwrite a file we cannot read: it may hold store lineage.
      throw new Error('The extension provenance file is damaged. Move it aside and try again.');
    }
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
      throw new Error('The extension provenance file is damaged. Move it aside and try again.');
    }
    /* Entries this build does not understand are kept as they are, so a
       newer Store unit's fields survive an older writer. */
    return raw as ProvenanceFile;
  }

  const usable = (localId: string, value: unknown): ProvenanceRecord | null => {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    const record = value as Partial<ProvenanceRecord>;
    if (typeof record.envKey !== 'string' || record.envKey.length === 0) return null;
    return { ...(record as ProvenanceRecord), localId };
  };

  async function save(next: ProvenanceFile): Promise<void> {
    await mkdir(path.dirname(file), { recursive: true });
    const temporary = `${file}.${process.pid}.${Date.now()}.tmp`;
    try {
      await writeFile(temporary, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
      await rename(temporary, file);
    } catch (error) {
      await rm(temporary, { force: true }).catch(() => undefined);
      throw error;
    }
  }

  const validate = (localId: string): string => {
    if (typeof localId !== 'string' || !EXTENSION_ID.test(localId)) throw new Error('Invalid extension id.');
    return localId;
  };

  return {
    file,
    read: () => serial(async () => {
      const result: ProvenanceFile = {};
      for (const [localId, value] of Object.entries(await load())) {
        const record = EXTENSION_ID.test(localId) ? usable(localId, value) : null;
        if (record) result[localId] = record;
      }
      return result;
    }),
    get: (localId) => serial(async () => {
      validate(localId);
      return usable(localId, (await load())[localId]);
    }),
    ensureEnvKey: (localId) => serial(async () => {
      validate(localId);
      const current = await load();
      const existing = current[localId] as Partial<ProvenanceRecord> | undefined;
      const valid = usable(localId, existing);
      if (valid) return valid.envKey;
      // A store install is keyed by its repo so a reinstall finds its values.
      const envKey = typeof existing?.origin?.repoId === 'string' && existing.origin.repoId
        ? existing.origin.repoId
        : `local:${uuid()}`;
      current[localId] = { ...(existing && typeof existing === 'object' ? existing : {}), localId, envKey } as ProvenanceRecord;
      await save(current);
      return envKey;
    }),
    update: (localId, mutate) => serial(async () => {
      validate(localId);
      const current = await load();
      const next = mutate(usable(localId, current[localId]));
      if (next === null) {
        if (!(localId in current)) return null;
        delete current[localId];
      } else {
        if (typeof next.envKey !== 'string' || next.envKey.length === 0) throw new Error('A provenance record needs an env key.');
        current[localId] = { ...next, localId };
      }
      await save(current);
      return next === null ? null : { ...next, localId };
    }),
    remove: (localId) => serial(async () => {
      validate(localId);
      const current = await load();
      if (!(localId in current)) return;
      delete current[localId];
      await save(current);
    })
  };
}
