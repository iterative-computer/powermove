import { expect, it } from 'vitest';
import { decodeProjectContainer, encodeProjectContainer, encodeProjectContainerAsync, encodeTextChunks, projectContainerIndex } from './project-container';

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

it('rejects duplicate, truncated and unsafe media ranges before import', () => {
  const source = { id: 'video', type: 'video/webm', offset: 0, length: 10 };
  for (const media of [[source, source], [{ ...source, length: 11 }], [{ ...source, offset: -1 }],
    [{ ...source, offset: Number.MAX_SAFE_INTEGER }], [{ ...source, length: 1.5 }]]) {
    expect(() => projectContainerIndex({ document: {}, media }, 9, 19)).toThrow('index');
  }
});
