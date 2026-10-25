import type { ElectronApplication, Page } from 'playwright';

export const BUDGET_WIDTH = 256;
export const BUDGET_HEIGHT = 144;
export const BUDGET_VERTICES = 600_000;

/** One self-contained model over the inline 500k budget, below glTF's 1M cap.
 * Dense mode makes 200k distinct triangles for a frame-time stress sample;
 * sparse mode keeps the same vertex and triangle counts with degenerate tail
 * triangles, isolating import/scene budgeting from raster workload. */
export async function installBudgetScene(page: Page, placements: number, dense: boolean) {
  return page.evaluate(async ({ placements, dense, count, width, height }) => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('scene3d-budget-parity');
    const positions = new Float32Array(count * 3);
    if (dense) {
      // A 500 x 400 triangle grid spans the same [-1,1] model bounds.
      for (let cell = 0; cell < count / 3; cell++) {
        const x = cell % 500, y = Math.floor(cell / 500);
        const left = -1 + x * (2 / 500), bottom = -1 + y * (2 / 400);
        positions.set([left, bottom, 0, left + 2 / 500, bottom, 0,
          left, bottom + 2 / 400, 0], cell * 9);
      }
    } else positions.set([-1, -1, 0, 1, -1, 0, 0, 1, 0]);
    const graph = {
      asset: { version: '2.0' }, scene: 0, scenes: [{ nodes: [0] }],
      nodes: [{ mesh: 0 }],
      meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] }],
      materials: [{ pbrMetallicRoughness: { baseColorFactor: [0.95, 0.15, 0.1, 1] },
        extensions: { KHR_materials_unlit: {} }, doubleSided: true }],
      extensionsUsed: ['KHR_materials_unlit'],
      buffers: [{ byteLength: positions.byteLength }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: positions.byteLength }],
      accessors: [{ bufferView: 0, componentType: 5126, count, type: 'VEC3',
        min: [-1, -1, 0], max: [1, 1, 0] }],
    };
    const json = new TextEncoder().encode(JSON.stringify(graph));
    const jsonLength = (json.length + 3) & ~3;
    const binLength = positions.byteLength;
    const bytes = new Uint8Array(12 + 8 + jsonLength + 8 + binLength);
    const view = new DataView(bytes.buffer);
    bytes.set([0x67, 0x6c, 0x54, 0x46]); view.setUint32(4, 2, true);
    view.setUint32(8, bytes.length, true);
    view.setUint32(12, jsonLength, true); bytes.set([0x4a, 0x53, 0x4f, 0x4e], 16);
    bytes.fill(0x20, 20, 20 + jsonLength); bytes.set(json, 20);
    const binaryChunk = 20 + jsonLength;
    view.setUint32(binaryChunk, binLength, true);
    bytes.set([0x42, 0x49, 0x4e, 0], binaryChunk + 4);
    bytes.set(new Uint8Array(positions.buffer), binaryChunk + 8);
    const model = await PM.assets.add(new File([new Uint8Array(bytes)], 'budget.glb',
      { type: 'model/gltf-binary' }), { silent: true });
    const asset = PM.assets.get(model.id);
    let geometryVertices = 0, geometryTriangles = 0, meshes = 0;
    asset?.object3d?.traverse((o: any) => {
      if (!o.isMesh) return;
      meshes++;
      geometryVertices += o.geometry.attributes.position?.count ?? 0;
      geometryTriangles += Math.floor((o.geometry.index?.count ?? o.geometry.attributes.position?.count ?? 0) / 3);
    });
    PM.proj.w = width; PM.proj.h = height; PM.setTime(0);
    api.scene3d.edit({ operation: 'create', scene: {
      objects: Array.from({ length: placements }, (_, i) => ({
        id: `budget-${i}`, source: { assetId: model.id },
        p: { x: (i - (placements - 1) / 2) * 1.8 }, useSourceMaterials: true,
      })),
      camera: { p: { x: 0, y: 0, z: 6 } },
      environment: { p: { ambient: 0 }, shadows: false },
    } });
    PM.proj.w = width; PM.proj.h = height; PM.touch(); PM.GL.resize(width, height);
    return { assetId: model.id, modelBytes: bytes.length, assetVertices: asset?.vertices ?? null,
      geometryVertices, geometryTriangles, meshes, placements, totalPlacedVertices: geometryVertices * placements };
  }, { placements, dense, count: BUDGET_VERTICES, width: BUDGET_WIDTH, height: BUDGET_HEIGHT });
}

export async function addFourthBudgetPlacement(page: Page, assetId: string) {
  return page.evaluate((assetId) => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('scene3d-budget-parity');
    try {
      api.scene3d.edit({ operation: 'add_object', object: {
        id: 'budget-3', source: { assetId }, p: { x: 2.7 }, useSourceMaterials: true,
      } });
      PM.touch();
      return { ok: true as const, error: null };
    } catch (error) {
      return { ok: false as const, error: String(error) };
    }
  }, assetId);
}

/** Use each lane's public capture path. Both return premultiplied RGBA;
 * normalize the native top-down rows to WebGL's bottom-up readPixels order. */
export async function measureBudgetFrame(page: Page, engine: boolean, repeats = 12) {
  return page.evaluate(({ engine, repeats, width, height }) => {
    const PM = (window as any).PM;
    // The engine's own split of each capture: queue wait and render+readback
    // in the engine (µs), and main's request-to-answer time (ms).
    const split: { waitedUs: number; renderUs: number; mainMs: number }[] = [];
    let measuring = false;
    const draw = (): Uint8Array => {
      if (!engine) return new Uint8Array(PM.GL.renderToPixels(0, width, height,
        { transparent: true, mblur: false }));
      const shot = PM.EngineRender.snapshotSync({ time: 0, width, height,
        pixels: 'premultiplied', motionBlurSamples: 0, background: false });
      if (measuring) split.push({ waitedUs: shot.waitedUs, renderUs: shot.renderUs, mainMs: shot.mainMs });
      return new Uint8Array(shot.data);
    };
    const times: number[] = [];
    let pixels: Uint8Array;
    try {
      for (let i = 0; i < 3; i++) draw();
      measuring = true;
      pixels = new Uint8Array(width * height * 4);
      for (let i = 0; i < repeats; i++) {
        const start = performance.now();
        pixels = draw();
        times.push(performance.now() - start);
      }
    } catch (error) {
      return { ok: false as const, error: String(error), timesMs: times,
        pixels: null, png: null, occupied: 0, occupiedColumns: [] as number[],
        webglContextLost: !!PM.GL.gl?.isContextLost?.() };
    }
    if (engine) {
      const topDown = pixels.slice();
      for (let y = 0; y < height; y++) pixels.set(
        topDown.subarray(y * width * 4, (y + 1) * width * 4),
        (height - 1 - y) * width * 4);
    }
    const columns = new Set<number>(); let occupied = 0;
    for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      if (pixels[i + 3]! > 64 && pixels[i]! > pixels[i + 1]!) {
        occupied++; columns.add(x);
      }
    }
    const topDown = new Uint8ClampedArray(pixels.length);
    for (let y = 0; y < height; y++) topDown.set(
      pixels.subarray(y * width * 4, (y + 1) * width * 4),
      (height - 1 - y) * width * 4);
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
    canvas.getContext('2d')!.putImageData(new ImageData(topDown, width, height), 0, 0);
    return { ok: true as const, error: null, timesMs: times, split,
      pixels: [...pixels], png: canvas.toDataURL('image/png'), occupied,
      occupiedColumns: [...columns].sort((a, b) => a - b),
      webglContextLost: !!PM.GL.gl?.isContextLost?.() };
  }, { engine, repeats, width: BUDGET_WIDTH, height: BUDGET_HEIGHT });
}

export async function budgetMemorySample(app: ElectronApplication) {
  return app.evaluate(({ app }) => app.getAppMetrics().map((m: any) => ({
    pid: m.pid, type: m.type, service: m.serviceName ?? m.name ?? null,
    workingSetKiB: m.memory?.workingSetSize ?? null,
    peakWorkingSetKiB: m.memory?.peakWorkingSetSize ?? null,
    privateKiB: m.memory?.privateBytes ?? null,
  })));
}

export function percentile(values: number[], p: number): number | null {
  if (!values.length) return null;
  const ordered = [...values].sort((a, b) => a - b);
  return ordered[Math.ceil(p * ordered.length) - 1]!;
}
