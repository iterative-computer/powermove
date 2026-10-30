import { expect, test } from './helpers/app';

test.beforeEach(async ({ session }) => { await session.openEditor(); });

test('@viewer imports SVG as durable editable Shape paths and preserves its source', async ({ session }) => {
  const { page } = session;
  await page.waitForFunction(() => { const PM = (window as any).PM, viewer = PM.Kernel.services.get('viewer'); return Boolean(viewer?.ov && PM?.GL?.gl); });

  const proof = await page.evaluate(async () => {
    const PM = (window as any).PM;
    const viewer = PM.Kernel.services.get('viewer');
    const project = PM.mkProject({ name: 'SVG source', w: 640, h: 360, fps: 30, dur: 4, bg: '#000000' });
    PM.replaceProject(project);
    const source = '<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32" viewBox="0 0 64 32"><rect width="64" height="32" rx="8" fill="#ff5a1f"/><circle cx="48" cy="16" r="8" fill="#ffffff"/></svg>';
    await PM.importFiles([new File([source], 'vector-card.svg', { type: '' })]);
    const asset = Object.values(PM.proj.assets).find((candidate: any) => candidate.name === 'vector-card.svg') as any;
    const layer = PM.proj.layers.find((candidate: any) => candidate.d?.svgSourceAsset === asset?.id);
    if (!asset || !layer) throw new Error('SVG import did not create an editable Shape layer');
    delete asset.format;
    PM.assets.clear();
    await PM.assets.restoreProject(PM.proj);
    layer.p['scale.x'].v = 200; layer.p['scale.y'].v = 200;
    const path = layer.d.paths[0], vertex = path.vertices[0];
    const vertexChannel = `g.${path.id}.v.${vertex.id}.x`;
    const originalVertexX = vertex.p.x.v;
    const edited = PM.Edit.apply({ type: 'set_property', target: layer.id, path: vertexChannel, value: originalVertexX + 8, time: 1, preserveHandEdits: false }, { label: 'Edit SVG vertex', origin: 'canvas' });
    const editedVertexX = vertex.p.x.v;
    PM.hist.undo();
    const restoredVertexX = PM.L(layer.id).d.paths[0].vertices[0].p.x.v;
    PM.setTime(1, { raw: true, force: true });
    viewer.fit = true;
    viewer.layout();
    PM.invalidate('render');
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));

    PM.perf.auto = false;
    PM.quality = 1;
    PM.previewResolution = '1';
    layer.p['scale.x'].v = 3471.293; layer.p['scale.y'].v = 3471.293;
    viewer.setZoom(8);
    PM.invalidate('render');
    await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    const highZoom = {
      viewport: { ...PM.GL.previewViewport },
      viewportVectors: PM.GL.stats.viewportVectors,
      canvas: { width: PM.GL.canvas.width, height: PM.GL.canvas.height },
      viewportTextureBytes: PM.GL.texes.get('viewport-path:' + layer.id)?.bytes || 0,
    };
    layer.p['scale.x'].v = 200; layer.p['scale.y'].v = 200;
    viewer.fit = true;
    viewer.layout();

    const stored = await PM.MediaStore.get(asset);
    const beforeUndo = PM.proj.layers.length;
    PM.hist.undo();
    return {
      asset: { kind: asset.kind, format: asset.format, width: asset.w, height: asset.h },
      layer: {
        type: layer.type, sourceAsset: layer.d.svgSourceAsset,
        paths: layer.d.paths.length, vertices: layer.d.paths.map((candidate: any) => candidate.vertices.length),
      },
      vertexEdit: { ok: edited.ok, originalVertexX, editedVertexX, restoredVertexX },
      stored: stored ? await stored.text() : '',
      highZoom,
      undo: { before: beforeUndo, after: PM.proj.layers.length, assetPreserved: Boolean(PM.proj.assets[asset.id]) },
    };
  });

  expect(proof.asset).toEqual({ kind: 'image', format: 'svg', width: 64, height: 32 });
  expect(proof.layer).toEqual({ type: 'shape', sourceAsset: expect.any(String), paths: 2, vertices: [8, 4] });
  expect(proof.vertexEdit.ok).toBe(true);
  expect(proof.vertexEdit.editedVertexX).toBe(proof.vertexEdit.originalVertexX + 8);
  expect(proof.vertexEdit.restoredVertexX).toBe(proof.vertexEdit.originalVertexX);
  expect(proof.stored).toContain('<svg');
  expect(proof.highZoom.viewport.width).toBeLessThan(640);
  expect(proof.highZoom.viewport.height).toBeLessThan(360);
  expect(proof.highZoom.viewportVectors).toBeGreaterThan(0);
  expect(proof.highZoom.canvas.width).toBeGreaterThan(0);
  expect(proof.highZoom.canvas.height).toBeGreaterThan(0);
  expect(proof.highZoom.viewportTextureBytes).toBe(
    proof.highZoom.canvas.width * proof.highZoom.canvas.height * 4,
  );
  expect(proof.undo).toEqual({ before: 1, after: 0, assetPreserved: true });
  expect(session.diagnostics.pageErrors).toEqual([]);
});
