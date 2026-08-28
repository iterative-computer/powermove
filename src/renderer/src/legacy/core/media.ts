/* Ported from js/core/media.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

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

async function request(mode: any, action: any) {
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

PM.MediaStore = {
  async put(id: any, blob: any, meta: any = {}) {
    const key: any = meta.storageKey || id;
    if (!key || !blob) return false;
    const result: any = await request('readwrite', (store: any) => store.put({
      id: key,
      blob,
      fingerprint: meta.fingerprint || null,
      size: Number(blob.size) || 0,
      type: blob.type || meta.type || '',
      at: Date.now(),
    }));
    return result.ok;
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

function normalizedName(value: any) {
  return String(value || '').normalize('NFKC').trim().toLocaleLowerCase();
}

async function sha256(bytes: any) {
  if (window.crypto && window.crypto.subtle) {
    const digest: any = new Uint8Array(await window.crypto.subtle.digest('SHA-256', bytes));
    return [...digest].map((value: any) => value.toString(16).padStart(2, '0')).join('');
  }
  /* Deterministic fallback for older WebViews. SHA-256 is preferred; this path
     still keeps imports functional without reading the whole file. */
  let hash: any = 2166136261;
  for (const value of bytes) { hash ^= value; hash = Math.imul(hash, 16777619); }
  return (hash >>> 0).toString(16).padStart(8, '0');
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

function referenceCount(project: any, assetId: any) {
  let count: any = 0;
  layerLists(project).forEach((layers: any) => layers.forEach((layer: any) => {
    if (layer && layer.d && layer.d.asset === assetId) count++;
  }));
  return count;
}

function removeAsset(project: any, assetId: any) {
  if (!project || !project.assets || !project.assets[assetId]) return { removedLayers: 0, removedLayerIds: [] };
  const removedLayerIds: any = [];
  layerLists(project).forEach((layers: any) => {
    for (let index: any = layers.length - 1; index >= 0; index--) {
      const layer: any = layers[index];
      if (layer && layer.d && layer.d.asset === assetId) {
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
    if (layer && layer.d && retired.has(layer.d.asset)) { layer.d.asset = canonicalId; changed++; }
  }));
  retired.forEach((id: any) => { if (project.assets && project.assets[id]) delete project.assets[id]; });
  return changed;
}

PM.MediaImport = {
  SAMPLE_BYTES,
  normalizedName,
  fingerprint,
  storageKeyFor: (value: any) => value ? `media:${value}` : null,
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
  rate(L: any) { return L && L.type === 'video' ? Math.max(.0001, Number(L.d && L.d.speed) || 1) : 1; },
  trimAtStart(L: any, nextFrom: any) {
    const trim: any = Math.max(0, Number(L && L.d && L.d.trim) || 0);
    return Math.max(0, trim + (Number(nextFrom) - Number(L.from || 0)) * this.rate(L));
  },
  earliestStart(L: any) {
    const trim: any = Math.max(0, Number(L && L.d && L.d.trim) || 0);
    return Math.max(0, Number(L.from || 0) - trim / this.rate(L));
  },
};
}
