import { readdir, realpath, readFile, stat } from 'node:fs/promises';
import path from 'node:path';

import type { ArtifactFile } from '../../shared/ipc';
import { LIMITS } from '../../shared/ipc';

const MIME_BY_EXTENSION: Readonly<Record<string, string>> = {
  '.aac': 'audio/aac',
  '.aif': 'audio/aiff',
  '.aiff': 'audio/aiff',
  '.avif': 'image/avif',
  '.bmp': 'image/bmp',
  '.css': 'text/css',
  '.csv': 'text/csv',
  '.flac': 'audio/flac',
  '.gif': 'image/gif',
  '.htm': 'text/html',
  '.html': 'text/html',
  '.jpeg': 'image/jpeg',
  '.jpg': 'image/jpeg',
  '.js': 'text/javascript',
  '.json': 'application/json',
  '.m4a': 'audio/mp4',
  '.m4v': 'video/mp4',
  '.map': 'application/json',
  '.md': 'text/markdown',
  '.mov': 'video/quicktime',
  '.mp3': 'audio/mpeg',
  '.mp4': 'video/mp4',
  '.oga': 'audio/ogg',
  '.ogg': 'audio/ogg',
  '.ogv': 'video/ogg',
  '.otf': 'font/otf',
  '.pdf': 'application/pdf',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.tif': 'image/tiff',
  '.tiff': 'image/tiff',
  '.ttf': 'font/ttf',
  '.ts': 'text/typescript',
  '.txt': 'text/plain',
  '.wasm': 'application/wasm',
  '.wav': 'audio/wav',
  '.webm': 'video/webm',
  '.webp': 'image/webp',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.xml': 'application/xml',
  '.zip': 'application/zip'
};

const ENCODED_SEPARATOR = /%(?:2f|5c)/i;

export function mimeTypeForPath(filePath: string): string {
  return MIME_BY_EXTENSION[path.extname(filePath).toLowerCase()] ?? 'application/octet-stream';
}

function rejectUnsafeRelativePath(relativePath: string): void {
  if (relativePath.length === 0) throw new Error('Artifact path is empty.');
  if (relativePath.includes('\0')) throw new Error('Artifact path contains a NUL byte.');
  if (relativePath.includes('\\')) throw new Error('Artifact path contains a backslash.');
  if (ENCODED_SEPARATOR.test(relativePath)) throw new Error('Artifact path contains an encoded separator.');
  if (path.isAbsolute(relativePath)) throw new Error('Artifact path must be relative.');

  const segments = relativePath.split('/');
  if (segments.some((segment) => segment === '..')) {
    throw new Error('Artifact path contains a parent-directory segment.');
  }
}

/**
 * Resolve an artifact path through the filesystem and prove that its final
 * target is a regular, size-bounded file below the real artifact root.
 */
export async function validatedArtifactPath(root: string, relativePath: string): Promise<string> {
  rejectUnsafeRelativePath(relativePath);

  const realRoot = await realpath(root);
  const candidate = await realpath(path.resolve(realRoot, relativePath));
  const containment = path.relative(realRoot, candidate);
  if (containment === '' || containment.startsWith(`..${path.sep}`) || containment === '..' || path.isAbsolute(containment)) {
    throw new Error('Artifact path escapes its workspace.');
  }

  const metadata = await stat(candidate);
  if (!metadata.isFile()) throw new Error('Artifact is not a regular file.');
  if (metadata.size > LIMITS.artifactBytes) throw new Error('Artifact is larger than 64 MB.');
  return candidate;
}

export async function readArtifact(root: string, relativePath: string): Promise<ArtifactFile> {
  const filePath = await validatedArtifactPath(root, relativePath);
  const data = await readFile(filePath);
  if (data.byteLength > LIMITS.artifactBytes) throw new Error('Artifact is larger than 64 MB.');
  return {
    name: path.basename(filePath),
    mime: mimeTypeForPath(filePath),
    data: new Uint8Array(data)
  };
}

export interface ArtifactShell {
  showItemInFolder(fullPath: string): void;
}

/** Reveal only a path which passed the same containment checks as reads. */
export async function reveal(
  root: string,
  relativePath: string,
  shellOverride?: ArtifactShell
): Promise<void> {
  const filePath = await validatedArtifactPath(root, relativePath);
  const artifactShell = shellOverride ?? (await import('electron')).shell;
  artifactShell.showItemInFolder(filePath);
}

export const revealArtifact = reveal;

export interface CollectedArtifact {
  path: string;
  name: string;
  size: number;
  mime: string;
  importToTimeline: boolean;
}

const PACKAGE_EXTENSIONS = new Set([
  '.app',
  '.bundle',
  '.framework',
  '.kext',
  '.plugin',
  '.pkg',
  '.qlgenerator',
  '.rtfd',
  '.xcodeproj',
  '.xcworkspace',
  '.playground',
  '.pages',
  '.numbers',
  '.key',
  '.band',
  '.logicx',
  '.photoslibrary',
  '.photolibrary',
  '.imovielibrary',
  '.fcpbundle'
]);

function requestedImportPaths(requested: readonly unknown[]): Set<string> {
  const result = new Set<string>();
  for (const entry of requested) {
    if (typeof entry !== 'object' || entry === null || Array.isArray(entry)) continue;
    const artifact = entry as Record<string, unknown>;
    if (artifact.importToTimeline === true && typeof artifact.path === 'string') {
      result.add(artifact.path);
    }
  }
  return result;
}

/** Walk a run directory using the same externally visible fields as Swift. */
export async function collectArtifacts(
  runDirectory: string,
  runId: string,
  requested: readonly unknown[]
): Promise<CollectedArtifact[]> {
  const imports = requestedImportPaths(requested);
  const discovered: Array<{ localPath: string; fullPath: string }> = [];

  async function walk(directory: string, prefix: string): Promise<void> {
    if (discovered.length >= 80) return;
    let entries;
    try {
      entries = await readdir(directory, { withFileTypes: true });
    } catch {
      return;
    }
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      if (discovered.length >= 80) return;
      if (entry.name.startsWith('.')) continue;
      const localPath = prefix ? `${prefix}/${entry.name}` : entry.name;
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!PACKAGE_EXTENSIONS.has(path.extname(entry.name).toLowerCase())) await walk(fullPath, localPath);
      } else if (entry.isFile()) {
        discovered.push({ localPath, fullPath });
      }
    }
  }

  await walk(runDirectory, '');
  const artifacts: CollectedArtifact[] = [];
  for (const item of discovered.sort((left, right) => left.localPath.localeCompare(right.localPath)).slice(0, 80)) {
    let metadata;
    try {
      metadata = await stat(item.fullPath);
    } catch {
      continue;
    }
    if (!metadata.isFile()) continue;
    const artifactPath = `${runId}/${item.localPath}`;
    artifacts.push({
      path: artifactPath,
      name: path.basename(item.localPath),
      size: metadata.size,
      mime: mimeTypeForPath(item.fullPath),
      importToTimeline:
        imports.has(artifactPath) || imports.has(item.localPath) || imports.has(path.basename(item.localPath))
    });
  }
  return artifacts;
}
