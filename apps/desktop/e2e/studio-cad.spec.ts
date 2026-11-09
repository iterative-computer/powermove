import { readFileSync } from 'node:fs';
import path from 'node:path';
import { expect, test } from './helpers/app';

/* STEP import: OpenCascade tessellates the assembly in a worker into a GLB
   that keeps its parts, names and colors; Studio renders it. The model is a
   CAX-IF test assembly shipped with occt-import-js. */
const testfiles = path.join(path.dirname(require.resolve('occt-import-js/package.json')), 'test', 'testfiles');

test('a STEP assembly imports with its parts and renders in Studio', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const step = readFileSync(path.join(testfiles, 'cax-if', 'as1-oc-214.stp')).toString('base64');
  const result = await page.evaluate(async (data: string) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-cad');
    const bytes = Uint8Array.from(atob(data), c => c.charCodeAt(0));
    const t = performance.now();
    const packed = await api.scene3d.prepareImport([new File([bytes], 'as1-oc-214.stp')]);
    const tessellateMs = performance.now() - t;
    const imported = await PM.cmd('3d.import-obj', packed);
    const layer = PM.proj.layers.find((l: any) => l.d?.definition === 'powermove.3d.object');
    const asset = PM.assets.get(layer.d.data.object.source.assetId);
    let meshes = 0, named = 0;
    const colors = new Set<string>();
    asset.object3d.traverse((o: any) => {
      if (!o.isMesh) return;
      meshes++;
      if (o.name) named++;
      for (const m of [o.material].flat()) colors.add(m.color.getHexString());
    });
    return { ok: imported.ok, name: packed.name, format: asset.format, triangles: asset.triangles, meshes, named, colors: colors.size, tessellateMs };
  }, step);
  console.log(`STEP: ${result.meshes} parts, ${result.triangles} triangles, ${result.colors} colors, tessellated in ${result.tessellateMs.toFixed(0)} ms`);
  expect(result).toMatchObject({ ok: true, name: 'as1-oc-214.glb', format: 'glb' });
  // An assembly of plates, brackets, nuts and bolts.
  expect(result.meshes).toBeGreaterThan(10);
  expect(result.named).toBe(result.meshes);
  expect(result.colors).toBeGreaterThan(1);
  expect(result.triangles).toBeGreaterThan(1000);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const frame = await page.evaluate(() => {
    const PM = (window as any).PM;
    const before = PM.Studio.status().frames;
    PM.agentFrameCapture = true;
    const px = PM.GL.renderToPixels(0, 320, 180, { transparent: false, mblur: false });
    PM.agentFrameCapture = false;
    let lit = 0;
    for (let i = 0; i < px.length; i += 4) if (Math.abs(px[i]! - 246) + Math.abs(px[i + 1]! - 245) + Math.abs(px[i + 2]! - 248) > 30) lit++;
    return { studio: PM.Studio.status().frames > before, lit };
  });
  // Imported into an empty composition, it arrives in a lit studio set.
  expect(frame.studio).toBe(true);
  expect(frame.lit).toBeGreaterThan(2000);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
