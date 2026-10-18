/**
 * glTF parity cases for the native 3D pass (docs/engine/gltf.md): imported
 * models in the production `Scene3D` shape, rendered by the page exactly as
 * the compositor renders 3D layers (tools/three-ref/scene-page.ts), with
 * models loaded through the production importer (`loadGltf`) and the
 * runtime's mixer driven by `sync`'s time.
 *
 * Synthetic models (tools/three-ref/gltf/models.ts, from spike S7) cover
 * skinning, morphs, three blended clips, every r180 material extension,
 * vertex colors, alpha mask and blend, flat shading, quantization, bump,
 * TEXCOORD_1 occlusion, points, lines, instancing, embedded punctual
 * lights, transmission with volume, IOR, and dispersion, and WebP and
 * AVIF images. Khronos sample models (downloaded into the corpus
 * directory, never committed) cover the rest of r180's list. Each case
 * also exercises the material rule: source materials, the object's
 * material over the file's, slots, editor looks, depth sets with other
 * objects, and a legacy scene rendered whole.
 *
 * Pure data; tools/three-ref/run-gltf.ts writes it to
 * tests/fixtures/scene3d-gltf-cases.json and renders it.
 */
import * as THREE from '../../../../../apps/desktop/node_modules/three/build/three.module.js';

const ch = (v: any) => ({ v, kf: [], expr: null });
const p = (o: Record<string, any>) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, ch(v)]));
type Mat = Partial<{ color: string; roughness: number; metalness: number; emissive: string; emissiveIntensity: number; opacity: number }>;
const material = (m: Mat = {}, extra: Record<string, any> = {}) => ({
  p: p({ color: '#C7C4FF', roughness: 0.4, metalness: 0.1, emissive: '#000000', emissiveIntensity: 1, opacity: 1, ...m }),
  maps: {}, doubleSided: false, ...extra,
});
const transform = (o: Partial<Record<'x' | 'y' | 'z' | 'rx' | 'ry' | 'rz' | 'sx' | 'sy' | 'sz', number>> = {}, visible = true) =>
  p({ x: 0, y: 0, z: 0, rx: 0, ry: 0, rz: 0, sx: 1, sy: 1, sz: 1, ...o, visible });
const object = (id: string, source: any, m: any = material(), extra: Record<string, any> = {}) => ({
  id, name: id, parent: null, source, p: transform(), material: m, useSourceMaterials: false, slots: [],
  castShadow: true, receiveShadow: true, ...extra,
});
const model = (id: string, assetId: string, extra: Record<string, any> = {}) =>
  object(id, { assetId }, material(), { useSourceMaterials: true, ...extra });
type LightP = Partial<{ x: number; y: number; z: number; targetX: number; targetY: number; targetZ: number; color: string; intensity: number;
  distance: number; decay: number; angle: number; penumbra: number; width: number; height: number; visible: boolean }>;
const light = (id: string, type: 'sun' | 'point' | 'spot' | 'area', l: LightP = {}, castShadow = true) => ({
  id, name: id, type, castShadow,
  p: p({ x: -3, y: 5, z: 4, targetX: 0, targetY: 0, targetZ: 0, color: '#FFFFFF', intensity: 3, distance: 0, decay: 2, angle: 45,
    penumbra: 0.4, width: 1, height: 1, visible: true, ...l }),
});
type CamP = Partial<{ x: number; y: number; z: number; targetX: number; targetY: number; targetZ: number; fov: number; zoom: number; near: number; far: number }>;
const camera = (c: CamP = {}, projection: 'perspective' | 'orthographic' = 'perspective') => ({
  projection, p: p({ x: 0, y: 0.4, z: 3.2, targetX: 0, targetY: 0, targetZ: 0, fov: 45, zoom: 1, near: 0.01, far: 1000, ...c }),
});
const environment = (e: Partial<{ ambient: number; ambientColor: string; exposure: number }> = {}, background: string | null = '#1A1B20', shadows = true) => ({
  p: p({ ambient: 0.6, ambientColor: '#FFFFFF', exposure: 1, ...e }), background, shadows,
});
const matrix = (pos: number[], rot: number[] = [0, 0, 0], scale: number[] = [1, 1, 1]) => {
  const m = new THREE.Matrix4().compose(new THREE.Vector3(...pos),
    new THREE.Quaternion().setFromEuler(new THREE.Euler(...rot.map(d => d * Math.PI / 180), 'XYZ')), new THREE.Vector3(...scale));
  return Array.from(m.elements);
};

export type Output = { object?: string; depthIds?: string[]; background?: boolean; look?: { wireframe?: boolean; xray?: boolean } };
export type Case = {
  id: string; width: number; height: number; scene: any;
  matrices?: Record<string, number[]>;
  layers?: Record<string, { opacity: number; occludes: boolean }>;
  textures?: Record<string, string>;
  /** Model assets: id → file in the capture's `models/`. */
  models: Record<string, string>;
  /** `sync`'s time and per-object mixer times (`time − layer.from`). */
  time: number;
  animationTimes?: Record<string, number>;
  outputs: Output[];
  /** Rays (NDC) the page casts at the case's time for pick parity. */
  picks?: { cols: number; rows: number };
};

const W = 480, H = 270;
const set = (ids: string[]): Output[] => ids.map((id, i) => ({ object: id, depthIds: ids, background: i === ids.length - 1 }));
const floor = () => object('floor', { primitive: 'plane', parameters: { width: 8, height: 8 } }, material({ color: '#8A8F99', roughness: 0.9, metalness: 0 }));
const floorMatrix = (y = -0.6) => matrix([0, y, 0], [-90, 0, 0]);
const base = (lights: any[], env = environment(), cam = camera()) => ({ version: 1, lights, camera: cam, environment: env });

/** Khronos sample models in the corpus (file name → camera distance, extra notes). */
export const khronos: Record<string, { z?: number; times?: number[]; y?: number }> = {
  'AlphaBlendModeTest.glb': {}, 'AnimatedColorsCube.glb': {}, 'AnimatedMorphCube.glb': { times: [0, 0.4, 1.3] },
  'AnimatedMorphCube-quantized.gltf': { times: [0.6] }, 'AnisotropyBarnLamp.glb': {}, 'AnisotropyDiscTest.glb': {},
  'AnisotropyRotationTest.glb': {}, 'AnisotropyStrengthTest.glb': {}, 'AttenuationTest.glb': {}, 'Avocado.glb': {},
  'BoomBox.glb': {}, 'BoxAnimated.glb': { times: [0, 1.1, 2.9] }, 'BoxInterleaved.glb': {}, 'BoxTextured.glb': {},
  'BoxTexturedNonPowerOfTwo.glb': {}, 'BoxVertexColors.glb': {}, 'BrainStem.glb': { times: [0, 0.8, 2.2] }, 'CarbonFibre.glb': {},
  'CesiumMan.glb': { times: [0, 0.5, 1.7] }, 'CesiumMilkTruck.glb': { times: [0, 0.9] }, 'ClearCoatCarPaint.glb': {},
  'ClearCoatTest.glb': {}, 'ClearcoatWicker.glb': {}, 'CompareAlphaCoverage.glb': {}, 'CompareAmbientOcclusion.glb': {},
  'CompareAnisotropy.glb': {}, 'CompareBaseColor.glb': {}, 'CompareClearcoat.glb': {}, 'CompareDispersion.glb': {},
  'CompareEmissiveStrength.glb': {}, 'CompareIor.glb': {}, 'CompareIridescence.glb': {}, 'CompareMetallic.glb': {},
  'CompareNormal.glb': {}, 'CompareRoughness.glb': {}, 'CompareSheen.glb': {}, 'CompareSpecular.glb': {},
  'CompareTransmission.glb': {}, 'CompareVolume.glb': {}, 'Corset.glb': {}, 'CubeVisibility.glb': { times: [0, 1.5] },
  'DamagedHelmet.glb': {}, 'DirectionalLight.glb': {}, 'DispersionTest.glb': {}, 'Duck.glb': {},
  'EmissiveStrengthTest.glb': {}, 'Fox.glb': { times: [0, 0.7, 2.4] }, 'GlassBrokenWindow.glb': {},
  'GlassHurricaneCandleHolder.glb': {}, 'GlassVaseFlowers.glb': {}, 'IORTestGrid.glb': {}, 'InterpolationTest.glb': { times: [0, 0.6, 1.4, 3.1] },
  'IridescenceAbalone.glb': {}, 'IridescenceLamp.glb': {}, 'IridescenceSuzanne.glb': {}, 'IridescentDishWithOlives.glb': {},
  'Lantern.glb': {}, 'LightVisibility.glb': { times: [0, 1.2] }, 'LightsPunctualLamp.glb': {}, 'MetalRoughSpheres.glb': {},
  'MetalRoughSpheresNoTextures.glb': {}, 'MorphPrimitivesTest.glb': {}, 'MorphStressTest.glb': { times: [0, 1.0, 3.5] },
  'MosquitoInAmber.glb': {}, 'MultiUVTest.glb': {}, 'NegativeScaleTest.glb': {}, 'NodeVisibilityTest.glb': {},
  'NormalTangentMirrorTest.glb': {}, 'NormalTangentTest.glb': {}, 'OrientationTest.glb': {}, 'PlaysetLightTest.glb': {},
  'PointLightIntensityTest.glb': {}, 'PotOfCoals.glb': {}, 'RecursiveSkeletons.glb': { times: [0, 0.8] },
  'RiggedFigure.glb': { times: [0, 0.4, 1.0] }, 'RiggedSimple.glb': { times: [0, 0.6, 2.0] }, 'SheenChair.glb': {},
  'SheenTestGrid.glb': {}, 'SimpleInstancing.glb': {}, 'SpecularSilkPouf.glb': {}, 'SpecularTest.glb': {},
  'SunglassesKhronos.glb': {}, 'TextureCoordinateTest.glb': {}, 'TextureEncodingTest.glb': {},
  'TextureLinearInterpolationTest.glb': {}, 'TextureSettingsTest.glb': {}, 'TextureTransformMultiTest.glb': {}, 'ToyCar.glb': {},
  'TrafficCone.glb': {}, 'TransmissionOrderTest.glb': {}, 'TransmissionRoughnessTest.glb': {}, 'TransmissionTest.glb': {},
  'TransmissionThinwallTestGrid.glb': {}, 'UnlitTest.glb': {}, 'VertexColorTest.glb': {}, 'WaterBottle.glb': {},
  'XmpMetadataRoundedCube.glb': {}, 'MeshPrimitiveModes.gltf': {}, 'SimpleSkin.gltf': { times: [0, 1.3, 3.4] },
  'SimpleMorph.gltf': { times: [0, 0.8, 2.5] }, 'SimpleSparseAccessor.gltf': {}, 'Triangle.gltf': {}, 'TriangleWithoutIndices.gltf': {},
  'AnimatedTriangle.gltf': { times: [0, 0.7, 2.1] }, 'SimpleMeshes.gltf': {}, 'Cameras.gltf': {}, 'MultipleScenes.gltf': {},
  'SimpleTexture.gltf': {},
};

/** The cases. `available` lists the model files present in the capture. */
export function cases(available: Set<string>): Case[] {
  const out: Case[] = [];
  const add = (c: Case) => { if (Object.values(c.models).every(f => available.has(f))) out.push(c); };

  // Synthetic models (spike S7's corpus).
  for (const t of [0, 0.35, 0.7, 1.6, 2.9])
    add({ id: `skinned-${t}`, width: W, height: H, models: { rig: 'skinned.glb' }, time: t,
      scene: { ...base([light('sun', 'sun')]), objects: [model('rig', 'rig'), floor()] },
      matrices: { rig: matrix([0, 0.2, 0], [0, 0, 0], [1.6, 1.6, 1.6]), floor: floorMatrix() }, outputs: set(['rig', 'floor']),
      picks: t === 0.7 ? { cols: 32, rows: 18 } : undefined });
  // Per-object mixer times (`time − layer.from`).
  add({ id: 'skinned-layer-times', width: W, height: H, models: { rig: 'skinned.glb' }, time: 2.2, animationTimes: { rig: 0.45 },
    scene: { ...base([light('sun', 'sun')]), objects: [model('rig', 'rig'), floor()] },
    matrices: { rig: matrix([0.2, 0.2, 0], [0, 25, 0], [1.4, 1.4, 1.4]), floor: floorMatrix() }, outputs: set(['rig', 'floor']) });
  add({ id: 'pbr-ext', width: 640, height: 360, models: { grid: 'pbr-ext.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun', { intensity: 2 })], environment({ ambient: 0.4 }, '#26262B'), camera({ x: 0, y: 0.3, z: 5.2 })),
      objects: [model('grid', 'grid')] },
    matrices: { grid: matrix([0, 0, 0], [0, 0, 0], [2.6, 2.6, 2.6]) }, outputs: set(['grid']), picks: { cols: 40, rows: 24 } });
  add({ id: 'pbr-ext-shadowed', width: 640, height: 360, models: { grid: 'pbr-ext.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun', { x: 1, y: 3, z: 5, intensity: 2 })], environment({ ambient: 0.3 }, null), camera({ x: 0.4, y: 0.6, z: 5 })),
      objects: [model('grid', 'grid'), floor()] },
    matrices: { grid: matrix([0, 0.3, -0.2], [0, -12, 0], [2.2, 2.2, 2.2]), floor: floorMatrix(-1.6) }, outputs: set(['grid', 'floor']) });
  add({ id: 'transmission', width: W, height: H, models: { glass: 'transmission.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun')], environment({ ambient: 0.6 }, null), camera({ x: 0.5, y: 1.2, z: 4.5 })), objects: [floor(), model('glass', 'glass')] },
    matrices: { floor: floorMatrix(-0.5), glass: matrix([0, 0.4, 0], [0, 0, 0], [2, 2, 2]) }, outputs: set(['glass', 'floor']) });
  add({ id: 'transmission-background', width: W, height: H, models: { glass: 'transmission.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun')], environment({ ambient: 0.6 }, '#304050'), camera({ x: -0.4, y: 0.6, z: 3.6 })), objects: [model('glass', 'glass')] },
    matrices: { glass: matrix([0, 0, 0], [0, 15, 0], [2, 2, 2]) }, outputs: set(['glass']) });
  add({ id: 'webp-avif', width: W, height: H, models: { cards: 'webp-avif.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun')], environment({ ambient: 0.8 }), camera({ x: 0, y: 0, z: 4 })), objects: [model('cards', 'cards')] },
    matrices: { cards: matrix([0, 0, 0], [0, 0, 0], [2.4, 2.4, 2.4]) }, outputs: set(['cards']) });

  // The material rule: the object's material over the file's (reused
  // standard and physical materials keep their extensions; unlit ones are
  // replaced), slots (an array: nothing draws), editor looks.
  add({ id: 'pbr-ext-object-material', width: 640, height: 360, models: { grid: 'pbr-ext.glb' }, time: 0,
    scene: { ...base([light('sun', 'sun', { intensity: 2 })], environment({ ambient: 0.4 }), camera({ x: 0, y: 0.3, z: 5.2 })),
      objects: [model('grid', 'grid', { useSourceMaterials: false, material: material({ color: '#E0B070', roughness: 0.35, metalness: 0.2, opacity: 1 }, { doubleSided: true }) })] },
    matrices: { grid: matrix([0, 0, 0], [0, 0, 0], [2.6, 2.6, 2.6]) }, outputs: set(['grid']) });
  add({ id: 'skinned-object-material-transparent', width: W, height: H, models: { rig: 'skinned.glb' }, time: 1.1,
    scene: { ...base([light('sun', 'sun')]), objects: [model('rig', 'rig', { useSourceMaterials: false, material: material({ color: '#70C0FF', opacity: 0.6 }) }), floor()] },
    matrices: { rig: matrix([0, 0.2, 0], [0, 0, 0], [1.6, 1.6, 1.6]), floor: floorMatrix() }, outputs: set(['rig', 'floor']) });
  add({ id: 'slots-draw-nothing', width: W, height: H, models: { rig: 'skinned.glb' }, time: 0.3,
    scene: { ...base([light('sun', 'sun')]), objects: [model('rig', 'rig', { useSourceMaterials: false,
      slots: [{ id: 'a', name: 'A', material: material({ color: '#FF0000' }) }, { id: 'b', name: 'B', material: material({ color: '#00FF00' }) }] }), floor()] },
    matrices: { rig: matrix([0, 0.2, 0], [0, 0, 0], [1.6, 1.6, 1.6]), floor: floorMatrix() }, outputs: set(['rig', 'floor']) });
  // Each look in its own runtime: three.js does not rebuild a program
  // when a look changes `transparent` or `wireframe` (no `needsUpdate`),
  // so a look after another render depends on history.
  for (const look of ['wireframe', 'xray'] as const)
    add({ id: `pbr-ext-${look}`, width: 640, height: 360, models: { grid: 'pbr-ext.glb' }, time: 0,
      scene: { ...base([light('sun', 'sun', { intensity: 2 })], environment({ ambient: 0.4 }), camera({ x: 0, y: 0.3, z: 5.2 })), objects: [model('grid', 'grid')] },
      matrices: { grid: matrix([0, 0, 0], [0, 0, 0], [2.6, 2.6, 2.6]) },
      outputs: [{ object: 'grid', depthIds: ['grid'], background: true, look: { [look]: true } }] });

  // Depth sets with scene objects; two model layers; layer opacity.
  add({ id: 'models-in-a-set', width: W, height: H, models: { rig: 'skinned.glb', glass: 'transmission.glb' }, time: 0.9,
    scene: { ...base([light('sun', 'sun'), light('bulb', 'point', { x: 1, y: 1.5, z: 1.5, intensity: 4 })]),
      objects: [model('rig', 'rig'), model('glass', 'glass'), object('ball', { primitive: 'sphere', parameters: { radius: 0.4 } }, material({ color: '#E8C547' })), floor()] },
    matrices: { rig: matrix([-0.9, 0.2, 0], [0, 20, 0], [1.4, 1.4, 1.4]), glass: matrix([0.8, 0.1, -0.3], [0, -20, 0], [1.6, 1.6, 1.6]),
      ball: matrix([0, -0.2, 0.6]), floor: floorMatrix() },
    layers: { glass: { opacity: 0.8, occludes: true } },
    outputs: set(['ball', 'rig', 'glass', 'floor']) });

  // A legacy scene layer with a model (its own hierarchy, rendered whole).
  add({ id: 'legacy-scene-model', width: W, height: H, models: { rig: 'skinned.glb', grid: 'pbr-ext.glb' }, time: 0.5,
    scene: { ...base([light('sun', 'sun')], environment({ ambient: 0.5 }, '#202024'), camera({ x: 0.2, y: 0.8, z: 4 })),
      objects: [
        { ...model('rig', 'rig'), p: transform({ x: -0.8, y: 0.2, sx: 1.4, sy: 1.4, sz: 1.4 }) },
        { ...model('grid', 'grid'), p: transform({ x: 0.9, y: 0.3, ry: -15, sx: 1.5, sy: 1.5, sz: 1.5 }) },
        { ...floor(), p: transform({ y: -0.6, rx: -90 }) }] },
    outputs: [{}] });

  // Khronos sample models with their own materials.
  for (const [file, o] of Object.entries(khronos)) {
    const id = file.replace(/\.(glb|gltf)$/, '');
    for (const t of o.times || [0])
      add({ id: `khr-${id}${o.times ? `-${t}` : ''}`, width: W, height: H, models: { m: file }, time: t,
        scene: { ...base([light('sun', 'sun', { x: -2, y: 4, z: 5, intensity: 2.5 })], environment({ ambient: 0.7 }, '#202226'), camera({ x: 0, y: 0.25, z: o.z ?? 3.2 })),
          objects: [model('m', 'm')] },
        matrices: { m: matrix([0, o.y ?? 0, 0], [0, 0, 0], [1.6, 1.6, 1.6]) }, outputs: set(['m']),
        picks: (o.times && t === o.times[1]) || id === 'MeshPrimitiveModes' || id === 'SimpleInstancing' ? { cols: 24, rows: 14 } : undefined });
  }
  return out;
}
