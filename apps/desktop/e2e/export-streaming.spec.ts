import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { decodeProjectContainer } from '../src/shared/project-container';
import { expect, test } from './helpers/app';

for (const format of ['json', 'web']) test(`streams large ${format === 'json' ? 'project' : 'code'} exports with identical source media`, async ({ session }) => {
  await session.openEditor();
  const output = path.join(session.userData, format === 'json' ? 'portable.pmv' : 'portable.zip');
  await writeFile(output, 'previous export');
  await session.app.evaluate(({ dialog }, filePath) => {
    dialog.showSaveDialog = async () => ({ canceled: false, filePath });
  }, output);
  const result = await session.page.evaluate(async format => {
    const PM = (window as any).PM;
    PM.pause();
    PM.proj = PM.mkProject({ name: 'Portable streaming', w: 64, h: 64, fps: 10, dur: 1 });
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 16;
    const context = canvas.getContext('2d')!; context.fillStyle = '#00ff00'; context.fillRect(0, 0, 16, 16);
    const png = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!)));
    const file = new File([png, ...Array(12).fill(new Uint8Array(1024 * 1024))], 'large.png', { type: 'image/png' });
    await PM.importFiles([file]); await PM.app.importQueue;
    const before = PM.serialize(), time = PM.time;
    const result = await PM.Export.run({ format, audio: false });
    return { result, preserved: PM.serialize() === before && PM.time === time, prefix: Array.from(new Uint8Array(await png.arrayBuffer())), bytes: file.size };
  }, format);
  expect(result.result).toEqual({ cancelled: false });
  expect(result.preserved).toBe(true);
  const media = format === 'json'
    ? decodeProjectContainer(await readFile(output)).media[0]!.data
    : execFileSync('unzip', ['-p', output, 'assets/media-0.png'], { maxBuffer: 16 * 1024 * 1024 });
  expect(media.length).toBe(result.bytes);
  const expected = createHash('sha256').update(new Uint8Array(result.prefix)).update(Buffer.alloc(12 * 1024 * 1024)).digest('hex');
  expect(createHash('sha256').update(media).digest('hex')).toBe(expected);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
