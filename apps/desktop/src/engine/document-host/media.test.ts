import { expect, it } from 'vitest';
import { WorkerMedia } from './media';

it('retains imported metadata and confirms stored models as available', async () => {
  const media = new WorkerMedia(async () => ({ path: '/store/object', origin: 'source' }), () => 'P', () => {});
  const assets = {
    svg: { id: 'svg', kind: 'image', format: 'svg', svg: { paths: [{ d: 'M0 0L1 1' }] }, editablePaths: 1, poster: true },
    animated: { id: 'animated', kind: 'video', format: 'gif', animated: true, dur: 1 },
    model: { id: 'model', kind: 'model', format: 'obj', model: { meshes: 1 }, vertices: 3, triangles: 1, layerDefinition: 'object' }
  };
  media.refresh(assets); await media.settled();
  for (const [id, asset] of Object.entries(assets)) expect(media.get(id)).toEqual({ ...asset, hasAudio: false });
  // Replacing only metadata still refreshes the view.
  const replaced = { ...assets, svg: { ...assets.svg, poster: false, editablePaths: 2 } };
  media.refresh(replaced); await media.settled();
  expect(media.get('svg')).toEqual({ ...replaced.svg, hasAudio: false });
  media.refresh({}); await media.settled(); expect(media.map.size).toBe(0);
});

it('does not present metadata as available when the original is missing', async () => {
  const media = new WorkerMedia(async () => null, () => 'P', () => {});
  media.refresh({ model: { id: 'model', kind: 'model', format: 'glb' } }); await media.settled();
  expect(media.get('model')).toBeUndefined();
});
