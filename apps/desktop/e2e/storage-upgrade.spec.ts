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

test('first launch automatically optimizes old data, notifies, preserves unique work and stays finished after relaunch', async () => {
  test.setTimeout(90_000);
  const f = await fixture(); let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ userData: f.profile });
    await seedBrowser(session, f.png);
    const notice = session.page.locator('.toast').filter({ hasText: 'Storage optimized' });
    await expect(notice).toBeVisible({ timeout: 30_000 }); await expect(notice).toContainText('Freed about');
    const journal = JSON.parse(await readFile(f.marker, 'utf8'));
    expect(journal.origins['app://powermove'].steps).toEqual(['recovery', 'browser', 'sources', 'cleanup']);
    expect(journal.origins['app://powermove'].freedBytes).toBeGreaterThan(500_000);
    expect((await stat(path.join(f.profile, 'store', 'project.legacy.json'))).size).toBeLessThan(20_000);
    expect(await readdir(f.backups)).toEqual(['2026-10-02T00-00-00-000Z.pmv', '2026-10-03T00-00-00-000Z.pmv', 'uncertain.recovery.pmv']);
    expect(await readFile(path.join(f.workspace, 'artifacts', 'keep.wav'), 'utf8')).toBe('unique generated media');
    await expect(stat(path.join(f.workspace, 'node_modules'))).rejects.toMatchObject({ code: 'ENOENT' });
    expect(await readFile(f.original)).toEqual(Buffer.from(f.png));
    const remaining = await session.page.evaluate(async () => {
      const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open('powermove-media', 2); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      try { return await new Promise<IDBValidKey[]>((resolve, reject) => { const r = db.transaction('assets').objectStore('assets').getAllKeys(); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); }); }
      finally { db.close(); }
    });
    expect(remaining).toEqual(['media:conflict']);
    const native = new LibraryMedia(path.join(f.profile, 'Media Store'));
    expect(await native.fileFor('media:legacy')).toBe(f.original);
    expect(await readFile((await native.fileFor('browser-only'))!, 'utf8')).toBe('unique pasted media');

    // The notice uses the native toast styling in both themes.
    await session.page.evaluate(() => (window as any).PM.SettingsUI.open('general'));
    for (const theme of ['light', 'dark']) {
      await session.page.locator('select[aria-label="Appearance"]').selectOption(theme);
      await session.page.screenshot({ path: `/tmp/powermove-storage-upgrade-${theme}.png` });
    }
    await notice.getByRole('button', { name: 'View Storage', exact: true }).click();
    await expect(session.page.getByRole('region', { name: 'Storage', exact: true }).getByRole('heading', { name: 'Storage', exact: true })).toBeVisible();
    await expect.poll(async () => JSON.parse(await readFile(f.marker, 'utf8')).origins['app://powermove'].notified).toBe(true);
    const before = await readFile(f.marker, 'utf8');
    await session.relaunch();
    await session.page.waitForTimeout(6500);
    await expect(session.page.locator('.toast').filter({ hasText: 'Storage optimized' })).toHaveCount(0);
    expect(await readFile(f.marker, 'utf8')).toBe(before);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session?.close(); await rm(f.root, { recursive: true, force: true }); }
});

test('an interrupted browser pass resumes next launch and completion remains visible until acknowledged', async () => {
  test.setTimeout(90_000);
  const f = await fixture(); let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ userData: f.profile }); await seedBrowser(session, f.png);
    await session.page.evaluate(() => { (window as any).PM.MediaStore.optimizeLegacy = () => new Promise<void>(() => {}); });
    await expect(session.page.getByRole('button', { name: /^Optimizing storage/ })).toBeVisible({ timeout: 20_000 });
    await expect.poll(async () => JSON.parse(await readFile(f.marker, 'utf8')).origins['app://powermove'].steps).toEqual(['recovery']);
    await session.relaunch();
    await expect(session.page.locator('.toast').filter({ hasText: 'Storage optimized' })).toBeVisible({ timeout: 30_000 });
    expect(JSON.parse(await readFile(f.marker, 'utf8')).origins['app://powermove'].notified).toBeUndefined();
    await session.relaunch();
    await expect(session.page.locator('.toast').filter({ hasText: 'Storage optimized' })).toBeVisible({ timeout: 20_000 });
    const notice = session.page.locator('.toast').filter({ hasText: 'Storage optimized' });
    await notice.getByRole('button', { name: /dismiss/i }).click();
    await expect.poll(async () => JSON.parse(await readFile(f.marker, 'utf8')).origins['app://powermove'].notified).toBe(true);
    expect(await readFile(f.original)).toEqual(Buffer.from(f.png)); expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session?.close(); await rm(f.root, { recursive: true, force: true }); }
});
