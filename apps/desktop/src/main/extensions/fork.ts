import { randomBytes } from 'node:crypto';
import * as fs from 'node:fs/promises';
import path from 'node:path';

import { EXTENSION_ID, parseManifest } from '../../shared/extensions';

export interface ForkBuiltinExtensionOptions {
  resourcesDir: string;
  id: string;
  targetDir: string;
  forkId?: string;
}

export interface ForkBuiltinExtensionResult {
  forkId: string;
  dir: string;
  files: string[];
  forkedFrom: string;
}

/**
 * Copies a shipped extension into a writable extension root. The pristine
 * `.forked-from` tree is deliberately retained as the merge base for a future
 * three-way "update fork" operation when the shipped built-in changes.
 */
export async function forkBuiltinExtension({
  resourcesDir,
  id: rawId,
  targetDir,
  forkId: rawForkId
}: ForkBuiltinExtensionOptions): Promise<ForkBuiltinExtensionResult> {
  const id = validateId(rawId, 'built-in extension');
  const forkId = validateId(rawForkId ?? `${id}-fork`, 'fork');
  if (forkId === id) throw new Error('Fork id must differ from the built-in extension id.');
  if (!path.isAbsolute(resourcesDir) || !path.isAbsolute(targetDir)) {
    throw new Error('Extension fork roots must be absolute paths.');
  }

  const resourcesRoot = await fs.realpath(resourcesDir);
  const source = path.join(resourcesRoot, id);
  await requireImmediateDirectory(resourcesRoot, source, 'Built-in extension');

  const manifestPath = path.join(source, 'manifest.json');
  const manifestMetadata = await fs.lstat(manifestPath);
  if (manifestMetadata.isSymbolicLink() || !manifestMetadata.isFile()) {
    throw new Error(`Built-in extension ${id} manifest is not a regular file.`);
  }
  const manifestText = await fs.readFile(manifestPath, 'utf8');
  let rawManifest: unknown;
  try {
    rawManifest = JSON.parse(manifestText) as unknown;
  } catch {
    throw new Error(`Built-in extension ${id} has invalid manifest JSON.`);
  }
  const parsed = parseManifest(rawManifest);
  if (!parsed.ok) throw new Error(`Built-in extension ${id} has an invalid manifest: ${parsed.error}`);
  if (parsed.manifest.id !== id) {
    throw new Error(`Built-in extension directory ${id} does not match manifest id ${parsed.manifest.id}.`);
  }

  await fs.mkdir(targetDir, { recursive: true });
  const targetRoot = await fs.realpath(targetDir);
  const destination = path.join(targetRoot, forkId);
  await assertMissing(destination);

  const forkedFrom = `${id}@${parsed.manifest.version}`;
  const rewritten = {
    ...(rawManifest as Record<string, unknown>),
    id: forkId,
    name: `${parsed.manifest.name} (fork)`,
    replaces: [...new Set([...(parsed.manifest.replaces ?? []), id])],
    forkedFrom,
    author: parsed.manifest.author ?? 'user'
  };
  const rewrittenParse = parseManifest(rewritten);
  if (!rewrittenParse.ok) throw new Error(`Fork manifest would be invalid: ${rewrittenParse.error}`);

  const temporary = path.join(targetRoot, `.${forkId}.tmp-${randomBytes(8).toString('hex')}`);
  await fs.mkdir(temporary);
  try {
    const files = await copySourceTree(source, temporary);
    const pristine = path.join(temporary, '.forked-from');
    await fs.mkdir(pristine);
    await copySourceTree(source, pristine);
    await fs.writeFile(path.join(temporary, 'manifest.json'), `${JSON.stringify(rewritten, null, 2)}\n`, 'utf8');
    await fs.rename(temporary, destination);
    return { forkId, dir: destination, files, forkedFrom };
  } catch (error) {
    await fs.rm(temporary, { recursive: true, force: true }).catch(() => undefined);
    throw error;
  }
}

async function copySourceTree(source: string, destination: string, prefix = ''): Promise<string[]> {
  const copied: string[] = [];
  const entries = await fs.readdir(source, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (entry.name === '.DS_Store' || entry.name.startsWith('.') && entry.isDirectory()) continue;
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory() && entry.name === '__tests__') continue;
    if (entry.isFile() && entry.name.endsWith('.test.ts')) continue;

    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    const metadata = await fs.lstat(sourcePath);
    if (metadata.isSymbolicLink()) throw new Error(`Built-in extension contains an unsupported symbolic link: ${relative}`);
    if (metadata.isDirectory()) {
      await fs.mkdir(destinationPath);
      copied.push(...await copySourceTree(sourcePath, destinationPath, relative));
    } else if (metadata.isFile()) {
      await fs.copyFile(sourcePath, destinationPath, fs.constants.COPYFILE_EXCL);
      copied.push(relative);
    } else {
      throw new Error(`Built-in extension contains an unsupported file: ${relative}`);
    }
  }
  return copied;
}

function validateId(value: string, label: string): string {
  if (typeof value !== 'string' || !EXTENSION_ID.test(value)) throw new Error(`Invalid ${label} id.`);
  return value;
}

async function requireImmediateDirectory(root: string, candidate: string, label: string): Promise<void> {
  let metadata;
  try {
    metadata = await fs.lstat(candidate);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') throw new Error(`${label} does not exist.`);
    throw error;
  }
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) throw new Error(`${label} is not a regular directory.`);
  const resolved = await fs.realpath(candidate);
  if (path.dirname(resolved) !== root) throw new Error(`${label} path escapes its root.`);
}

async function assertMissing(candidate: string): Promise<void> {
  try {
    await fs.lstat(candidate);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw error;
  }
  throw new Error('Extension fork target already exists.');
}
