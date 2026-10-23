import { applyEngineStamp } from './engine-stamp';

export const isLinkedAsset = (asset: any) => asset?.storage === 'linked' && !!asset?.link && typeof asset.link.path === 'string';

/**
 * Export › Project and agent `save_project` (section 21): every linked
 * original is embedded, so the exported file is self-contained as before.
 * Embedded linked assets become copied assets in the exported document and
 * the engine stamp is recomputed, so the copy opens in either engine.
 */
export function embedLinkedAssets(document: any, embedded: ReadonlySet<string>): number {
  const assets = (document?.proj || document)?.assets;
  let changed = 0;
  for (const [id, asset] of Object.entries<any>(assets || {})) {
    if (!isLinkedAsset(asset) || !embedded.has(id)) continue;
    if (!asset.contentHash && asset.link.hash) asset.contentHash = asset.link.hash;
    if (!asset.sourcePath) asset.sourcePath = asset.link.path;
    delete asset.storage;
    delete asset.link;
    changed++;
  }
  applyEngineStamp(document?.proj || document);
  return changed;
}

/** Include media reachable only by undo/redo, such as a deleted image. */
export function fileAssets(document: any): Record<string, any> {
  const assets: Record<string, any> = Object.create(null);
  for (const entry of document.history?.entries || []) {
    for (const patch of [...(entry.forward || []), ...(entry.backward || [])]) {
      if (!patch?.exists || !Array.isArray(patch.path)) continue;
      if (!patch.path.length) Object.assign(assets, patch.value?.assets || {});
      else if (patch.path[0] === 'assets' && patch.path.length === 1) Object.assign(assets, patch.value || {});
      else if (patch.path[0] === 'assets' && patch.path.length === 2 && patch.value?.id) assets[patch.path[1]] = patch.value;
    }
  }
  return Object.assign(assets, (document.proj || document).assets || {});
}
