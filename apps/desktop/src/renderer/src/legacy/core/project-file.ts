import { LIMITS } from '../../../../shared/ipc';
import { decodeProjectContainer, encodeProjectContainer } from '../../../../shared/project-container';

type MediaStore = { get(asset: any): Promise<Blob | null>; put(id: string, blob: Blob, metadata: any): Promise<boolean> };

/** Saved files own their media; session-local blob URLs cannot survive reopening. */
export async function packProjectFile(snapshot: string | any, store: MediaStore): Promise<Uint8Array> {
  const document = typeof snapshot === 'string' ? JSON.parse(snapshot) : snapshot;
  const project = document.proj || document;
  const media: Array<{ id: string; type: string; data: Uint8Array }> = [];
  let estimatedBytes = new TextEncoder().encode(JSON.stringify(document)).length;
  for (const [id, asset] of Object.entries<any>(project.assets || {})) {
    const blob = await store.get(asset);
    if (!blob) throw new Error(`The original media for “${asset.name || id}” is missing. Reimport it before saving.`);
    estimatedBytes += blob.size + 1024;
    if (estimatedBytes > LIMITS.fileSaveBytes) throw new Error('This project is too large to save as one file (256 MB maximum).');
    const data = new Uint8Array(await blob.arrayBuffer());
    media.push({ id, type: blob.type || asset.type || '', data });
  }
  return encodeProjectContainer(document, media);
}

export async function restoreProjectFileMedia(document: any, store: MediaStore): Promise<void> {
  if (document?.containerMedia) {
    const project = document.proj || document;
    for (const source of document.containerMedia as Array<{ id: string; type: string; data: Uint8Array }>) {
      const asset = project.assets?.[source.id];
      if (!asset || !(source.data instanceof Uint8Array)) continue;
      if (!await store.put(source.id, new Blob([new Uint8Array(source.data)], { type: source.type }), asset)) {
        throw new Error(`Could not restore ${asset.name || source.id}. Check available disk space.`);
      }
    }
    return;
  }
  if (!document.media) return; // Older files still use the local media store.
  const project = document.proj || document;
  const entries: Array<{ id: string; asset: any; blob: Blob }> = [];
  let bytes = 0;
  for (const [id, source] of Object.entries<any>(document.media)) {
    const asset = project.assets?.[id];
    if (!asset) continue;
    if (!source || typeof source.type !== 'string' || typeof source.data !== 'string') {
      throw new Error(`Invalid saved media: ${asset.name || id}`);
    }
    bytes += source.data.length;
    if (bytes > LIMITS.fileSaveBytes) throw new Error('Saved media exceeds the project file size limit.');
    // A repeated four-character capture can overflow the regex stack on video
    // files. This flat scan stays safe for the full supported project size.
    if (source.data.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(source.data)) {
      throw new Error(`Invalid saved media: ${asset.name || id}`);
    }
    const binary = atob(source.data);
    const data = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) data[index] = binary.charCodeAt(index);
    entries.push({ id, asset, blob: new Blob([data], { type: source.type }) });
  }
  for (const { id, asset, blob } of entries) {
    if (!await store.put(id, blob, asset)) throw new Error(`Could not restore ${asset.name || id}. Check available disk space.`);
  }
}

export function unpackProjectFile(input: string | Uint8Array | ArrayBuffer): any {
  const decoded = decodeProjectContainer(input);
  if (decoded.media.length) decoded.document.containerMedia = decoded.media;
  return decoded.document;
}
