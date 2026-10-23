import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, test } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';
import { fixturePath } from './helpers/media';

for (const alpha of [false, true]) {
  test(`real media import prepares a playable ${alpha ? 'transparent' : 'multi-chunk'} editing preview without replacing its source`, async ({ session }) => {
    test.setTimeout(60_000);
    await session.openEditor();
    const file = path.join(session.userData, alpha ? 'preview-alpha.mov' : 'preview-chunks.mp4');
    let sourceHash: string | undefined;
    if (alpha) {
      const frame = Buffer.alloc(64 * 64 * 4);
      for (let y = 16; y < 48; y++) for (let x = 16; x < 48; x++) {
        frame[(y * 64 + x) * 4] = 255;
        frame[(y * 64 + x) * 4 + 3] = 255;
      }
      // Powermove's FFmpeg has no prores_ks: ProRes 4444 with alpha encodes
      // through VideoToolbox (as import-prores-alpha.spec.ts does).
      execFileSync(ffmpegBinary, [
        '-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgba', '-s', '64x64', '-r', '30', '-i', 'pipe:0',
        '-c:v', 'prores_videotoolbox', '-allow_sw', '1', '-profile:v', '4444', '-pix_fmt', 'ayuv64le', file,
      ], { input: Buffer.concat(Array.from({ length: 30 }, () => frame)) });
    } else {
      // A valid MP4 free box makes the upload cross three 1 MiB chunks without
      // making this I/O guard depend on a slow, high-resolution codec fixture.
      const padding = Buffer.alloc(2 * 1024 * 1024 + 8);
      padding.writeUInt32BE(padding.length); padding.write('free', 4, 'ascii');
      const source = Buffer.concat([await readFile(fixturePath('h264-aac.mp4')), padding]);
      expect(source.length).toBeGreaterThan(2 * 1024 * 1024);
      sourceHash = createHash('sha256').update(source).digest('hex');
      await writeFile(file, source);
    }

    const chooser = session.page.waitForEvent('filechooser');
    await session.page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(file);
    const name = path.basename(file);
    await session.page.waitForFunction(name => [...(window as any).PM.assets.map.values()]
      .some((asset: any) => asset.name === name && asset.preview?.el?.readyState >= 2), name);

    const result = await session.page.evaluate(async name => {
      const PM = (window as any).PM;
      const asset = [...PM.assets.map.values()].find((item: any) => item.name === name) as any;
      const source = await PM.MediaStore.get(PM.proj.assets[asset.id]);
      const key = `${asset.storageKey}:preview:intra-1280-v1:${asset.playbackProxyVersion || 0}:${source.size}`;
      // Installation emits before the optional cache write settles.
      let cached: Blob | null = null;
      for (let attempt = 0; attempt < 100 && !cached; attempt++) {
        cached = await PM.MediaStore.get(key);
        if (!cached) await new Promise(resolve => setTimeout(resolve, 20));
      }
      const digest = await crypto.subtle.digest('SHA-256', await source.arrayBuffer());
      const hash = [...new Uint8Array(digest)].map(value => value.toString(16).padStart(2, '0')).join('');
      const video = asset.preview.el as HTMLVideoElement;
      const canvas = document.createElement('canvas'); canvas.width = video.videoWidth; canvas.height = video.videoHeight;
      const context = canvas.getContext('2d')!; context.drawImage(video, 0, 0);
      const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
      let minAlpha = 255, maxAlpha = 0;
      for (let i = 3; i < pixels.length; i += 4) {
        minAlpha = Math.min(minAlpha, pixels[i]!); maxAlpha = Math.max(maxAlpha, pixels[i]!);
      }
      return { hash, bytes: source.size, cachedBytes: cached?.size || 0, minAlpha, maxAlpha,
        separateElement: video !== asset.el, width: video.videoWidth, height: video.videoHeight,
        center: [...context.getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data] };
    }, name);

    expect(result.separateElement).toBe(true);
    expect(result.cachedBytes).toBeGreaterThan(0);
    expect(result.width).toBeGreaterThan(0); expect(result.height).toBeGreaterThan(0);
    expect(result.maxAlpha).toBe(255);
    expect(result.center[0]).toBeGreaterThan(200);
    expect(result.center[1]).toBeLessThan(60);
    if (alpha) expect(result.minAlpha).toBe(0);
    else {
      expect(result.minAlpha).toBe(255);
      expect(result.bytes).toBeGreaterThan(2 * 1024 * 1024);
      expect(result.hash).toBe(sourceHash);
    }
    expect(session.diagnostics.console.filter(record => record.text.includes('Video preview unavailable'))).toEqual([]);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
}
