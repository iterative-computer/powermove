import { mkdtemp, rename, rm, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { transcriptKey } from './cache';

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });
async function fixture() {
  const root = await mkdtemp(path.join(tmpdir(), 'pm-transcript-cloud-')); roots.push(root);
  const file = path.join(root, 'synthetic.wav'); await writeFile(file, 'original audio');
  const time = new Date('2026-01-01'); await utimes(file, time, time);
  return { file, time };
}

it('invalidates a transcript after an edit with preserved size, inode and mtime', async () => {
  const { file, time } = await fixture();
  const before = await transcriptKey(file, 'model', {});
  await writeFile(file, 'modified audio'); await utimes(file, time, time);
  expect(await transcriptKey(file, 'model', {})).not.toBe(before);
});

it('reuses a transcript after metadata changes or an identical sync replacement', async () => {
  const { file } = await fixture();
  const before = await transcriptKey(file, 'model', {});
  await utimes(file, new Date(), new Date());
  expect(await transcriptKey(file, 'model', {})).toBe(before);
  await writeFile(file + '.incoming', 'original audio'); await rename(file + '.incoming', file);
  expect(await transcriptKey(file, 'model', {})).toBe(before);
});

it('keeps model, language and requested span in the transcript key', async () => {
  const { file } = await fixture();
  const keys = await Promise.all([
    transcriptKey(file, 'model', {}), transcriptKey(file, 'other-model', {}),
    transcriptKey(file, 'model', { language: 'de' }), transcriptKey(file, 'model', { start: 1 }),
    transcriptKey(file, 'model', { end: 1 })
  ]);
  expect(new Set(keys).size).toBe(keys.length);
});
