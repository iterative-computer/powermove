import { lstat, readFile, readdir } from 'node:fs/promises';
import path from 'node:path';

import {
  MANIFEST_LIMITS,
  parseManifest,
  type ExtensionManifest
, EXTENSION_ID } from '../../shared/extensions';

export interface DiscoveredExtension {
  id: string;
  scope: 'user' | 'project';
  dir: string;
  manifest: ExtensionManifest | null;
  error?: string;
}

interface ScanDirectory {
  dir: string;
  scope: 'user' | 'project';
}

class SourceFileLimitError extends Error {}

/**
 * Discovers immediate extension children without allowing symlinks to escape an
 * extension root. Missing or unreadable scan roots are ignored so one optional
 * scope cannot prevent the other from loading.
 */
export async function scanExtensionDirs(dirs: ScanDirectory[]): Promise<DiscoveredExtension[]> {
  const discovered = (await Promise.all(dirs.map(scanDirectory))).flat();
  return discovered.sort(compareDiscovered);
}

async function scanDirectory({ dir, scope }: ScanDirectory): Promise<DiscoveredExtension[]> {
  let names: string[];
  try {
    names = await readdir(dir);
  } catch {
    return [];
  }

  const candidates = await Promise.all(
    names
      .filter((name) => EXTENSION_ID.test(name))
      .map((name) => scanCandidate(path.resolve(dir, name), name, scope))
  );
  return candidates.filter((candidate): candidate is DiscoveredExtension => candidate !== null);
}

async function scanCandidate(
  candidateDir: string,
  directoryName: string,
  scope: 'user' | 'project'
): Promise<DiscoveredExtension | null> {
  try {
    const candidateStats = await lstat(candidateDir);
    if (candidateStats.isSymbolicLink() || !candidateStats.isDirectory()) return null;

    const manifestPath = path.join(candidateDir, 'manifest.json');
    let manifestStats;
    try {
      manifestStats = await lstat(manifestPath);
    } catch {
      return null;
    }
    if (manifestStats.isSymbolicLink() || !manifestStats.isFile()) return null;

    let raw: unknown;
    try {
      raw = JSON.parse(await readFile(manifestPath, 'utf8'));
    } catch (error) {
      return invalidDiscovery(directoryName, scope, candidateDir, errorMessage(error));
    }

    const parsed = parseManifest(raw);
    if (!parsed.ok) return invalidDiscovery(directoryName, scope, candidateDir, parsed.error);
    if (parsed.manifest.id !== directoryName) {
      return invalidDiscovery(
        directoryName,
        scope,
        candidateDir,
        `manifest id "${parsed.manifest.id}" must match directory name "${directoryName}"`
      );
    }

    await enforceSourceFileLimit(candidateDir);
    return {
      id: parsed.manifest.id,
      scope,
      dir: candidateDir,
      manifest: parsed.manifest
    };
  } catch (error) {
    return invalidDiscovery(directoryName, scope, candidateDir, errorMessage(error));
  }
}

async function enforceSourceFileLimit(root: string): Promise<void> {
  let files = 0;
  const pending = [root];

  while (pending.length > 0) {
    const directory = pending.pop();
    if (!directory) break;

    const names = await readdir(directory);
    for (const name of names) {
      if (name.startsWith('.')) continue;
      const itemPath = path.join(directory, name);
      const stats = await lstat(itemPath);
      if (stats.isSymbolicLink()) continue;
      if (stats.isDirectory()) {
        pending.push(itemPath);
        continue;
      }

      files += 1;
      if (files > MANIFEST_LIMITS.sourceFiles) {
        throw new SourceFileLimitError(
          `extension has more than ${MANIFEST_LIMITS.sourceFiles} source files`
        );
      }
    }
  }
}

function invalidDiscovery(
  id: string,
  scope: 'user' | 'project',
  dir: string,
  error: string
): DiscoveredExtension {
  return {
    id,
    scope,
    dir,
    manifest: null,
    error: error.slice(0, MANIFEST_LIMITS.errorChars)
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function compareDiscovered(a: DiscoveredExtension, b: DiscoveredExtension): number {
  const scopeOrder = { user: 0, project: 1 } as const;
  return (
    scopeOrder[a.scope] - scopeOrder[b.scope] ||
    compareText(a.id, b.id) ||
    compareText(a.dir, b.dir)
  );
}

function compareText(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}
