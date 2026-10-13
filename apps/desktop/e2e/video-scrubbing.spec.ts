import { execFileSync } from 'node:child_process';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { expect, repoRoot, test } from './helpers/app';

for (const sequence of [false, true]) test(`${sequence ? '4K image sequence' : 'HD video'} scrubbing presents frames throughout a drag and settles at the release position`, async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  let sources: string[];
  if (sequence) {
    // Import real numbered frames so both live and saved media carry the
    // sequence metadata that previously disabled editing previews.
    const images = await page.evaluate(() => {
      const canvas = document.createElement('canvas'); canvas.width = 2160; canvas.height = 3840;
      const context = canvas.getContext('2d')!;
      return Array.from({ length: 24 }, (_, frame) => {
        context.fillStyle = `rgb(${20 + frame * 7},${20 + frame * 7},${20 + frame * 7})`;
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.fillStyle = 'white'; context.fillRect(frame * 80, 960, 80, 1920);
        return canvas.toDataURL('image/png').split(',')[1]!;
      });
    });
    sources = images.map((_, frame) => path.join(session.userData, `scrub_${String(frame).padStart(3, '0')}.png`));
    for (const [frame, source] of sources.entries()) await writeFile(source, Buffer.from(images[frame]!, 'base64'));
  } else {
    sources = [path.join(session.userData, 'scrub-source.mp4')];
    // Ordinary compressed footage with a long interval between reference frames.
    execFileSync(path.join(repoRoot, 'node_modules/ffmpeg-static/ffmpeg'), [
      '-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=1920x1080:rate=30:duration=4',
      '-c:v', 'libx264', '-preset', 'ultrafast', '-g', '120', '-keyint_min', '120',
      '-sc_threshold', '0', '-pix_fmt', 'yuv420p', sources[0]!,
    ]);
  }
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(sources);
  if (sequence) {
    const dialog = page.getByRole('dialog', { name: 'Import image sequence', exact: true });
    await dialog.getByRole('spinbutton', { name: 'Sequence frame rate' }).fill('30');
    await dialog.getByRole('button', { name: 'Import sequence', exact: true }).click();
  }
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  await page.evaluate(async () => {
    const PM = (window as any).PM, layer = PM.proj.layers.find((l: any) => l.type === 'video');
    PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { from: 0 } });
    await PM.assets.get(layer.d.asset).previewReady;
  });
  const result = await page.evaluate(async ({ startTime, span, releaseTime }) => {
    const PM = (window as any).PM, asset = PM.assets.get(PM.proj.layers.find((l: any) => l.type === 'video').d.asset);
    const original = PM.GL.render;
    let dragging = false;
    let presentedTime: number | undefined;
    const shown: number[] = [];
    PM.GL.render = (time: number, options: any) => {
      const result = original(time, options);
      if (result !== false) presentedTime = time;
      if (dragging && result !== false) shown.push(time);
      return result;
    };
    try {
      dragging = true;
      const start = performance.now();
      await new Promise<void>(resolve => {
        const move = () => {
          const elapsed = performance.now() - start;
          if (elapsed >= 1200) { resolve(); return; }
          // Forward then reverse, with changing positions on every display tick.
          const fraction = elapsed / 1200;
          PM.setTime(startTime + span * (fraction < .5 ? fraction * 2 : 2 - fraction * 2), { raw: true });
          requestAnimationFrame(move);
        };
        move();
      });
      dragging = false;
      PM.setTime(releaseTime, { raw: true, force: true });
      await new Promise(r => setTimeout(r, 350));
      const video = asset.preview?.el || asset.el;
      const frame = new VideoFrame(video), timestamp = frame.timestamp / 1e6;
      frame.close();
      const time = PM.time;
      const settledTime = presentedTime;
      // Offline preparation must keep original pixels and the sequence frame grid.
      PM.agentFrameCapture = true;
      let originalFrame: { width: number; height: number; timestamp: number };
      try {
        await PM.prepareFrame(releaseTime);
        const source = [...PM.preparedVideoFrames.values()][0] as HTMLCanvasElement;
        const decoded = new VideoFrame(asset.el);
        originalFrame = { width: source.width, height: source.height, timestamp: decoded.timestamp / 1e6 };
        decoded.close();
      } finally { PM.agentFrameCapture = false; PM.preparedVideoFrames = null; PM.invalidate('render'); }
      return { shown, timestamp, time, settledTime, preview: !!asset.preview, originalWidth: asset.el.videoWidth,
        previewWidth: video.videoWidth, previewHeight: video.videoHeight, originalFrame,
        sequence: asset.imageSequence, savedSequence: PM.proj.assets[asset.id].imageSequence };
    } finally { PM.GL.render = original; }
  }, sequence ? { startTime: .05, span: .65, releaseTime: .5 } : { startTime: .2, span: 3, releaseTime: 2.5 });
  expect(result.preview).toBe(true);
  expect(result.originalWidth).toBe(sequence ? 2160 : 1920);
  expect(Math.max(result.previewWidth, result.previewHeight)).toBe(1280);
  expect(new Set(result.shown).size).toBeGreaterThan(sequence ? 12 : 20);
  expect(Math.max(...result.shown) - Math.min(...result.shown)).toBeGreaterThan(sequence ? .4 : 2);
  expect(result.shown.some((time, i) => i > 0 && time < result.shown[i - 1]!)).toBe(true);
  expect(result.time).toBe(sequence ? .5 : 2.5);
  expect(result.settledTime).toBe(result.time);
  expect(result.timestamp).toBeCloseTo(result.time, 2);
  expect(result.originalFrame.width).toBe(sequence ? 2160 : 1920);
  expect(result.originalFrame.height).toBe(sequence ? 3840 : 1080);
  expect(result.originalFrame.timestamp).toBeCloseTo(result.timestamp, 2);
  if (sequence) {
    expect(result.sequence).toEqual({ fps: 30, frames: 24 });
    expect(result.savedSequence).toEqual(result.sequence);
  }
  expect(session.diagnostics.pageErrors).toEqual([]);
});
