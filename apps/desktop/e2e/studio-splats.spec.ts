import { expect, test } from './helpers/app';

/** A 3DGS PLY: a rainbow torus of `n` small Gaussians (y-down, as captures are). */
function torusPly(n: number): Buffer {
  const names = ['x', 'y', 'z', 'f_dc_0', 'f_dc_1', 'f_dc_2', 'opacity', 'scale_0', 'scale_1', 'scale_2', 'rot_0', 'rot_1', 'rot_2', 'rot_3'];
  const head = Buffer.from(`ply\nformat binary_little_endian 1.0\nelement vertex ${n}\n${names.map(p => `property float ${p}`).join('\n')}\nend_header\n`);
  const body = Buffer.alloc(n * names.length * 4);
  const SH = 0.28209479;
  for (let i = 0; i < n; i++) {
    const u = (i / n) * Math.PI * 2 * 1, v = ((i * 7919) % n) / n * Math.PI * 2;
    const x = (1 + 0.35 * Math.cos(v)) * Math.cos(u), z = (1 + 0.35 * Math.cos(v)) * Math.sin(u), y = -0.35 * Math.sin(v);
    const r = 0.5 + 0.5 * Math.cos(u), g = 0.5 + 0.5 * Math.cos(u + 2.09), b = 0.5 + 0.5 * Math.cos(u + 4.19);
    const values = [x, y, z, (r - 0.5) / SH, (g - 0.5) / SH, (b - 0.5) / SH, 3, Math.log(0.03), Math.log(0.03), Math.log(0.012), 1, 0, 0, 0];
    values.forEach((value, k) => body.writeFloatLE(value, (i * names.length + k) * 4));
  }
  return Buffer.concat([head, body]);
}

/* A Gaussian-splat capture imports as a model layer, draws with the splat
   rasterizer and composites with Studio-rendered layers by layer order. */
test('a Gaussian splat capture imports and renders beside Studio layers', async ({ session }) => {
  test.setTimeout(120_000);
  await session.openEditor();
  const { page } = session;
  const result = await page.evaluate(async (data: string) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-splats');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 1.2, z: 3.2, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'area', p: { x: -1, y: 3, z: 2, intensity: 6, width: 2, height: 1.5 } } });
    const bytes = Uint8Array.from(atob(data), c => c.charCodeAt(0));
    const packed = await api.scene3d.prepareImport([new File([bytes], 'torus.ply')]);
    const imported = await PM.cmd('3d.import-obj', packed);
    api.scene3d.edit({ operation: 'add_object', object: { id: 'cube', source: { primitive: 'box' }, p: { sx: 0.4, sy: 0.4, sz: 0.4 }, material: { p: { color: '#E8E8EA', roughness: 0.4 } } } });
    const layer = PM.proj.layers.find((l: any) => l.d?.data?.object?.source?.assetId);
    const asset = PM.assets.get(layer.d.data.object.source.assetId);
    return { ok: imported.ok, format: asset.format, splats: asset.splats };
  }, torusPly(80_000).toString('base64'));
  expect(result).toMatchObject({ ok: true, format: 'splat', splats: 80_000 });
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const frame = await page.evaluate(() => {
    const PM = (window as any).PM;
    const before = PM.Studio.status().frames;
    PM.agentFrameCapture = true;
    const px = PM.GL.renderToPixels(0, 480, 270, { transparent: true, mblur: false });
    PM.agentFrameCapture = false;
    let covered = 0, red = 0, green = 0, blue = 0;
    for (let i = 0; i < px.length; i += 4) {
      if (px[i + 3]! < 40) continue;
      covered++;
      const [r, g, b] = [px[i]!, px[i + 1]!, px[i + 2]!];
      if (r > g + 60 && r > b + 60) red++;
      if (g > r + 60 && g > b + 60) green++;
      if (b > r + 60 && b > g + 60) blue++;
    }
    return { covered, red, green, blue, studio: PM.Studio.status().frames > before };
  });
  // The torus's rainbow (splats) and the cube (Studio) are both in the frame.
  expect(frame.covered).toBeGreaterThan(8000);
  expect(Math.min(frame.red, frame.green, frame.blue)).toBeGreaterThan(200);
  expect(frame.studio).toBe(true);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
