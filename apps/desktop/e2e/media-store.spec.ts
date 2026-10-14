/*
 * The host media store and linked media (plan section 21; Phase 2 gate
 * items): import in both modes, content-hash relink after a move, save and
 * reopen, self-contained Export › Project, agent imports copied, project
 * deletion leaving linked originals alone, the IndexedDB copy verified by
 * hash and never deleted, and Use New Engine on → off → on with no loss.
 * Hidden Electron, isolated profiles, real files and the real store.
 */
import { copyFile, mkdir, mkdtemp, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import type { Page } from 'playwright';

import { blake3Hex } from '../src/shared/blake3';
import { REQUIRES_NATIVE_MESSAGE } from '../src/shared/media-store';
import { readIncrementalIndex } from '../src/shared/project-incremental';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';
import { fixturePath } from './helpers/media';

const ON = { POWERMOVE_NEW_ENGINE: '1' };
const OFF = { POWERMOVE_NEW_ENGINE: '0' };

let footage: string;
test.beforeEach(async () => { footage = await mkdtemp(path.join(os.tmpdir(), 'pm-footage-')); });
test.afterEach(async () => { await rm(footage, { recursive: true, force: true }); });

const hashOf = async (file: string) => blake3Hex(new Uint8Array(await readFile(file)));

async function place(name: string, folder = footage, as = name): Promise<string> {
  const target = path.join(folder, as);
  await mkdir(path.dirname(target), { recursive: true });
  await copyFile(fixturePath(name), target);
  return target;
}

async function importFile(page: Page, file: string): Promise<string> {
  const name = path.basename(file);
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(file);
  await page.waitForFunction((name) => Object.values((window as any).PM.proj.assets).some((asset: any) => asset.name === name && (window as any).PM.assets.get(asset.id)), name, { timeout: 20_000 });
  return page.evaluate((name) => (Object.values((window as any).PM.proj.assets).find((asset: any) => asset.name === name) as any).id, name);
}

const meta = (page: Page, id: string) => page.evaluate((id) => JSON.parse(JSON.stringify((window as any).PM.proj.assets[id])), id);

const storeRecords = (session: LaunchedApp) => session.app.evaluate(async () => (globalThis as any).__powermoveMediaStore.store.records());

const indexedDbRecord = (page: Page, key: string) => page.evaluate((key) => new Promise<{ size: number; bytes: number[] } | null>((resolve) => {
  const request = indexedDB.open('powermove-media');
  request.onerror = () => resolve(null);
  request.onsuccess = () => {
    const db = request.result;
    if (!db.objectStoreNames.contains('assets')) { db.close(); resolve(null); return; }
    const get = db.transaction('assets').objectStore('assets').get(key);
    get.onerror = () => { db.close(); resolve(null); };
    get.onsuccess = async () => {
      const record = get.result;
      db.close();
      resolve(record?.blob ? { size: record.blob.size, bytes: [...new Uint8Array(await record.blob.arrayBuffer())] } : null);
    };
  };
}), key);

/**
 * A 16-bit mono WAV, 1 MiB of samples, silent except a tone between bytes
 * 128 KiB and 384 KiB whose pitch is `variant`. The import fingerprint
 * samples 64 KiB at the start, the middle, and the end, so every variant
 * shares one fingerprint (and one storage key) while its content differs.
 */
function roomToneWav(variant: number): Buffer {
  const samples = 512 * 1024;
  const header = Buffer.alloc(44);
  header.write('RIFF', 0); header.writeUInt32LE(36 + samples * 2, 4); header.write('WAVE', 8);
  header.write('fmt ', 12); header.writeUInt32LE(16, 16); header.writeUInt16LE(1, 20); header.writeUInt16LE(1, 22);
  header.writeUInt32LE(48_000, 24); header.writeUInt32LE(96_000, 28); header.writeUInt16LE(2, 32); header.writeUInt16LE(16, 34);
  header.write('data', 36); header.writeUInt32LE(samples * 2, 40);
  const data = Buffer.alloc(samples * 2);
  for (let i = 64 * 1024; i < 192 * 1024; i++) data.writeInt16LE(Math.round(Math.sin(i * 0.02 * (variant + 1)) * 8000), i * 2);
  return Buffer.concat([header, data]);
}

async function setImportMode(page: Page, mode: 'copy' | 'link'): Promise<void> {
  // Through Settings › Project › Media, as a person would.
  await page.evaluate(() => (window as any).PM.SettingsUI.open('project'));
  const select = page.locator('select[aria-label="When importing"]');
  await select.waitFor({ state: 'attached' });
  await select.evaluate((element: HTMLSelectElement, mode) => { element.value = mode; element.dispatchEvent(new Event('change')); }, mode);
  await expect.poll(() => page.evaluate(() => (window as any).PM.HostMedia.projectStorage())).toBe(mode);
  await page.evaluate(() => (window as any).PM.SettingsUI.close());
}

async function engineRead(session: LaunchedApp, assetId: string): Promise<{ hash: string; path: string; access: string }> {
  return session.app.evaluate(async (_electron, assetId) => {
    const services = (globalThis as any).__powermoveNativeEngine.engine.services;
    const grant = await services.serve('interactive', 900, { type: 'mintHandle', kind: { kind: 'mediaRead', asset: assetId }, session: 1 }, 1);
    const resolved = await services.serve('interactive', 900, { type: 'resolveHandle', handle: grant.handle, session: 1 }, 1);
    return { hash: Buffer.from(grant.contentHash).toString('hex'), path: resolved.path, access: resolved.access };
  }, assetId);
}

async function openProject(page: Page, id: string): Promise<boolean> {
  return page.evaluate(async (id) => {
    const PM = (window as any).PM;
    const opened = await PM.openProjectHere(id);
    if (opened) PM.ProjectsScreen?.hide?.();
    return opened;
  }, id);
}

test.describe('media store with Use New Engine on', () => {
  test.use({ desktopLaunchOptions: { env: ON } });

  test('imports copy into the store by default and link in place when the project links', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    expect(await page.evaluate(() => (window as any).PM.HostMedia.enabled)).toBe(true);

    // Copy into project (default): the original is kept, the store holds a verified copy.
    const wav = await place('tone.wav');
    const before = await stat(wav);
    const copiedId = await importFile(page, wav);
    const copied = await meta(page, copiedId);
    expect(copied.storage).toBeUndefined();
    expect(copied.link).toBeUndefined();
    expect(copied.contentHash).toBe(await hashOf(wav));
    const records = await storeRecords(session);
    expect(records.find((record: any) => record.key === copied.storageKey)).toMatchObject({ hash: copied.contentHash, size: before.size, source: 'import' });
    expect(await indexedDbRecord(page, copied.storageKey)).toBeNull();
    expect((await stat(wav)).mtimeMs).toBe(before.mtimeMs);
    expect(await hashOf(wav)).toBe(copied.contentHash);

    // A ProRes original is kept; the browser's playback proxy is generated media.
    const prores = await place('prores422.mov');
    const proresId = await importFile(page, prores);
    const proresMeta = await meta(page, proresId);
    expect(proresMeta).toMatchObject({ playbackProxy: false, rendition: true, contentHash: await hashOf(prores) });
    const proresRecords = await storeRecords(session);
    expect(proresRecords.find((record: any) => record.key === proresMeta.storageKey)?.hash).toBe(await hashOf(prores));
    expect(proresRecords.find((record: any) => record.key === `rendition:${proresMeta.contentHash}`)).toMatchObject({ source: 'rendition' });

    // Link to files.
    await setImportMode(page, 'link');
    const png = await place('still-red.png', path.join(footage, 'Day 1'));
    const linkedId = await importFile(page, png);
    const linked = await meta(page, linkedId);
    expect(linked).toMatchObject({ storage: 'linked', link: { path: png, hash: await hashOf(png), size: (await stat(png)).size }, contentHash: await hashOf(png) });
    expect((await storeRecords(session)).some((record: any) => record.key === linked.storageKey)).toBe(false);
    expect(await page.evaluate(() => (window as any).PM.proj.requiresEngine)).toBe('native');

    // The engine reads each asset only through a handle main mints (contract 14).
    await expect.poll(() => engineRead(session, linkedId).catch((error) => String(error))).toEqual({ hash: linked.link.hash, path: png, access: 'read' });
    const copiedRead = await engineRead(session, copiedId);
    expect(copiedRead.hash).toBe(copied.contentHash);
    expect(await hashOf(copiedRead.path)).toBe(copied.contentHash);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('a converted still keeps its TIFF or HEIF original; the viewer shows a rendition', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    const tiff = await place('still.tiff');
    const id = await importFile(page, tiff);
    const asset = await meta(page, id);
    expect(asset).toMatchObject({ kind: 'image', format: 'tiff', rendition: true, contentHash: await hashOf(tiff), size: (await stat(tiff)).size });
    const records = await storeRecords(session);
    expect(records.find((record: any) => record.key === asset.storageKey)).toMatchObject({ hash: await hashOf(tiff), source: 'import' });
    expect(records.find((record: any) => record.key === `rendition:${asset.contentHash}`)).toMatchObject({ type: 'image/png', source: 'rendition' });
    await expect.poll(() => engineRead(session, id).then((read) => read.hash).catch((error) => String(error))).toBe(await hashOf(tiff));

    const restore = () => page.evaluate(async (id) => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const result = await PM.assets.restoreProject(PM.proj);
      const live = PM.assets.get(id);
      return { restored: result.restored.length, missing: result.missing.length, w: live?.w ?? 0, h: live?.h ?? 0 };
    }, id);
    expect(await restore()).toMatchObject({ restored: 1, missing: 0, w: asset.w, h: asset.h });
    // Generated media can go; the next open converts the kept original again.
    await session.app.evaluate(async (_e, key) => (globalThis as any).__powermoveMediaStore.store.removeKey(key), `rendition:${asset.contentHash}`);
    expect(await restore()).toMatchObject({ restored: 1, missing: 0, w: asset.w, h: asset.h });
    expect((await storeRecords(session)).find((record: any) => record.key === `rendition:${asset.contentHash}`)).toMatchObject({ type: 'image/png' });

    // Linked: the HEIF stays where it is; only its rendition is generated media.
    await setImportMode(page, 'link');
    const heic = await place('still.heic');
    const linkedId = await importFile(page, heic);
    const linked = await meta(page, linkedId);
    expect(linked).toMatchObject({ storage: 'linked', rendition: true, link: { path: heic, hash: await hashOf(heic) } });
    const after = await storeRecords(session);
    expect(after.some((record: any) => record.key === linked.storageKey)).toBe(false);
    expect(after.find((record: any) => record.key === `rendition:${linked.link.hash}`)).toMatchObject({ type: 'image/png' });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('relinks a moved and renamed original by content hash', async ({ session }) => {
    test.setTimeout(60_000);
    await session.openEditor();
    const { page } = session;
    await setImportMode(page, 'link');
    const original = await place('still-red.png', path.join(footage, 'Day 1'), 'A001_C003.png');
    const id = await importFile(page, original);
    const hash = await hashOf(original);
    const moved = path.join(footage, 'Selects', 'hero shot.png');
    await mkdir(path.dirname(moved), { recursive: true });
    await rename(original, moved);
    // A same-named file with other content where the original was is never chosen.
    await place('still-alpha.png', path.join(footage, 'Day 1'), 'A001_C003.png');
    const restored = await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const result = await PM.assets.restoreProject(PM.proj);
      return { restored: result.restored.length, missing: result.missing.length, live: !!PM.assets.get(id), edit: PM.proj.edits.at(-1)?.label, origin: PM.proj.edits.at(-1)?.origin };
    }, id);
    expect(restored).toEqual({ restored: 1, missing: 0, live: true, edit: 'Relink media', origin: 'engine' });
    expect(await meta(page, id)).toMatchObject({ storage: 'linked', sourcePath: moved, link: { path: moved, hash } });

    // Gone everywhere: the asset is offline, not silently replaced.
    await rm(moved);
    const offline = await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const result = await PM.assets.restoreProject(PM.proj);
      return { missing: result.missing.map((item: any) => item.id), live: !!PM.assets.get(id) };
    }, id);
    expect(offline).toEqual({ missing: [id], live: false });
    await expect(page.locator(`[data-asset-id="${id}"]`)).toContainText('offline');
  });

  test('saves PMV4 with linked records and stored media, and reopens it', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    const wav = await place('tone.wav');
    const copiedId = await importFile(page, wav);
    await setImportMode(page, 'link');
    const png = await place('still-red.png');
    const linkedId = await importFile(page, png);
    const destination = path.join(session.userData, 'Linked.pmv');
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
    expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);

    const bytes = await readFile(destination);
    const index = await readIncrementalIndex(bytes.length, async (offset, length) => new Uint8Array(bytes.subarray(offset, offset + length)));
    expect(index.linked).toEqual([{ id: linkedId, hash: await hashOf(png), size: (await stat(png)).size, path: png, type: '' }]);
    expect(index.media.map((item) => item.id)).toEqual([copiedId]);
    const record = index.media[0]!;
    expect(blake3Hex(new Uint8Array(bytes.subarray(record.offset, record.offset + record.length)))).toBe(await hashOf(wav));
    expect(record.revision).toBe(await hashOf(wav));

    await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, destination);
    await page.evaluate(() => (window as any).PM.openProject());
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      const metas = Object.values(PM.proj.assets) as any[];
      return metas.length === 2 && metas.every((asset) => PM.assets.get(asset.id));
    }, undefined, { timeout: 20_000 });
    const reopened = await page.evaluate(() => Object.values((window as any).PM.proj.assets).map((asset: any) => ({ name: asset.name, storage: asset.storage ?? 'copied', linkPath: asset.link?.path ?? null })).sort((a, b) => a.name.localeCompare(b.name)));
    expect(reopened).toEqual([{ name: 'still-red.png', storage: 'linked', linkPath: png }, { name: 'tone.wav', storage: 'copied', linkPath: null }]);
    expect(await page.evaluate(() => (window as any).PM.proj.requiresEngine)).toBe('native');
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('Export › Project embeds linked originals and opens self-contained', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    await setImportMode(page, 'link');
    const png = await place('still-red.png');
    const hash = await hashOf(png);
    await importFile(page, png);
    const exported = path.join(session.userData, 'Exported.pmv');
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, exported);
    expect(await page.evaluate(() => (window as any).PM.Export.run({ format: 'json' }))).toEqual({ cancelled: false });
    // The original goes away; the export does not need it.
    await rm(png);
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, exported);
    await page.evaluate(() => (window as any).PM.openProject());
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      const metas = Object.values(PM.proj.assets) as any[];
      return PM.proj.name !== 'E2E composition' || (metas.length === 1 && PM.assets.get(metas[0].id) && metas[0].storage === undefined);
    }, undefined, { timeout: 20_000 });
    const opened = await page.evaluate(async () => {
      const PM = (window as any).PM;
      const asset = Object.values(PM.proj.assets)[0] as any;
      const blob = await PM.MediaStore.get(asset);
      return { storage: asset.storage ?? null, link: asset.link ?? null, contentHash: asset.contentHash, live: !!PM.assets.get(asset.id), size: blob?.size ?? 0, stamped: PM.proj.requiresEngine ?? null };
    });
    expect(opened).toEqual({ storage: null, link: null, contentHash: hash, live: true, size: 310, stamped: null });
  });

  test('agent imports are copied unless they ask to link; agent workspace files are always copied', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    await setImportMode(page, 'link');
    const first = await place('still-red.png', footage, 'agent-copy.png');
    const second = await place('still-alpha.png', footage, 'agent-link.png');
    const projectId = await page.evaluate(() => (window as any).PM.proj.id);
    const workspaceFile = path.join(session.userData, 'Agent Workspaces', projectId, 'renders', 'frame.png');
    await mkdir(path.dirname(workspaceFile), { recursive: true });
    await copyFile(fixturePath('still.webp'), workspaceFile);
    const results = await page.evaluate(async ({ first, second, workspaceFile }) => {
      const PM = (window as any).PM;
      const tool = async (args: any) => {
        const reply = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'media-store', callId: `c-${Math.random()}`, tool: 'import_media', arguments: args, baseRevision: PM.proj.revision });
        return JSON.parse(reply.content[0].text);
      };
      return [await tool({ path: first }), await tool({ path: second, storage: 'link' }), await tool({ path: workspaceFile, storage: 'link' })];
    }, { first, second, workspaceFile });
    expect(results.map((result: any) => [result.status, result.storage])).toEqual([['imported', 'copied'], ['imported', 'linked'], ['imported', 'copied']]);
    const metas = await Promise.all(results.map((result: any) => meta(page, result.assetId)));
    expect(metas[0].storage).toBeUndefined();
    expect(metas[1]).toMatchObject({ storage: 'linked', link: { path: second } });
    expect(metas[2].storage).toBeUndefined();
    const records = await storeRecords(session);
    expect(records.find((record: any) => record.key === metas[2].storageKey)?.hash).toBe(await hashOf(workspaceFile));
  });

  test('files that share a sampled fingerprint stay separate assets, each with its own bytes', async ({ session }) => {
    test.setTimeout(90_000);
    await session.openEditor();
    const { page } = session;
    const first = path.join(footage, 'room tone A.wav');
    const second = path.join(footage, 'room tone B.wav');
    await writeFile(first, roomToneWav(1));
    await writeFile(second, roomToneWav(2));
    const firstId = await importFile(page, first);
    const secondId = await importFile(page, second);
    expect(secondId).not.toBe(firstId);
    const [a, b] = [await meta(page, firstId), await meta(page, secondId)];
    // Same fingerprint and storage key; the content hash tells them apart.
    expect(b.fingerprint).toBe(a.fingerprint);
    expect(b.storageKey).toBe(a.storageKey);
    expect([a.contentHash, b.contentHash]).toEqual([await hashOf(first), await hashOf(second)]);
    const readBack = (id: string) => page.evaluate(async (id) => {
      const PM = (window as any).PM;
      const blob = await PM.MediaStore.get(PM.proj.assets[id]);
      return PM.HostMedia.hash(blob);
    }, id);
    expect(await readBack(firstId)).toBe(a.contentHash);
    expect(await readBack(secondId)).toBe(b.contentHash);
    // The engine's handle names the first asset's own content, not the key's latest.
    await expect.poll(() => engineRead(session, firstId).then((read) => read.hash).catch((error) => String(error))).toBe(a.contentHash);

    // Agents: the same content is deduplicated; other content is a new asset.
    const sameAsFirst = path.join(footage, 'copy of A.wav');
    await copyFile(first, sameAsFirst);
    const third = path.join(footage, 'room tone C.wav');
    await writeFile(third, roomToneWav(3));
    const agent = await page.evaluate(async (paths) => {
      const PM = (window as any).PM;
      const out = [];
      for (const path of paths) {
        const reply = await PM.AgentHarness.test.handleLiveAgentTool({ runId: 'fingerprint', callId: `c-${Math.random()}`, tool: 'import_media', arguments: { path }, baseRevision: PM.proj.revision });
        out.push(JSON.parse(reply.content[0].text));
      }
      return out;
    }, [sameAsFirst, third]);
    expect(agent[0]).toMatchObject({ status: 'already_imported', assetId: firstId });
    expect(agent[1].status).toBe('imported');
    expect([firstId, secondId]).not.toContain(agent[1].assetId);
    expect(await readBack(agent[1].assetId)).toBe(await hashOf(third));

    // Desktop Save embeds each asset's own bytes.
    const destination = path.join(session.userData, 'Room tone.pmv');
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, destination);
    expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    const bytes = await readFile(destination);
    const index = await readIncrementalIndex(bytes.length, async (offset, length) => new Uint8Array(bytes.subarray(offset, offset + length)));
    const saved = Object.fromEntries(index.media.map((item) => [item.id, blake3Hex(new Uint8Array(bytes.subarray(item.offset, item.offset + item.length)))]));
    expect(saved).toEqual({ [firstId]: a.contentHash, [secondId]: b.contentHash, [agent[1].assetId]: await hashOf(third) });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('deleting a project never touches its linked originals', async ({ session }) => {
    test.setTimeout(60_000);
    await session.openEditor();
    const { page } = session;
    await setImportMode(page, 'link');
    const png = await place('still-red.png');
    const hash = await hashOf(png);
    await importFile(page, png);
    const id = await page.evaluate(() => (window as any).PM.proj.id);
    const objectCount = (await storeRecords(session)).length;
    await page.evaluate(async (id) => {
      const PM = (window as any).PM;
      window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Other' }) }));
      PM.Projects.trash(id);
      PM.Projects.destroy(id);
    }, id);
    await expect.poll(() => page.evaluate((id) => (window as any).PM.Projects.get(id), id)).toBeNull();
    expect(await hashOf(png)).toBe(hash);
    expect((await stat(png)).isFile()).toBe(true);
    expect((await storeRecords(session)).length).toBe(objectCount);
  });
});

test('opening a project file on another Mac copies its media into the store in main, verified', async () => {
  test.setTimeout(120_000);
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-project-'));
  const project = path.join(folder, 'Handoff.pmv');
  try {
    // Saved on one Mac (profile)…
    let session = await launchApp({ env: ON });
    let expected: Record<string, string> = {};
    try {
      await session.openEditor();
      const { page } = session;
      const wav = await place('tone.wav');
      const png = await place('still-red.png');
      await importFile(page, wav);
      await importFile(page, png);
      expected = { 'tone.wav': await hashOf(wav), 'still-red.png': await hashOf(png) };
      await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, project);
      expect(await page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    } finally { await session.close(); }

    // …opened on another, whose store is empty.
    session = await launchApp({ env: ON });
    try {
      await session.openEditor();
      const { page } = session;
      expect(await storeRecords(session)).toEqual([]);
      await session.app.evaluate(({ dialog }, filePath) => { dialog.showOpenDialog = async () => ({ canceled: false, filePaths: [filePath] }); }, project);
      await page.evaluate(() => (window as any).PM.openProject());
      await page.waitForFunction(() => {
        const PM = (window as any).PM;
        const metas = Object.values(PM.proj.assets) as any[];
        return metas.length === 2 && metas.every((asset) => PM.assets.get(asset.id));
      }, undefined, { timeout: 30_000 });
      const assets = await page.evaluate(() => Object.values((window as any).PM.proj.assets).map((asset: any) => ({ name: asset.name, key: asset.storageKey })));
      const records = await storeRecords(session);
      for (const asset of assets) {
        // Main copied the range from the file itself: the record says `project`.
        expect(records.find((record: any) => record.key === asset.key)).toMatchObject({ hash: expected[asset.name], source: 'project' });
        const object = await session.app.evaluate(async (_e, hash) => (globalThis as any).__powermoveMediaStore.store.objectPath(hash), expected[asset.name]);
        expect(await hashOf(object)).toBe(expected[asset.name]);
      }
      expect(await indexedDbRecord(page, assets[0]!.key)).toBeNull();
      expect(session.diagnostics.pageErrors).toEqual([]);
    } finally { await session.close(); }
  } finally { await rm(folder, { recursive: true, force: true }); }
});

test('IndexedDB media is copied to the store, verified by hash, and never deleted', async () => {
  test.setTimeout(120_000);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-e2e-'));
  try {
    let session = await launchApp({ userData, env: OFF });
    let projectId = '';
    let storageKey = '';
    try {
      await session.openEditor();
      expect(await session.page.evaluate(() => (window as any).PM.HostMedia.enabled)).toBe(false);
      const id = await importFile(session.page, fixturePath('still-red.png'));
      ({ projectId, storageKey } = await session.page.evaluate((id) => ({ projectId: (window as any).PM.proj.id, storageKey: (window as any).PM.proj.assets[id].storageKey }), id));
      expect((await indexedDbRecord(session.page, storageKey))?.size).toBe(310);
      expect(await storeRecords(session)).toEqual([]);
      await session.page.evaluate(() => (window as any).PM.Projects.put((window as any).PM.proj));
    } finally { await session.close(); }

    session = await launchApp({ userData, env: ON });
    try {
      await session.page.waitForFunction(() => Boolean((window as any).PM?.openProjectHere));
      if (await session.page.evaluate(() => (window as any).PM.proj.id) !== projectId) expect(await openProject(session.page, projectId)).toBe(true);
      const expected = blake3Hex(new Uint8Array(await readFile(fixturePath('still-red.png'))));
      await expect.poll(async () => (await storeRecords(session)).find((record: any) => record.key === storageKey), { timeout: 20_000 })
        .toMatchObject({ hash: expected, size: 310, source: 'indexeddb' });
      // The copy is the IndexedDB bytes, and IndexedDB still has them.
      const idb = await indexedDbRecord(session.page, storageKey);
      expect(idb?.size).toBe(310);
      expect(blake3Hex(Uint8Array.from(idb!.bytes))).toBe(expected);
      const object = await session.app.evaluate(async (_e, hash) => (globalThis as any).__powermoveMediaStore.store.objectPath(hash), expected);
      expect(await hashOf(object)).toBe(expected);
    } finally { await session.close(); }
  } finally { await rm(userData, { recursive: true, force: true }); }
});

test('Use New Engine on → off → on loses nothing; a linked project is refused cleanly while off', async () => {
  test.setTimeout(180_000);
  const userData = await mkdtemp(path.join(os.tmpdir(), 'powermove-e2e-'));
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-footage-'));
  try {
    // On: a copied project and a linked one.
    let session = await launchApp({ userData, env: ON });
    let copiedProject = '';
    let linkedProject = '';
    let before: any;
    try {
      await session.openEditor();
      const { page } = session;
      await importFile(page, fixturePath('tone.wav'));
      await importFile(page, fixturePath('h264-aac.mp4'));
      // Originals the legacy viewer cannot show, kept with their renditions.
      await importFile(page, fixturePath('prores422.mov'));
      await importFile(page, fixturePath('still.tiff'));
      copiedProject = await page.evaluate(() => (window as any).PM.proj.id);
      before = await page.evaluate(() => JSON.parse(JSON.stringify({ assets: (window as any).PM.proj.assets, layers: (window as any).PM.proj.layers.length })));
      await page.evaluate(() => (window as any).PM.Projects.put((window as any).PM.proj));
      await page.evaluate(() => { const PM = (window as any).PM; window.dispatchEvent(new CustomEvent('pm-open-project', { detail: PM.mkProject({ name: 'Linked' }) })); });
      await setImportMode(page, 'link');
      const png = path.join(folder, 'linked.png');
      await copyFile(fixturePath('still-red.png'), png);
      await importFile(page, png);
      linkedProject = await page.evaluate(() => (window as any).PM.proj.id);
      expect(await page.evaluate(() => (window as any).PM.proj.requiresEngine)).toBe('native');
      await page.evaluate(() => (window as any).PM.Projects.put((window as any).PM.proj));
    } finally { await session.close(); }

    // Off: the copied project opens with every asset; the linked one is refused.
    session = await launchApp({ userData, env: OFF });
    try {
      const { page } = session;
      await page.waitForFunction(() => Boolean((window as any).PM?.openProjectHere));
      // The linked project was last on screen; booting must not open it.
      expect(await page.evaluate(() => (window as any).PM.proj.requiresEngine ?? null)).toBeNull();
      expect(await openProject(page, linkedProject)).toBe(false);
      await expect(page.getByText(REQUIRES_NATIVE_MESSAGE)).toBeVisible();
      if (await page.evaluate(() => (window as any).PM.proj.id) !== copiedProject) expect(await openProject(page, copiedProject)).toBe(true);
      await page.waitForFunction((count) => {
        const PM = (window as any).PM;
        const metas = Object.values(PM.proj.assets) as any[];
        return metas.length === count && metas.every((asset) => PM.assets.get(asset.id));
      }, Object.keys(before.assets).length, { timeout: 30_000 });
      expect(await page.evaluate(() => JSON.parse(JSON.stringify((window as any).PM.proj.assets)))).toEqual(before.assets);
      // New media while off goes to IndexedDB, as before.
      await importFile(page, fixturePath('still-red.png'));
      await page.evaluate(() => (window as any).PM.Projects.put((window as any).PM.proj));
      before = await page.evaluate(() => JSON.parse(JSON.stringify({ assets: (window as any).PM.proj.assets, layers: (window as any).PM.proj.layers.length })));
    } finally { await session.close(); }

    // On again: everything opens; the media imported while off moves to the store.
    session = await launchApp({ userData, env: ON });
    try {
      const { page } = session;
      await page.waitForFunction(() => Boolean((window as any).PM?.openProjectHere));
      if (await page.evaluate(() => (window as any).PM.proj.id) !== copiedProject) expect(await openProject(page, copiedProject)).toBe(true);
      await page.waitForFunction((count) => {
        const PM = (window as any).PM;
        const metas = Object.values(PM.proj.assets) as any[];
        return metas.length === count && metas.every((asset) => PM.assets.get(asset.id));
      }, Object.keys(before.assets).length, { timeout: 30_000 });
      const after = await page.evaluate(() => JSON.parse(JSON.stringify({ assets: (window as any).PM.proj.assets, layers: (window as any).PM.proj.layers.length })));
      expect(after).toEqual(before);
      const png = Object.values<any>(after.assets).find((asset) => asset.name === 'still-red.png');
      await expect.poll(async () => (await storeRecords(session)).find((record: any) => record.key === png.storageKey)?.source, { timeout: 20_000 }).toBe('indexeddb');
      expect(await openProject(page, linkedProject)).toBe(true);
      await page.waitForFunction(() => {
        const PM = (window as any).PM;
        const metas = Object.values(PM.proj.assets) as any[];
        return metas.length === 1 && PM.assets.get(metas[0].id) && metas[0].storage === 'linked';
      }, undefined, { timeout: 20_000 });
    } finally { await session.close(); }
  } finally {
    await rm(userData, { recursive: true, force: true });
    await rm(folder, { recursive: true, force: true });
  }
});
