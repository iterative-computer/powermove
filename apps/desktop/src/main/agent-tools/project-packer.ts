/** Agent delivery uses the host's PMV4 writer and original media, including
 * linked originals. No dialog, renderer upload or live exporter is involved. */
import { open, realpath, stat } from 'node:fs/promises';
import { agentExportDestination, withAgentExport } from '../agent-export';
import { fileVersion, publishFile } from '../durable-file';
import { FILE_CHUNK_BYTES } from '../file-upload';
import { embedLinkedAssets, fileAssets } from '../../shared/project-assets';
import { ProjectSave } from '../project-save';
import { suggestedName } from '../engine/agent-jobs';
import type { SaveMedia } from '../../shared/project-incremental';

export type PackProjectOptions = {
  directory: string;
  resolveAsset(asset: string): Promise<{ path: string } | null>;
};

export async function packAgentProject(project: Record<string, any>, options: PackProjectOptions, documentValue: unknown = project): Promise<string> {
  // Capture before file operations: the caller may continue editing immediately.
  const saved = JSON.parse(JSON.stringify(documentValue)) as Record<string, any>;
  const media: SaveMedia[] = [];
  const sources = new Map<string, { path: string; size: number; mtimeMs: number; ino: number; dev: number }>();
  for (const [id, asset] of Object.entries<any>(fileAssets(saved))) {
    const source = await options.resolveAsset(id);
    if (!source) throw new Error(`mediaOffline: Cannot save a self-contained project without original media ${asset?.name ?? id} (${id}).`);
    const info = await stat(source.path);
    if (!info.isFile()) throw new Error(`mediaOffline: The original media ${id} is not a regular file.`);
    sources.set(id, { path: source.path, size: info.size, mtimeMs: info.mtimeMs, ino: info.ino, dev: info.dev });
    media.push({ id, revision: String(asset?.revision ?? asset?.fingerprint ?? '1'), type: String(asset?.type ?? asset?.mimeType ?? 'application/octet-stream'), length: info.size });
  }
  embedLinkedAssets(saved, new Set(media.map(item => item.id)));
  return reserveAndPack(project, Buffer.from(JSON.stringify(saved)), media, sources, options.directory);
}

async function reserveAndPack(project: Record<string, any>, document: Buffer, media: SaveMedia[], sources: Map<string, { path: string; size: number; mtimeMs: number; ino: number; dev: number }>, directory: string): Promise<string> {
  const owner = {};
  return withAgentExport(owner, directory, async () => {
    const destination = await agentExportDestination(owner, `${suggestedName(project, undefined)}.pmv`);
    if (!destination) throw new Error('project-save-unavailable: The host could not reserve an output.');
    const reserved = await fileVersion(destination);
    const save = await ProjectSave.create(destination, reserved, media, document.length, async (temporary, hash) => {
      await publishFile(temporary, destination, reserved, hash);
      return realpath(destination);
    }, () => {}, false);
    try {
      for (let offset = 0; offset < document.length; offset += FILE_CHUNK_BYTES) await save.write(null, document.subarray(offset, offset + FILE_CHUNK_BYTES));
      for (const id of save.required) {
        const expected = sources.get(id)!;
        const source = await open(expected.path, 'r');
        try {
          const unchanged = async () => {
            const now = await source.stat();
            if (now.size !== expected.size || now.mtimeMs !== expected.mtimeMs || now.ino !== expected.ino || now.dev !== expected.dev) throw new Error(`mediaChanged: Original media ${id} changed during the project save.`);
          };
          await unchanged();
          const chunk = Buffer.allocUnsafe(FILE_CHUNK_BYTES);
          for (let offset = 0; offset < expected.size;) {
            const { bytesRead } = await source.read(chunk, 0, Math.min(chunk.length, expected.size - offset), offset);
            if (!bytesRead) throw new Error(`mediaChanged: Original media ${id} was truncated during the project save.`);
            await save.write(id, chunk.subarray(0, bytesRead));
            offset += bytesRead;
          }
          await unchanged();
        } finally { await source.close(); }
      }
      return await save.finish();
    } finally { await save.dispose(); }
  });
}
