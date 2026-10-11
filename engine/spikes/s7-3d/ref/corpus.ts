// Builds the S7 3D corpus into out/corpus: textures, glTF models, and scene files.
// Scenes use the production Scene3D document shape (`d.data` of 3D layers), so the
// reference page feeds them straight into the production SceneRuntime.
//
//   bun ref/corpus.ts            # everything except the 1M-vertex pick model
//   bun ref/corpus.ts --big      # also the 1M-vertex skinned model (≈50 MB)
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { encodePng } from './png';
import * as tex from './textures';
import * as models from './models';

const root = join(dirname(new URL(import.meta.url).pathname), '..');
const out = join(root, 'out', 'corpus');
mkdirSync(join(out, 'assets'), { recursive: true });
mkdirSync(join(out, 'scenes'), { recursive: true });

const write = (name: string, bytes: Uint8Array) => { writeFileSync(join(out, 'assets', name), bytes); return `assets/${name}`; };
const pngOf = (img: tex.Image8) => encodePng(img.width, img.height, img.pixels);

// Textures.
const textures: Record<string, () => tex.Image8> = {
  checker: tex.checker, orientation: tex.orientation, bumps: tex.bumps, roughness: tex.roughness,
  metalness: tex.metalness, ao: tex.ao, emissive: tex.emissive, cutout: tex.cutout, ramp: tex.ramp, orm: tex.orm,
};
for (const [name, fn] of Object.entries(textures)) write(`${name}.png`, pngOf(fn()));

// WebP (lossless) and AVIF (lossy, 4:4:4) encodes for EXT_texture_webp / EXT_texture_avif.
// ffmpeg-static (6.0, libaom) supports 4:4:4 still-picture AV1; Homebrew's SVT-AV1 does not.
const ffmpeg: string = process.env.FFMPEG || createRequire(join(root, '../../../apps/desktop/package.json'))('ffmpeg-static');
execFileSync('cwebp', ['-quiet', '-lossless', '-exact', join(out, 'assets/orientation.png'), '-o', join(out, 'assets/orientation.webp')]);
execFileSync(ffmpeg, ['-loglevel', 'error', '-y', '-i', join(out, 'assets/checker.png'), '-c:v', 'libaom-av1', '-still-picture', '1',
  '-crf', '18', '-cpu-used', '4', '-pix_fmt', 'yuv444p', '-color_primaries', 'bt709', '-color_trc', 'iec61966-2-1', '-colorspace', 'bt709',
  '-color_range', 'pc', join(out, 'assets/checker.avif')]);

// Models.
write('skinned.glb', models.skinned());
write('pbr-ext.glb', models.pbrExtensions());
write('transmission.glb', models.transmission());
write('webp-avif.glb', models.webpAvif(readFileSync(join(out, 'assets/orientation.webp')), readFileSync(join(out, 'assets/checker.avif'))));
if (process.argv.includes('--big') && !existsSync(join(out, 'assets/big-skinned.glb'))) write('big-skinned.glb', models.bigSkinned());

// Scenes.
type Obj = Record<string, any>;
const prim = (id: string, primitive: string, p: Obj, material: Obj = {}, extra: Obj = {}) =>
  ({ id, name: id, source: { primitive, ...(extra.parameters ? { parameters: extra.parameters } : {}) }, p, material: { p: material.p || {}, maps: material.maps || {}, ...(material.placement ? { placement: material.placement } : {}), ...(material.doubleSided !== undefined ? { doubleSided: material.doubleSided } : {}) }, ...(extra.castShadow !== undefined ? { castShadow: extra.castShadow } : {}), ...(extra.receiveShadow !== undefined ? { receiveShadow: extra.receiveShadow } : {}) });
const floor = (color = '#8A8A8A') => prim('floor', 'plane', { rx: -90, y: -0.5, sx: 2, sy: 2 }, { p: { color, roughness: 0.9, metalness: 0 } });
const sun = (p: Obj = {}, castShadow = true) => ({ id: 'sun', name: 'Sun', type: 'sun', p: { x: -3, y: 5, z: 4, intensity: 3, ...p }, castShadow });
const model = (id: string, assetId: string, p: Obj = {}, useSourceMaterials = true) => ({ id, name: id, source: { assetId }, p, useSourceMaterials });

type Scene = { id: string; width: number; height: number; time: number; scene: Obj; assets: Record<string, Obj>; sets: string[][]; opacity?: Record<string, number>; notes: string };
const W = 640, H = 360;
const basic = [
  floor(),
  prim('box', 'box', { x: -1.5, y: 0, rx: 10, ry: 30 }, { p: { color: '#D94F30', roughness: 0.45, metalness: 0 } }),
  prim('ball', 'sphere', { x: 0, y: 0.1, z: 0.3 }, { p: { color: '#C7C4FF', roughness: 0.2, metalness: 0.8 } }),
  prim('ring', 'torus', { x: 1.4, y: 0.25, rx: 60 }, { p: { color: '#3FA796', roughness: 0.35, metalness: 0.3 } }),
  prim('cone', 'cone', { x: 0.6, y: 0, z: -1.2 }, { p: { color: '#E8C547', roughness: 0.6, metalness: 0 } }),
];
const scenes: Scene[] = [
  {
    id: 'standard-sun', width: W, height: H, time: 0, assets: {}, notes: 'Standard materials, studio-default sun with PCF soft shadows, hemisphere ambient, transparent background.',
    scene: { objects: basic, lights: [sun()], camera: {}, environment: {} },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'exposure-background', width: W, height: H, time: 0, assets: {}, notes: 'Exposure 1.8 through ACES Filmic, tinted ambient, solid background on the bottom layer.',
    scene: { objects: basic, lights: [sun({ color: '#FFE2C4', intensity: 2.2 })], camera: { p: { x: 2.5, y: 2.6, z: 5.5, fov: 40 } }, environment: { p: { exposure: 1.8, ambient: 0.35, ambientColor: '#B4C8FF' }, background: '#1D2230' } },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'point-spot-shadows', width: W, height: H, time: 0, assets: {}, notes: 'Point light (cube shadow atlas), spot light with penumbra, sun without shadows; distance and decay.',
    scene: {
      objects: [floor('#9A9A9A'), ...basic.slice(1)],
      lights: [
        { id: 'bulb', name: 'Bulb', type: 'point', p: { x: 0.2, y: 1.6, z: 0.9, intensity: 6, color: '#FFD9A8', distance: 0, decay: 2 }, castShadow: true },
        { id: 'spot', name: 'Spot', type: 'spot', p: { x: -2.5, y: 3.5, z: 1.5, targetX: 0, targetY: 0, targetZ: 0, intensity: 30, angle: 28, penumbra: 0.5, distance: 12, decay: 2, color: '#A8C8FF' }, castShadow: true },
        sun({ intensity: 0.6 }, false),
      ],
      camera: {}, environment: { p: { ambient: 0.15 } },
    },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'four-shadow-lights', width: W, height: H, time: 0, assets: {}, notes: 'Five shadow-casting lights: only the first four in layer order cast (runtime limit).',
    scene: {
      objects: basic,
      lights: [
        sun({ intensity: 1.2 }),
        { id: 'spotA', type: 'spot', p: { x: 2.5, y: 3, z: 2, intensity: 15, angle: 35, penumbra: 0.3 }, castShadow: true },
        { id: 'bulb', type: 'point', p: { x: -0.8, y: 1.4, z: 1.2, intensity: 3, color: '#FFB0B0' }, castShadow: true },
        { id: 'spotB', type: 'spot', p: { x: 0, y: 4, z: -2, intensity: 12, angle: 40, penumbra: 0.6, color: '#B0FFB0' }, castShadow: true },
        { id: 'sun2', type: 'sun', p: { x: 4, y: 3, z: -1, intensity: 0.8, color: '#C0C0FF' }, castShadow: true },
      ],
      camera: {}, environment: { p: { ambient: 0.2 } },
    },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'area-light', width: W, height: H, time: 0, assets: {}, notes: 'Rectangular area light through the LTC tables (no shadows) over glossy and rough surfaces.',
    scene: {
      objects: [prim('floor', 'plane', { rx: -90, y: -0.5, sx: 2, sy: 2 }, { p: { color: '#7F7F7F', roughness: 0.25, metalness: 0 } }), ...basic.slice(1)],
      lights: [
        { id: 'panel', type: 'area', p: { x: 0, y: 2.2, z: 2, targetX: 0, targetY: 0, targetZ: 0, width: 2.4, height: 1, intensity: 6, color: '#FFF1E0' } },
        sun({ intensity: 0.4 }),
      ],
      camera: {}, environment: { p: { ambient: 0.1 } },
    },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'material-maps', width: W, height: H, time: 0, notes: 'Color (sRGB), normal, roughness, metalness, emissive (sRGB), and AO maps with UV placement.',
    assets: Object.fromEntries(['checker', 'bumps', 'roughness', 'metalness', 'emissive', 'ao', 'orientation'].map(n => [`tex_${n}`, { kind: 'image', file: `assets/${n}.png` }])),
    scene: {
      objects: [
        prim('floor', 'plane', { rx: -90, y: -0.5, sx: 2, sy: 2 }, { p: { color: '#FFFFFF', roughness: 0.8 }, maps: { color: 'tex_checker', normal: 'tex_bumps' }, placement: { scaleX: 3, scaleY: 3, offsetX: 0.1, offsetY: 0, rotation: 15 } }),
        prim('box', 'box', { x: -1.5, y: 0.05, ry: 35 }, { p: { color: '#FFFFFF', roughness: 1, metalness: 1 }, maps: { color: 'tex_orientation', roughness: 'tex_roughness', metalness: 'tex_metalness' } }),
        prim('ball', 'sphere', { x: 0, y: 0.1, z: 0.3 }, { p: { color: '#FFFFFF', roughness: 0.5, emissive: '#FFFFFF', emissiveIntensity: 1.5 }, maps: { color: 'tex_checker', emissive: 'tex_emissive', ao: 'tex_ao' } }),
        prim('ring', 'torus', { x: 1.4, y: 0.25, rx: 60 }, { p: { color: '#E0E0E0', roughness: 0.4, metalness: 0.2 }, maps: { normal: 'tex_bumps', ao: 'tex_ao' } }),
      ],
      lights: [sun()], camera: {}, environment: {},
    },
    sets: [['floor', 'box', 'ball', 'ring']],
  },
  {
    id: 'transparent-doublesided', width: W, height: H, time: 0, assets: {}, notes: 'Transparent materials (no depth write, double-sided back-then-front), single-sided planes seen from behind.',
    scene: {
      objects: [
        floor(),
        prim('glassA', 'sphere', { x: -0.5, y: 0.2, z: 0.4 }, { p: { color: '#66CCFF', opacity: 0.45, roughness: 0.1 } }),
        prim('glassB', 'box', { x: 0.4, y: 0.1, z: -0.1, ry: 25 }, { p: { color: '#FF8844', opacity: 0.6, roughness: 0.3 } }),
        prim('cardFront', 'plane', { x: 1.5, y: 0.3, ry: 20, sx: 0.8, sy: 0.8 }, { p: { color: '#E0E070' }, doubleSided: false }),
        prim('cardBack', 'plane', { x: -1.6, y: 0.3, ry: 200, sx: 0.8, sy: 0.8 }, { p: { color: '#70E0A0' }, doubleSided: false }),
        prim('solid', 'torus', { x: 0, y: 0.15, z: -1.3, rx: 80 }, { p: { color: '#B070E0', roughness: 0.3 } }),
      ],
      lights: [sun()], camera: {}, environment: {},
    },
    sets: [['floor', 'glassA', 'glassB', 'cardFront', 'cardBack', 'solid']],
  },
  {
    id: 'orthographic', width: W, height: H, time: 0, assets: {}, notes: 'Orthographic camera (half-height 3 / zoom) on a landscape frame; appendix A item 38.',
    scene: { objects: basic, lights: [sun()], camera: { projection: 'orthographic', p: { x: 4, y: 3.5, z: 6, zoom: 1.3 } }, environment: {} },
    sets: [['floor', 'box', 'ball', 'ring', 'cone']],
  },
  {
    id: 'primitives', width: W, height: H, time: 0, assets: {}, notes: 'Every primitive, lathe, extrude with bevel, and an indexed mesh source.',
    scene: {
      objects: [
        floor(),
        prim('box', 'box', { x: -2.4, y: 0.6 }, { p: { color: '#E06050' } }),
        prim('sphere', 'sphere', { x: -1.2, y: 0.6 }, { p: { color: '#50A0E0' } }, { parameters: { segments: 32 } }),
        prim('cylinder', 'cylinder', { x: 0, y: 0.6 }, { p: { color: '#60C070' } }, { parameters: { radiusTop: 0.3, radiusBottom: 0.5, height: 0.9 } }),
        prim('cone', 'cone', { x: 1.2, y: 0.6 }, { p: { color: '#E0C050' } }),
        prim('torus', 'torus', { x: 2.4, y: 0.6, rx: 45 }, { p: { color: '#B060E0' } }),
        prim('capsule', 'capsule', { x: -1.8, y: -0.4, z: 1, rz: 90 }, { p: { color: '#E07090' } }),
        prim('ico', 'icosahedron', { x: -0.6, y: -0.4, z: 1 }, { p: { color: '#70E0D0', roughness: 0.2 } }, { parameters: { detail: 1 } }),
        { id: 'lathe', name: 'lathe', source: { lathe: [[0, -0.4], [0.35, -0.35], [0.2, 0], [0.3, 0.3], [0, 0.4]], segments: 32 }, p: { x: 0.6, y: -0.3, z: 1 }, material: { p: { color: '#D0D0D0', roughness: 0.2, metalness: 0.9 }, maps: {} } },
        { id: 'extrude', name: 'extrude', source: { extrude: [[-0.3, -0.3], [0.3, -0.3], [0.4, 0.1], [0, 0.4], [-0.4, 0.1]], depth: 0.2, bevel: 0.05 }, p: { x: 1.8, y: -0.3, z: 1, ry: 30 }, material: { p: { color: '#F09040' }, maps: {} } },
        { id: 'mesh', name: 'mesh', source: { mesh: { positions: [-0.4, 0, 0, 0.4, 0, 0, 0, 0.6, 0, 0, 0, 0.4], indices: [0, 1, 2, 0, 3, 1, 1, 3, 2, 2, 3, 0] } }, p: { x: 0, y: -0.45, z: 2 }, material: { p: { color: '#90A0F0' }, maps: {} } },
      ],
      lights: [sun()], camera: { p: { x: 0, y: 2.5, z: 6.5, fov: 50 } }, environment: {},
    },
    sets: [['floor', 'box', 'sphere', 'cylinder', 'cone', 'torus', 'capsule', 'ico', 'lathe', 'extrude', 'mesh']],
  },
  {
    id: 'depth-sets', width: W, height: H, time: 0, assets: {}, notes: 'Two depth sets split by a 2D barrier; a half-opacity layer that writes no depth; shadows only within a set.',
    opacity: { ball: 0.5 },
    scene: { objects: basic, lights: [sun()], camera: {}, environment: { background: '#202024' } },
    sets: [['floor', 'box', 'ball'], ['ring', 'cone']],
  },
  {
    id: 'missing-model', width: W, height: H, time: 0, assets: {}, notes: 'Missing model asset: orange wireframe box (output), next to a normal object.',
    scene: { objects: [floor(), { id: 'ghost', name: 'ghost', source: { assetId: 'asset_missing' }, p: { x: -0.6, y: 0.2 } }, prim('ball', 'sphere', { x: 0.9, y: 0.1 })], lights: [sun()], camera: {}, environment: {} },
    sets: [['floor', 'ghost', 'ball']],
  },
  {
    id: 'gltf-skinned', width: W, height: H, time: 0.7, notes: 'Skinned and morphing glTF with three clips mixed at weight 1 (shared j0 rotation); LINEAR, STEP, CUBICSPLINE.',
    assets: { model_skinned: { kind: 'model', file: 'assets/skinned.glb' } },
    scene: { objects: [floor(), model('rig', 'model_skinned', { sx: 1.6, sy: 1.6, sz: 1.6, y: 0.3 })], lights: [sun()], camera: {}, environment: {} },
    sets: [['floor', 'rig']],
  },
  {
    id: 'gltf-pbr-ext', width: W, height: H, time: 0, notes: 'r180 material extensions, vertex colors, alpha mask and blend, flat shading, quantization, bump, ORM with TEXCOORD_1, points, lines, instancing, punctual lights.',
    assets: { model_pbr: { kind: 'model', file: 'assets/pbr-ext.glb' } },
    scene: { objects: [model('grid', 'model_pbr', { sx: 2.6, sy: 2.6, sz: 2.6 })], lights: [sun({ intensity: 2 })], camera: { p: { x: 0, y: 0.3, z: 5.2, fov: 45 } }, environment: { background: '#26262B', p: { ambient: 0.4 } } },
    sets: [['grid']],
  },
  {
    id: 'gltf-transmission', width: W, height: H, time: 0, notes: 'Transmission pass (only the layer\'s own objects), volume attenuation, IOR, dispersion, rough transmission.',
    assets: { model_glass: { kind: 'model', file: 'assets/transmission.glb' } },
    scene: { objects: [floor(), model('glass', 'model_glass', { sx: 2, sy: 2, sz: 2, y: 0.4 })], lights: [sun()], camera: { p: { x: 0.5, y: 1.2, z: 4.5 } }, environment: { p: { ambient: 0.6 } } },
    sets: [['floor', 'glass']],
  },
  {
    id: 'gltf-webp-avif', width: W, height: H, time: 0, notes: 'EXT_texture_webp (lossless orientation chart) and EXT_texture_avif images.',
    assets: { model_images: { kind: 'model', file: 'assets/webp-avif.glb' } },
    scene: { objects: [model('cards', 'model_images', { sx: 2.4, sy: 2.4, sz: 2.4 })], lights: [sun()], camera: { p: { x: 0, y: 0, z: 4 } }, environment: { p: { ambient: 0.8 } } },
    sets: [['cards']],
  },
];

for (const s of scenes) writeFileSync(join(out, 'scenes', `${s.id}.json`), JSON.stringify(s, null, 1));
writeFileSync(join(out, 'index.json'), JSON.stringify(scenes.map(s => s.id)));
console.log(`corpus: ${scenes.length} scenes in ${out}`);
