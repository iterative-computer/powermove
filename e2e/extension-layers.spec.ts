import { expect, test } from './helpers/app';

test.describe('@extensions structured programmable layers', () => {
  test('the bundled 3D layer renders, edits through source, and undoes atomically', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => {
      const PM = (window as any).PM;
      return Boolean(PM?.layerDefinition?.('powermove.3d.studio-cube') && PM.GL?.gl && PM.GL?.canvas);
    });

    const proof = await page.evaluate(() => {
      const PM = (window as any).PM;
      PM.setTime(0);
      PM.cmd('3d.add-studio-cube');
      const layer = PM.proj.layers.find((candidate: any) => candidate.d?.definition === 'powermove.3d.studio-cube');
      const before = PM.Export.snapshot(0, 320);
      const edit = PM.Edit.apply({
        type: 'set_property', target: layer.id, path: 'x.objectColor', value: '#FF3344', mode: 'static'
      }, { label: 'Recolor 3D object', origin: 'interface' });
      const after = PM.Export.snapshot(0, 320);
      const shaderKey = PM.UIState.getShaderMeta(layer)?.shaderKey;
      const serialized = JSON.parse(PM.serialize());
      const savedLayer = serialized.proj.layers.find((candidate: any) => candidate.id === layer.id);
      PM.hist.undo();
      const restored = PM.L(layer.id);
      return {
        edit,
        type: layer.type,
        definition: layer.d.definition,
        storedKeys: Object.keys(layer.d).sort(),
        object: layer.d.data.objects[0],
        beforeLength: before.length,
        changedPixels: before !== after,
        compileError: shaderKey ? PM.GL.compileError(shaderKey) ?? null : 'renderer did not run',
        savedDefinition: savedLayer.d.definition,
        savedObjectColor: savedLayer.d.params.objectColor.v,
        serializedHasRendererCode: JSON.stringify(savedLayer).includes('void main'),
        colorAfterUndo: restored.d.params.objectColor.v,
      };
    });

    expect(proof).toMatchObject({
      type: 'extension',
      definition: 'powermove.3d.studio-cube',
      storedKeys: ['data', 'definition', 'h', 'params', 'version', 'w'],
      object: { id: 'cube', geometry: 'rounded-box', material: 'metal' },
      changedPixels: true,
      compileError: null,
      savedDefinition: 'powermove.3d.studio-cube',
      savedObjectColor: '#FF3344',
      serializedHasRendererCode: false,
      colorAfterUndo: '#C7C4FF',
    });
    expect(proof.edit.ok).toBe(true);
    expect(proof.beforeLength).toBeGreaterThan(2_000);
    expect(session.diagnostics.pageErrors).toEqual([]);
  });

  test('imports a durable OBJ asset and renders its structured mesh layer', async ({ session }) => {
    const { page } = session;
    await page.waitForFunction(() => Boolean((window as any).PM?.layerDefinition?.('powermove.3d.obj-model')));

    const proof = await page.evaluate(async () => {
      const PM = (window as any).PM;
      PM.setTime(0);
      const source = `
v -1 -1 -1
v  1 -1 -1
v  1  1 -1
v -1  1 -1
v -1 -1  1
v  1 -1  1
v  1  1  1
v -1  1  1
f 1 2 3 4
f 5 8 7 6
f 1 5 6 2
f 2 6 7 3
f 3 7 8 4
f 5 1 4 8
`;
      const file = new File([source], 'proof-cube.obj', { type: 'model/obj' });
      const result = await PM.cmd('3d.import-obj', file);
      const layer = PM.proj.layers.find((candidate: any) => candidate.d?.definition === 'powermove.3d.obj-model');
      const assetId = layer.d.data.assetId;
      const asset = PM.proj.assets[assetId];
      const first = PM.Export.snapshot(0, 320);
      const shaderKey = PM.UIState.getShaderMeta(layer)?.shaderKey;
      const recolor = PM.Edit.apply({
        type: 'set_property', target: layer.id, path: 'x.objectColor', value: '#22CC88', mode: 'static'
      }, { label: 'Recolor OBJ', origin: 'interface' });
      const second = PM.Export.snapshot(0, 320);
      const serialized = JSON.parse(PM.serialize());
      const savedLayer = serialized.proj.layers.find((candidate: any) => candidate.id === layer.id);
      const savedAsset = serialized.proj.assets[assetId];
      PM.assets.map.delete(assetId);
      PM.Export.snapshot(0, 160);
      const missingMeta = PM.UIState.getShaderMeta(layer);
      const referenceCount = PM.MediaImport.referenceCount(PM.proj, assetId);
      PM.hist.undo();
      const colorAfterEditUndo = PM.L(layer.id)?.d?.params?.objectColor?.v;
      PM.hist.undo();
      return {
        result,
        recolor,
        layerType: layer.type,
        definition: layer.d.definition,
        asset: { kind: asset.kind, format: asset.format, vertices: asset.vertices, triangles: asset.triangles },
        rendered: first.length > 2_000,
        changedPixels: first !== second,
        compileError: shaderKey ? PM.GL.compileError(shaderKey) ?? null : 'mesh renderer did not run',
        savedData: savedLayer.d.data,
        savedAsset,
        savedLayerHasGeometry: JSON.stringify(savedLayer).includes('positions'),
        missingAsset: missingMeta.missing === true && missingMeta.missingAsset === assetId,
        referenceCount,
        colorAfterEditUndo,
        layerAfterUndo: Boolean(PM.L(layer.id)),
        assetAfterUndo: Boolean(PM.proj.assets[assetId]),
      };
    });

    expect(proof.result.ok).toBe(true);
    expect(proof.recolor.ok).toBe(true);
    expect(proof).toMatchObject({
      layerType: 'extension',
      definition: 'powermove.3d.obj-model',
      asset: { kind: 'model', format: 'obj', vertices: 8, triangles: 12 },
      rendered: true,
      changedPixels: true,
      compileError: null,
      savedData: { assetId: proof.savedAsset.id },
      savedLayerHasGeometry: false,
      missingAsset: true,
      referenceCount: 1,
      colorAfterEditUndo: '#C7C4FF',
      layerAfterUndo: false,
      assetAfterUndo: true,
    });
    expect(session.diagnostics.pageErrors).toEqual([]);
  });
});
