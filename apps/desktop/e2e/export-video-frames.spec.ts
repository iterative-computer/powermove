import { execFileSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { expect, test, repoRoot } from './helpers/app';

test('exports each decoded video frame even when the preview is at another time', async ({ session }) => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'powermove-export-frames-'));
  const ffmpeg = path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg');
  try {
    const source = path.join(root, 'steps.webm');
    const raw = Buffer.concat(Array.from({ length: 20 }, (_, i) => Buffer.alloc(64 * 64 * 3, 20 + i * 10)));
    execFileSync(ffmpeg, ['-v', 'error', '-f', 'rawvideo', '-pix_fmt', 'rgb24', '-s', '64x64', '-r', '10', '-i', 'pipe:0',
      '-c:v', 'libvpx-vp9', '-lossless', '1', source], { input: raw });
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.proj = PM.mkProject({ name: 'Frame test', w: 64, h: 64, fps: 10, dur: 2 });
      PM.proj.work = [0, 2]; PM.bus.emit('project');
      const input = document.createElement('input'); input.type = 'file'; input.id = 'frame-source'; document.body.appendChild(input);
    });
    await session.page.locator('#frame-source').setInputFiles(source);
    const bytes = await session.page.evaluate(async () => {
      const PM = (window as any).PM;
      await PM.importFiles([(document.querySelector('#frame-source') as HTMLInputElement).files![0]]);
      const layer = PM.proj.layers.find((l: any) => l.type === 'video'); layer.from = 0; layer.dur = 2;
      PM.setTime(1.5, { force: true });
      await new Promise(resolve => setTimeout(resolve, 300));
      let data: number[] = [];
      PM.download = async (blob: Blob) => { data = Array.from(new Uint8Array(await blob.arrayBuffer())); };
      const result = await PM.Export.run({ format: 'webm', scale: 1, fps: 10, range: 'all', quality: 'high', mblur: false, audio: false });
      if (result.error) throw new Error(result.error);
      return data;
    });
    const output = path.join(root, 'export.webm'); await writeFile(output, Buffer.from(bytes));
    const decoded = execFileSync(ffmpeg, ['-v', 'error', '-i', output, '-vf', 'scale=1:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', 'pipe:1']);
    const levels = Array.from({ length: decoded.length / 3 }, (_, i) => decoded[i * 3]);
    expect(levels.length).toBe(20);
    for (let i = 0; i < levels.length; i++) expect(Math.abs(levels[i]! - (20 + i * 10)), JSON.stringify(levels)).toBeLessThan(5);
  } finally { await rm(root, { recursive: true, force: true }); }
});
