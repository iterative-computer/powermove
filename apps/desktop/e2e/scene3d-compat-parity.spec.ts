/* The flag-off asset path is the test-only GLTFLoader reference. The flag-on
 * asset path must construct the deprecated Object3D from engine import data. */
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import path from 'node:path';
import { expect, launchApp, test, type LaunchedApp } from './helpers/app';

const corpus = path.join(homedir(), 'Library/Application Support/Powermove Engine Corpus/khronos-gltf');
const models = [
  'BoxTextured.glb', 'TextureTransformMultiTest.glb', 'CompareClearcoat.glb',
  'CompareSheen.glb', 'CesiumMan.glb', 'Fox.glb',
].filter(name => existsSync(path.join(corpus, name)));

async function snapshot(session: LaunchedApp, name: string) {
  const encoded = readFileSync(path.join(corpus, name)).toString('base64');
  return session.page.evaluate(async ({ encoded, name }) => {
    const PM = (window as any).PM;
    const binary = atob(encoded);
    const bytes = Uint8Array.from(binary, char => char.charCodeAt(0));
    const imported = await PM.assets.add(new File([bytes], name, { type: 'model/gltf-binary' }), { silent: true });
    const asset = PM.assets.get(imported.id);
    if (!asset?.object3d) throw new Error(`${name}: missing deprecated Object3D`);
    const numeric = (value: any): any => typeof value === 'number' ? Math.round(value * 1e5) / 1e5 : value;
    const color = (value: any) => value?.isColor ? value.toArray().map(numeric) : null;
    const vector = (value: any) => value?.toArray ? value.toArray().map(numeric) : null;
    const mapProps = ['map', 'normalMap', 'aoMap', 'emissiveMap', 'metalnessMap', 'roughnessMap',
      'clearcoatMap', 'clearcoatRoughnessMap', 'clearcoatNormalMap', 'sheenColorMap', 'sheenRoughnessMap',
      'transmissionMap', 'specularIntensityMap', 'specularColorMap', 'iridescenceMap',
      'iridescenceThicknessMap', 'thicknessMap', 'anisotropyMap', 'bumpMap'];
    const factorProps = ['opacity', 'alphaTest', 'metalness', 'roughness', 'emissiveIntensity',
      'aoMapIntensity', 'clearcoat', 'clearcoatRoughness', 'sheen', 'sheenRoughness', 'transmission',
      'ior', 'specularIntensity', 'iridescence', 'iridescenceIOR', 'thickness', 'attenuationDistance',
      'anisotropy', 'anisotropyRotation', 'dispersion', 'bumpScale'];
    const maps = new Map<any, any>();
    const describeMap = (texture: any) => {
      if (!texture) return null;
      if (maps.has(texture)) return maps.get(texture);
      const image = texture.image;
      let pixels: number | null = null;
      if (image?.width && image?.height) {
        const canvas = document.createElement('canvas'); canvas.width = image.width; canvas.height = image.height;
        const context = canvas.getContext('2d')!; context.drawImage(image, 0, 0);
        let hash = 2166136261;
        for (const byte of context.getImageData(0, 0, canvas.width, canvas.height).data)
          hash = Math.imul(hash ^ byte, 16777619);
        pixels = hash >>> 0;
      }
      const result = { name: texture.name, userData: texture.userData,
        width: image?.width ?? null, height: image?.height ?? null, pixels,
        channel: texture.channel, colorSpace: texture.colorSpace, flipY: texture.flipY,
        minFilter: texture.minFilter, magFilter: texture.magFilter, wrapS: texture.wrapS, wrapT: texture.wrapT,
        offset: vector(texture.offset), repeat: vector(texture.repeat), rotation: numeric(texture.rotation) };
      maps.set(texture, result); return result;
    };
    const describeMaterial = (material: any) => material && ({
      type: material.type, name: material.name, userData: material.userData,
      side: material.side, transparent: material.transparent,
      vertexColors: material.vertexColors, flatShading: material.flatShading,
      color: color(material.color), emissive: color(material.emissive), sheenColor: color(material.sheenColor),
      specularColor: color(material.specularColor), attenuationColor: color(material.attenuationColor),
      normalScale: vector(material.normalScale), clearcoatNormalScale: vector(material.clearcoatNormalScale),
      iridescenceThicknessRange: material.iridescenceThicknessRange?.map(numeric) ?? null,
      factors: Object.fromEntries(factorProps.filter(key => material[key] !== undefined).map(key => [key, numeric(material[key])])),
      maps: Object.fromEntries(mapProps.filter(key => material[key]).map(key => [key, describeMap(material[key])])),
    });
    const objects: any[] = [];
    asset.object3d.traverse((object: any) => {
      const geometry = object.geometry;
      objects.push({ name: object.name, type: object.type, userData: object.userData,
        parent: object.parent ? objects.findIndex(item => item.identity === object.parent) : -1,
        identity: object,
        matrix: object.matrix.toArray().map(numeric),
        attributes: geometry ? Object.fromEntries(Object.entries(geometry.attributes).map(([key, attribute]: [string, any]) =>
          [key, { count: attribute.count, itemSize: attribute.itemSize }])) : null,
        geometryUserData: geometry?.userData ?? null,
        indices: geometry?.index?.count ?? null,
        materials: object.material ? (Array.isArray(object.material) ? object.material : [object.material]).map(describeMaterial) : null,
      });
    });
    const graph = objects.map(({ identity: _identity, ...rest }) => rest);
    return { graph,
      clips: (asset.animations ?? []).map((clip: any) => ({ name: clip.name, duration: numeric(clip.duration),
        tracks: clip.tracks.map((track: any) => ({ property: track.name.split('.').at(-1), size: track.times.length })) })) };
  }, { encoded, name });
}

test('engine deprecated Object3D matches GLTFLoader for textured Khronos models', async () => {
  test.setTimeout(240_000);
  test.skip(!models.length, 'Khronos glTF corpus is unavailable');
  const selected = models.filter(name => !process.env.PM_COMPAT_MODEL || process.env.PM_COMPAT_MODEL === name);
  expect(selected.length).toBeGreaterThan(0);
  const reference = new Map<string, Awaited<ReturnType<typeof snapshot>>>();
  let session: LaunchedApp | undefined;
  try {
    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '0' } });
    await session.openEditor();
    for (const name of selected) reference.set(name, await snapshot(session, name));
    expect(session.diagnostics.pageErrors).toEqual([]);
    await session.close(); session = undefined;

    session = await launchApp({ env: { POWERMOVE_NEW_ENGINE: '1' } });
    await session.openEditor();
    for (const name of selected) expect(await snapshot(session, name), name).toEqual(reference.get(name));
    expect(session.diagnostics.pageErrors).toEqual([]);
  } finally { await session?.close(); }
});
