/** The half-scale video viewer must preserve today's editing-copy pixels. */
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect, launchApp } from './helpers/app';
import { ffmpegBinary } from './helpers/ffmpeg';

test('video at Half viewer resolution matches the current editing preview', async () => {
  test.setTimeout(240_000);
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-half-video-'));
  const evidence = process.env.PM_PHASE4A_EVIDENCE;
  const frames: Array<{ data: number[]; width: number; height: number }> = [];
  try {
    const source = path.join(root, 'pattern.mov');
    execFileSync(ffmpegBinary, ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=24',
      '-vf', 'loop=loop=-1:size=1:start=0', '-frames:v', '24', '-c:v', 'prores_videotoolbox', '-profile:v', '2', '-pix_fmt', 'yuv422p10le', source], { timeout: 60_000 });
    for (const native of [false, true]) {
      let session: Awaited<ReturnType<typeof launchApp>> | null = null;
      try {
        session = await launchApp({ userData: path.join(root, native ? 'native' : 'current'), env: { POWERMOVE_NEW_ENGINE: native ? '1' : '0' } });
        const { app, page } = session;
        await session.openEditor();
        await page.evaluate(() => {
          const PM = (window as any).PM;
          PM.proj = PM.mkProject({ name: 'Half scale parity', w: 640, h: 360, fps: 24, dur: 1 });
          PM.touch(); PM.bus.emit('project');
          const input = document.createElement('input'); input.type = 'file'; input.id = 'half-source'; document.body.appendChild(input);
        });
        await page.locator('#half-source').setInputFiles(source);
        await page.evaluate(async () => {
          const PM = (window as any).PM;
          await PM.importFiles([(document.querySelector('#half-source') as HTMLInputElement).files![0]]);
          PM.sel.layers = []; PM.perf.auto = false; PM.previewResolution = '0.5'; PM.quality = .5;
          PM.pause(); PM.setTime(0, { raw: true, force: true }); PM.touch(); PM.bus.emit('quality');
          const viewer = PM.Kernel.services.get('viewer'); viewer.setZoom(.5); viewer.layout(); PM.invalidate();
        });
        await page.waitForFunction(() => {
          const PM = (window as any).PM;
          const layer = PM.proj.layers.find((l: any) => l.type === 'video');
          return !!PM.assets.get(layer.d.asset)?.preview?.el;
        }, undefined, { timeout: 90_000 }).catch((error) => {
          throw new Error(`${String(error)}\n${JSON.stringify(session?.diagnostics)}`);
        });
        await page.evaluate(() => { const PM = (window as any).PM; PM.setTime(0, { raw: true, force: true }); PM.invalidate(); });
        if (native) await page.waitForFunction(() => (window as any).PM.EngineConnection?.session()?.lastFrame?.quality?.resolution === 'half', undefined, { timeout: 30_000 });
        await page.waitForTimeout(500);
        const clip = await page.locator('#gl').boundingBox();
        expect(clip).not.toBeNull();
        const png = await page.screenshot({ clip: clip!, animations: 'disabled' });
        if (evidence) { await mkdir(evidence, { recursive: true }); await writeFile(path.join(evidence, native ? 'native-half.png' : 'current-half.png'), png); }
        frames.push(await app.evaluate(({ nativeImage }, bytes) => {
          const image = nativeImage.createFromBuffer(Buffer.from(bytes));
          const size = image.getSize(); return { data: [...image.toBitmap()], ...size };
        }, [...png]));
      } finally { await session?.close(); }
    }
    expect([frames[1]!.width, frames[1]!.height]).toEqual([frames[0]!.width, frames[0]!.height]);
    let max = 0, overOne = 0;
    for (let i = 0; i < frames[0]!.data.length; i += 4) {
      const diff = Math.max(...[0, 1, 2].map((c) => Math.abs(frames[0]!.data[i + c]! - frames[1]!.data[i + c]!)));
      max = Math.max(max, diff); if (diff > 1) overOne++;
    }
    const metrics = { max, overOne, pixels: frames[0]!.width * frames[0]!.height };
    if (evidence) await writeFile(path.join(evidence, 'half-video-metrics.json'), JSON.stringify(metrics));
    expect(max).toBeLessThanOrEqual(4);
    expect(overOne / metrics.pixels).toBeLessThanOrEqual(.001);
  } finally { await rm(root, { recursive: true, force: true }); }
});
