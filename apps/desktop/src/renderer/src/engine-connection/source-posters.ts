/** Persist engine source posters only after import/replacement has bound its original. */
import { writeDocument } from '../engine-doc/write';

export async function persistSourcePoster(PM: any, asset: any, publish: (id: string, blob: Blob) => void): Promise<Blob | null> {
  const source = PM.EngineSourceMedia;
  if (!source || !['image', 'video'].includes(asset?.kind)) return null;
  const project = PM.proj, meta = project.assets?.[asset.id];
  if (!meta?.storageKey) return null;
  try {
    await PM.HostMedia?.flushBindings?.(project);
    const blob = await source.poster(asset.id);
    if (!blob || PM.proj !== project || project.assets[asset.id] !== meta || PM.assets.get(asset.id) !== asset) return null;
    const key = PM.MediaImport.posterKeyFor(meta.storageKey);
    const stored = await PM.MediaStore.put(key, blob, { storageKey: key, type: blob.type });
    if (!stored || PM.proj !== project || project.assets[asset.id] !== meta || PM.assets.get(asset.id) !== asset) return null;
    asset.poster = true;
    writeDocument(PM, [['assets', asset.id, 'poster']], () => { meta.poster = true; });
    publish(asset.id, blob); PM.bus.emit('assets');
    return blob;
  } catch (error) {
    console.warn('Could not capture the engine source poster', error);
    return null;
  }
}
