import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, test, launchApp, type LaunchedApp } from './helpers/app';
import { importFixture } from './helpers/media';

async function capture(session: LaunchedApp, fixture: string, output: string, label: string) {
  await session.openEditor();
  const { page } = session;
  await page.evaluate(() => {
    const PM = (window as any).PM;
    PM.replaceProject(PM.mkProject({ w: 160, h: 90, fps: 30, dur: 2, name: 'Persisted animation parity' }));
    PM.ProjectsScreen.hide();
  });
  await importFixture(page, fixture);
  await page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'video'));
  if (label === 'native') {
    await expect.poll(() => page.evaluate(() => {
      const s = (window as any).PM.EngineConnection.status();
      return { route: s.route, reason: s.routeReason };
    }), { timeout: 45_000 }).toEqual({ route: 'engine', reason: null });
  }
  const stored = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const asset = Object.values(PM.proj.assets)[0];
    const blob = await PM.MediaStore.get(asset);
    return { bytes: [...new Uint8Array(await blob.arrayBuffer())], document: PM.proj };
  });
  expect(stored.bytes.slice(0, 4)).toEqual([0x1a, 0x45, 0xdf, 0xa3]);
  await writeFile(path.join(output, `${label}-source.webm`), new Uint8Array(stored.bytes));
  await writeFile(path.join(output, `${label}-document.json`), JSON.stringify(stored.document));
  const frames: number[][] = [];
  for (const frame of [0, 1, 2]) {
    const filePath = path.join(output, `${label}-${frame}.png`);
    await session.app.evaluate(({ dialog }, filePath) => {
      dialog.showSaveDialog = async () => ({ canceled: false, filePath });
    }, filePath);
    expect(await page.evaluate(async time => {
      const PM = (window as any).PM;
      PM.setTime(time, { raw: true, force: true });
      return PM.Export.run({ format: 'still', w: 160, h: 90, alpha: true, mblur: false });
    }, frame / 10)).toEqual({ cancelled: false });
    frames.push(await page.evaluate(async bytes => {
      const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(160, 90), ctx = canvas.getContext('2d')!;
      ctx.drawImage(image, 0, 0); image.close();
      return [...ctx.getImageData(0, 0, 160, 90).data];
    }, [...await readFile(filePath)]));
  }
  return frames;
}

for (const fixture of ['animated.gif', 'animated-apng.png', 'animated.webp']) {
  test(`persisted ${fixture} matches Chromium within the video class`, async () => {
    test.setTimeout(180_000);
    const output = process.env.PM_MEDIA_EVIDENCE
      ? path.join(process.env.PM_MEDIA_EVIDENCE, fixture)
      : await mkdtemp(path.join(os.tmpdir(), 'pm-animation-parity-'));
    await mkdir(output, { recursive: true });
    let session: LaunchedApp | null = null;
    try {
      session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
      const legacy = await capture(session, fixture, output, 'legacy');
      await session.close(); session = null;
      session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
      const native = await capture(session, fixture, output, 'native');
      const metrics = native.map((pixels, frame) => {
        const reference = legacy[frame]!;
        let max = 0, alpha = 0, interiorPixels = 0;
        const premul = (p: number[], i: number, c: number) => p[i + c]! * p[i + 3]! / 255;
        for (let y = 0; y < 90; y++) for (let x = 0; x < 160; x++) {
          const i = (y * 160 + x) * 4;
          alpha = Math.max(alpha, Math.abs(pixels[i + 3]! - reference[i + 3]!));
          if (x === 0 || y === 0 || x === 159 || y === 89) continue;
          let uniform = true;
          for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
            const n = ((y + dy) * 160 + x + dx) * 4;
            for (let c = 0; c < 3; c++) if (Math.abs(premul(reference, n, c) - premul(reference, i, c)) > 1) uniform = false;
          }
          if (!uniform) continue;
          interiorPixels++;
          for (let c = 0; c < 3; c++) max = Math.max(max, Math.abs(premul(pixels, i, c) - premul(reference, i, c)));
        }
        return { frame, max, alpha, interiorPixels };
      });
      await writeFile(path.join(output, 'metrics.json'), JSON.stringify(metrics, null, 2));
      for (const m of metrics) {
        expect(m.interiorPixels).toBeGreaterThan(0);
        expect(m.max, JSON.stringify(m)).toBeLessThanOrEqual(2);
        expect(m.alpha, JSON.stringify(m)).toBeLessThanOrEqual(1);
      }
    } finally {
      await session?.close();
      if (!process.env.PM_MEDIA_EVIDENCE) await rm(output, { recursive: true, force: true });
    }
  });
}
