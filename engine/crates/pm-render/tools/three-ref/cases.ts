/**
 * The 3D sweep for the native scene pass (docs/engine/scene3d.md): scenes in
 * the production `Scene3D` shape (channels `{ v, kf: [], expr: null }`), each
 * rendered by the page exactly as the compositor renders 3D layers. Covers
 * every light type with and without shadows, the four-shadow-light limit,
 * shadows off, invisible lights, transparency (single- and double-sided),
 * layer opacity and masks without depth writes, depth sets, perspective and
 * orthographic cameras with roll and zoom, every primitive and source kind,
 * material maps with placement, slots and groups, node-material previews,
 * source materials, mirrored objects, missing models, editor looks (X-ray,
 * Wireframe, an editor camera), and a legacy scene layer rendered whole.
 *
 * Pure data; tools/three-ref/run.ts writes it to
 * tests/fixtures/scene3d-cases.json and renders it.
 */
import * as THREE from '../../../../../apps/desktop/node_modules/three/build/three.module.js';

const ch = (v: any) => ({ v, kf: [], expr: null });
const p = (o: Record<string, any>) => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, ch(v)]));

type Mat = Partial<{ color: string; roughness: number; metalness: number; emissive: string; emissiveIntensity: number; opacity: number }>;
const material = (m: Mat = {}, extra: Record<string, any> = {}) => ({
  p: p({ color: '#C7C4FF', roughness: 0.4, metalness: 0.1, emissive: '#000000', emissiveIntensity: 1, opacity: 1, ...m }),
  maps: {}, doubleSided: true, ...extra,
});
const transform = (o: Partial<Record<'x' | 'y' | 'z' | 'rx' | 'ry' | 'rz' | 'sx' | 'sy' | 'sz', number>> = {}, visible = true) =>
  p({ x: 0, y: 0, z: 0, rx: 0, ry: 0, rz: 0, sx: 1, sy: 1, sz: 1, ...o, visible });
const object = (id: string, source: any, m: any = material(), extra: Record<string, any> = {}) => ({
  id, name: id, parent: null, source, p: transform(), material: m, useSourceMaterials: false, slots: [],
  castShadow: true, receiveShadow: true, ...extra,
});
type LightP = Partial<{ x: number; y: number; z: number; targetX: number; targetY: number; targetZ: number; color: string; intensity: number;
  distance: number; decay: number; angle: number; penumbra: number; width: number; height: number; visible: boolean }>;
const light = (id: string, type: 'sun' | 'point' | 'spot' | 'area', l: LightP = {}, castShadow = true) => ({
  id, name: id, type, castShadow,
  p: p({ x: -3, y: 5, z: 4, targetX: 0, targetY: 0, targetZ: 0, color: '#FFFFFF', intensity: 3, distance: 0, decay: 2, angle: 45,
    penumbra: 0.4, width: 1, height: 1, visible: true, ...l }),
});
type CamP = Partial<{ x: number; y: number; z: number; targetX: number; targetY: number; targetZ: number; fov: number; zoom: number; near: number; far: number }>;
const camera = (c: CamP = {}, projection: 'perspective' | 'orthographic' = 'perspective') => ({
  projection, p: p({ x: 3, y: 2, z: 6, targetX: 0, targetY: 0, targetZ: 0, fov: 45, zoom: 1, near: 0.01, far: 1000, ...c }),
});
const environment = (e: Partial<{ ambient: number; ambientColor: string; exposure: number }> = {}, background: string | null = null, shadows = true) => ({
  p: p({ ambient: 0.5, ambientColor: '#FFFFFF', exposure: 1, ...e }), background, shadows,
});

/** A world matrix (column-major) from position, Euler XYZ degrees, and scale. */
const matrix = (pos: number[], rot: number[] = [0, 0, 0], scale: number[] = [1, 1, 1]) => {
  const m = new THREE.Matrix4().compose(new THREE.Vector3(...pos),
    new THREE.Quaternion().setFromEuler(new THREE.Euler(...rot.map(d => d * Math.PI / 180), 'XYZ')), new THREE.Vector3(...scale));
  return Array.from(m.elements);
};

export type Output = { object?: string; depthIds?: string[]; background?: boolean; look?: { wireframe?: boolean; xray?: boolean };
  editorCamera?: { position: number[]; target: number[]; up: number[]; projection: 'perspective' | 'orthographic'; fov: number; zoom: number; near: number; far: number } };
export type Case = {
  id: string; width: number; height: number; scene: any;
  /** Composition placement: world matrices per object (absent: a legacy scene's own hierarchy). */
  matrices?: Record<string, number[]>;
  /** `userData.layerOpacity` and `layerOccludes` per object. */
  layers?: Record<string, { opacity: number; occludes: boolean }>;
  /** A camera layer's pose (`compositionRuntime`: position, aim, and up). */
  cameraPose?: { position: number[]; target: number[]; up: number[] };
  /** Texture assets: id → generated image name. */
  textures?: Record<string, string>;
  outputs: Output[];
};

const W = 640, H = 360;
const floor = (id = 'floor', y = -0.5) => object(id, { primitive: 'plane', parameters: { width: 8, height: 8 } }, material({ color: '#8A8F99', roughness: 0.9, metalness: 0 }));
const floorMatrix = (y = -0.5) => matrix([0, y, 0], [-90, 0, 0]);
/** Every object in one depth set; outputs top to bottom (the last draws the background). */
const set = (ids: string[]): Output[] => ids.map((id, i) => ({ object: id, depthIds: ids, background: i === ids.length - 1 }));

export function cases(): Case[] {
  const out: Case[] = [];
  const add = (c: Case) => out.push(c);
  const base = (lights: any[], env = environment({}, '#101218'), cam = camera()) =>
    ({ version: 1, lights, camera: cam, environment: env });

  // Lights and shadows.
  add({ id: 'sun-shadow', width: W, height: H,
    scene: { ...base([light('sun', 'sun')]), objects: [object('box', { primitive: 'box' }, material({ color: '#FF6B1A' })), floor()] },
    matrices: { box: matrix([0, 0.1, 0], [0, 30, 0]), floor: floorMatrix() }, outputs: set(['box', 'floor']) });
  add({ id: 'sun-shadow-single-sided', width: W, height: H,
    scene: { ...base([light('sun', 'sun', { x: 2, y: 6, z: 1 })]),
      objects: [object('ring', { primitive: 'torus' }, material({ color: '#4C8DFF', metalness: 0.6, roughness: 0.25 }, { doubleSided: false })),
        object('floor', { primitive: 'plane', parameters: { width: 8, height: 8 } }, material({ color: '#D0D0D0', roughness: 1, metalness: 0 }, { doubleSided: false }))] },
    matrices: { ring: matrix([0, 0.4, 0], [60, 0, 20]), floor: floorMatrix() }, outputs: set(['ring', 'floor']) });
  add({ id: 'point-shadow', width: W, height: H,
    scene: { ...base([light('bulb', 'point', { x: 0.5, y: 2.2, z: 0.8, intensity: 12, distance: 0, decay: 2 })], environment({ ambient: 0.15 }, '#0C0D12')),
      objects: [object('ball', { primitive: 'sphere', parameters: { radius: 0.6 } }, material({ color: '#E8E2D0', roughness: 0.3 })), floor()] },
    matrices: { ball: matrix([0, 0.2, 0]), floor: floorMatrix() }, outputs: set(['ball', 'floor']) });
  add({ id: 'point-distance-decay', width: W, height: H,
    scene: { ...base([light('bulb', 'point', { x: -1, y: 1.5, z: 1.5, intensity: 6, distance: 6, decay: 1 })], environment({ ambient: 0.2 })),
      objects: [object('cyl', { primitive: 'cylinder', parameters: { radiusTop: 0.3, radiusBottom: 0.6, height: 1.4, segments: 24 } }, material({ color: '#7FD6A0' })), floor()] },
    matrices: { cyl: matrix([0, 0.2, 0], [0, 0, 10]), floor: floorMatrix() }, outputs: set(['cyl', 'floor']) });
  add({ id: 'spot-shadow', width: W, height: H,
    scene: { ...base([light('spot', 'spot', { x: 2, y: 4, z: 2, intensity: 40, angle: 30, penumbra: 0.5, decay: 2 })], environment({ ambient: 0.1 }, '#08090C')),
      objects: [object('cone', { primitive: 'cone', parameters: { radius: 0.5, height: 1.2 } }, material({ color: '#FFB36B' })), floor()] },
    matrices: { cone: matrix([0, 0.1, 0]), floor: floorMatrix() }, outputs: set(['cone', 'floor']) });
  add({ id: 'spot-no-shadow-distance', width: W, height: H,
    scene: { ...base([light('spot', 'spot', { x: -2, y: 3, z: 3, intensity: 20, angle: 20, penumbra: 0, distance: 8, decay: 1.5 }, false)], environment({ ambient: 0.2 })),
      objects: [object('box', { primitive: 'box', parameters: { width: 1.2, height: 0.8, depth: 1 } }, material({ color: '#9B8CFF' })), floor()] },
    matrices: { box: matrix([0, 0, 0], [0, 25, 0]), floor: floorMatrix() }, outputs: set(['box', 'floor']) });
  add({ id: 'five-casting-lights', width: W, height: H,
    scene: { ...base([
      light('a', 'sun', { x: -4, y: 5, z: 2, intensity: 0.8 }), light('b', 'point', { x: 1.5, y: 2, z: 1.5, intensity: 3 }),
      light('c', 'spot', { x: -1, y: 4, z: 3, intensity: 10, angle: 35 }), light('d', 'sun', { x: 3, y: 4, z: -2, intensity: 0.6, color: '#FFD0A0' }),
      light('e', 'spot', { x: 0, y: 5, z: -3, intensity: 10, angle: 25, color: '#A0C8FF' })], environment({ ambient: 0.1 })),
      objects: [object('ico', { primitive: 'icosahedron', parameters: { radius: 0.6, detail: 1 } }, material({ color: '#FFFFFF', roughness: 0.5 })), floor()] },
    matrices: { ico: matrix([0, 0.3, 0], [10, 20, 0]), floor: floorMatrix() }, outputs: set(['ico', 'floor']) });
  add({ id: 'shadows-off', width: W, height: H,
    scene: { ...base([light('sun', 'sun'), light('bulb', 'point', { x: 1, y: 2, z: 1, intensity: 4 })], environment({}, null, false)),
      objects: [object('cap', { primitive: 'capsule', parameters: { radius: 0.35, height: 0.8 } }, material({ color: '#FF8FB1' })), floor()] },
    matrices: { cap: matrix([0, 0.3, 0], [0, 0, 30]), floor: floorMatrix() }, outputs: set(['cap', 'floor']) });
  add({ id: 'invisible-casting-light', width: W, height: H,
    scene: { ...base([light('hidden', 'sun', { visible: false }), light('a', 'sun', { x: 3, y: 5, z: 2 }), light('b', 'sun', { x: -2, y: 3, z: 4, intensity: 1 }),
      light('c', 'point', { x: 0, y: 2, z: 2, intensity: 2 }), light('d', 'spot', { x: 2, y: 3, z: -1, intensity: 8 })]),
      objects: [object('box', { primitive: 'box' }), floor()] },
    matrices: { box: matrix([0, 0, 0], [0, 45, 0]), floor: floorMatrix() }, outputs: set(['box', 'floor']) });
  add({ id: 'area-light', width: W, height: H,
    scene: { ...base([light('panel', 'area', { x: 0, y: 2.5, z: 2, targetX: 0, targetY: 0, targetZ: 0, intensity: 6, width: 2, height: 1, color: '#FFE8C0' }),
      light('fill', 'sun', { intensity: 0.4 }, false)], environment({ ambient: 0.1 }, '#050608')),
      objects: [object('ball', { primitive: 'sphere', parameters: { radius: 0.7 } }, material({ color: '#D8D8E0', roughness: 0.2, metalness: 0.8 })), floor()] },
    matrices: { ball: matrix([0, 0.2, 0]), floor: floorMatrix() }, outputs: set(['ball', 'floor']) });
  add({ id: 'hemisphere-exposure', width: W, height: H,
    scene: { ...base([light('sun', 'sun', { intensity: 2 }, false)], environment({ ambient: 1.2, ambientColor: '#8899FF', exposure: 1.8 }, '#202020')),
      objects: [object('knot', { primitive: 'torus', parameters: { radius: 0.7, tube: 0.25, segments: 64 } }, material({ color: '#FFFFFF', emissive: '#331100', emissiveIntensity: 3 }))] },
    matrices: { knot: matrix([0, 0, 0], [20, 30, 0]) }, outputs: set(['knot']) });

  // Transparency, opacity, masks, depth sets.
  add({ id: 'transparency', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#14161C')),
      objects: [
        object('glassA', { primitive: 'box' }, material({ color: '#88CCFF', opacity: 0.45, roughness: 0.1 })),
        object('glassB', { primitive: 'sphere', parameters: { radius: 0.5 } }, material({ color: '#FF8888', opacity: 0.6 }, { doubleSided: false })),
        object('solid', { primitive: 'cylinder', parameters: { radius: 0.4, height: 1.5 } }, material({ color: '#FFDD55' })), floor()] },
    matrices: { glassA: matrix([-0.6, 0.1, 0.8], [0, 20, 0]), glassB: matrix([0.8, 0, 0.6]), solid: matrix([0, 0.25, -0.8]), floor: floorMatrix() },
    outputs: set(['glassA', 'glassB', 'solid', 'floor']) });
  add({ id: 'layer-opacity-and-mask', width: W, height: H,
    scene: { ...base([light('sun', 'sun')]),
      objects: [object('front', { primitive: 'box' }, material({ color: '#55AAFF' })), object('masked', { primitive: 'sphere', parameters: { radius: 0.6 } }, material({ color: '#FF5577' })),
        object('back', { primitive: 'box', parameters: { width: 2, height: 2, depth: 0.2 } }, material({ color: '#EEEEEE' }))] },
    matrices: { front: matrix([0, 0, 1.2], [0, 30, 0]), masked: matrix([0.9, 0.3, 0.2]), back: matrix([0, 0, -1]) },
    layers: { front: { opacity: 0.5, occludes: true }, masked: { opacity: 1, occludes: false } },
    outputs: set(['front', 'masked', 'back']) });
  add({ id: 'depth-sets', width: W, height: H,
    scene: { ...base([light('sun', 'sun', { x: 1, y: 6, z: 3 })], environment({}, '#101010')),
      objects: [object('top', { primitive: 'box', parameters: { width: 0.8, height: 0.8, depth: 0.8 } }, material({ color: '#FF6B1A' })),
        object('mid', { primitive: 'sphere', parameters: { radius: 0.5 } }, material({ color: '#4C8DFF' })), floor()] },
    matrices: { top: matrix([0.4, 0.3, 0.3], [0, 40, 0]), mid: matrix([-0.3, 0, -0.2]), floor: floorMatrix() },
    // A 2D barrier between `top` and the rest: two sets.
    outputs: [{ object: 'top', depthIds: ['top'], background: true }, { object: 'mid', depthIds: ['mid', 'floor'], background: false },
      { object: 'floor', depthIds: ['mid', 'floor'], background: true }] });

  // Cameras.
  add({ id: 'orthographic-zoom', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#1A1A1A'), camera({ x: 4, y: 3, z: 5, zoom: 1.6, near: 0.1, far: 50 }, 'orthographic')),
      objects: [object('box', { primitive: 'box' }, material({ color: '#C0FFC0' })), object('ball', { primitive: 'sphere', parameters: { radius: 0.4 } }), floor()] },
    matrices: { box: matrix([-0.5, 0, 0]), ball: matrix([0.8, -0.1, 0.4]), floor: floorMatrix() }, outputs: set(['box', 'ball', 'floor']) });
  add({ id: 'camera-roll-fov', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#0E1420'), camera({ fov: 70, near: 0.5, far: 30 })),
      objects: [object('cone', { primitive: 'cone' }, material({ color: '#FFAA00' })), floor()] },
    matrices: { cone: matrix([0, 0, 0], [0, 0, 0]), floor: floorMatrix() },
    cameraPose: { position: [2.5, 1.5, 3], target: [0, 0.2, 0], up: [Math.sin(0.35), Math.cos(0.35), 0] }, outputs: set(['cone', 'floor']) });
  add({ id: 'orthographic-roll', width: 480, height: 480,
    scene: { ...base([light('sun', 'sun')], environment({}, '#202830'), camera({ zoom: 0.8 }, 'orthographic')),
      objects: [object('torus', { primitive: 'torus' }, material({ color: '#88FFEE' })), floor()] },
    matrices: { torus: matrix([0, 0.2, 0], [70, 0, 0]), floor: floorMatrix() },
    cameraPose: { position: [0, 4, 5], target: [0, 0, 0], up: [-Math.sin(0.5), Math.cos(0.5), 0] }, outputs: set(['torus', 'floor']) });

  // Sources and materials.
  const prims: [string, any][] = [['box', {}], ['sphere', { radius: 0.45, segments: 32 }], ['plane', { width: 0.9, height: 0.9 }], ['cylinder', {}],
    ['cone', {}], ['torus', { radius: 0.4, tube: 0.12 }], ['capsule', { radius: 0.25, height: 0.5 }], ['icosahedron', { radius: 0.5, detail: 0 }]];
  add({ id: 'primitives', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#18181C'), camera({ x: 0, y: 3, z: 7 })),
      objects: prims.map(([k, params], i) => object(k, { primitive: k, parameters: params }, material({ color: ['#FF6B1A', '#4C8DFF', '#7FD6A0', '#FFB36B', '#9B8CFF', '#FF8FB1', '#E8E2D0', '#88FFEE'][i] }))) },
    matrices: Object.fromEntries(prims.map(([k], i) => [k, matrix([(i % 4) * 1.4 - 2.1, i < 4 ? 0.8 : -0.8, 0], [20, 30 + i * 10, 0])])),
    outputs: set(prims.map(([k]) => k)) });
  add({ id: 'lathe-extrude-mesh', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#121212')),
      objects: [
        object('vase', { lathe: [[0, -0.8], [0.35, -0.75], [0.45, -0.2], [0.2, 0.3], [0.32, 0.7], [0, 0.8]], segments: 32 }, material({ color: '#E0C090' })),
        object('star', { extrude: Array.from({ length: 10 }, (_, i) => { const a = i / 10 * Math.PI * 2, r = i % 2 ? 0.25 : 0.6; return [Number((Math.cos(a) * r).toFixed(4)) + 0, Number((Math.sin(a) * r).toFixed(4)) + 0]; }), depth: 0.3, bevel: 0.05 }, material({ color: '#FFDD55', metalness: 0.5 })),
        object('tri', { mesh: { positions: [-0.5, -0.4, 0, 0.5, -0.4, 0, 0, 0.5, 0.3, 0, -0.4, 0.6], indices: [0, 1, 2, 1, 3, 2, 3, 0, 2] } }, material({ color: '#88AAFF' })),
        floor()] },
    matrices: { vase: matrix([-1.3, 0.3, 0]), star: matrix([0.2, 0.2, 0], [0, -20, 0]), tri: matrix([1.4, 0.1, 0.3], [0, 30, 0]), floor: floorMatrix() },
    outputs: set(['vase', 'star', 'tri', 'floor']) });
  // Use the renderer's existing fallback name explicitly: parseScene rejects
  // an empty slot name, while the material and recorded pixels stay Surface.
  add({ id: 'slots-and-groups', width: W, height: H,
    scene: { ...base([light('sun', 'sun')]),
      objects: [
        object('crate', { primitive: 'box' }, material(), { slots: [{ id: 's0', name: 'Red', material: material({ color: '#FF4040' }) },
          { id: 's1', name: 'Green', material: material({ color: '#40FF40' }) }, { id: 's2', name: 'Surface', material: material({ color: '#4040FF', opacity: 0.5 }) },
          { id: 's3', name: 'Glow', material: material({ color: '#202020', emissive: '#FFAA00', emissiveIntensity: 2 }) }] }),
        object('sheet', { mesh: { positions: [-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0, -1, -1, -1, 1, -1, -1], indices: [0, 1, 2, 0, 2, 3, 0, 4, 5, 0, 5, 1],
          groups: [{ start: 0, count: 6, materialIndex: 0 }, { start: 6, count: 6, materialIndex: 1 }], uvs: [0, 0, 1, 0, 1, 1, 0, 1, 0, 1, 1, 1] } }, material(),
          { slots: [{ id: 'front', name: 'Front', material: material({ color: '#FF6B1A' }) }, { id: 'floor', name: 'Floor', material: material({ color: '#4C8DFF', metalness: 0.3 }) }] })] },
    matrices: { crate: matrix([-0.9, 0, 0], [25, 35, 0]), sheet: matrix([1, 0, 0], [0, -30, 0], [0.7, 0.7, 0.7]) },
    outputs: set(['crate', 'sheet']) });
  add({ id: 'maps-placement', width: W, height: H,
    textures: { tex_color: 'checker-128x96', tex_normal: 'bumps-64', tex_rough: 'ramp-64', tex_emit: 'dots-96x64', tex_ao: 'ramp-64' },
    scene: { ...base([light('sun', 'sun')], environment({}, '#101010')),
      objects: [
        object('panel', { primitive: 'plane', parameters: { width: 2, height: 1.5 } }, material({ color: '#FFFFFF', roughness: 0.6 }, {
          maps: { color: 'tex_color', normal: 'tex_normal', roughness: 'tex_rough', metalness: 'tex_rough', emissive: 'tex_emit', ao: 'tex_ao' },
          placement: { scaleX: 2, scaleY: 1.5, offsetX: 0.1, offsetY: -0.2, rotation: 15 } })),
        object('cube', { primitive: 'box' }, material({ color: '#FFFFFF' }, { maps: { color: 'tex_color' } }))] },
    matrices: { panel: matrix([-0.8, 0, 0], [0, 25, 0]), cube: matrix([1.1, 0, 0], [20, -30, 0]) }, outputs: set(['panel', 'cube']) });
  add({ id: 'node-material-and-source', width: W, height: H,
    scene: { ...base([light('sun', 'sun')]),
      objects: [
        object('noded', { primitive: 'sphere' }, material({ color: '#888888' }, { shader: { id: 'm1', name: 'Clay', graph: { nodes: [{ id: 'surface', type: 'ShaderNodeBsdfPrincipled', inputs: {}, properties: {} }, { id: 'output', type: 'ShaderNodeOutputMaterial', inputs: {}, properties: {} }], links: [{ from: 'surface', output: 'BSDF', to: 'output', input: 'Surface' }] }, inputs: [{ id: 'base', label: 'Base Color', kind: 'color', node: 'surface', socket: 'Base Color' }, { id: 'rough', label: 'Roughness', kind: 'number', node: 'surface', socket: 'Roughness' }, { id: 'glow', label: 'Emission Strength', kind: 'number', node: 'surface', socket: 'Emission Strength' }],
          p: p({ base: '#3366FF', rough: 0.15, glow: 0.5 }) } })),
        object('source', { primitive: 'box' }, material({ color: '#FF0000' }), { useSourceMaterials: true })] },
    matrices: { noded: matrix([-0.8, 0, 0]), source: matrix([0.9, 0, 0], [0, 30, 0]) }, outputs: set(['noded', 'source']) });
  add({ id: 'mirrored-single-sided', width: W, height: H,
    scene: { ...base([light('sun', 'sun')]),
      objects: [object('mirror', { primitive: 'cone' }, material({ color: '#FFAA88' }, { doubleSided: false })),
        object('normal', { primitive: 'cone' }, material({ color: '#88AAFF' }, { doubleSided: false }))] },
    matrices: { mirror: matrix([-0.8, 0, 0], [0, 0, 0], [-1, 1, 1]), normal: matrix([0.8, 0, 0]) }, outputs: set(['mirror', 'normal']) });
  add({ id: 'missing-model', width: W, height: H,
    scene: { ...base([light('sun', 'sun')], environment({}, '#101418')),
      objects: [object('ghost', { assetId: 'nope' }), object('ball', { primitive: 'sphere' }), floor()] },
    matrices: { ghost: matrix([-0.7, 0.1, 0], [0, 30, 0]), ball: matrix([0.8, 0, 0]), floor: floorMatrix() }, outputs: set(['ghost', 'ball', 'floor']) });

  // Editor looks (view override).
  const looks = { ...base([light('sun', 'sun')], environment({}, '#202020')),
    objects: [object('box', { primitive: 'box' }, material({ color: '#FF6B1A' })), object('ball', { primitive: 'sphere', parameters: { radius: 0.5 } }, material({ color: '#4C8DFF', opacity: 0.7 })), floor()] };
  const looksM = { box: matrix([-0.4, 0, 0], [0, 30, 0]), ball: matrix([0.5, 0, 0.5]), floor: floorMatrix() };
  add({ id: 'look-wireframe', width: W, height: H, scene: looks, matrices: looksM,
    outputs: set(['box', 'ball', 'floor']).map(o => ({ ...o, look: { wireframe: true } })) });
  add({ id: 'look-xray', width: W, height: H, scene: looks, matrices: looksM,
    outputs: set(['box', 'ball', 'floor']).map(o => ({ ...o, look: { xray: true } })) });
  add({ id: 'look-editor-camera', width: W, height: H, scene: looks, matrices: looksM,
    outputs: set(['box', 'ball', 'floor']).map(o => ({ ...o, editorCamera: { position: [-3, 4, 3], target: [0, 0, 0], up: [0, 1, 0], projection: 'perspective', fov: 50, zoom: 1, near: 0.01, far: 1000 } })) });
  add({ id: 'look-editor-ortho-wireframe', width: W, height: H, scene: looks, matrices: looksM,
    outputs: set(['box', 'ball', 'floor']).map(o => ({ ...o, look: { wireframe: true, xray: true },
      editorCamera: { position: [0, 200, 0.001], target: [0, 0, 0], up: [0, 1, 0], projection: 'orthographic', fov: 50, zoom: 1.5, near: 0.01, far: 400 } })) });

  // A legacy scene layer: its own hierarchy, visibility, and background, rendered whole.
  add({ id: 'legacy-scene', width: 480, height: 270,
    scene: { version: 1, lights: [light('key', 'sun', { x: 2, y: 4, z: 3 }), light('rim', 'point', { x: -2, y: 1, z: -2, intensity: 5 }, false)],
      camera: camera({ x: 0, y: 1.5, z: 5 }), environment: environment({ ambient: 0.4 }, '#0C0D12'),
      objects: [
        { ...object('cube', { primitive: 'box' }, material({ color: '#FF6B1A' })), p: transform({ ry: 35 }) },
        { ...object('ring', { primitive: 'torus' }, material({ color: '#4C8DFF' })), parent: 'cube', p: transform({ x: 1.2, rx: 90 }) },
        { ...object('hidden', { primitive: 'sphere' }), p: transform({ y: 1.5 }, false) },
        { ...object('child', { primitive: 'sphere', parameters: { radius: 0.2 } }), parent: 'hidden', p: transform({ x: 0.5 }) },
        { ...object('glass', { primitive: 'sphere', parameters: { radius: 0.4 } }, material({ color: '#FFFFFF', opacity: 0.4 })), p: transform({ x: -1.2, y: 0.2, z: 0.5 }) },
        { ...object('floor', { primitive: 'plane', parameters: { width: 6, height: 6 } }, material({ color: '#555555' })), p: transform({ y: -0.6, rx: -90 }) }] },
    outputs: [{}] });
  return out;
}

/** A legacy OBJ mesh layer render (`meshExtensionQuad`). */
export type MeshCase = {
  id: string; width: number; height: number;
  /** The OBJ text (absent: the missing-definition checkerboard). */
  obj?: string;
  /** u_yaw, u_pitch, u_distance, u_rotationX, u_rotationY, u_rotationZ, u_scale, u_roughness, u_metalness. */
  values: number[];
  objectColor: string; lightColor: string; background: string;
};

/** A deterministic OBJ: a lumpy UV sphere with quads, normals, and texture coordinates. */
function lumpyObj(rings: number, segments: number, normals: boolean): string {
  const lines = ['# generated', 'o lumpy'];
  for (let r = 0; r <= rings; r++) for (let s = 0; s < segments; s++) {
    const t = r / rings * Math.PI, p = s / segments * Math.PI * 2;
    const k = 1 + 0.15 * Math.sin(3 * p) * Math.sin(2 * t);
    lines.push(`v ${(k * Math.sin(t) * Math.cos(p) * 2 + 3).toFixed(5)} ${(k * Math.cos(t) * 2 - 1).toFixed(5)} ${(k * Math.sin(t) * Math.sin(p) * 2).toFixed(5)}`);
    if (normals) lines.push(`vn ${(Math.sin(t) * Math.cos(p)).toFixed(4)} ${Math.cos(t).toFixed(4)} ${(Math.sin(t) * Math.sin(p)).toFixed(4)}`);
  }
  for (let r = 0; r < rings; r++) for (let s = 0; s < segments; s++) {
    const a = r * segments + s + 1, b = r * segments + (s + 1) % segments + 1, c = a + segments, d = b + segments;
    lines.push(normals ? `f ${a}//${a} ${b}//${b} ${d}//${d} ${c}//${c}` : `f ${a} ${b} ${d} ${c}`);
  }
  return lines.join('\n');
}

export function meshCases(): MeshCase[] {
  const base = [28, 18, 4.8, 0, 25, 0, 1.35, 0.25, 0.7];
  return [
    { id: 'mesh-defaults', width: 640, height: 360, obj: lumpyObj(16, 24, true), values: base, objectColor: '#C7C4FF', lightColor: '#FFB36B', background: '#0C0D12' },
    { id: 'mesh-face-normals', width: 480, height: 480, obj: lumpyObj(10, 14, false),
      values: [-40, 35, 3.5, 20, 70, -15, 1.1, 0.8, 0.1], objectColor: '#FF6B1A', lightColor: '#A0C8FF', background: '#202830' },
    { id: 'mesh-rough-metal', width: 512, height: 288, obj: lumpyObj(20, 32, true),
      values: [120, -10, 6, 0, 0, 0, 2, 1, 1], objectColor: '#88FFEE', lightColor: '#FFFFFF', background: '#000000' },
    { id: 'mesh-missing', width: 400, height: 300, values: base, objectColor: '#C7C4FF', lightColor: '#FFB36B', background: '#0C0D12' },
  ];
}
