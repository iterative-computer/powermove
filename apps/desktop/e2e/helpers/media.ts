import path from 'node:path';
import { readFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';

import type { Page } from 'playwright';

import { expect, repoRoot, type launchApp } from './app';

export function fixturePath(name: string): string {
  return path.join(repoRoot, 'e2e/fixtures', name);
}

export async function importFixture(page: Page, name: string): Promise<void> {
  const chooser = page.waitForEvent('filechooser');
  await page.evaluate(() => (window as any).PM.pickFiles());
  await (await chooser).setFiles(fixturePath(name));
}

/** Sample the delivered PNG, including the real native save path. */
export async function exportStillPixels(session: Awaited<ReturnType<typeof launchApp>>, time: number,
  width: number, height: number, points: number[][]): Promise<number[][]> {
  const output = path.join(session.userData, `sample-${randomUUID()}.png`);
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, output);
  const result = await session.page.evaluate(async ({ time, width, height }) => {
    const PM = (window as any).PM;
    PM.setTime(time, { raw: true, force: true });
    return PM.Export.run({ format: 'still', w: width, h: height, alpha: true, mblur: false });
  }, { time, width, height });
  expect(result).toEqual({ cancelled: false });
  return session.page.evaluate(async ({ bytes, width, height, points }) => {
    const image = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: 'image/png' }));
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
    const ctx = canvas.getContext('2d')!;
    ctx.drawImage(image, 0, 0, width, height); image.close();
    return points.map(([x, y]) => [...ctx.getImageData(x!, y!, 1, 1).data]);
  }, { bytes: [...await readFile(output)], width, height, points });
}
