/* Powermove — durable imported media and source-time helpers. */
(() => {
const PM = window.PM;

const DB_NAME = 'powermove-media';
const DB_VERSION = 1;
const STORE = 'assets';
let dbPromise = null;

function openDB() {
  if (!window.indexedDB) return Promise.resolve(null);
  if (dbPromise) return dbPromise;
  dbPromise = new Promise(resolve => {
    let req;
    try { req = window.indexedDB.open(DB_NAME, DB_VERSION); }
    catch (e) { resolve(null); return; }
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE, { keyPath: 'id' });
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => resolve(null);
    req.onblocked = () => resolve(null);
  });
  return dbPromise;
}

async function request(mode, action) {
  const db = await openDB();
  if (!db) return null;
  return new Promise(resolve => {
    let settled = false;
    const finish = value => { if (!settled) { settled = true; resolve(value); } };
    let tx, req;
    try {
      tx = db.transaction(STORE, mode);
      req = action(tx.objectStore(STORE));
    } catch (e) { finish(null); return; }
    if (req) {
      req.onsuccess = () => finish(req.result == null ? true : req.result);
      req.onerror = () => finish(null);
    }
    tx.oncomplete = () => finish(true);
    tx.onerror = () => finish(null);
    tx.onabort = () => finish(null);
  });
}

PM.MediaStore = {
  async put(id, blob) {
    if (!id || !blob) return false;
    const result = await request('readwrite', store => store.put({ id, blob, at: Date.now() }));
    return result != null;
  },
  async get(id) {
    const record = id ? await request('readonly', store => store.get(id)) : null;
    return record && record.blob instanceof Blob ? record.blob : null;
  },
  async remove(id) {
    return id ? (await request('readwrite', store => store.delete(id))) != null : false;
  },
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
