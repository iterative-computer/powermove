import { mkdtemp, readdir, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { FileUpload, FILE_CHUNK_BYTES } from './file-upload';
import { atomicWrite } from './project-files';

it('spools bounded chunks, rejects concurrent writes, streams once, and removes staging data', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'pm-upload-unit-'));
  const upload = await FileUpload.create(FILE_CHUNK_BYTES + 3, directory);
  try {
    const first = new Uint8Array(FILE_CHUNK_BYTES).fill(37);
    const pending = upload.write(first);
    expect(() => upload.write(new Uint8Array([1]))).toThrow('Invalid save chunk');
    expect(() => upload.claim()).toThrow('incomplete');
    await pending;
    expect(() => upload.claim()).toThrow('incomplete');
    await upload.write(new Uint8Array([1, 2, 3]));
    const destination = path.join(directory, 'result');
    await atomicWrite(destination, upload.claim());
    const result = await readFile(destination);
    expect(result.length).toBe(FILE_CHUNK_BYTES + 3);
    expect(result.subarray(0, FILE_CHUNK_BYTES)).toEqual(Buffer.from(first));
    expect([...result.subarray(FILE_CHUNK_BYTES)]).toEqual([1, 2, 3]);
    expect(() => upload.claim()).toThrow('incomplete');
    await upload.dispose();
    expect(await readdir(directory)).toEqual(['result']);
  } finally { await upload.dispose(); await rm(directory, { recursive: true, force: true }); }
});

it('allows multi-gigabyte declarations without allocating them and cleans up aborted writes', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'pm-upload-unit-'));
  try {
    const upload = await FileUpload.create(5 * 1024 ** 3, directory);
    expect(() => upload.write(new Uint8Array(FILE_CHUNK_BYTES + 1))).toThrow('Invalid save chunk');
    const pending = upload.write(new Uint8Array([1, 2, 3]));
    await upload.dispose(); await pending;
    expect(await readdir(directory)).toEqual([]);
    expect(() => upload.write(new Uint8Array([1]))).toThrow('Invalid save chunk');
  } finally { await rm(directory, { recursive: true, force: true }); }
});
