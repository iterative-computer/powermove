import { it, expect } from 'vitest';
import { makePM } from '../../__tests__/make-pm';
function editor() {
  const P = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  P.proj = P.mkProject();
  return P;
}
it('includes fresh import image records in one structural history step and save provenance', () => {
  const P = editor(),
    before = P.serialize(),
    asset = { id: 'preview', kind: 'image', w: 100, h: 60, name: 'Preview' };
  const result = P.Edit.mutate(
    'Import Figma',
    () => {
      P.proj.layers.push(P.mkLayer('image', { d: { asset: 'preview' } }));
    },
    { origin: 'import', importedAssets: { preview: asset } },
  );
  expect(result.ok).toBe(true);
  expect(P.proj.assets.preview).toEqual(asset);
  expect(P.proj.edits.at(-1).structural.importedAssets).toEqual({ preview: asset });
  expect(P.hist.undo()).toBe(true);
  expect(P.serialize()).toBe(before);
  expect(P.hist.redo()).toBe(true);
  expect(P.proj.assets.preview).toEqual(asset);
});
it('rejects asset overwrites and unrelated project edits', () => {
  const P = editor();
  P.proj.assets.preview = { id: 'preview', kind: 'image' };
  expect(
    P.Edit.mutate('Overwrite', () => {}, {
      origin: 'import',
      importedAssets: { preview: { id: 'preview', kind: 'image' } },
    }).ok,
  ).toBe(false);
  const before = P.serialize();
  expect(
    P.Edit.mutate(
      'Invalid change',
      () => {
        P.proj.w = 1;
      },
      { origin: 'import', importedAssets: { new: { id: 'new', kind: 'image' } } },
    ).ok,
  ).toBe(false);
  expect(P.serialize()).toBe(before);
});
