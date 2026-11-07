import { it, expect } from 'vitest';
import { clipboardImages, imageHash } from './images';
import { allFeatures } from '../../../../../shared/figma/__fixtures__/all-features';
import { figmaNode } from './__fixtures__/clipboard';
it('recovers a hash-only image from kiwi blobs with an exact digest match', async () => {
  const bytes = Uint8Array.from(Buffer.from(allFeatures().images.checker!, 'base64')),
    hash = new Uint8Array(await crypto.subtle.digest('SHA-1', bytes));
  const selection = {
    nodes: [figmaNode(1, 'RECTANGLE', { fillPaints: [{ type: 'IMAGE', image: { hash } }] })],
    blobs: [Uint8Array.from([1, 2, 3]), bytes],
  };
  expect(await clipboardImages(selection)).toEqual({ [imageHash(hash)]: bytes });
});
it('does not guess when an image hash cannot be associated with any blob', async () => {
  const bytes = Uint8Array.from(Buffer.from(allFeatures().images.checker!, 'base64'));
  expect(
    await clipboardImages({
      nodes: [
        figmaNode(1, 'RECTANGLE', { fillPaints: [{ type: 'IMAGE', image: { hash: new Uint8Array(20) } }] }),
      ],
      blobs: [bytes],
    }),
  ).toEqual({});
});
