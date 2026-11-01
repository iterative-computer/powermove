import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, readdir, rm, stat, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test, launchApp, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';
import { LibraryMedia } from '../src/main/library-media';
import { STORAGE_UPGRADE_FILE } from '../src/main/storage-upgrade';

async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-upgrade-proof-')), profile = path.join(root, 'profile');
  const store = path.join(profile, 'store'), original = path.join(root, 'original.png');
  await mkdir(store, { recursive: true });
  const png = await readFile(fixturePath('still-red.png')); await writeFile(original, png);
  const library = new LibraryMedia(path.join(profile, 'Media Store')); await library.initialize();
  for (const [key, bytes] of [['media:legacy', png], ['media:conflict', Buffer.from('new native version')]] as const) {
    const upload = await library.begin(1, { key, size: bytes.length, type: 'image/png', revision: 'native-revision' });
    await library.chunk(1, upload, 0, bytes); await library.finish(1, upload);
  }
  await writeFile(path.join(store, 'projects.json'), '[]');
  await writeFile(path.join(store, 'project.legacy.json'), JSON.stringify({ id: 'legacy', name: 'Keep my work', w: 100, h: 100, layers: [],
    assets: { clip: { id: 'clip', kind: 'image', name: 'original.png', sourcePath: original, storageKey: 'media:legacy' } }, notes: 'Recoverable edits 🟢'.repeat(40_000) }));
  const backups = path.join(profile, 'backups', 'legacy'); await mkdir(backups, { recursive: true });
  await writeFile(path.join(backups, '2026-10-01T00-00-00-000Z.pmv'), 'same saved version');
  await writeFile(path.join(backups, '2026-10-02T00-00-00-000Z.pmv'), 'same saved version');
  await writeFile(path.join(backups, '2026-10-03T00-00-00-000Z.pmv'), 'unique earlier work');
  await writeFile(path.join(backups, 'uncertain.recovery.pmv'), 'unresolved work');
  const workspace = path.join(profile, 'Agent Workspaces', 'legacy'), cache = path.join(workspace, 'node_modules');
  await mkdir(cache, { recursive: true }); await writeFile(path.join(cache, 'generated.js'), 'disposable dependency');
  await mkdir(path.join(workspace, 'artifacts')); await writeFile(path.join(workspace, 'artifacts', 'keep.wav'), 'unique generated media');
  const old = new Date('2026-01-01');
  for (const file of [path.join(cache, 'generated.js'), cache, path.join(workspace, 'artifacts', 'keep.wav'), path.join(workspace, 'artifacts'), workspace]) await utimes(file, old, old);
  return { root, profile, png: [...png], original, backups, workspace, marker: path.join(profile, STORAGE_UPGRADE_FILE) };
}
async function seedBrowser(session: LaunchedApp, png: number[]) {
  await session.page.evaluate(async png => {
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.open('powermove-media', 2);
      request.onupgradeneeded = () => { if (!request.result.objectStoreNames.contains('assets')) request.result.createObjectStore('assets', { keyPath: 'id' }); };
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const db = request.result, tx = db.transaction('assets', 'readwrite'), store = tx.objectStore('assets');
        for (const [id, blob] of [
          ['media:legacy', new Blob([new Uint8Array(png)], { type: 'image/png' })],
          ['browser-only', new Blob(['unique pasted media'], { type: 'application/octet-stream' })],
          ['media:conflict', new Blob(['older unique version'], { type: 'image/png' })],
        ] as const) store.put({ id, blob, size: blob.size, revision: 'older-revision', at: 1 });
        tx.oncomplete = () => { db.close(); resolve(); }; tx.onabort = () => { db.close(); reject(tx.error); };
      };
    });
  }, png);
}

test('TR preparation retains old browser copies, playback files, authored data and recovery copies', async () => {
  test.setTimeout(90_000);
  const f = await fixture(); let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ userData: f.profile }); await seedBrowser(session, f.png);
    await session.relaunch();
    const checkpoint = path.join(f.profile, 'Media Store', 'migration.json');
    await expect.poll(async () => JSON.parse(await readFile(checkpoint, 'utf8')).state, { timeout: 30_000 }).toBe('complete');
    const journal = JSON.parse(await readFile(checkpoint, 'utf8'));
    expect(journal.completed).toHaveLength(3);
    expect(existsSync(f.marker)).toBe(false);
    expect((await stat(path.join(f.profile, 'store', 'project.legacy.json'))).size).toBeGreaterThan(500_000);
    expect(await readdir(f.backups)).toHaveLength(4);
    expect(await readFile(path.join(f.workspace, 'artifacts', 'keep.wav'), 'utf8')).toBe('unique generated media');
    expect(await readFile(path.join(f.workspace, 'node_modules', 'generated.js'), 'utf8')).toBe('disposable dependency');
    expect(await readFile(f.original)).toEqual(Buffer.from(f.png));
    const remaining = await session.page.evaluate(async () => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('powermove-media'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      try { return await new Promise<IDBValidKey[]>((resolve, reject) => { const r = db.transaction('assets').objectStore('assets').getAllKeys(); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); }); }
      finally { db.close(); }
    });
    expect(remaining).toEqual(['browser-only', 'media:conflict', 'media:legacy']);
    const library = new LibraryMedia(path.join(f.profile, 'Media Store'));
    expect(await readFile((await library.fileFor('media:legacy'))!)).toEqual(Buffer.from(f.png));
    expect(await readFile((await library.fileFor('media:conflict'))!, 'utf8')).toBe('new native version');
    for (const receipt of Object.values<any>(journal.payloads)) {
      expect(await stat(path.join(f.profile, 'Media Store', 'objects', receipt.hash.slice(0, 2), receipt.hash))).toMatchObject({ size: receipt.payload.size });
    }
    await expect(session.page.locator('.toast').filter({ hasText: 'Storage optimized' })).toHaveCount(0);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session?.close(); await rm(f.root, { recursive: true, force: true }); }
});

test('TR resumes an interrupted record pass without replacing earlier copied bytes', async () => {
  test.setTimeout(120_000);
  const f = await fixture(); let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ userData: f.profile }); await seedBrowser(session, f.png);
    await session.page.evaluate(async () => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('powermove-media'); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      try {
        await new Promise<void>((resolve, reject) => {
          const tx = db.transaction('assets', 'readwrite');
          for (let i = 0; i < 80; i++) tx.objectStore('assets').put({ id: `resume-${i}`, blob: new Blob([new Uint8Array(128 * 1024).fill(i)]), revision: `synthetic-${i}` });
          tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
        });
      } finally { db.close(); }
    });
    const checkpoint = path.join(f.profile, 'Media Store', 'migration.json');
    await session.relaunch();
    await expect.poll(async () => {
      const value = JSON.parse(await readFile(checkpoint, 'utf8'));
      return value.state === 'copying' && value.completed.length > 0 && value.completed.length < 83;
    }, { timeout: 30_000, intervals: [50] }).toBe(true);
    const before = JSON.parse(await readFile(checkpoint, 'utf8'));
    const receipts = Object.keys(before.payloads);
    expect(receipts.length).toBeGreaterThan(0);
    await session.relaunch();
    await expect.poll(async () => JSON.parse(await readFile(checkpoint, 'utf8')).state, { timeout: 60_000 }).toBe('complete');
    const after = JSON.parse(await readFile(checkpoint, 'utf8'));
    expect(after.completed).toHaveLength(83);
    expect(Object.keys(after.payloads)).toEqual(expect.arrayContaining(receipts));
    for (const id of receipts) expect(after.payloads[id]).toEqual(before.payloads[id]);
    expect(await readFile(f.original)).toEqual(Buffer.from(f.png));
    expect(existsSync(f.marker)).toBe(false);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session?.close(); await rm(f.root, { recursive: true, force: true }); }
});
