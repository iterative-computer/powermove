import { expect, test } from './helpers/app';

/** A GLB assembly of `parts` rippled panels, 2·n² triangles each (like a CAD export). */
function assembly(parts: number, n: number): Buffer {
  const chunks: Buffer[] = [], meshes: any[] = [], nodes: any[] = [], accessors: any[] = [], views: any[] = [];
  let offset = 0;
  const view = (data: Buffer, target: number) => { views.push({ buffer: 0, byteOffset: offset, byteLength: data.length, target }); chunks.push(data); offset += data.length; return views.length - 1; };
  for (let p = 0; p < parts; p++) {
    const ox = (p % 5) * 2.2 - 4.4, oz = Math.floor(p / 5) * 2.2 - 3.3;
    const pos = new Float32Array((n + 1) * (n + 1) * 3);
    for (let y = 0, k = 0; y <= n; y++) for (let x = 0; x <= n; x++, k += 3) {
      const u = x / n, v = y / n;
      pos[k] = ox + u * 2; pos[k + 1] = 0.15 * Math.sin(u * 12 + p) * Math.cos(v * 9); pos[k + 2] = oz + v * 2;
    }
    const idx = new Uint32Array(n * n * 6);
    for (let y = 0, k = 0; y < n; y++) for (let x = 0; x < n; x++, k += 6) {
      const a = y * (n + 1) + x, b = a + 1, c = a + n + 1, d = c + 1;
      idx.set([a, c, b, b, c, d], k);
    }
    let lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let k = 0; k < pos.length; k += 3) for (let a = 0; a < 3; a++) { lo[a] = Math.min(lo[a]!, pos[k + a]!); hi[a] = Math.max(hi[a]!, pos[k + a]!); }
    accessors.push({ bufferView: view(Buffer.from(pos.buffer), 34962), componentType: 5126, count: pos.length / 3, type: 'VEC3', min: lo, max: hi });
    accessors.push({ bufferView: view(Buffer.from(idx.buffer), 34963), componentType: 5125, count: idx.length, type: 'SCALAR' });
    meshes.push({ primitives: [{ attributes: { POSITION: accessors.length - 2 }, indices: accessors.length - 1 }] });
    nodes.push({ mesh: p, name: `part${p}` });
  }
  const bin = Buffer.concat(chunks);
  const json = Buffer.from(JSON.stringify({ asset: { version: '2.0' }, scene: 0, scenes: [{ nodes: nodes.map((_, i) => i) }], nodes, meshes, accessors, bufferViews: views, buffers: [{ byteLength: bin.length }] }));
  const pad = (b: Buffer, fill: number) => Buffer.concat([b, Buffer.alloc((4 - (b.length % 4)) % 4, fill)]);
  const j = pad(json, 0x20), b = pad(bin, 0);
  const header = Buffer.alloc(12); header.writeUInt32LE(0x46546c67, 0); header.writeUInt32LE(2, 4); header.writeUInt32LE(12 + 8 + j.length + 8 + b.length, 8);
  const jh = Buffer.alloc(8); jh.writeUInt32LE(j.length, 0); jh.writeUInt32LE(0x4e4f534a, 4);
  const bh = Buffer.alloc(8); bh.writeUInt32LE(b.length, 0); bh.writeUInt32LE(0x004e4942, 4);
  return Buffer.concat([header, jh, j, bh, b]);
}

/* Large models: the editor keeps drawing (classic renderer) while their
   BVHs build in workers, then Studio takes over; exports build at once. */
test('a 1M-triangle assembly prepares in the background, then Studio draws it', async ({ session }) => {
  test.setTimeout(180_000);
  await session.openEditor();
  const { page } = session;
  const imported = await page.evaluate(async (glb: string) => {
    const PM = (window as any).PM, api = PM.Kernel.api('studio-large');
    api.scene3d.edit({ operation: 'add_camera', camera: { p: { x: 0, y: 0.9, z: 1.6, targetX: 0, targetY: 0, targetZ: 0 } } });
    api.scene3d.edit({ operation: 'add_light', light: { id: 'key', type: 'area', p: { x: -1, y: 2, z: 1.5, intensity: 6, width: 1.5, height: 1 } } });
    const bytes = Uint8Array.from(atob(glb), c => c.charCodeAt(0));
    const packed = await api.scene3d.prepareImport([new File([bytes], 'assembly.glb', { type: 'model/gltf-binary' })]);
    const r = await PM.cmd('3d.import-obj', packed);
    // From here, record every main-thread task over 50 ms until Studio is ready.
    const w = window as any;
    w.__longTasks = [];
    w.__longTaskObserver = new PerformanceObserver(list => { for (const e of list.getEntries()) w.__longTasks.push(e.duration); });
    w.__longTaskObserver.observe({ type: 'longtask' });
    return r.ok ? true : JSON.stringify(r).slice(0, 400);
  }, assembly(20, 160).toString('base64'));
  expect(imported).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).PM.Studio?.status().ready), { timeout: 30_000 }).toBe(true);
  const first = await page.evaluate(() => {
    const PM = (window as any).PM;
    const t = performance.now();
    PM.GL.render(PM.time, { mblur: false });
    return { ms: performance.now() - t, status: PM.Studio.status() };
  });
  // The frame did not wait for the BVHs: they are still building.
  expect(first.status.preparing?.total).toBeGreaterThan(1_000_000);
  expect(first.status.preparing?.pending).toBeGreaterThan(0);
  const started = Date.now();
  await expect.poll(async () => page.evaluate(() => {
    const PM = (window as any).PM;
    PM.GL.render(PM.time, { mblur: false });
    return PM.Studio.status().preparing;
  }), { timeout: 120_000, intervals: [250] }).toBe(null);
  const readyMs = Date.now() - started;
  const longest = await page.evaluate(() => { const w = window as any; w.__longTaskObserver.disconnect(); return Math.max(0, ...w.__longTasks); });
  const result = await page.evaluate(() => {
    const PM = (window as any).PM;
    const before = PM.Studio.status().frames;
    PM.agentFrameCapture = true;
    const t = performance.now();
    const px = PM.GL.renderToPixels(0, 320, 180, { transparent: true, mblur: false });
    const finalMs = performance.now() - t;
    PM.agentFrameCapture = false;
    let covered = 0;
    for (let i = 3; i < px.length; i += 4) if (px[i]! > 8) covered++;
    return { covered, studioDrew: PM.Studio.status().frames > before, finalMs, declined: PM.Studio.status().declined };
  });
  console.log(`large scene: ${first.status.preparing.total} triangles; longest main-thread task while preparing ${longest.toFixed(0)} ms; Studio ready ${readyMs} ms after the first frame; Final frame ${result.finalMs.toFixed(0)} ms`);
  // The editor stays responsive: no frame waits for the BVH builds.
  expect(longest).toBeLessThan(400);
  expect(result.studioDrew).toBe(true);
  expect(result.declined).toBe('');
  expect(result.covered).toBeGreaterThan(5000);
  expect(session.diagnostics.pageErrors).toEqual([]);
});
