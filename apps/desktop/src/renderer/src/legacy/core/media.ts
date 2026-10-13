import { sha256HexOf } from '../../../../shared/sha256';
import { resolveContent } from './content-properties';
import { clipCuesToEnd, rebaseCuesForStart } from '../../captions/model';
/* Ported from js/core/media.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { bridge as hostBridge } from '../../kernel/bridge';

export function install(PM: PMRegistry): void {
const DB_NAME: any = 'powermove-media';
const DB_VERSION: any = 2;
const STORE: any = 'assets';
const SAMPLE_BYTES: any = 64 * 1024;
let dbPromise: any = null;

function openDB() {
  if (!window.indexedDB) return Promise.resolve(null);
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve: any) => {
    let settled: any = false;
    let timer: any = 0;
    const finish: any = (value: any) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    let req: any;
    try { req = window.indexedDB.open(DB_NAME, DB_VERSION); }
    catch (e: any) { finish(null); return; }
    timer = setTimeout(() => { dbPromise = null; finish(null); }, 5000);
    req.onupgradeneeded = () => {
      const db: any = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE, { keyPath: 'id' });
    };
    req.onsuccess = () => {
      const db: any = req.result;
      db.onversionchange = () => { try { db.close(); } catch (e: any) { } dbPromise = null; };
      finish(db);
    };
    req.onerror = () => { dbPromise = null; finish(null); };
    /* A stale tab may briefly hold the previous schema. Keep waiting for it to
       close instead of permanently disabling persistence for this session. */
    req.onblocked = () => {};
  });
  return dbPromise;
}

async function request(mode: any, action: any): Promise<{ ok: boolean; value: any }> {
  const db: any = await openDB();
  if (!db) return { ok: false, value: null };
  return new Promise((resolve: any) => {
    let settled: any = false;
    const finish: any = (result: any) => { if (!settled) { settled = true; resolve(result); } };
    let tx: any, req: any;
    try {
      tx = db.transaction(STORE, mode);
      req = action(tx.objectStore(STORE));
    } catch (e: any) { finish({ ok: false, value: null }); return; }
    let value: any = true;
    if (req) {
      // Request success is not a committed write. A close/abort can still
      // discard the transaction after this event.
      req.onsuccess = () => { value = req.result; };
      req.onerror = () => finish({ ok: false, value: null });
    }
    tx.oncomplete = () => finish({ ok: true, value });
    tx.onerror = () => finish({ ok: false, value: null });
    tx.onabort = () => finish({ ok: false, value: null });
  });
}

const storageKey: any = (value: any) => {
  if (value && typeof value === 'object') return value.storageKey || value.id || null;
  return value || null;
};
const mediaRevision = () => crypto.randomUUID?.() ?? `media-${Date.now()}-${Math.random().toString(36).slice(2)}`;
async function readMediaRecord(value: any) {
  const key = storageKey(value);
  if (!key) return null;
  let result = await request('readonly', (store: any) => store.get(key));
  if (!result.ok) throw new Error('Media storage is unavailable. Try saving again.');
  if (!result.value && value && typeof value === 'object' && value.id && value.id !== key) {
    result = await request('readonly', (store: any) => store.get(value.id));
    if (!result.ok) throw new Error('Media storage is unavailable. Try saving again.');
  }
  return result.value;
}

PM.MediaStore = {
  async put(id: any, blob: any, meta: any = {}, savedRevision?: string) {
    const key: any = meta.storageKey || id;
    if (!key || !blob) return false;
    const result: any = await request('readwrite', (store: any) => store.put({
      id: key,
      blob,
      // A new stored payload always gets a new identity, even if a sampled
      // import fingerprint collides. Verified file imports retain their identity.
      revision: savedRevision || mediaRevision(),
      fingerprint: meta.fingerprint || null,
      size: Number(blob.size) || 0,
      type: blob.type || meta.type || '',
      at: Date.now(),
    }));
    return result.ok;
  },
  async getForSave(value: any) {
    let record = await readMediaRecord(value);
    if (!record || !(record.blob instanceof Blob)) return null;
    if (!record.revision) {
      const result = await request('readwrite', (store: any) => {
        const get = store.get(record.id);
        get.addEventListener('success', () => {
          const current = get.result;
          if (current && !current.revision) { current.revision = mediaRevision(); store.put(current); }
        });
        return get;
      });
      if (!result.ok) throw new Error('Could not prepare media for saving.');
      record = result.value;
    }
    return record?.blob instanceof Blob ? { blob: record.blob, revision: record.revision } : null;
  },
  async get(value: any) {
    const key: any = storageKey(value);
    if (!key) return null;
    let result: any = await request('readonly', (store: any) => store.get(key));
    /* Projects created before content-addressed storage keep their blob under
       the asset ID. This fallback migrates them lazily without a blocking pass. */
    if ((!result.ok || !result.value) && value && typeof value === 'object' && value.id && value.id !== key) {
      result = await request('readonly', (store: any) => store.get(value.id));
    }
    const record: any = result.ok ? result.value : null;
    return record && record.blob instanceof Blob ? record.blob : null;
  },
  async remove(value: any) {
    const key: any = storageKey(value);
    return key ? ((await request('readwrite', (store: any) => store.delete(key))) as any).ok : false;
  },
};
/* On a remote host this browser's IndexedDB is one device's cache: the host
   keeps the bytes for every device. Wrapped here, before the first project
   restore, so a miss is filled from the host. */
if (typeof window !== 'undefined' && (hostBridge() as any)?.wrapMediaStore) PM.MediaStore = (hostBridge() as any).wrapMediaStore(PM.MediaStore);

function normalizedName(value: any) {
  return String(value || '').normalize('NFKC').trim().toLocaleLowerCase();
}

async function sha256(bytes: any) {
  return sha256HexOf(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
}

async function fingerprint(file: any) {
  if (!file || typeof file.slice !== 'function') throw new Error('Media file could not be read');
  const size: any = Math.max(0, Number(file.size) || 0);
  const sample: any = Math.min(SAMPLE_BYTES, size);
  const offsets: any = [...new Set([
    0,
    Math.max(0, Math.floor((size - sample) / 2)),
    Math.max(0, size - sample),
  ])];
  const chunks: any = await Promise.all(offsets.map((offset: any) => file.slice(offset, offset + sample).arrayBuffer()));
  /* File.type varies between browser engines and native file pickers. Content
     identity must remain stable across those surfaces and after a rename. */
  const header: any = new TextEncoder().encode(`powermove-media-v2\n${size}\n`);
  const length: any = header.length + chunks.reduce((sum: any, chunk: any) => sum + chunk.byteLength, 0);
  const bytes: any = new Uint8Array(length);
  let at: any = 0;
  bytes.set(header, at); at += header.length;
  chunks.forEach((chunk: any) => { bytes.set(new Uint8Array(chunk), at); at += chunk.byteLength; });
  return `v2:${size}:${await sha256(bytes)}`;
}

async function mapBounded(items: any, limit: any, worker: any) {
  const values: any = Array.from(items || []);
  const output: any = new Array(values.length);
  let next: any = 0;
  const count: any = Math.max(1, Math.min(values.length || 1, Math.floor(Number(limit) || 1)));
  await Promise.all(Array.from({ length: count }, async () => {
    while (true) {
      const index: any = next++;
      if (index >= values.length) return;
      output[index] = await worker(values[index], index);
    }
  }));
  return output;
}

function layerLists(project: any) {
  const lists: any = [project && project.layers];
  Object.values(project && project.comps || {}).forEach((comp: any) => lists.push(comp && comp.layers));
  return lists.filter(Array.isArray);
}

function layerAssetId(layer: any) {
  return layer?.d?.asset || (layer?.type === 'extension' ? (layer?.d?.data?.object?.source?.assetId || layer?.d?.data?.assetId) : null) || null;
}

function replaceLayerAssetId(layer: any, assetId: any) {
  if (layer?.d?.asset) layer.d.asset = assetId;
  else if(layer?.d?.data?.object?.source?.assetId)layer.d.data.object.source.assetId=assetId;
  else if (layer?.type === 'extension' && layer?.d?.data?.assetId) layer.d.data.assetId = assetId;
}

function referenceCount(project: any, assetId: any) {
  let count: any = 0;
  layerLists(project).forEach((layers: any) => layers.forEach((layer: any) => {
    if (layerAssetId(layer) === assetId) count++;
    for(const texture of Object.values(layer?.d?.data?.object?.material?.maps || {}))if(texture===assetId)count++;
    for(const object of layer?.d?.data?.scene?.objects || []) {
      if(object.source?.assetId===assetId)count++;
      for(const texture of Object.values(object.material?.maps || {}))if(texture===assetId)count++;
    }
  }));
  return count;
}

function removeAsset(project: any, assetId: any, { preserveRuntime = false }: { preserveRuntime?: boolean } = {}) {
  if (!project || !project.assets || !project.assets[assetId]) return { removedLayers: 0, removedLayerIds: [] };
  const removedLayerIds: any = [];
  layerLists(project).forEach((layers: any) => {
    for (let index: any = layers.length - 1; index >= 0; index--) {
      const layer: any = layers[index];
      for(const [slot,id] of Object.entries(layer?.d?.data?.object?.material?.maps || {}))if(id===assetId)delete layer.d.data.object.material.maps[slot];
      const scene=layer?.d?.data?.scene;
      if(scene) {
        const removedObjects=new Set(scene.objects.filter((o:any)=>o.source?.assetId===assetId).map((o:any)=>o.id));
        for(let n=0;n<scene.objects.length;n++)for(const o of scene.objects)if(removedObjects.has(o.parent))removedObjects.add(o.id);
        scene.objects=scene.objects.filter((o:any)=>!removedObjects.has(o.id));
        for(const o of scene.objects)for(const [slot,id] of Object.entries(o.material?.maps || {}))if(id===assetId)delete o.material.maps[slot];
      }
      if (layerAssetId(layer) === assetId) {
        removedLayerIds.push(layer.id);
        layers.splice(index, 1);
      }
    }
  });
  const removed: any = new Set(removedLayerIds);
  layerLists(project).forEach((layers: any) => layers.forEach((layer: any) => {
    if (layer && removed.has(layer.parent)) layer.parent = null;
  }));
  delete project.assets[assetId];
  if (!preserveRuntime) PM.assets?.revokePoster?.(assetId);
  return { removedLayers: removedLayerIds.length, removedLayerIds };
}

function matchesLegacy(meta: any, identity: any) {
  if (!meta || meta.kind !== identity.kind || normalizedName(meta.name) !== normalizedName(identity.name)) return false;
  const sameSize: any = Number(meta.size) > 0 && Number(identity.size) > 0 && Number(meta.size) === Number(identity.size);
  const sameDuration: any = Number(meta.dur) > 0 && Number(identity.dur) > 0 && Math.abs(Number(meta.dur) - Number(identity.dur)) <= .05;
  return sameSize || sameDuration;
}

function match(project: any, liveAssets: any, identity: any) {
  const metas: any = Object.values(project && project.assets || {}).filter((meta: any) => meta && meta.id);
  const candidates: any = metas.filter((meta: any) =>
    (identity.fingerprint && meta.fingerprint === identity.fingerprint) ||
    (!meta.fingerprint && matchesLegacy(meta, identity)));
  if (!candidates.length) return { canonicalId: null, aliases: [] };
  candidates.sort((a: any, b: any) => {
    const score: any = (meta: any) =>
      (liveAssets && liveAssets.has(meta.id) ? 1000 : 0) +
      referenceCount(project, meta.id) * 20 +
      (meta.fingerprint === identity.fingerprint ? 8 : 0) +
      (Number(meta.size) > 0 ? 2 : 0);
    return score(b) - score(a);
  });
  return { canonicalId: candidates[0].id, aliases: candidates.slice(1).map((meta: any) => meta.id) };
}

function coalesce(project: any, canonicalId: any, aliases: any) {
  if (!project || !canonicalId || !aliases || !aliases.length) return 0;
  const retired: any = new Set(aliases.filter((id: any) => id && id !== canonicalId));
  let changed: any = 0;
  layerLists(project).forEach((layers: any) => layers.forEach((layer: any) => {
    if (retired.has(layerAssetId(layer))) { replaceLayerAssetId(layer, canonicalId); changed++; }
  }));
  retired.forEach((id: any) => {
    if (project.assets && project.assets[id]) delete project.assets[id];
    PM.assets?.revokePoster?.(id);
  });
  return changed;
}

PM.MediaImport = {
  SAMPLE_BYTES,
  normalizedName,
  fingerprint,
  storageKeyFor: (value: any) => value ? `media:${value}` : null,
  posterKeyFor: (storageKey: string) => `${storageKey}:poster`,
  mapBounded,
  match,
  coalesce,
  referenceCount,
  removeAsset,
};

/* A trim-in or split moves the layer's composition start, but the media must
   continue at the matching source time instead of restarting at zero. */
PM.MediaTiming = {
  isTimed(L: any) { return !!L && (L.type === 'audio' || L.type === 'video'); },
  rate(L: any) { return L && L.type === 'video' ? Math.max(.0001, Number(resolveContent(PM, L, PM.time).speed) || 1) : 1; },
  trimAtStart(L: any, nextFrom: any) {
    const trim: any = Math.max(0, Number(L ? resolveContent(PM, L, PM.time).trim : 0) || 0);
    return Math.max(0, trim + (Number(nextFrom) - Number(L.from || 0)) * this.rate(L));
  },
  earliestStart(L: any) {
    const trim: any = Math.max(0, Number(L ? resolveContent(PM, L, PM.time).trim : 0) || 0);
    return Math.max(0, Number(L.from || 0) - trim / this.rate(L));
  },
  /* Content patch that keeps a clip's content where it was in composition
     time when its In point moves to `nextFrom` (trim-in, a split's tail, the
     timeline's in-edge drag). Media continue at the matching source time;
     captions re-base their layer-time cues. Null for untimed content. */
  startPatch(L: any, nextFrom: any): Record<string, unknown> | null {
    if (this.isTimed(L)) return { trim: this.trimAtStart(L, nextFrom) };
    if (L?.type === 'captions' && Array.isArray(L.d?.cues)) {
      return { cues: rebaseCuesForStart(L.d.cues, Number(nextFrom) - Number(L.from || 0)) };
    }
    return null;
  },
  /* Content patch for the head of a split at composition time `cut`: content
     that now belongs to the tail leaves the head. Null when nothing changes. */
  endPatch(L: any, cut: any): Record<string, unknown> | null {
    if (L?.type === 'captions' && Array.isArray(L.d?.cues)) {
      return { cues: clipCuesToEnd(L.d.cues, Number(cut) - Number(L.from || 0)) };
    }
    return null;
  },
};
}
