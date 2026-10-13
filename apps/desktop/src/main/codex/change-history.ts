import { createHash, randomUUID } from 'node:crypto';
import * as fs from 'node:fs/promises';
import path from 'node:path';

import type { AgentExtensionChange } from '../../shared/ipc';
import { EXTENSION_ID, parseManifest } from '../../shared/extensions';
import { AgentResultValidationError } from './result-repair';

const MAX_FILES = 4_000;
const MAX_BYTES = 32 * 1024 * 1024;
const RECORD_FILE = 'change-set.json';
const SNAPSHOT_PREFIX = '.snapshot-';

const liveTransactions = new Map<string, Promise<void>>();

/** One live tree owns its publication, recovery and baseline snapshots. */
function withLiveTransaction<T>(liveDirectory: string, task: () => Promise<T>): Promise<T> {
  const key = path.resolve(liveDirectory);
  const next = (liveTransactions.get(key) ?? Promise.resolve()).then(task);
  const settled = next.then(() => undefined, () => undefined);
  liveTransactions.set(key, settled);
  void settled.then(() => { if (liveTransactions.get(key) === settled) liveTransactions.delete(key); });
  return next;
}

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
  return withLiveTransaction(options.liveDirectory, () => prepareExtensionStageLocked(options));
}

async function prepareExtensionStageLocked(options: Parameters<typeof prepareExtensionStage>[0]): Promise<ExtensionStage> {
  for (const directory of [options.liveDirectory, options.stagingDirectory, options.historyRoot]) {
    if (!path.isAbsolute(directory)) throw new Error('Extension isolation paths must be absolute.');
  }
  await recoverInterruptedExtensionTransactionsLocked(options.historyRoot, options.liveDirectory);
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

/** A private stage copy; `unchanged` folders matched their baseline and were not copied. */
export type StageSnapshot = ExtensionStage & { compiledDirectory: string; unchanged: readonly string[] };

/**
 * Run `use` on a private copy of the stage's extensions, taken once in the
 * app-owned history folder that agent processes cannot write. Validate and
 * publish the copy: a process that outlived its command can still rewrite
 * the stage, but not what was checked and ships. Only changed and `reported`
 * folders are copied; the rest publish as their baseline.
 */
export async function withStageSnapshot<T>(
  stage: ExtensionStage,
  use: (snapshot: StageSnapshot) => Promise<T>,
  reported: readonly string[] = []
): Promise<T> {
  const directory = path.join(stage.historyRoot, `${SNAPSHOT_PREFIX}${stage.runId}-${randomUUID()}`);
  const stagingDirectory = path.join(directory, 'stage');
  const unchanged: string[] = [];
  try {
    await fs.mkdir(stagingDirectory, { recursive: true });
    // Only what publishing reads: extension folders, each within its own copy limits.
    for (const entry of await exists(stage.stagingDirectory) ? await fs.readdir(stage.stagingDirectory, { withFileTypes: true }) : []) {
      if (!entry.isDirectory() || !EXTENSION_ID.test(entry.name)) continue;
      const source = path.join(stage.stagingDirectory, entry.name);
      const baseline = stage.baselineHashes[entry.name];
      if (baseline !== undefined && !reported.includes(entry.name) && await directoryHash(source) === baseline) {
        unchanged.push(entry.name);
        continue;
      }
      await copyRegularTree(source, path.join(stagingDirectory, entry.name));
    }
    return await use({ ...stage, stagingDirectory, compiledDirectory: path.join(directory, 'compiled'), unchanged });
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}

export async function publishExtensionChanges(
  stage: ExtensionStage & { unchanged?: readonly string[] },
  declaredChanges: readonly AgentExtensionChange[],
  assertCurrent?: () => void
): Promise<ExtensionChangeSetRecord | null> {
  return withLiveTransaction(stage.liveDirectory, () => publishExtensionChangesLocked(stage, declaredChanges, assertCurrent));
}

async function publishExtensionChangesLocked(
  stage: ExtensionStage & { unchanged?: readonly string[] },
  declaredChanges: readonly AgentExtensionChange[],
  assertCurrent?: () => void
): Promise<ExtensionChangeSetRecord | null> {
  assertCurrent?.();
  const currentHashes = await extensionHashes(stage.liveDirectory);
  if (hashMap(currentHashes) !== stage.baselineRootHash) {
    throw new Error('Extensions changed while the agent was working. Nothing was overwritten; retry the request.');
  }

  const stagedHashes = await extensionHashes(stage.stagingDirectory);
  for (const id of stage.unchanged ?? []) stagedHashes[id] = stage.baselineHashes[id]!;
  const actualIds = changedIds(stage.baselineHashes, stagedHashes);
  const declared = new Map(declaredChanges.map((change) => [change.id, change]));
  const declaredIds = [...declared.keys()].sort();
  const expectedChanges = actualIds.map(id => ({
    id,
    action: stage.baselineHashes[id] === undefined ? 'created' : stagedHashes[id] === undefined ? 'removed' : 'updated'
  }));
  const reportError = (message: string): never => {
    throw new AgentResultValidationError(`${message} Nothing was applied. Actual staged extension changes: ${JSON.stringify(expectedChanges)}. Inspect these changes and return each once in extensions with the indicated action, or revert unintended staged edits before completing.`);
  };
  if (new Set(declaredIds).size !== declaredChanges.length) {
    reportError('The agent reported the same extension change more than once.');
  }
  if (actualIds.join('\0') !== declaredIds.join('\0')) {
    reportError('The agent change report did not match its staged files.');
  }
  if (actualIds.length === 0) return null;

  for (const id of actualIds) {
    const before = stage.baselineHashes[id];
    const after = stagedHashes[id];
    const expected = before === undefined ? 'created' : after === undefined ? 'removed' : 'updated';
    if (declared.get(id)?.action !== expected) {
      reportError(`The agent reported ${id} as ${declared.get(id)?.action ?? 'missing'}, but it was ${expected}.`);
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
    // A queued client may have disconnected during preparation. Check once
    // before any live swap; a started promotion still completes or rolls back.
    assertCurrent?.();
    for (const id of actualIds) {
      const live = path.join(stage.liveDirectory, id);
      if (await exists(live)) await fs.rename(live, path.join(before, id));
      const replacement = path.join(next, id);
      if (await exists(replacement)) await fs.rename(replacement, live);
    }
    await fs.rename(pending, final);
    return record;
  } catch (error) {
    // Retain the journal and backups if rollback itself fails so boot can retry.
    try {
      await rollbackPendingChangeSet(pending, stage.liveDirectory, record);
      await fs.rm(pending, { recursive: true, force: true });
    } catch { /* Recovery will retry the pending transaction. */ }
    throw error;
  }
}

export async function restoreExtensionChangeSet(options: {
  liveDirectory: string;
  historyRoot: string;
  changeSetId: string;
}): Promise<ExtensionChangeSetRecord> {
  return withLiveTransaction(options.liveDirectory, () => restoreExtensionChangeSetLocked(options));
}

async function restoreExtensionChangeSetLocked(options: Parameters<typeof restoreExtensionChangeSet>[0]): Promise<ExtensionChangeSetRecord> {
  if (!/^[A-Za-z0-9_-]{1,160}$/.test(options.changeSetId)) throw new Error('Invalid extension change-set id.');
  await recoverInterruptedExtensionTransactionsLocked(options.historyRoot, options.liveDirectory);
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
  const touched: string[] = [];
  try {
    for (const change of record.changes) {
      const live = path.join(options.liveDirectory, change.id);
      if (await exists(live)) await fs.rename(live, path.join(redo, change.id));
      touched.push(change.id);
      const snapshot = path.join(before, change.id);
      if (await exists(snapshot)) await copyRegularTree(snapshot, live);
    }
  } catch (error) {
    for (const id of touched.reverse()) {
      const live = path.join(options.liveDirectory, id);
      await fs.rm(live, { recursive: true, force: true }).catch(() => undefined);
      const previous = path.join(redo, id);
      if (await exists(previous)) await fs.rename(previous, live).catch(() => undefined);
    }
    throw error;
  }
  return record;
}

export async function recoverInterruptedExtensionTransactions(historyRoot: string, liveDirectory: string): Promise<void> {
  return withLiveTransaction(liveDirectory, () => recoverInterruptedExtensionTransactionsLocked(historyRoot, liveDirectory));
}

async function recoverInterruptedExtensionTransactionsLocked(historyRoot: string, liveDirectory: string): Promise<void> {
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
  return withLiveTransaction(liveDirectory, () => recoverAllInterruptedExtensionTransactionsLocked(userData, liveDirectory));
}

async function recoverAllInterruptedExtensionTransactionsLocked(userData: string, liveDirectory: string): Promise<void> {
  const root = path.join(userData, 'Agent Change History');
  if (!await exists(root)) return;
  for (const entry of await fs.readdir(root, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const historyRoot = path.join(root, entry.name);
    await recoverInterruptedExtensionTransactionsLocked(historyRoot, liveDirectory);
    // Only at boot: a run on this project may be using its snapshot now.
    for (const leftover of await fs.readdir(historyRoot)) {
      if (leftover.startsWith(SNAPSHOT_PREFIX)) await fs.rm(path.join(historyRoot, leftover), { recursive: true, force: true });
    }
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
    if (!EXTENSION_ID.test(change.id)) throw new Error('Invalid pending extension id.');
    const previous = path.join(pending, 'before', change.id);
    if (await exists(previous)) {
      // Keep the backup until the entire journal is removed. A crash during
      // recovery must never turn a partially restored tree into the only copy.
      const restored = path.join(pending, 'restore', change.id);
      await fs.rm(restored, { recursive: true, force: true });
      await copyRegularTree(previous, restored);
      await fs.rm(live, { recursive: true, force: true });
      await fs.rename(restored, live);
    } else if (change.action === 'created' && !await exists(path.join(pending, 'next', change.id))) {
      // Only a creation whose prepared replacement was moved can be ours.
      await fs.rm(live, { recursive: true, force: true });
    }
    // Updated/removed entries without a backup have not been touched.
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
      // The stage is agent-writable: a file swapped since the listing for a
      // link, FIFO or device is refused, never followed or waited on.
      const handle = await fs.open(full, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK).catch((error: NodeJS.ErrnoException) => {
        throw error.code === 'ELOOP' ? new Error(`Extension contains an unsupported symbolic link: ${nextRelative}`) : error;
      });
      let data: Buffer;
      try {
        const opened = await handle.stat();
        if (!opened.isFile()) throw new Error(`Extension contains an unsupported file: ${nextRelative}`);
        files += 1;
        bytes += opened.size;
        if (files > MAX_FILES || bytes > MAX_BYTES) throw new Error('Extension staging exceeds the safe copy limits.');
        // Read what fstat promised and one byte more, so a file growing meanwhile is caught, not buffered.
        data = Buffer.alloc(opened.size + 1);
        let length = 0;
        for (;;) {
          const { bytesRead } = await handle.read(data, length, data.byteLength - length, length);
          if (!bytesRead) break;
          length += bytesRead;
          if (length > opened.size) throw new Error(`Extension staging changed while it was being read: ${nextRelative}`);
        }
        data = data.subarray(0, length);
      } finally { await handle.close(); }
      hash.update(`f\0${nextRelative}\0`);
      hash.update(data);
    }
  }
  await walk(root, '');
  return hash.digest('hex');
}

const sameFile = (a: { dev: number; ino: number }, b: { dev: number; ino: number }) => a.dev === b.dev && a.ino === b.ino;

/**
 * Copy folders and regular files only. Agent processes may still swap a
 * folder for a link while it is read, so each folder must be the same one
 * after its listing and still resolve beneath the source, and each file read
 * must be the file now at its place there; otherwise the copy aborts.
 */
async function copyRegularTree(source: string, destination: string): Promise<void> {
  if (!(await fs.lstat(source)).isDirectory()) throw new Error('Extension staging is not a folder.');
  const root = await fs.realpath(source);
  const changed = () => new Error('Extension staging changed while it was being copied. Nothing was copied from it.');
  await fs.mkdir(destination, { recursive: true });
  let files = 0;
  let bytes = 0;
  async function walk(from: string, to: string): Promise<void> {
    const expected = path.join(root, path.relative(source, from));
    const before = await fs.lstat(from);
    const entries = await fs.readdir(from, { withFileTypes: true });
    const after = await fs.lstat(from);
    if (!before.isDirectory() || !after.isDirectory() || !sameFile(before, after) || await fs.realpath(from) !== expected) throw changed();
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
      // A file swapped for a link since the listing is refused, not followed.
      const handle = await fs.open(input, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
      let data: Buffer;
      try {
        const opened = await handle.stat();
        if (!opened.isFile() || !sameFile(opened, await fs.lstat(input)) || await fs.realpath(from) !== expected) throw changed();
        data = await handle.readFile();
      } finally { await handle.close(); }
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
