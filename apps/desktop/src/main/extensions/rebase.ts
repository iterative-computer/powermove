import * as fs from 'node:fs/promises';
import path from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { mergeText } from '../cloud/pull';

import {
  EXTENSION_ID,
  EXTENSION_VERSION,
  parseManifest,
  type ExtensionManifest
} from '../../shared/extensions';

const MAX_FILES = 4_000;
const MAX_BYTES = 32 * 1024 * 1024;
const FORK_BASE_DIRECTORY = '.forked-from';

export interface StageForkRebaseResult extends ForkRebaseInfo {
  workingDir: string;
  baseDir: string;
  oursDir: string;
  changedByUser: string[];
  changedUpstream: string[];
  conflicts: string[];
  mergedFiles: string[];
}

export interface ForkRebaseInfo {
  forkId: string;
  forkedFrom: string;
  base: string;
  current: string;
}

interface ForkRebaseOptions {
  forkId: string;
  stagingDirectory?: string;
  userExtensionsDir: string;
  builtinExtensionsDir: string;
}

interface TreeSnapshot {
  directories: string[];
  files: Map<string, Buffer>;
}

interface CopyBudget {
  files: number;
  bytes: number;
}

interface ValidatedFork {
  forkId: string;
  forkedFrom: string;
  baseVersion: string;
  currentVersion: string;
  manifestRaw: Record<string, unknown>;
  user: TreeSnapshot;
  base: TreeSnapshot;
  current: TreeSnapshot;
}

/** Validate a user fork and report the built-in version it can be rebased onto. */
export async function readForkRebaseInfo(options: {
  forkId: string;
  userExtensionsDir: string;
  builtinExtensionsDir: string;
}): Promise<ForkRebaseInfo> {
  validateAbsoluteDirectories(options.userExtensionsDir, options.builtinExtensionsDir);
  const fork = await inspectFork(options);
  return {
    forkId: fork.forkId,
    forkedFrom: fork.forkedFrom,
    base: fork.baseVersion,
    current: fork.currentVersion
  };
}

/**
 * Build a three-way rebase workspace without touching the live user extension.
 * The agent edits `workingDir`; `baseDir` and `oursDir` are read-only inputs by
 * convention and live below the hidden staging metadata directory.
 */
export async function stageForkRebase(options: {
  forkId: string;
  stagingDirectory: string;
  userExtensionsDir: string;
  builtinExtensionsDir: string;
}): Promise<StageForkRebaseResult> {
  validateAbsoluteDirectories(
    options.stagingDirectory,
    options.userExtensionsDir,
    options.builtinExtensionsDir
  );
  assertSeparateRoots(
    options.stagingDirectory,
    options.userExtensionsDir,
    options.builtinExtensionsDir
  );

  const fork = await inspectFork(options);
  const stagingDirectory = path.resolve(options.stagingDirectory);
  const workingDir = containedChild(stagingDirectory, fork.forkId);
  const rebaseDir = containedChild(containedChild(stagingDirectory, '.rebase'), fork.forkId);
  const baseDir = containedChild(rebaseDir, 'base');
  const oursDir = containedChild(rebaseDir, 'ours');

  const changedByUser = changedFiles(fork.base.files, fork.user.files);
  const changedUpstream = changedFiles(fork.base.files, fork.current.files);
  const merged = mergeFork(fork);

  // All sources have been read and validated before any staged state is removed.
  await fs.mkdir(stagingDirectory, { recursive: true });
  await Promise.all([
    fs.rm(workingDir, { recursive: true, force: true }),
    fs.rm(rebaseDir, { recursive: true, force: true })
  ]);

  await Promise.all([
    writeTree(workingDir, merged.tree),
    writeTree(baseDir, fork.base),
    writeTree(oursDir, fork.current)
  ]);

  const workingBase = containedChild(workingDir, FORK_BASE_DIRECTORY);
  await fs.rm(workingBase, { recursive: true, force: true });
  await writeTree(workingBase, fork.current);
  await fs.writeFile(
    path.join(workingDir, 'manifest.json'),
    `${JSON.stringify({
      ...merged.manifest,
      forkedFrom: `${fork.forkedFrom}@${fork.currentVersion}`
    }, null, 2)}\n`,
    { mode: 0o600 }
  );

  return {
    forkId: fork.forkId,
    workingDir,
    baseDir,
    oursDir,
    changedByUser,
    changedUpstream,
    conflicts: merged.conflicts,
    mergedFiles: merged.mergedFiles,
    forkedFrom: fork.forkedFrom,
    base: fork.baseVersion,
    current: fork.currentVersion
  };
}

/** Keep custom identity; merge the remaining manifest fields as JSON values so
 * formatting and the fork's bookkeeping never create artificial conflicts. */
function mergeManifest(fork: ValidatedFork): { manifest: Record<string, unknown>; conflict: boolean } {
  const base = parseManifestObject(fork.base.files.get('manifest.json'), 'fork base');
  const current = parseManifestObject(fork.current.files.get('manifest.json'), 'shipped built-in');
  const user = fork.manifestRaw;
  const manifest = { ...user };
  const identity = new Set(['id', 'name', 'author', 'version', 'forkedFrom']);
  for (const input of [base, current]) {
    input.replaces = [...new Set([...(input.replaces as string[] | undefined ?? []), fork.forkedFrom])];
  }
  let conflict = false;
  for (const key of new Set([...Object.keys(base), ...Object.keys(user), ...Object.keys(current)])) {
    if (identity.has(key) || isDeepStrictEqual(user[key], current[key]) || isDeepStrictEqual(base[key], current[key])) continue;
    if (!isDeepStrictEqual(user[key], base[key])) { conflict = true; continue; }
    if (Object.hasOwn(current, key)) manifest[key] = current[key];
    else delete manifest[key];
  }
  return { manifest, conflict };
}

function mergeFork(fork: ValidatedFork): {
  tree: TreeSnapshot; manifest: Record<string, unknown>; conflicts: string[]; mergedFiles: string[];
} {
  const files = new Map(fork.user.files);
  const conflicts: string[] = [];
  const mergedFiles: string[] = [];
  const { manifest, conflict } = mergeManifest(fork);
  if (conflict) conflicts.push('manifest.json');
  else mergedFiles.push('manifest.json');
  for (const relative of changedFiles(fork.base.files, fork.current.files)) {
    if (relative === 'manifest.json') continue;
    const base = fork.base.files.get(relative);
    const user = fork.user.files.get(relative);
    const current = fork.current.files.get(relative);
    if (isDeepStrictEqual(user, current)) continue;
    if (isDeepStrictEqual(user, base)) {
      if (current) files.set(relative, current);
      else files.delete(relative);
      mergedFiles.push(relative);
    } else if (user && current) {
      const merged = mergeText(base ?? null, user, current);
      if (merged) { files.set(relative, Buffer.from(merged)); mergedFiles.push(relative); }
      else conflicts.push(relative);
    } else conflicts.push(relative);
  }
  // A new file can collide with a customized folder (or vice versa). Leave
  // those structural changes to the agent instead of writing a partial tree.
  for (const relative of files.keys()) {
    const parts = relative.split('/');
    for (let i = 1; i < parts.length; i++) {
      if (files.has(parts.slice(0, i).join('/'))) {
        return { tree: fork.user, manifest: fork.manifestRaw,
          conflicts: changedFiles(fork.base.files, fork.current.files), mergedFiles: [] };
      }
    }
  }
  const directories = new Set(fork.user.directories.filter(dir => {
    const parts = dir.split('/');
    return !parts.some((_, i) => files.has(parts.slice(0, i + 1).join('/')));
  }));
  for (const relative of files.keys()) {
    const parts = relative.split('/');
    for (let i = 1; i < parts.length; i++) directories.add(parts.slice(0, i).join('/'));
  }
  return { tree: { files, directories: [...directories].sort() }, manifest,
    conflicts: conflicts.sort(), mergedFiles: mergedFiles.sort() };
}

async function inspectFork(options: ForkRebaseOptions): Promise<ValidatedFork> {
  if (typeof options.forkId !== 'string' || !EXTENSION_ID.test(options.forkId)) {
    throw new Error('Invalid extension id.');
  }

  const userRoot = path.resolve(options.userExtensionsDir);
  const builtinRoot = path.resolve(options.builtinExtensionsDir);
  const userDirectory = containedChild(userRoot, options.forkId);
  const sameNamedBuiltin = containedChild(builtinRoot, options.forkId);

  const userKind = await entryKind(userDirectory);
  if (userKind === 'missing') {
    if (await entryKind(sameNamedBuiltin) !== 'missing') {
      throw new Error(`Only user extensions can be rebased: ${options.forkId}`);
    }
    throw new Error(`Unknown extension: ${options.forkId}`);
  }
  if (userKind !== 'directory') {
    throw new Error(`Invalid user extension directory: ${options.forkId}`);
  }

  const baseDirectory = containedChild(userDirectory, FORK_BASE_DIRECTORY);
  if (await entryKind(baseDirectory) !== 'directory') {
    throw new Error(`Fork ${options.forkId} is missing its ${FORK_BASE_DIRECTORY} base.`);
  }

  const budget: CopyBudget = { files: 0, bytes: 0 };
  const user = await readRegularTree(userDirectory, budget, FORK_BASE_DIRECTORY);
  const userManifest = parseSnapshotManifest(user, options.forkId, 'user fork');
  const manifestRaw = parseManifestObject(user.files.get('manifest.json'), 'user fork');
  if (!userManifest.forkedFrom) {
    throw new Error(`Extension ${options.forkId} is not a fork.`);
  }

  const origin = parseForkedFrom(userManifest.forkedFrom);
  if (!userManifest.replaces?.includes(origin.id)) {
    throw new Error(`Fork ${options.forkId} must replace ${origin.id}.`);
  }

  const builtinDirectory = containedChild(builtinRoot, origin.id);
  if (await entryKind(builtinDirectory) !== 'directory') {
    throw new Error(`Fork ${options.forkId} references an unknown built-in: ${origin.id}`);
  }

  const base = await readRegularTree(baseDirectory, budget);
  const current = await readRegularTree(builtinDirectory, budget);
  const baseManifest = parseSnapshotManifest(base, origin.id, 'fork base');
  if (baseManifest.version !== origin.version) {
    throw new Error(
      `Fork ${options.forkId} records ${origin.id}@${origin.version}, but its base is version ${baseManifest.version}.`
    );
  }
  const currentManifest = parseSnapshotManifest(current, origin.id, 'shipped built-in');
  if (currentManifest.version === origin.version) {
    throw new Error(`Fork ${options.forkId} is already based on the shipped ${origin.id}@${origin.version}.`);
  }

  return {
    forkId: options.forkId,
    forkedFrom: origin.id,
    baseVersion: origin.version,
    currentVersion: currentManifest.version,
    manifestRaw,
    user,
    base,
    current
  };
}

function parseForkedFrom(value: string): { id: string; version: string } {
  const separator = value.lastIndexOf('@');
  const id = value.slice(0, separator);
  const version = value.slice(separator + 1);
  if (separator < 1 || !EXTENSION_ID.test(id) || !EXTENSION_VERSION.test(version)) {
    throw new Error(`Malformed forkedFrom value: ${value}`);
  }
  return { id, version };
}

function parseSnapshotManifest(
  snapshot: TreeSnapshot,
  expectedId: string,
  label: string
): ExtensionManifest {
  const raw = parseManifestObject(snapshot.files.get('manifest.json'), label);
  const parsed = parseManifest(raw);
  if (!parsed.ok) throw new Error(`Invalid ${label} manifest: ${parsed.error}`);
  if (parsed.manifest.id !== expectedId) {
    throw new Error(`${label} manifest id ${parsed.manifest.id} does not match ${expectedId}.`);
  }
  return parsed.manifest;
}

function parseManifestObject(data: Buffer | undefined, label: string): Record<string, unknown> {
  if (!data) throw new Error(`${label} is missing manifest.json.`);
  let raw: unknown;
  try {
    raw = JSON.parse(data.toString('utf8'));
  } catch {
    throw new Error(`Invalid ${label} manifest JSON.`);
  }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new Error(`Invalid ${label} manifest.`);
  }
  return raw as Record<string, unknown>;
}

async function readRegularTree(
  root: string,
  budget: CopyBudget,
  excludedTopLevel?: string
): Promise<TreeSnapshot> {
  const rootStats = await fs.lstat(root);
  if (rootStats.isSymbolicLink() || !rootStats.isDirectory()) {
    throw new Error(`Extension rebase requires a regular directory: ${root}`);
  }

  const directories: string[] = [];
  const files = new Map<string, Buffer>();
  async function walk(directory: string, relativeDirectory: string): Promise<void> {
    const entries = (await fs.readdir(directory, { withFileTypes: true }))
      .sort((a, b) => compareText(a.name, b.name));
    for (const entry of entries) {
      if (!relativeDirectory && entry.name === excludedTopLevel) continue;
      const relative = relativeDirectory
        ? `${relativeDirectory}/${entry.name}`
        : entry.name;
      const full = path.join(directory, entry.name);
      const stats = await fs.lstat(full);
      if (stats.isSymbolicLink()) {
        throw new Error(`Extension rebase does not allow symbolic links: ${relative}`);
      }
      if (stats.isDirectory()) {
        directories.push(relative);
        await walk(full, relative);
        continue;
      }
      if (!stats.isFile()) {
        throw new Error(`Extension rebase does not allow special files: ${relative}`);
      }
      const data = await fs.readFile(full);
      budget.files += 1;
      budget.bytes += data.byteLength;
      if (budget.files > MAX_FILES || budget.bytes > MAX_BYTES) {
        throw new Error('Extension rebase exceeds the safe copy limits.');
      }
      files.set(relative, data);
    }
  }
  await walk(root, '');
  return { directories, files };
}

async function writeTree(destination: string, snapshot: TreeSnapshot): Promise<void> {
  await fs.mkdir(destination, { recursive: true });
  for (const relative of snapshot.directories) {
    await fs.mkdir(path.join(destination, ...relative.split('/')), { recursive: true });
  }
  for (const [relative, data] of snapshot.files) {
    const output = path.join(destination, ...relative.split('/'));
    await fs.mkdir(path.dirname(output), { recursive: true });
    await fs.writeFile(output, data, { mode: 0o600 });
  }
}

function changedFiles(before: Map<string, Buffer>, after: Map<string, Buffer>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])]
    .filter((relative) => !before.get(relative)?.equals(after.get(relative) ?? Buffer.alloc(0))
      || !before.has(relative)
      || !after.has(relative))
    .sort(compareText);
}

async function entryKind(target: string): Promise<'missing' | 'directory' | 'other'> {
  try {
    const stats = await fs.lstat(target);
    return !stats.isSymbolicLink() && stats.isDirectory() ? 'directory' : 'other';
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 'missing';
    throw error;
  }
}

function validateAbsoluteDirectories(...directories: string[]): void {
  if (directories.some((directory) => typeof directory !== 'string' || !path.isAbsolute(directory))) {
    throw new Error('Extension rebase paths must be absolute.');
  }
}

function assertSeparateRoots(staging: string, user: string, builtin: string): void {
  const stageRoot = path.resolve(staging);
  for (const sourceRoot of [path.resolve(user), path.resolve(builtin)]) {
    if (isContained(stageRoot, sourceRoot) || isContained(sourceRoot, stageRoot)) {
      throw new Error('Extension rebase staging must not overlap an extension source root.');
    }
  }
}

function containedChild(root: string, child: string): string {
  const result = path.resolve(root, child);
  if (!isContained(root, result) || result === path.resolve(root)) {
    throw new Error('Extension rebase path escapes its root.');
  }
  return result;
}

function isContained(root: string, candidate: string): boolean {
  const relative = path.relative(path.resolve(root), path.resolve(candidate));
  return relative === '' || (!relative.startsWith(`..${path.sep}`) && relative !== '..' && !path.isAbsolute(relative));
}

function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
