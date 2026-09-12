import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test, repoRoot } from './helpers/app';

test('imports ProRes 4444 with transparency and restores playable media', async ({ session }) => {
  test.setTimeout(180_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-alpha-import-'));
  try {
    const fixture = process.env.POWERMOVE_ALPHA_FIXTURE || path.join(root, 'transparent.mov');
    if (!process.env.POWERMOVE_ALPHA_FIXTURE) {
      const frame = Buffer.alloc(64 * 64 * 4);
      for (let y = 16; y < 48; y++) for (let x = 16; x < 48; x++) {
        frame[(y * 64 + x) * 4] = 255;
        frame[(y * 64 + x) * 4 + 3] = 255;
      }
      execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), [
        '-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', '64x64', '-r', '30', '-i', 'pipe:0',
        '-c:v', 'prores_ks', '-profile:v', '4', '-pix_fmt', 'yuva444p10le', fixture
      ], { input: Buffer.concat(Array.from({ length: 60 }, () => frame)) });
    }
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.newProject();
      const input = document.createElement('input'); input.type = 'file'; input.id = 'alpha-import';
      document.body.appendChild(input);
    });
    await session.page.getByRole('button', { name: 'Create', exact: true }).click();
    await session.page.locator('#alpha-import').setInputFiles(fixture);
    const imported = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      const file = (document.querySelector('#alpha-import') as HTMLInputElement).files![0];
      await PM.importFiles([file]);
      const asset = [...PM.assets.map.values()].find((item: any) => item.name === file.name) as any;
      if (!asset) throw new Error('Import did not create a video asset');
      const canvas = document.createElement('canvas'); canvas.width = asset.w; canvas.height = asset.h;
      const ctx = canvas.getContext('2d')!; ctx.drawImage(asset.el, 0, 0);
      const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
      let min = 255, max = 0;
      for (let i = 3; i < pixels.length; i += 4) { min = Math.min(min, pixels[i]); max = Math.max(max, pixels[i]); }
      await PM.flushProject();
      return { id: asset.id, proxy: asset.playbackProxy, min, max, w: asset.w, h: asset.h };
    });
    expect(imported.proxy).toBe(true);
    expect(imported.min).toBe(0);
    expect(imported.max).toBe(255);
    if (!process.env.POWERMOVE_ALPHA_FIXTURE) {
      const upgraded = await session.page.evaluate(async id => {
        const PM = (window as any).PM, old = PM.assets.map.get(id);
        // Older builds persisted H.264 proxies without a version. Reimporting
        // must replace that runtime while keeping the existing layer/asset ID.
        delete old.playbackProxyVersion;
        const file = (document.querySelector('#alpha-import') as HTMLInputElement).files![0];
        const next = await PM.assets.add(file);
        return { id: next.id, replaced: next !== old, version: next.playbackProxyVersion };
      }, imported.id);
      expect(upgraded).toEqual({ id: imported.id, replaced: true, version: 3 });
    }
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, path.join(root, 'Transparent.pmv'));
    expect(await session.page.evaluate(() => (window as any).PM.saveProject())).toBe(true);
    await session.relaunch();
    await session.page.waitForFunction(id => (window as any).PM.assets.map.has(id), imported.id);
    const restored = await session.page.evaluate(async id => {
      const PM = (window as any).PM, asset = PM.assets.map.get(id);
      await asset.el.play();
      await new Promise<void>(resolve => asset.el.requestVideoFrameCallback(() => resolve()));
      asset.el.pause();
      const c = document.createElement('canvas'); c.width = asset.w; c.height = asset.h;
      const ctx = c.getContext('2d')!; ctx.drawImage(asset.el, 0, 0);
      return { alpha: ctx.getImageData(0, 0, 1, 1).data[3], type: (await PM.MediaStore.get(PM.proj.assets[id])).type };
    }, imported.id);
    expect(restored.alpha).toBe(0);
    expect(restored.type).toBe('video/webm');
  } finally { await rm(root, { recursive: true, force: true }); }
});
