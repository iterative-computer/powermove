import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { blake3Hex } from '../src/shared/blake3';
import { expect, test } from './helpers/app';

test.use({ desktopLaunchOptions: { env: { POWERMOVE_NEW_ENGINE: '1' } } });

test('linked UHD ProRes still resolves to its original after rendition creation and restore', async ({ session }) => {
  test.setTimeout(360_000);
  const folder = await mkdtemp(path.join(os.tmpdir(), 'pm-linked-prores-'));
  try {
    const original = path.join(folder, 'linked-uhd.mov');
    const ffmpeg = path.join(process.env.PM_FFMPEG_DIR!, 'bin/ffmpeg');
    await promisify(execFile)(ffmpeg, [
      '-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=red:s=3840x2160:r=24000/1001',
      '-frames:v', '288', '-c:v', 'prores_videotoolbox', '-profile:v', '3', '-pix_fmt', 'yuv422p10le',
      '-threads', '2', '-color_primaries', 'bt709', '-color_trc', 'bt709', '-colorspace', 'bt709', original,
    ]);
    const before = await stat(original);
    const hash = blake3Hex(new Uint8Array(await readFile(original)));
    await session.openEditor();
    const { page } = session;
    await page.evaluate(() => (window as any).PM.HostMedia.setProjectStorage('link'));
    const chooser = page.waitForEvent('filechooser');
    await page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(original);
    await page.waitForFunction(() => Object.values((window as any).PM.proj.assets).some((a: any) => a.name === 'linked-uhd.mov'), undefined, { timeout: 300_000 });
    const metadata = await page.evaluate(() => JSON.parse(JSON.stringify(Object.values((window as any).PM.proj.assets).find((a: any) => a.name === 'linked-uhd.mov'))));
    expect(metadata).toMatchObject({ storage: 'linked', rendition: true, playbackProxy: false, contentHash: hash, link: { path: original, hash } });
    const resolve = () => session.app.evaluate(async (_electron, assetId) => {
      const store = (globalThis as any).__powermoveMediaStore;
      const source = await store.resolveAsset(assetId);
      const rendition = await store.store.record(`rendition:${Buffer.from(source?.contentHash ?? []).toString('hex')}`);
      return { path: source?.path ?? null, hash: Buffer.from(source?.contentHash ?? []).toString('hex'), rendition: rendition?.source ?? null };
    }, metadata.id);
    // Do not poll: ingest returning must mean the host can resolve it now.
    expect(await resolve()).toEqual({ path: original, hash, rendition: 'rendition' });
    const restored = await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.assets.clear();
      const r = await PM.assets.restoreProject(PM.proj);
      await PM.HostMedia.flushBindings();
      return { restored: r.restored.length, missing: r.missing.length };
    });
    expect(restored).toEqual({ restored: 1, missing: 0 });
    expect(await resolve()).toEqual({ path: original, hash, rendition: 'rendition' });
    expect((await stat(original)).mtimeMs).toBe(before.mtimeMs);
    expect(blake3Hex(new Uint8Array(await readFile(original)))).toBe(hash);
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await rm(folder, { recursive: true, force: true }); }
});
