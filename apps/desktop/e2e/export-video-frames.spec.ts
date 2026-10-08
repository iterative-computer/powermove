import { execFileSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, test, repoRoot } from './helpers/app';

const cases = [false, true].flatMap(restore => [10, 30].flatMap(fps =>
  ['webm', 'mp4'].map(format => ({ restore, fps, format, sourceFormat: 'webm' }))));
cases.push({ restore: true, fps: 30, format: 'mp4', sourceFormat: 'mp4' });

for (const { restore, fps, format, sourceFormat } of cases) test(`exports each ${sourceFormat} source frame to ${format} at ${fps} fps ${restore ? 'after restoring media' : 'after import'}`, async ({ session }, info) => {
  await session.openEditor();
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-frames-'));
  const ffmpeg = path.join(repoRoot, `node_modules/ffmpeg-static/ffmpeg${process.platform === 'win32' ? '.exe' : ''}`);
  try {
    const source = path.join(root, `steps.${sourceFormat}`);
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 10)));
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', String(fps), '-i', 'pipe:0',
      ...(sourceFormat === 'webm' ? ['-c:v', 'libvpx-vp9', '-lossless', '1'] : ['-c:v', 'libx264', '-crf', '0', '-pix_fmt', 'yuv420p']), source], { input: raw });
    await session.page.evaluate(fps => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Frame test', w: 64, h: 64, fps, dur: 20 / fps });
      PM.proj.work = [0, 20 / fps]; PM.bus.emit('project');
      const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.appendChild(input);
    }, fps);
    await session.page.locator('#frame-source').setInputFiles(source);
    const output = path.join(root, `export.${format}`);
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, output);
    const result = await session.page.evaluate(async ({ format, fps, restore }) => {
      const PM = (window as any).PM;
      await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
      // Ordinary WebM imports and older projects have no image-sequence hint.
      if (restore) await PM.assets.restoreProject(PM.proj);
      const layer = PM.proj.layers.find((l: any) => l.type === 'video'); layer.from = 0; layer.dur = 20 / fps;
      PM.setTime(15 / fps, { force: true });
      await new Promise(resolve => setTimeout(resolve, 300));
      const result = await PM.Export.run({ format, scale: 1, fps, range: 'all', quality: 'high', mblur: false, audio: false });
      if (result.error) throw new Error(result.error);
      return result;
    }, { format, fps, restore });
    expect(result).toEqual({ cancelled: false });
    await info.attach(`encoded-${format}`, { path: output, contentType: `video/${format}` });
    const decoded = execFileSync(ffmpeg, ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
    const levels = Array.from({ length: decoded.length / 3 }, (_, i) => decoded[i * 3]);
    expect(levels.length).toBe(20);
    for (let i = 0; i < levels.length; i++) expect(Math.abs(levels[i]! - (20 + i * 10)), JSON.stringify(levels)).toBeLessThan(5);
  } finally { await rm(root, { recursive: true, force: true }); }
});
