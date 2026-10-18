// Corpus glTF models for the native 3D pass's glTF parity (docs/engine/gltf.md), from spike S7. Each exercises a set of r180 GLTFLoader features (plan sections 8 and 25).
import { Gltf, sphere, tube } from './glb';
import { encodePng } from './png';
import * as tex from './textures';

const ARRAY_BUFFER = 34962, ELEMENT_ARRAY_BUFFER = 34963;
const png = (img: tex.Image8) => encodePng(img.width, img.height, img.pixels);
const quat = (axis: [number, number, number], degrees: number) => {
  const h = (degrees * Math.PI) / 360, s = Math.sin(h);
  return [axis[0] * s, axis[1] * s, axis[2] * s, Math.cos(h)];
};

function addMesh(g: Gltf, geo: { positions: number[]; normals?: number[]; uvs?: number[]; indices?: number[] }, material: number, extra: Record<string, number> = {}, mode?: number) {
  const attributes: Record<string, number> = { POSITION: g.accessor(geo.positions, 'VEC3', 'f32', { target: ARRAY_BUFFER }) };
  if (geo.normals) attributes.NORMAL = g.accessor(geo.normals, 'VEC3', 'f32', { target: ARRAY_BUFFER });
  if (geo.uvs) attributes.TEXCOORD_0 = g.accessor(geo.uvs, 'VEC2', 'f32', { target: ARRAY_BUFFER });
  Object.assign(attributes, extra);
  const primitive: Record<string, unknown> = { attributes, material };
  if (geo.indices) primitive.indices = g.accessor(geo.indices, 'SCALAR', geo.positions.length / 3 > 65535 ? 'u32' : 'u16', { target: ELEMENT_ARRAY_BUFFER });
  if (mode !== undefined) primitive.mode = mode;
  return g.mesh({ primitives: [primitive] });
}

/** Skinned tube with a 3-joint chain and a morphing sphere; three clips, one shared channel. */
export function skinned(): Uint8Array {
  const g = new Gltf();
  const chart = g.texture(g.image(png(tex.orientation()), 'image/png'));
  const skinMat = g.material({ name: 'Skin', pbrMetallicRoughness: { baseColorTexture: { index: chart }, metallicFactor: 0.1, roughnessFactor: 0.55 } });
  const morphMat = g.material({ name: 'Morph', pbrMetallicRoughness: { baseColorFactor: [0.95, 0.45, 0.12, 1], metallicFactor: 0.5, roughnessFactor: 0.3 } });

  const t = tube(0.18, 0, 1.8, 24, 18);
  const count = t.positions.length / 3, joints: number[] = [], weights: number[] = [];
  for (let i = 0; i < count; i++) {
    const y = t.positions[i * 3 + 1]!;
    // Smooth weights between neighboring joints, 0.6 apart.
    const f = Math.min(2, Math.max(0, y / 0.6 - 0.5));
    const k = Math.min(1, Math.floor(f)), w = f - k;
    joints.push(k, k + 1, 0, 0);
    weights.push(1 - w, w, 0, 0);
  }
  const skinMesh = addMesh(g, t, skinMat, {
    JOINTS_0: g.accessor(joints, 'VEC4', 'u8', { target: ARRAY_BUFFER, minmax: false }),
    WEIGHTS_0: g.accessor(weights, 'VEC4', 'f32', { target: ARRAY_BUFFER, minmax: false }),
  });
  const j2 = g.node({ name: 'j2', translation: [0, 0.6, 0] }, false);
  const j1 = g.node({ name: 'j1', translation: [0, 0.6, 0], children: [j2] }, false);
  const j0 = g.node({ name: 'j0', translation: [-0.5, 0, 0], children: [j1] });
  const ibm = g.accessor([
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0.5, 0, 0, 1,
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0.5, -0.6, 0, 1,
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0.5, -1.2, 0, 1,
  ], 'MAT4', 'f32', { minmax: false });
  g.json.skins = [{ joints: [j0, j1, j2], inverseBindMatrices: ibm, skeleton: j0 }];
  g.node({ name: 'tube', mesh: skinMesh, skin: 0 });

  const s = sphere(0.3, 24, 16), n = s.positions.length / 3;
  const bulge: number[] = [], squash: number[] = [], bulgeN: number[] = [];
  for (let i = 0; i < n; i++) {
    const [x, y, z] = [s.normals[i * 3]!, s.normals[i * 3 + 1]!, s.normals[i * 3 + 2]!];
    const amp = 0.12 * (0.5 + 0.5 * Math.sin(x * 9) * Math.cos(z * 7));
    bulge.push(x * amp, y * amp, z * amp);
    bulgeN.push(0.15 * x * y, -0.1 * z, 0.05 * x);
    squash.push(0.08 * x, -0.12 * y, 0.08 * z);
  }
  const morphMesh = g.mesh({
    weights: [0.3, 0],
    extras: { targetNames: ['bulge', 'squash'] },
    primitives: [{
      attributes: {
        POSITION: g.accessor(s.positions, 'VEC3', 'f32', { target: ARRAY_BUFFER }),
        NORMAL: g.accessor(s.normals, 'VEC3', 'f32', { target: ARRAY_BUFFER }),
      },
      indices: g.accessor(s.indices, 'SCALAR', 'u16', { target: ELEMENT_ARRAY_BUFFER }),
      material: morphMat,
      targets: [
        { POSITION: g.accessor(bulge, 'VEC3', 'f32', { target: ARRAY_BUFFER }), NORMAL: g.accessor(bulgeN, 'VEC3', 'f32', { target: ARRAY_BUFFER }) },
        { POSITION: g.accessor(squash, 'VEC3', 'f32', { target: ARRAY_BUFFER }) },
      ],
    }],
  });
  const morphNode = g.node({ name: 'morph', mesh: morphMesh, translation: [0.55, 0.6, 0] });

  const times = (v: number[]) => g.accessor(v, 'SCALAR', 'f32');
  const out = (v: number[], type: string) => g.accessor(v, type, 'f32', { minmax: false });
  g.json.animations = [
    {
      name: 'bend',
      samplers: [
        { input: times([0, 1, 2]), output: out([...quat([0, 0, 1], 0), ...quat([0, 0, 1], 35), ...quat([0, 0, 1], 0)], 'VEC4'), interpolation: 'LINEAR' },
        { input: times([0, 1, 2]), output: out([...quat([0, 0, 1], 0), ...quat([0, 0, 1], 30), ...quat([0, 0, 1], 0)], 'VEC4'), interpolation: 'LINEAR' },
        { input: times([0, 2]), output: out([...quat([1, 0, 0], 0), ...quat([1, 0, 0], 20)], 'VEC4'), interpolation: 'LINEAR' },
      ],
      channels: [
        { sampler: 0, target: { node: j1, path: 'rotation' } },
        { sampler: 1, target: { node: j2, path: 'rotation' } },
        { sampler: 2, target: { node: j0, path: 'rotation' } },
      ],
    },
    {
      name: 'twist',
      samplers: [
        { input: times([0, 1.5, 3]), output: out([...quat([0, 1, 0], 0), ...quat([0, 1, 0], 60), ...quat([0, 1, 0], 0)], 'VEC4'), interpolation: 'LINEAR' },
        // Cubic spline: in-tangent, value, out-tangent per key.
        { input: times([0, 1.5, 3]), output: out([0, 0, 0, -0.5, 0, 0, 0, 0, 0, 0.2, 0, 0, -0.3, 0.15, 0, 0.2, 0.1, 0, 0, 0, 0, -0.5, 0, 0, 0, 0, 0], 'VEC3'), interpolation: 'CUBICSPLINE' },
      ],
      channels: [
        { sampler: 0, target: { node: j0, path: 'rotation' } },
        { sampler: 1, target: { node: j0, path: 'translation' } },
      ],
    },
    {
      name: 'morph',
      samplers: [
        { input: times([0, 1, 2]), output: out([0.3, 0, 1, 0.6, 0.3, 0], 'SCALAR'), interpolation: 'LINEAR' },
        { input: times([0, 0.8, 1.6]), output: out([1, 1, 1, 1.15, 0.85, 1.15, 1, 1, 1], 'VEC3'), interpolation: 'STEP' },
      ],
      channels: [
        { sampler: 0, target: { node: morphNode, path: 'weights' } },
        { sampler: 1, target: { node: morphNode, path: 'scale' } },
      ],
    },
  ];
  return g.glb();
}

/** Grid of r180 material extensions, primitives, instancing, quantization, and punctual lights. */
export function pbrExtensions(): Uint8Array {
  const g = new Gltf();
  for (const ext of ['KHR_materials_clearcoat', 'KHR_materials_sheen', 'KHR_materials_iridescence', 'KHR_materials_specular',
    'KHR_materials_ior', 'KHR_materials_emissive_strength', 'KHR_materials_anisotropy', 'KHR_texture_transform',
    'KHR_materials_unlit', 'KHR_lights_punctual', 'EXT_materials_bump', 'EXT_mesh_gpu_instancing']) g.useExtension(ext);
  g.useExtension('KHR_mesh_quantization', true);
  const checker = g.texture(g.image(png(tex.checker()), 'image/png'));
  const ramp = g.texture(g.image(png(tex.ramp()), 'image/png'));
  const normal = g.texture(g.image(png(tex.bumps()), 'image/png'));
  const orm = g.texture(g.image(png(tex.orm()), 'image/png'));
  const cutout = g.texture(g.image(png(tex.cutout()), 'image/png'), { magFilter: 9729, minFilter: 9729, wrapS: 33071, wrapT: 33071 });

  const base = (c: number[], metal = 0, rough = 0.5) => ({ baseColorFactor: [...c, 1], metallicFactor: metal, roughnessFactor: rough });
  const materials = [
    g.material({ name: 'plain', pbrMetallicRoughness: base([0.8, 0.3, 0.2]) }),
    g.material({ name: 'clearcoat', pbrMetallicRoughness: base([0.7, 0.05, 0.05], 0, 0.6), extensions: { KHR_materials_clearcoat: { clearcoatFactor: 1, clearcoatRoughnessFactor: 0.08 } } }),
    g.material({ name: 'sheen', pbrMetallicRoughness: base([0.1, 0.05, 0.12], 0, 0.8), extensions: { KHR_materials_sheen: { sheenColorFactor: [0.9, 0.3, 0.6], sheenRoughnessFactor: 0.4 } } }),
    g.material({ name: 'iridescence', pbrMetallicRoughness: base([0.05, 0.05, 0.05], 1, 0.15), extensions: { KHR_materials_iridescence: { iridescenceFactor: 1, iridescenceIor: 1.3, iridescenceThicknessMinimum: 100, iridescenceThicknessMaximum: 400, iridescenceThicknessTexture: { index: ramp } } } }),
    g.material({ name: 'specular', pbrMetallicRoughness: base([0.2, 0.25, 0.6], 0, 0.25), extensions: { KHR_materials_specular: { specularFactor: 0.5, specularColorFactor: [1, 0.6, 0.2] }, KHR_materials_ior: { ior: 1.8 } } }),
    g.material({ name: 'emissive', pbrMetallicRoughness: base([0.1, 0.1, 0.1]), emissiveFactor: [1, 0.5, 0.1], extensions: { KHR_materials_emissive_strength: { emissiveStrength: 4 } } }),
    g.material({ name: 'anisotropy', pbrMetallicRoughness: base([0.9, 0.85, 0.8], 1, 0.4), extensions: { KHR_materials_anisotropy: { anisotropyStrength: 0.8, anisotropyRotation: 0.6 } } }),
    g.material({ name: 'transform', pbrMetallicRoughness: { baseColorTexture: { index: checker, extensions: { KHR_texture_transform: { offset: [0.25, 0], rotation: 0.3, scale: [2, 2] } } }, metallicFactor: 0, roughnessFactor: 0.6 } }),
    g.material({ name: 'unlit', pbrMetallicRoughness: base([0.2, 0.6, 0.9]), extensions: { KHR_materials_unlit: {} } }),
    g.material({ name: 'vertexcolor', pbrMetallicRoughness: base([1, 1, 1], 0, 0.45) }),
    g.material({ name: 'mask', pbrMetallicRoughness: { baseColorTexture: { index: cutout }, metallicFactor: 0, roughnessFactor: 0.7 }, alphaMode: 'MASK', alphaCutoff: 0.5, doubleSided: true }),
    g.material({ name: 'blend', pbrMetallicRoughness: { baseColorFactor: [0.2, 0.8, 0.4, 0.5], metallicFactor: 0, roughnessFactor: 0.3 }, alphaMode: 'BLEND' }),
    g.material({ name: 'flat', pbrMetallicRoughness: base([0.6, 0.6, 0.65], 0.2, 0.5) }),
    g.material({ name: 'quantized', pbrMetallicRoughness: { baseColorTexture: { index: checker }, metallicFactor: 0, roughnessFactor: 0.5 } }),
    g.material({ name: 'bump', pbrMetallicRoughness: base([0.75, 0.7, 0.6], 0, 0.6), extensions: { EXT_materials_bump: { bumpFactor: 0.05, bumpTexture: { index: ramp } } } }),
    g.material({ name: 'orm', pbrMetallicRoughness: { baseColorFactor: [0.9, 0.9, 0.9, 1], metallicRoughnessTexture: { index: orm }, metallicFactor: 1, roughnessFactor: 1 }, normalTexture: { index: normal, scale: 1.2 }, occlusionTexture: { index: orm, texCoord: 1, strength: 0.9 } }),
    g.material({ name: 'points', pbrMetallicRoughness: base([1, 0.9, 0.3]) }),
    g.material({ name: 'lines', pbrMetallicRoughness: base([0.3, 1, 0.9]) }),
    g.material({ name: 'instanced', pbrMetallicRoughness: base([0.9, 0.5, 0.9], 0.3, 0.4) }),
    g.material({ name: 'doublesided', pbrMetallicRoughness: { baseColorTexture: { index: checker }, metallicFactor: 0, roughnessFactor: 0.5 }, doubleSided: true }),
  ];
  const s = sphere(0.3, 32, 20);
  const columns = 5, spacing = 0.8;
  const place = (i: number): [number, number, number] => [((i % columns) - (columns - 1) / 2) * spacing, (1.5 - Math.floor(i / columns)) * spacing, 0];
  const node = (i: number, mesh: number, extra: Record<string, unknown> = {}) => g.node({ name: `cell${i}`, mesh, translation: place(i), ...extra });

  for (const i of [0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 14]) node(i, addMesh(g, s, materials[i]!));
  // 9: vertex colors (RGBA float).
  {
    const n = s.positions.length / 3, colors: number[] = [];
    for (let k = 0; k < n; k++) colors.push(0.5 + 0.5 * s.normals[k * 3]!, 0.5 + 0.5 * s.normals[k * 3 + 1]!, 0.5 + 0.5 * s.normals[k * 3 + 2]!, 1);
    node(9, addMesh(g, s, materials[9]!, { COLOR_0: g.accessor(colors, 'VEC4', 'f32', { target: ARRAY_BUFFER }) }));
  }
  // 10: alpha-masked quad, double sided, facing the camera.
  node(10, addMesh(g, { positions: [-0.35, -0.35, 0, 0.35, -0.35, 0, 0.35, 0.35, 0, -0.35, 0.35, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 1, 1, 1, 1, 0, 0, 0], indices: [0, 1, 2, 0, 2, 3] }, materials[10]!), { rotation: quat([0, 1, 0], 25) });
  // 12: flat shading (no NORMAL attribute).
  node(12, addMesh(g, { positions: sphere(0.3, 10, 7).positions, indices: sphere(0.3, 10, 7).indices }, materials[12]!));
  // 13: KHR_mesh_quantization: normalized int16 positions, int8 normals, uint16 UVs.
  {
    const q = sphere(1, 24, 16);
    const primitive = {
      attributes: {
        POSITION: g.accessor(q.positions.map(v => Math.round(v * 32767)), 'VEC3', 'i16', { target: ARRAY_BUFFER, normalized: true }),
        NORMAL: g.accessor(q.normals.map(v => Math.round(v * 127)), 'VEC3', 'i8', { target: ARRAY_BUFFER, normalized: true }),
        TEXCOORD_0: g.accessor(q.uvs.map(v => Math.round(v * 65535)), 'VEC2', 'u16', { target: ARRAY_BUFFER, normalized: true }),
      },
      indices: g.accessor(q.indices, 'SCALAR', 'u16', { target: ELEMENT_ARRAY_BUFFER }),
      material: materials[13],
    };
    node(13, g.mesh({ primitives: [primitive] }), { scale: [0.3, 0.3, 0.3] });
  }
  // 15: ORM + normal map, occlusion on TEXCOORD_1.
  {
    const uv1 = s.uvs.map((v, k) => (k % 2 ? 1 - v : v) * 0.5 + 0.25);
    node(15, addMesh(g, s, materials[15]!, { TEXCOORD_1: g.accessor(uv1, 'VEC2', 'f32', { target: ARRAY_BUFFER }) }));
  }
  // 16: points (mode 0), 17: lines (mode 1) plus a line loop (mode 2).
  {
    const pts: number[] = [];
    for (let y = 0; y < 9; y++) for (let x = 0; x < 9; x++) pts.push((x - 4) * 0.07, (y - 4) * 0.07, 0);
    node(16, addMesh(g, { positions: pts }, materials[16]!, {}, 0));
    const ring: number[] = [], loop: number[] = [];
    for (let k = 0; k < 24; k++) {
      const a0 = (k / 24) * Math.PI * 2, a1 = ((k + 1) / 24) * Math.PI * 2;
      ring.push(Math.cos(a0) * 0.3, Math.sin(a0) * 0.3, 0, Math.cos(a1) * 0.3, Math.sin(a1) * 0.3, 0);
      loop.push(Math.cos(a0) * 0.18, Math.sin(a0) * 0.18, 0);
    }
    const lines = g.mesh({ primitives: [
      { attributes: { POSITION: g.accessor(ring, 'VEC3', 'f32', { target: ARRAY_BUFFER }) }, material: materials[17], mode: 1 },
      { attributes: { POSITION: g.accessor(loop, 'VEC3', 'f32', { target: ARRAY_BUFFER }) }, material: materials[17], mode: 2 },
    ] });
    node(17, lines);
  }
  // 18: EXT_mesh_gpu_instancing, six small boxes.
  {
    const box = boxGeometry(0.12);
    const mesh = addMesh(g, box, materials[18]!);
    const t: number[] = [], r: number[] = [], sc: number[] = [];
    for (let k = 0; k < 6; k++) {
      const a = (k / 6) * Math.PI * 2;
      t.push(Math.cos(a) * 0.22, Math.sin(a) * 0.22, 0);
      r.push(...quat([0.3, 0.8, 0.5].map(v => v / Math.hypot(0.3, 0.8, 0.5)) as [number, number, number], k * 25));
      sc.push(1, 1 + k * 0.15, 1);
    }
    node(18, mesh, { extensions: { EXT_mesh_gpu_instancing: { attributes: {
      TRANSLATION: g.accessor(t, 'VEC3', 'f32'), ROTATION: g.accessor(r, 'VEC4', 'f32'), SCALE: g.accessor(sc, 'VEC3', 'f32'),
    } } } });
  }
  // 19: double-sided plane turned to show its back face.
  node(19, addMesh(g, { positions: [-0.3, -0.3, 0, 0.3, -0.3, 0, 0.3, 0.3, 0, -0.3, 0.3, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 1, 1, 1, 1, 0, 0, 0], indices: [0, 1, 2, 0, 2, 3] }, materials[19]!), { rotation: quat([0, 1, 0], 150) });

  // Embedded punctual lights: they light the scene but are outside the 16-light limit.
  g.json.extensions = { KHR_lights_punctual: { lights: [
    { type: 'spot', color: [1, 0.85, 0.7], intensity: 6, range: 12, spot: { innerConeAngle: 0.25, outerConeAngle: 0.55 } },
    { type: 'point', color: [0.6, 0.7, 1], intensity: 2.5 },
  ] } };
  g.node({ name: 'spot', translation: [1.5, 2.5, 2.5], rotation: lookRotation([1.5, 2.5, 2.5], [0, 0, 0]), extensions: { KHR_lights_punctual: { light: 0 } } });
  g.node({ name: 'point', translation: [-2, -0.5, 1.5], extensions: { KHR_lights_punctual: { light: 1 } } });
  return g.glb();
}

/** Transmission with volume, IOR, and dispersion over a backdrop owned by the same model. */
export function transmission(): Uint8Array {
  const g = new Gltf();
  for (const ext of ['KHR_materials_transmission', 'KHR_materials_volume', 'KHR_materials_ior', 'KHR_materials_dispersion']) g.useExtension(ext);
  const checker = g.texture(g.image(png(tex.checker()), 'image/png'));
  const backdrop = g.material({ name: 'backdrop', pbrMetallicRoughness: { baseColorTexture: { index: checker }, metallicFactor: 0, roughnessFactor: 0.8 } });
  const glass = g.material({ name: 'glass', pbrMetallicRoughness: { baseColorFactor: [1, 1, 1, 1], metallicFactor: 0, roughnessFactor: 0.05 },
    extensions: { KHR_materials_transmission: { transmissionFactor: 1 }, KHR_materials_ior: { ior: 1.5 },
      KHR_materials_volume: { thicknessFactor: 0.6, attenuationColor: [0.6, 0.9, 1], attenuationDistance: 1.5 },
      KHR_materials_dispersion: { dispersion: 0.4 } } });
  const frosted = g.material({ name: 'frosted', pbrMetallicRoughness: { baseColorFactor: [1, 0.95, 0.9, 1], metallicFactor: 0, roughnessFactor: 0.45 },
    extensions: { KHR_materials_transmission: { transmissionFactor: 0.9 }, KHR_materials_ior: { ior: 1.3 } } });
  const colors = [[0.9, 0.2, 0.15], [0.15, 0.7, 0.25], [0.2, 0.3, 0.9]];
  const plane = { positions: [-1.6, -1, 0, 1.6, -1, 0, 1.6, 1, 0, -1.6, 1, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 1, 1, 1, 1, 0, 0, 0], indices: [0, 1, 2, 0, 2, 3] };
  g.node({ name: 'backdrop', mesh: addMesh(g, plane, backdrop), translation: [0, 0, -0.9] });
  colors.forEach((c, k) => {
    const m = g.material({ name: `box${k}`, pbrMetallicRoughness: { baseColorFactor: [...c, 1], metallicFactor: 0, roughnessFactor: 0.4 } });
    g.node({ name: `box${k}`, mesh: addMesh(g, boxGeometry(0.3), m), translation: [-0.6 + k * 0.6, -0.15, -0.45], rotation: quat([0, 1, 0], 20 + k * 15) });
  });
  const s = sphere(0.42, 48, 32);
  g.node({ name: 'glass', mesh: addMesh(g, s, glass), translation: [-0.45, 0, 0.2] });
  g.node({ name: 'frosted', mesh: addMesh(g, sphere(0.32, 48, 32), frosted), translation: [0.55, 0.05, 0.25] });
  return g.glb();
}

/** EXT_texture_webp and EXT_texture_avif images on two quads (no PNG fallback). */
export function webpAvif(webp: Uint8Array, avif: Uint8Array): Uint8Array {
  const g = new Gltf();
  g.useExtension('EXT_texture_webp', true);
  g.useExtension('EXT_texture_avif', true);
  const webpTex = g.texture(g.image(webp, 'image/webp'), undefined, { EXT_texture_webp: { source: 0 } });
  const avifTex = g.texture(g.image(avif, 'image/avif'), undefined, { EXT_texture_avif: { source: 1 } });
  const quad = { positions: [-0.5, -0.5, 0, 0.5, -0.5, 0, 0.5, 0.5, 0, -0.5, 0.5, 0], normals: [0, 0, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1], uvs: [0, 1, 1, 1, 1, 0, 0, 0], indices: [0, 1, 2, 0, 2, 3] };
  const a = g.material({ name: 'webp', pbrMetallicRoughness: { baseColorTexture: { index: webpTex }, metallicFactor: 0, roughnessFactor: 0.7 } });
  const b = g.material({ name: 'avif', pbrMetallicRoughness: { baseColorTexture: { index: avifTex }, metallicFactor: 0, roughnessFactor: 0.7 } });
  g.node({ name: 'webp', mesh: addMesh(g, quad, a), translation: [-0.55, 0, 0] });
  g.node({ name: 'avif', mesh: addMesh(g, quad, b), translation: [0.55, 0, 0] });
  return g.glb();
}

/**
 * 1,000,000-vertex skinned model for pick timing: 250,000 separate quads (500,000
 * triangles, the import ceiling) on a cylinder, weighted to a 4-joint chain, with a clip.
 */
export function bigSkinned(): Uint8Array {
  const g = new Gltf();
  const mat = g.material({ name: 'big', pbrMetallicRoughness: { baseColorFactor: [0.7, 0.7, 0.75, 1], metallicFactor: 0, roughnessFactor: 0.5 } });
  const around = 500, up = 500, quads = around * up;
  const positions = new Float32Array(quads * 12), normals = new Float32Array(quads * 12);
  const joints = new Uint8Array(quads * 16), weights = new Float32Array(quads * 16), indices = new Uint32Array(quads * 6);
  const height = 3, radius = 0.5;
  let q = 0;
  for (let j = 0; j < up; j++) for (let i = 0; i < around; i++, q++) {
    const a0 = (i / around) * Math.PI * 2, a1 = ((i + 0.85) / around) * Math.PI * 2;
    const y0 = (j / up) * height, y1 = ((j + 0.85) / up) * height;
    const corners = [[a0, y0], [a1, y0], [a1, y1], [a0, y1]];
    corners.forEach(([a, y], k) => {
      const v = q * 4 + k;
      positions.set([Math.sin(a!) * radius, y!, Math.cos(a!) * radius], v * 3);
      normals.set([Math.sin(a!), 0, Math.cos(a!)], v * 3);
      const f = Math.min(3, Math.max(0, y! / 1 - 0.5)), b = Math.min(2, Math.floor(f)), w = f - b;
      joints.set([b, b + 1, 0, 0], v * 4);
      weights.set([1 - w, w, 0, 0], v * 4);
    });
    indices.set([q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3], q * 6);
  }
  const mesh = g.mesh({ primitives: [{
    attributes: {
      POSITION: g.accessor(positions, 'VEC3', 'f32', { target: ARRAY_BUFFER }),
      NORMAL: g.accessor(normals, 'VEC3', 'f32', { target: ARRAY_BUFFER, minmax: false }),
      JOINTS_0: g.accessor(joints, 'VEC4', 'u8', { target: ARRAY_BUFFER, minmax: false }),
      WEIGHTS_0: g.accessor(weights, 'VEC4', 'f32', { target: ARRAY_BUFFER, minmax: false }),
    },
    indices: g.accessor(indices, 'SCALAR', 'u32', { target: ELEMENT_ARRAY_BUFFER, minmax: false }),
    material: mat,
  }] });
  const j3 = g.node({ name: 'j3', translation: [0, 1, 0] }, false);
  const j2 = g.node({ name: 'j2', translation: [0, 1, 0], children: [j3] }, false);
  const j1 = g.node({ name: 'j1', translation: [0, 1, 0], children: [j2] }, false);
  const j0 = g.node({ name: 'j0', translation: [0, 0, 0], children: [j1] });
  const ibm: number[] = [];
  for (let k = 0; k < 4; k++) ibm.push(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, -k, 0, 1);
  g.json.skins = [{ joints: [j0, j1, j2, j3], inverseBindMatrices: g.accessor(ibm, 'MAT4', 'f32', { minmax: false }) }];
  g.node({ name: 'big', mesh, skin: 0 });
  const times = g.accessor([0, 1, 2], 'SCALAR', 'f32');
  g.json.animations = [{ name: 'wave', samplers: [1, 2, 3].map(k => ({ input: times, output: g.accessor([...quat([0, 0, 1], 0), ...quat([0, 0, 1], 12 * k), ...quat([0, 0, 1], 0)], 'VEC4', 'f32', { minmax: false }), interpolation: 'LINEAR' })),
    channels: [j1, j2, j3].map((node, k) => ({ sampler: k, target: { node, path: 'rotation' } })) }];
  return g.glb();
}

export function boxGeometry(size: number) {
  const h = size / 2, positions: number[] = [], normals: number[] = [], uvs: number[] = [], indices: number[] = [];
  const faces: [number[], number[], number[]][] = [
    [[1, 0, 0], [0, 0, -1], [0, 1, 0]], [[-1, 0, 0], [0, 0, 1], [0, 1, 0]], [[0, 1, 0], [1, 0, 0], [0, 0, -1]],
    [[0, -1, 0], [1, 0, 0], [0, 0, 1]], [[0, 0, 1], [1, 0, 0], [0, 1, 0]], [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
  ];
  for (const [n, u, v] of faces) {
    const start = positions.length / 3;
    for (const [su, sv] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
      positions.push((n[0]! + u[0]! * su! + v[0]! * sv!) * h, (n[1]! + u[1]! * su! + v[1]! * sv!) * h, (n[2]! + u[2]! * su! + v[2]! * sv!) * h);
      normals.push(...n);
      uvs.push((su! + 1) / 2, (1 - sv!) / 2);
    }
    indices.push(start, start + 1, start + 2, start, start + 2, start + 3);
  }
  return { positions, normals, uvs, indices };
}

/** Quaternion that points a node's -Z axis from `eye` toward `target` (glTF lights and cameras). */
function lookRotation(eye: number[], target: number[]): number[] {
  const z = [eye[0]! - target[0]!, eye[1]! - target[1]!, eye[2]! - target[2]!];
  const zl = Math.hypot(...z); z.forEach((v, i) => (z[i] = v / zl));
  const x = [z[2]!, 0, -z[0]!]; const xl = Math.hypot(...x); x.forEach((v, i) => (x[i] = v / xl));
  const y = [z[1]! * x[2]! - z[2]! * x[1]!, z[2]! * x[0]! - z[0]! * x[2]!, z[0]! * x[1]! - z[1]! * x[0]!];
  const [m00, m01, m02, m10, m11, m12, m20, m21, m22] = [x[0]!, y[0]!, z[0]!, x[1]!, y[1]!, z[1]!, x[2]!, y[2]!, z[2]!];
  const trace = m00 + m11 + m22;
  if (trace > 0) { const s = 0.5 / Math.sqrt(trace + 1); return [(m21 - m12) * s, (m02 - m20) * s, (m10 - m01) * s, 0.25 / s]; }
  if (m00 > m11 && m00 > m22) { const s = 2 * Math.sqrt(1 + m00 - m11 - m22); return [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]; }
  if (m11 > m22) { const s = 2 * Math.sqrt(1 + m11 - m00 - m22); return [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]; }
  const s = 2 * Math.sqrt(1 + m22 - m00 - m11); return [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s];
}
