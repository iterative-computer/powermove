import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { test, expect, launchApp, type LaunchedApp } from './helpers/app';

async function capture(session: LaunchedApp, files: string[], output: string, label: string) {
  await session.openEditor();
  const frames: number[][] = [];
  for (const file of files) {
    await session.page.evaluate(() => {
      const PM = (window as any).PM;
      PM.replaceProject(PM.mkProject({ w: 96, h: 64, fps: 30, dur: 1, name: 'ICC parity' }));
      PM.ProjectsScreen.hide();
    });
    const chooser = session.page.waitForEvent('filechooser');
    await session.page.evaluate(() => (window as any).PM.pickFiles());
    await (await chooser).setFiles(file);
    await session.page.waitForFunction(() => (window as any).PM.proj.layers.some((l: any) => l.type === 'image'));
    if (label === 'native') await expect.poll(() => session.page.evaluate(() => {
      const s = (window as any).PM.EngineConnection.status(); return { route: s.route, reason: s.routeReason };
    })).toEqual({ route: 'engine', reason: null });
    const outputFile = path.join(output, `${label}-${path.basename(file)}.png`);
    await session.app.evaluate(({ dialog }, filePath) => { dialog.showSaveDialog = async () => ({ canceled: false, filePath }); }, outputFile);
    expect(await session.page.evaluate(async () => (window as any).PM.Export.run({ format: 'still', w: 96, h: 64, alpha: true, mblur: false }))).toEqual({ cancelled: false });
    frames.push(await session.page.evaluate(async bytes => {
      const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(96, 64), ctx = canvas.getContext('2d')!; ctx.drawImage(image, 0, 0); image.close();
      return [...ctx.getImageData(0, 0, 96, 64).data];
    }, [...await readFile(outputFile)]));
  }
  return frames;
}

test('Opaque ICC PNG and preserved TIFF originals match Chromium before filtering', async () => {
  test.setTimeout(180_000);
  const output = process.env.PM_MEDIA_EVIDENCE ? path.join(process.env.PM_MEDIA_EVIDENCE, 'icc-opaque') : await mkdtemp(path.join(os.tmpdir(), 'pm-icc-parity-'));
  await mkdir(output, { recursive: true });
  let session: LaunchedApp | null = null;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    const source = path.join(output, 'source.png');
    const data = await session.page.evaluate(() => {
      const c = document.createElement('canvas'); c.width = 192; c.height = 128; const ctx = c.getContext('2d')!;
      const pixels = ctx.createImageData(c.width, c.height);
      for (let y = 0; y < c.height; y++) for (let x = 0; x < c.width; x++) {
        const i = (y * c.width + x) * 4;
        pixels.data.set([x % 64 * 4, y % 64 * 4, (x + y) % 64 * 4, 255], i);
      }
      ctx.putImageData(pixels, 0, 0); return c.toDataURL('image/png').split(',')[1]!;
    });
    await writeFile(source, Buffer.from(data, 'base64'));
    const script = path.join(output, 'tag.swift');
    await writeFile(script, `import Foundation
import ImageIO
import CoreGraphics
let a = CommandLine.arguments
let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: a[1]) as CFURL, nil)!
let image = CGImageSourceCreateImageAtIndex(source, 0, nil)!
let space = CGColorSpace(name: a[4] == "p3" ? CGColorSpace.displayP3 : CGColorSpace.adobeRGB1998)!
let tagged = image.copy(colorSpace: space)!
let dest = CGImageDestinationCreateWithURL(URL(fileURLWithPath: a[2]) as CFURL, a[3] as CFString, 1, nil)!
CGImageDestinationAddImage(dest, tagged, nil)
exit(CGImageDestinationFinalize(dest) ? 0 : 1)
`);
    const files: string[] = [];
    for (const color of ['p3', 'adobe']) for (const [extension, uti] of [['png', 'public.png'], ['tiff', 'public.tiff']]) {
      const file = path.join(output, `${color}.${extension}`);
      execFileSync('/usr/bin/swift', [script, source, file, uti!, color], { stdio: 'pipe' }); files.push(file);
    }
    const legacy = await capture(session, files, output, 'legacy'); await session.close(); session = null;
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    const native = await capture(session, files, output, 'native');
    const metrics = native.map((pixels, file) => {
      let max = 0, over1 = 0, alpha = 0;
      for (let i = 0; i < pixels.length; i += 4) {
        const a = pixels[i + 3]!, b = legacy[file]![i + 3]!; alpha = Math.max(alpha, Math.abs(a - b));
        let d = Math.abs(a - b);
        for (let c = 0; c < 3; c++) d = Math.max(d, Math.abs(pixels[i + c]! * a / 255 - legacy[file]![i + c]! * b / 255));
        max = Math.max(max, d); if (d > 1) over1++;
      }
      return { file: path.basename(files[file]!), max, over1, pixels: pixels.length / 4, alpha };
    });
    await writeFile(path.join(output, 'metrics.json'), JSON.stringify(metrics, null, 2));
    for (const m of metrics) {
      expect(m.max, JSON.stringify(m)).toBeLessThanOrEqual(4);
      expect(m.over1 / m.pixels, JSON.stringify(m)).toBeLessThanOrEqual(0.001);
      expect(m.alpha, JSON.stringify(m)).toBeLessThanOrEqual(1);
    }
  } finally {
    await session?.close(); if (!process.env.PM_MEDIA_EVIDENCE) await rm(output, { recursive: true, force: true });
  }
});
