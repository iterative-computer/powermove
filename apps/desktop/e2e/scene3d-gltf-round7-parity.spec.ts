import { mkdir, writeFile } from 'node:fs/promises';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const WIDTH = 256;
const HEIGHT = 144;
const OUT = '/tmp/ne-w5/gltf-r7-chrome-proof';

async function capture(session: LaunchedApp, engine: boolean) {
  await session.openEditor();
  return session.page.evaluate(async ({ engine, width, height }) => {
    const PM = (window as any).PM;
    const api = PM.Kernel.api('gltf-r7-pixel-parity');
    const position = new Float32Array([-1, -1, 0, 1, -1, 0, 0, 1, 0]);
    const normal = new Float32Array([0, 0, 1, 0, 0, 1]);
    const bytes = new Uint8Array(position.byteLength + normal.byteLength);
    bytes.set(new Uint8Array(position.buffer));
    bytes.set(new Uint8Array(normal.buffer), position.byteLength);
    let binary = '';
    for (const byte of bytes) binary += String.fromCharCode(byte);
    const base: any = {
      asset: { version: '2.0' }, scene: 0,
      buffers: [{ byteLength: bytes.length, uri: `data:application/octet-stream;base64,${btoa(binary)}` }],
      bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: position.byteLength },
        { buffer: 0, byteOffset: position.byteLength, byteLength: normal.byteLength }],
      accessors: [
        { bufferView: 0, componentType: 5126, count: 3, type: 'VEC3', min: [-1, -1, 0], max: [1, 1, 0] },
        { bufferView: 1, componentType: 5126, count: 2, type: 'VEC3' },
      ],
      materials: [
        { pbrMetallicRoughness: { baseColorFactor: [1, .1, .1, 1] },
          extensions: { KHR_materials_unlit: {} }, doubleSided: true },
        { pbrMetallicRoughness: { baseColorFactor: [.1, 1, .1, 1] },
          extensions: { KHR_materials_unlit: {} }, doubleSided: true },
      ],
      extensionsUsed: ['KHR_materials_unlit'],
    };
    const shared = {
      ...base,
      meshes: [{ primitives: [{ attributes: { POSITION: 0 }, material: 0 }] },
        { primitives: [{ attributes: { POSITION: 0 }, material: 1 }] }],
      nodes: [
        { name: 'first', translation: [-.75, 0, 0], children: [2] },
        { name: 'second', translation: [.75, 0, 0], children: [2] },
        { name: 'shared', mesh: 0 },
        { name: 'anchor', translation: [0, -.7, 0], mesh: 1 },
      ],
      scenes: [{ nodes: [0, 1, 3] }],
    };
    const shortNormal = {
      ...base,
      meshes: [{ primitives: [{ attributes: { POSITION: 0, NORMAL: 1 }, material: 0 }] }],
      nodes: [{ mesh: 0 }], scenes: [{ nodes: [0] }],
    };
    const add = async (document: any, name: string) => {
      const asset = await PM.assets.add(new File([JSON.stringify(document)], name,
        { type: 'model/gltf+json' }), { silent: true });
      return asset.id;
    };
    const sharedId = await add(shared, 'round7-shared.gltf');
    const shortId = await add(shortNormal, 'round7-short-normal.gltf');
    PM.proj.w = width; PM.proj.h = height; PM.setTime(0);
    api.scene3d.edit({ operation: 'create', scene: {
      objects: [
        { id: 'shared', source: { assetId: sharedId }, p: { x: -1.2 }, useSourceMaterials: true },
        { id: 'unlit', source: { assetId: shortId }, p: { x: 1.0 }, useSourceMaterials: true },
        { id: 'override', source: { assetId: shortId }, p: { x: 2.2 },
          useSourceMaterials: false, material: { p: { color: '#0000FF', metalness: 0 } } },
      ],
      camera: { p: { x: 0, y: 0, z: 6 } },
    } });
    PM.proj.w = width; PM.proj.h = height; PM.touch(); PM.GL.resize(width, height);
    const frame = engine
      ? PM.EngineRender.snapshotSync({ time: 0, width, height, pixels: 'premultiplied',
        motionBlurSamples: 0, background: false }).data
      : PM.GL.renderToPixels(0, width, height, { transparent: true, mblur: false });
    const pixels = new Uint8Array(frame);
    // EngineSnapshot documents top-down RGBA; legacy readPixels is bottom-up.
    // Normalize to GL row order before comparing either pixels or PNGs.
    if (engine) {
      const topDown = pixels.slice();
      for (let y = 0; y < height; y++)
        pixels.set(topDown.subarray(y * width * 4, (y + 1) * width * 4), (height - 1 - y) * width * 4);
    }
    // Flip the normalized GL rows only for a viewable PNG.
    const topDown = new Uint8ClampedArray(pixels.length);
    for (let y = 0; y < height; y++)
      topDown.set(pixels.subarray(y * width * 4, (y + 1) * width * 4), (height - 1 - y) * width * 4);
    const canvas = document.createElement('canvas'); canvas.width = width; canvas.height = height;
    canvas.getContext('2d')!.putImageData(new ImageData(topDown, width, height), 0, 0);
    return { pixels: [...pixels], png: canvas.toDataURL('image/png') };
  }, { engine, width: WIDTH, height: HEIGHT });
}

test('shared-node ownership and short final-material inputs match captured legacy/native pixels', async () => {
  test.setTimeout(180_000);
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    const reference = await capture(session, false);
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close(); session = undefined;
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    const native = await capture(session, true);
    expect(session.diagnostics.pageErrors).toEqual([]);
    await mkdir(OUT, { recursive: true });
    await Promise.all([
      writeFile(`${OUT}/legacy.png`, Buffer.from(reference.png.split(',')[1]!, 'base64')),
      writeFile(`${OUT}/native.png`, Buffer.from(native.png.split(',')[1]!, 'base64')),
    ]);
    let withinTwo = 0, occupied = 0;
    let maximum = 0;
    for (let i = 0; i < reference.pixels.length; i += 4) {
      const diff = Math.max(...[0, 1, 2, 3].map(k => Math.abs(reference.pixels[i + k]! - native.pixels[i + k]!)));
      if (diff <= 2) withinTwo++;
      maximum = Math.max(maximum, diff);
      if (reference.pixels[i + 3]! > 0) occupied++;
    }
    const fraction = withinTwo / (WIDTH * HEIGHT);
    await writeFile(`${OUT}/measure.json`, JSON.stringify({ fractionWithin2CV: fraction,
      maximumCV: maximum, occupiedReferencePixels: occupied }, null, 2));
    expect(occupied).toBeGreaterThan(100);
    expect(fraction).toBeGreaterThanOrEqual(.99934); // existing S7 3D bar
  } finally { await session?.close(); }
});
