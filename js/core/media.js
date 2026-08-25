/* No longer loaded — superseded by src/renderer/src/legacy/core/media.ts; kept for the legacy test oracle until Phase 6. */
/* Powermove — high-performance durable media identity, storage, and timing. */
(() => {
const PM = window.PM;

const DB_NAME = 'powermove-media';
const DB_VERSION = 2;
const STORE = 'assets';
const SAMPLE_BYTES = 64 * 1024;
let dbPromise = null;

function openDB() {
  if (!window.indexedDB) return Promise.resolve(null);
  if (dbPromise) return dbPromise;
  dbPromise = new Promise(resolve => {
    let settled = false;
    let timer = 0;
    const finish = value => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    let req;
    try { req = window.indexedDB.open(DB_NAME, DB_VERSION); }
    catch (e) { finish(null); return; }
    timer = setTimeout(() => { dbPromise = null; finish(null); }, 5000);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE, { keyPath: 'id' });
    };
    req.onsuccess = () => {
      const db = req.result;
      db.onversionchange = () => { try { db.close(); } catch (e) { } dbPromise = null; };
      finish(db);
    };
    req.onerror = () => { dbPromise = null; finish(null); };
    /* A stale tab may briefly hold the previous schema. Keep waiting for it to
       close instead of permanently disabling persistence for this session. */
    req.onblocked = () => {};
  });
  return dbPromise;
}

async function request(mode, action) {
  const db = await openDB();
  if (!db) return { ok: false, value: null };
  return new Promise(resolve => {
    let settled = false;
    const finish = result => { if (!settled) { settled = true; resolve(result); } };
    let tx, req;
    try {
      tx = db.transaction(STORE, mode);
      req = action(tx.objectStore(STORE));
    } catch (e) { finish({ ok: false, value: null }); return; }
    if (req) {
      req.onsuccess = () => finish({ ok: true, value: req.result });
      req.onerror = () => finish({ ok: false, value: null });
    }
    tx.oncomplete = () => finish({ ok: true, value: true });
    tx.onerror = () => finish({ ok: false, value: null });
    tx.onabort = () => finish({ ok: false, value: null });
  });
}

const storageKey = value => {
  if (value && typeof value === 'object') return value.storageKey || value.id || null;
  return value || null;
};

PM.MediaStore = {
  async put(id, blob, meta = {}) {
    const key = meta.storageKey || id;
    if (!key || !blob) return false;
    const result = await request('readwrite', store => store.put({
      id: key,
      blob,
      fingerprint: meta.fingerprint || null,
      size: Number(blob.size) || 0,
      type: blob.type || meta.type || '',
      at: Date.now(),
    }));
    return result.ok;
  },
  async get(value) {
    const key = storageKey(value);
    if (!key) return null;
    let result = await request('readonly', store => store.get(key));
    /* Projects created before content-addressed storage keep their blob under
       the asset ID. This fallback migrates them lazily without a blocking pass. */
    if ((!result.ok || !result.value) && value && typeof value === 'object' && value.id && value.id !== key) {
      result = await request('readonly', store => store.get(value.id));
    }
    const record = result.ok ? result.value : null;
    return record && record.blob instanceof Blob ? record.blob : null;
  },
  async remove(value) {
    const key = storageKey(value);
    return key ? (await request('readwrite', store => store.delete(key))).ok : false;
  },
};

function normalizedName(value) {
  return String(value || '').normalize('NFKC').trim().toLocaleLowerCase();
}

async function sha256(bytes) {
  if (window.crypto && window.crypto.subtle) {
    const digest = new Uint8Array(await window.crypto.subtle.digest('SHA-256', bytes));
    return [...digest].map(value => value.toString(16).padStart(2, '0')).join('');
  }
  /* Deterministic fallback for older WebViews. SHA-256 is preferred; this path
     still keeps imports functional without reading the whole file. */
  let hash = 2166136261;
  for (const value of bytes) { hash ^= value; hash = Math.imul(hash, 16777619); }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

async function fingerprint(file) {
  if (!file || typeof file.slice !== 'function') throw new Error('Media file could not be read');
  const size = Math.max(0, Number(file.size) || 0);
  const sample = Math.min(SAMPLE_BYTES, size);
  const offsets = [...new Set([
    0,
    Math.max(0, Math.floor((size - sample) / 2)),
    Math.max(0, size - sample),
  ])];
  const chunks = await Promise.all(offsets.map(offset => file.slice(offset, offset + sample).arrayBuffer()));
  /* File.type varies between browser engines and native file pickers. Content
     identity must remain stable across those surfaces and after a rename. */
  const header = new TextEncoder().encode(`powermove-media-v2\n${size}\n`);
  const length = header.length + chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0);
  const bytes = new Uint8Array(length);
  let at = 0;
  bytes.set(header, at); at += header.length;
  chunks.forEach(chunk => { bytes.set(new Uint8Array(chunk), at); at += chunk.byteLength; });
  return `v2:${size}:${await sha256(bytes)}`;
}

async function mapBounded(items, limit, worker) {
  const values = Array.from(items || []);
  const output = new Array(values.length);
  let next = 0;
  const count = Math.max(1, Math.min(values.length || 1, Math.floor(Number(limit) || 1)));
  await Promise.all(Array.from({ length: count }, async () => {
    while (true) {
      const index = next++;
      if (index >= values.length) return;
      output[index] = await worker(values[index], index);
    }
  }));
  return output;
}

function layerLists(project) {
  const lists = [project && project.layers];
  Object.values(project && project.comps || {}).forEach(comp => lists.push(comp && comp.layers));
  return lists.filter(Array.isArray);
}

function referenceCount(project, assetId) {
  let count = 0;
  layerLists(project).forEach(layers => layers.forEach(layer => {
    if (layer && layer.d && layer.d.asset === assetId) count++;
  }));
  return count;
}

function removeAsset(project, assetId) {
  if (!project || !project.assets || !project.assets[assetId]) return { removedLayers: 0, removedLayerIds: [] };
  const removedLayerIds = [];
  layerLists(project).forEach(layers => {
    for (let index = layers.length - 1; index >= 0; index--) {
      const layer = layers[index];
      if (layer && layer.d && layer.d.asset === assetId) {
        removedLayerIds.push(layer.id);
        layers.splice(index, 1);
      }
    }
  });
  const removed = new Set(removedLayerIds);
  layerLists(project).forEach(layers => layers.forEach(layer => {
    if (layer && removed.has(layer.parent)) layer.parent = null;
  }));
  delete project.assets[assetId];
  return { removedLayers: removedLayerIds.length, removedLayerIds };
}

function matchesLegacy(meta, identity) {
  if (!meta || meta.kind !== identity.kind || normalizedName(meta.name) !== normalizedName(identity.name)) return false;
  const sameSize = Number(meta.size) > 0 && Number(identity.size) > 0 && Number(meta.size) === Number(identity.size);
  const sameDuration = Number(meta.dur) > 0 && Number(identity.dur) > 0 && Math.abs(Number(meta.dur) - Number(identity.dur)) <= .05;
  return sameSize || sameDuration;
}

function match(project, liveAssets, identity) {
  const metas = Object.values(project && project.assets || {}).filter(meta => meta && meta.id);
  const candidates = metas.filter(meta =>
    (identity.fingerprint && meta.fingerprint === identity.fingerprint) ||
    (!meta.fingerprint && matchesLegacy(meta, identity)));
  if (!candidates.length) return { canonicalId: null, aliases: [] };
  candidates.sort((a, b) => {
    const score = meta =>
      (liveAssets && liveAssets.has(meta.id) ? 1000 : 0) +
      referenceCount(project, meta.id) * 20 +
      (meta.fingerprint === identity.fingerprint ? 8 : 0) +
      (Number(meta.size) > 0 ? 2 : 0);
    return score(b) - score(a);
  });
  return { canonicalId: candidates[0].id, aliases: candidates.slice(1).map(meta => meta.id) };
}

function coalesce(project, canonicalId, aliases) {
  if (!project || !canonicalId || !aliases || !aliases.length) return 0;
  const retired = new Set(aliases.filter(id => id && id !== canonicalId));
  let changed = 0;
  layerLists(project).forEach(layers => layers.forEach(layer => {
    if (layer && layer.d && retired.has(layer.d.asset)) { layer.d.asset = canonicalId; changed++; }
  }));
  retired.forEach(id => { if (project.assets && project.assets[id]) delete project.assets[id]; });
  return changed;
}

PM.MediaImport = {
  SAMPLE_BYTES,
  normalizedName,
  fingerprint,
  storageKeyFor: value => value ? `media:${value}` : null,
  mapBounded,
  match,
  coalesce,
  referenceCount,
  removeAsset,
};

/* A trim-in or split moves the layer's composition start, but the media must
   continue at the matching source time instead of restarting at zero. */
PM.MediaTiming = {
  isTimed(L) { return !!L && (L.type === 'audio' || L.type === 'video'); },
  rate(L) { return L && L.type === 'video' ? Math.max(.0001, Number(L.d && L.d.speed) || 1) : 1; },
  trimAtStart(L, nextFrom) {
    const trim = Math.max(0, Number(L && L.d && L.d.trim) || 0);
    return Math.max(0, trim + (Number(nextFrom) - Number(L.from || 0)) * this.rate(L));
  },
  earliestStart(L) {
    const trim = Math.max(0, Number(L && L.d && L.d.trim) || 0);
    return Math.max(0, Number(L.from || 0) - trim / this.rate(L));
  },
};
})();
