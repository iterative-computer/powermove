import { randomBytes } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { expect, it } from 'vitest';
import { decodeStorageJSON, encodeStorageJSON } from './storage-codec';

it('compresses recovery losslessly, including Unicode and Undo patches', async () => {
  const value = { project: { text: 'Café 🟢'.repeat(30_000) }, history: { undo: [{ path: ['name'], value: 'Avant' }] } };
  const json = JSON.stringify(value), encoded = await encodeStorageJSON(json);
  expect(Buffer.byteLength(encoded)).toBeLessThan(Buffer.byteLength(json) / 10);
  expect(decodeStorageJSON(encoded, 1024 * 1024)).toEqual({ value, compressed: true, bytes: Buffer.byteLength(json) });
});

it('leaves small and poorly compressible records in ordinary JSON', async () => {
  for (const json of ['{"name":"small"}', JSON.stringify({ noise: randomBytes(50_000).toString('base64') })]) {
    expect(await encodeStorageJSON(json)).toBe(json);
    expect(decodeStorageJSON(json, 1024 * 1024).value).toEqual(JSON.parse(json));
  }
});

it('rejects corrupt, oversized and deceptive compressed recovery records', () => {
  const bytes = Buffer.from('{"text":"' + 'x'.repeat(100_000) + '"}');
  const envelope = { __powermoveDiskValue: 1, encoding: 'gzip', bytes: bytes.length, data: gzipSync(bytes).toString('base64') };
  expect(() => decodeStorageJSON(JSON.stringify(envelope), 1024)).toThrow('Invalid');
  expect(() => decodeStorageJSON(JSON.stringify({ ...envelope, bytes: 100 }), 1024)).toThrow();
  expect(() => decodeStorageJSON(JSON.stringify({ ...envelope, data: 'broken' }), 1024 * 1024)).toThrow();
  expect(() => decodeStorageJSON(JSON.stringify({ ...envelope, bytes: bytes.length + 1 }), 1024 * 1024)).toThrow('incomplete');
});
