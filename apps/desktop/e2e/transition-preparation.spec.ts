import { readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { blake3Hex } from '../src/shared/blake3';
import { expect, test } from './helpers/app';

test('TR imports keep browser rollback bytes and publish the same revision and BLAKE3 object', async ({ session }) => {
  const bytes = 'synthetic imported media';
  const revision = 'tr-synthetic-revision';
  expect(await session.page.evaluate(async ({ bytes, revision }) => {
    return (window as any).PM.MediaStore.put('media:tr-import', new Blob([bytes]), {}, revision);
  }, { bytes, revision })).toBe(true);
  const root = path.join(session.userData, 'Media Store');
  const records = await Promise.all((await readdir(path.join(root, 'keys'))).map(async file => JSON.parse(await readFile(path.join(root, 'keys', file), 'utf8'))));
  const record = records.find(r => r.key === 'media:tr-import');
  const hash = blake3Hex(new TextEncoder().encode(bytes));
  expect(record).toMatchObject({ revision, hash, size: bytes.length });
  expect(await readFile(path.join(root, 'objects', hash.slice(0, 2), hash), 'utf8')).toBe(bytes);
  expect(await session.page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open('powermove-media'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
    });
    try {
      const record: any = await new Promise((resolve, reject) => {
        const r = db.transaction('assets').objectStore('assets').get('media:tr-import'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
      });
      return { revision: record.revision, bytes: await record.blob.text() };
    } finally { db.close(); }
  })).toEqual({ revision, bytes });
});

test('TR copies all IndexedDB stores, Files, posters and metadata, retains sources, and reruns a delta after restart', async ({ session }) => {
  test.setTimeout(90_000);
  const root = path.join(session.userData, 'Media Store');
  const checkpoint = async () => JSON.parse(await readFile(path.join(root, 'migration.json'), 'utf8'));
  // A future/extension-owned store is deliberately outside the normal v2 media
  // API. The logical exporter must enumerate it without a schema downgrade.
  await session.page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open('powermove-media', 3);
      r.onupgradeneeded = () => {
        if (!r.result.objectStoreNames.contains('assets')) r.result.createObjectStore('assets', { keyPath: 'id' });
        r.result.createObjectStore('extension-owned');
      };
      r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
    });
    try {
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(['assets', 'extension-owned'], 'readwrite');
        tx.objectStore('assets').put({ id: 'bare-id', blob: new File(['original'], 'Unicode-🟢.mov', { type: 'video/quicktime', lastModified: 123 }), unknown: { preserve: true } });
        tx.objectStore('assets').put({ id: 'bare-id:poster', blob: new Blob(['poster'], { type: 'image/png' }) });
        tx.objectStore('extension-owned').put({ nested: new Map([['payload', new Blob(['nested'])]]), opaque: 123n }, ['compound', 42]);
        tx.objectStore('extension-owned').put({ metadataOnly: 'retained' }, 'metadata');
        tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
      });
    } finally { db.close(); }
    localStorage.setItem('tr-unknown', '{"unicode":"🟢"}');
    localStorage.setItem('tr-deleted-later', 'original setting');
  });
  await writeFile(path.join(session.userData, 'store/ext.tr-synthetic.json'), JSON.stringify({
    asset: { id: 'bare-id', storageKey: 'media:different-key', playbackProxyVersion: 1, fingerprint: 'v2:8:synthetic', sourcePath: '/synthetic/offline.mov' }
  }));
  // A pass may already have run on a fast launch before seeding; every launch
  // requires a fresh pass, independently of its prior completion flag.
  await session.relaunch();
  await expect.poll(async () => (await checkpoint()).state, { timeout: 30_000 }).toBe('complete');
  const first = await checkpoint();
  expect(first.completed).toHaveLength(4);
  expect(Object.keys(first.records)).toHaveLength(4);
  expect(Object.values<any>(first.payloads).find(r => r.payload.key === 'bare-id')).toMatchObject({ classification: 'proxy-of',
    proxyOf: [{ fingerprint: 'v2:8:synthetic', sourcePath: '/synthetic/offline.mov' }] });
  expect(JSON.stringify(first)).toContain('Unicode-🟢.mov');
  expect(JSON.stringify(first)).toContain('metadataOnly');
  for (const text of ['original', 'poster', 'nested']) {
    const hash = blake3Hex(new TextEncoder().encode(text));
    expect(await readFile(path.join(root, 'objects', hash.slice(0, 2), hash), 'utf8')).toBe(text);
  }
  const localFiles = await readdir(path.join(root, 'local-storage'));
  const local = await Promise.all(localFiles.map(async f => JSON.parse(await readFile(path.join(root, 'local-storage', f), 'utf8'))));
  expect(local.some(v => v.values['tr-unknown']?.raw === '{"unicode":"🟢"}')).toBe(true);
  const passes = await readdir(path.join(root, 'passes'));
  const passBytes = await Promise.all(passes.map(f => readFile(path.join(root, 'passes', f), 'utf8')));
  const receipts = Object.keys(first.payloads);
  await session.page.evaluate(async () => {
    localStorage.removeItem('tr-deleted-later');
    const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('powermove-media'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
    try {
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction('extension-owned', 'readwrite'); tx.objectStore('extension-owned').put({ delta: new Blob(['later']) }, 'later');
        tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
      });
    } finally { db.close(); }
  });
  await session.relaunch();
  await expect.poll(async () => (await checkpoint()).state, { timeout: 30_000 }).toBe('complete');
  const second = await checkpoint();
  expect(second.completed).toHaveLength(5);
  expect(Object.keys(second.payloads)).toEqual(expect.arrayContaining(receipts));
  for (const [index, file] of passes.entries()) expect(await readFile(path.join(root, 'passes', file), 'utf8')).toBe(passBytes[index]);
  for (const file of localFiles) expect(await readFile(path.join(root, 'local-storage', file), 'utf8')).toContain('tr-deleted-later');
  expect(await session.page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('powermove-media'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
    try {
      return await Promise.all(Array.from(db.objectStoreNames).map(name => new Promise<number>((resolve, reject) => {
        const r = db.transaction(name).objectStore(name).count(); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error);
      })));
    } finally { db.close(); }
  })).toEqual([2, 3]);
  await session.page.evaluate(() => (window as any).PM.SettingsUI.open('storage'));
  await expect(session.page.getByRole('button', { name: 'Switch to Native', exact: true })).toBeDisabled();
  expect(session.diagnostics.pageErrors).toEqual([]);
});
