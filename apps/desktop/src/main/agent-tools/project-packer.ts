/** Agent `save_project` with the new engine: the host packs the project the
 * agent sees, from original media and linked originals, without a dialog or
 * renderer upload. The file is today's agent save, byte for byte: the same
 * PMV3 container (shared/project-container.ts) Export › Project writes
 * through renderer/src/legacy/core/project-file.ts `packProjectFileBlob`,
 * with the same document, media order and skip rules. */
import { randomUUID } from 'node:crypto';
import { openAsBlob, type Stats } from 'node:fs';
import { open, rename, stat, unlink } from 'node:fs/promises';
import path from 'node:path';
import { agentExportDestination, withAgentExport } from '../agent-export';
import { syncDirectory, syncFile } from '../durable-file';
import { embedLinkedAssets, fileAssets, isLinkedAsset } from '../../shared/project-assets';
import { mimeOf } from '../../shared/media-mime';
import { encodeProjectContainerBlob } from '../../shared/project-container';
import { suggestedName } from '../engine/agent-jobs';

export type PackProjectOptions = {
  directory: string;
  resolveAsset(asset: string): Promise<{ path: string; type?: string } | null>;
};

type Source = { id: string; type: string; path: string; info: Stats; data: Blob };

export async function packAgentProject(project: Record<string, any>, options: PackProjectOptions, documentValue: unknown = project): Promise<string> {
  // Capture before file operations: the caller may continue editing immediately.
  const saved = JSON.parse(JSON.stringify(documentValue)) as Record<string, any>;
  const sources: Source[] = [];
  for (const [id, asset] of Object.entries<any>(fileAssets(saved))) {
    const source = await readable(options, id, asset);
    // Missing sources (including deleted assets retained by undo/redo, and
    // offline linked originals) must not prevent saving edits. Their metadata
    // stays in the document and history, as today.
    if (!source) continue;
    sources.push({ id, type: source.data.type || String(asset?.type || ''), ...source });
  }
  // Linked originals that were embedded become ordinary assets; the others stay linked.
  embedLinkedAssets(saved, new Set(sources.map(item => item.id)));
  const container = encodeProjectContainerBlob([new TextEncoder().encode(JSON.stringify(saved))], sources.map(({ id, type, data }) => ({ id, type, data })));
  const owner = {};
  return withAgentExport(owner, options.directory, async () => {
    const destination = await agentExportDestination(owner, `${suggestedName(project, undefined)}.pmv`);
    if (!destination) throw new Error('project-save-unavailable: The host could not reserve an output.');
    await writeAtomically(destination, container, sources);
    return destination;
  });
}

/** The asset's original, or null when today's packer would find no bytes for it. */
async function readable(options: PackProjectOptions, id: string, asset: any): Promise<{ path: string; info: Stats; data: Blob } | null> {
  const resolved = await options.resolveAsset(id).catch(() => null);
  if (!resolved) return null;
  const info = await stat(resolved.path).catch(() => null);
  if (!info?.isFile()) return null;
  // A file-backed Blob refuses to read once its file changes (NotReadableError).
  const fallbackType = String(asset?.type || mimeOf(asset?.name));
  const byHash = typeof asset?.contentHash === 'string' && /^[a-f0-9]{64}$/.test(asset.contentHash);
  const type = byHash || isLinkedAsset(asset) ? fallbackType : resolved.type || fallbackType;
  const data = await openAsBlob(resolved.path, { type }).catch(() => null);
  return data && data.size === info.size ? { path: resolved.path, info, data } : null;
}

/** Writes beside the reserved output and replaces it only once every byte is
 * on disk (main/project-files.ts `atomicWrite`). An original that changes
 * while it is copied fails the save with a named error and leaves no file. */
async function writeAtomically(destination: string, container: Blob, sources: Source[]): Promise<void> {
  const temporary = path.join(path.dirname(destination), `.${path.basename(destination)}.${randomUUID()}.tmp`);
  try {
    const handle = await open(temporary, 'wx', 0o600);
    try {
      try {
        const reader = container.stream().getReader();
        for (let next = await reader.read(); !next.done; next = await reader.read()) await handle.write(next.value);
      } catch (error) {
        if ((error as Error)?.name !== 'NotReadableError') throw error;
        await assertUnchanged(sources);
        throw new Error('mediaChanged: Original media changed during the project save.');
      }
      await assertUnchanged(sources);
      await syncFile(handle);
    } finally { await handle.close(); }
    await rename(temporary, destination);
    await syncDirectory(path.dirname(destination));
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    throw error;
  }
}

async function assertUnchanged(sources: Source[]): Promise<void> {
  for (const source of sources) {
    const now = await stat(source.path).catch(() => null);
    if (!now || now.size !== source.info.size || now.mtimeMs !== source.info.mtimeMs || now.ino !== source.info.ino || now.dev !== source.info.dev) {
      throw new Error(`mediaChanged: Original media ${source.id} changed during the project save.`);
    }
  }
}
