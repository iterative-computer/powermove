import { createHash, randomUUID } from 'node:crypto';
import * as fs from 'node:fs/promises';
import path from 'node:path';

import type { AgentExtensionChange } from '../../shared/ipc';
import { EXTENSION_ID, parseManifest } from '../../shared/extensions';

const MAX_FILES = 4_000;
const MAX_BYTES = 32 * 1024 * 1024;
const RECORD_FILE = 'change-set.json';

export interface ExtensionStage {
  liveDirectory: string;
  stagingDirectory: string;
  historyRoot: string;
  projectId: string;
  runId: string;
  baselineHashes: Record<string, string>;
  baselineRootHash: string;
}

export interface ExtensionChangeSetRecord {
  version: 1;
  id: string;
  projectId: string;
  createdAt: string;
  beforeRootHash: string;
  afterRootHash: string;
  changes: AgentExtensionChange[];
}

export async function prepareExtensionStage(options: {
  liveDirectory: string;
  stagingDirectory: string;
  historyRoot: string;
  projectId: string;
  runId: string;
}): Promise<ExtensionStage> {
  for (const directory of [options.liveDirectory, options.stagingDirectory, options.historyRoot]) {
    if (!path.isAbsolute(directory)) throw new Error('Extension isolation paths must be absolute.');
  }
  await recoverInterruptedExtensionTransactions(options.historyRoot, options.liveDirectory);
  await fs.mkdir(options.liveDirectory, { recursive: true });
  await fs.rm(options.stagingDirectory, { recursive: true, force: true });
  await copyRegularTree(options.liveDirectory, options.stagingDirectory);
  const baselineHashes = await extensionHashes(options.stagingDirectory);
  return {
    ...options,
    baselineHashes,
    baselineRootHash: hashMap(baselineHashes)
  };
}

export async function publishExtensionChanges(
  stage: ExtensionStage,
  declaredChanges: readonly AgentExtensionChange[]
): Promise<ExtensionChangeSetRecord | null> {
  const currentHashes = await extensionHashes(stage.liveDirectory);
  if (hashMap(currentHashes) !== stage.baselineRootHash) {
    throw new Error('Extensions changed while the agent was working. Nothing was overwritten; retry the request.');
  }

  const stagedHashes = await extensionHashes(stage.stagingDirectory);
  const actualIds = changedIds(stage.baselineHashes, stagedHashes);
  const declared = new Map(declaredChanges.map((change) => [change.id, change]));
  const declaredIds = [...declared.keys()].sort();
  if (new Set(declaredIds).size !== declaredChanges.length) {
    throw new Error('The agent reported the same extension change more than once. Nothing was applied.');
  }
  if (actualIds.join('\0') !== declaredIds.join('\0')) {
    throw new Error('The agent change report did not match its staged files. Nothing was applied.');
  }
  if (actualIds.length === 0) return null;

  for (const id of actualIds) {
    const before = stage.baselineHashes[id];
    const after = stagedHashes[id];
    const expected = before === undefined ? 'created' : after === undefined ? 'removed' : 'updated';
    if (declared.get(id)?.action !== expected) {
      throw new Error(`The agent reported ${id} as ${declared.get(id)?.action ?? 'missing'}, but it was ${expected}. Nothing was applied.`);
    }
    if (after !== undefined) await validateExtensionDirectory(stage.stagingDirectory, id);
  }

  const pending = path.join(stage.historyRoot, `.pending-${stage.runId}-${randomUUID()}`);
  const before = path.join(pending, 'before');
  const next = path.join(pending, 'next');
  const final = path.join(stage.historyRoot, stage.runId);
  await fs.rm(final, { recursive: true, force: true });
  await Promise.all([fs.mkdir(before, { recursive: true }), fs.mkdir(next, { recursive: true })]);
  for (const id of actualIds) {
    if (stagedHashes[id] !== undefined) {
      await copyRegularTree(path.join(stage.stagingDirectory, id), path.join(next, id));
    }
  }
  const afterHashes = { ...stage.baselineHashes };
  for (const id of actualIds) {
    if (stagedHashes[id] === undefined) delete afterHashes[id];
    else afterHashes[id] = stagedHashes[id];
  }
  const afterRootHash = hashMap(afterHashes);
  const record: ExtensionChangeSetRecord = {
    version: 1,
    id: stage.runId,
    projectId: stage.projectId,
    createdAt: new Date().toISOString(),
    beforeRootHash: stage.baselineRootHash,
    afterRootHash,
    changes: declaredChanges.map((change) => ({ ...change }))
  };
  await fs.writeFile(path.join(pending, RECORD_FILE), `${JSON.stringify(record, null, 2)}\n`, { mode: 0o600 });

  try {
    for (const id of actualIds) {
      const live = path.join(stage.liveDirectory, id);
      if (await exists(live)) await fs.rename(live, path.join(before, id));
      const replacement = path.join(next, id);
      if (await exists(replacement)) await fs.rename(replacement, live);
    }
    await fs.rename(pending, final);
    return record;
  } catch (error) {
    await rollbackPendingChangeSet(pending, stage.liveDirectory, record).catch(() => undefined);
    await fs.rm(pending, { recursive: true, force: true }).catch(() => undefined);
    throw error;
  }
}

export async function restoreExtensionChangeSet(options: {
  liveDirectory: string;
  historyRoot: string;
  changeSetId: string;
}): Promise<ExtensionChangeSetRecord> {
  if (!/^[A-Za-z0-9_-]{1,160}$/.test(options.changeSetId)) throw new Error('Invalid extension change-set id.');
  await recoverInterruptedExtensionTransactions(options.historyRoot, options.liveDirectory);
  const directory = path.join(options.historyRoot, options.changeSetId);
  const record = JSON.parse(await fs.readFile(path.join(directory, RECORD_FILE), 'utf8')) as ExtensionChangeSetRecord;
  const currentHash = hashMap(await extensionHashes(options.liveDirectory));
  if (currentHash !== record.afterRootHash) {
    throw new Error('Newer extension changes exist. Restore newer change sets first.');
  }
  const before = path.join(directory, 'before');
  const redo = path.join(directory, 'redo');
  await fs.rm(redo, { recursive: true, force: true });
  await fs.mkdir(redo, { recursive: true });
  try {
    for (const change of record.changes) {
      const live = path.join(options.liveDirectory, change.id);
      if (await exists(live)) await fs.rename(live, path.join(redo, change.id));
      const snapshot = path.join(before, change.id);
      if (await exists(snapshot)) await copyRegularTree(snapshot, live);
    }
  } catch (error) {
    for (const change of record.changes) {
      const live = path.join(options.liveDirectory, change.id);
      await fs.rm(live, { recursive: true, force: true }).catch(() => undefined);
      const previous = path.join(redo, change.id);
      if (await exists(previous)) await fs.rename(previous, live).catch(() => undefined);
    }
    throw error;
  }
  return record;
}

export async function recoverInterruptedExtensionTransactions(historyRoot: string, liveDirectory: string): Promise<void> {
  await fs.mkdir(historyRoot, { recursive: true });
  const entries = await fs.readdir(historyRoot, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory() || !entry.name.startsWith('.pending-')) continue;
    const pending = path.join(historyRoot, entry.name);
    const recordPath = path.join(pending, RECORD_FILE);
    if (await exists(recordPath)) {
      const record = JSON.parse(await fs.readFile(recordPath, 'utf8')) as ExtensionChangeSetRecord;
      await rollbackPendingChangeSet(pending, liveDirectory, record);
    }
    await fs.rm(pending, { recursive: true, force: true });
  }
}

/** Boot-time repair across every project before the extension registry scans live files. */
export async function recoverAllInterruptedExtensionTransactions(userData: string, liveDirectory: string): Promise<void> {
  const root = path.join(userData, 'Agent Change History');
  if (!await exists(root)) return;
  for (const entry of await fs.readdir(root, { withFileTypes: true })) {
    if (entry.isDirectory()) await recoverInterruptedExtensionTransactions(path.join(root, entry.name), liveDirectory);
  }
}

async function rollbackPendingChangeSet(
  pending: string,
  liveDirectory: string,
  record: ExtensionChangeSetRecord
): Promise<void> {
  await fs.mkdir(liveDirectory, { recursive: true });
  for (const change of record.changes) {
    const live = path.join(liveDirectory, change.id);
    await fs.rm(live, { recursive: true, force: true });
    const previous = path.join(pending, 'before', change.id);
    if (await exists(previous)) await fs.rename(previous, live);
  }
}

async function validateExtensionDirectory(root: string, id: string): Promise<void> {
  if (!EXTENSION_ID.test(id)) throw new Error(`Invalid extension id: ${id}`);
  const raw: unknown = JSON.parse(await fs.readFile(path.join(root, id, 'manifest.json'), 'utf8'));
  const parsed = parseManifest(raw);
  if (!parsed.ok) throw new Error(`Invalid extension ${id}: ${parsed.error}`);
  if (parsed.manifest.id !== id) throw new Error(`Extension folder ${id} does not match its manifest id.`);
}

async function extensionHashes(root: string): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
  if (!await exists(root)) return result;
  for (const entry of (await fs.readdir(root, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isDirectory() || !EXTENSION_ID.test(entry.name)) continue;
    result[entry.name] = await directoryHash(path.join(root, entry.name));
  }
  return result;
}

function changedIds(before: Record<string, string>, after: Record<string, string>): string[] {
  return [...new Set([...Object.keys(before), ...Object.keys(after)])]
    .filter((id) => before[id] !== after[id])
    .sort();
}

function hashMap(value: Record<string, string>): string {
  const hash = createHash('sha256');
  for (const key of Object.keys(value).sort()) hash.update(`${key}\0${value[key]}\0`);
  return hash.digest('hex');
}

async function directoryHash(root: string): Promise<string> {
  const hash = createHash('sha256');
  let files = 0;
  let bytes = 0;
  async function walk(directory: string, relative: string): Promise<void> {
    const entries = (await fs.readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      const nextRelative = relative ? `${relative}/${entry.name}` : entry.name;
      const full = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`Extension contains an unsupported symbolic link: ${nextRelative}`);
      if (entry.isDirectory()) {
        hash.update(`d\0${nextRelative}\0`);
        await walk(full, nextRelative);
        continue;
      }
      if (!entry.isFile()) throw new Error(`Extension contains an unsupported file: ${nextRelative}`);
      const data = await fs.readFile(full);
      files += 1;
      bytes += data.byteLength;
      if (files > MAX_FILES || bytes > MAX_BYTES) throw new Error('Extension staging exceeds the safe copy limits.');
      hash.update(`f\0${nextRelative}\0`);
      hash.update(data);
    }
  }
  await walk(root, '');
  return hash.digest('hex');
}

async function copyRegularTree(source: string, destination: string): Promise<void> {
  await fs.mkdir(destination, { recursive: true });
  let files = 0;
  let bytes = 0;
  async function walk(from: string, to: string): Promise<void> {
    const entries = await fs.readdir(from, { withFileTypes: true });
    for (const entry of entries) {
      const input = path.join(from, entry.name);
      const output = path.join(to, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`Extension staging does not allow symbolic links: ${entry.name}`);
      if (entry.isDirectory()) {
        await fs.mkdir(output, { recursive: true });
        await walk(input, output);
        continue;
      }
      if (!entry.isFile()) throw new Error(`Extension staging does not allow special files: ${entry.name}`);
      const data = await fs.readFile(input);
      files += 1;
      bytes += data.byteLength;
      if (files > MAX_FILES || bytes > MAX_BYTES) throw new Error('Extension staging exceeds the safe copy limits.');
      await fs.writeFile(output, data, { mode: 0o600 });
    }
  }
  await walk(source, destination);
}

async function exists(target: string): Promise<boolean> {
  try { await fs.access(target); return true; }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false;
    throw error;
  }
}
