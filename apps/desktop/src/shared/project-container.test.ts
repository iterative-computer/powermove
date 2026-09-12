import { expect, it } from 'vitest';
import { decodeProjectContainer, encodeProjectContainer, encodeProjectContainerAsync, encodeTextChunks } from './project-container';

it('preserves Unicode across encoding slices and matches the binary file format', async () => {
  const document = { text: 'a'.repeat(256 * 1024 - 10) + '🟢'.repeat(100) + '\ud800', history: [1, null] };
  const media = [{ id: 'asset', type: 'image/png', data: new Uint8Array([1, 2, 255]) }];
  const actual = await encodeProjectContainerAsync(JSON.stringify(document), media);
  expect(actual).toEqual(encodeProjectContainer(document, media));
  expect(decodeProjectContainer(actual)).toMatchObject({ document, media });
  const source = 'a'.repeat(256 * 1024 - 1) + '🟢end';
  const chunks = await encodeTextChunks(source);
  expect(chunks.map(chunk => new TextDecoder().decode(chunk)).join('')).toBe(source);
});
