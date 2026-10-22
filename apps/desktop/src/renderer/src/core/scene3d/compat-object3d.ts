/** Deprecated extension-facing three.js view of an engine-imported model.
 * All scene topology, attributes, material data and animation samples come
 * from pm-geom3d. The object is independent of the engine's render scene. */
import * as THREE from 'three';

const flat = (rows: number[] | number[][] | null | undefined): number[] =>
  rows ? (Array.isArray(rows[0]) ? (rows as number[][]).flat() : rows as number[]) : [];
const floatAttribute = (rows: number[] | number[][] | Float32Array, components: number) =>
  rows instanceof Float32Array
    ? new THREE.BufferAttribute(rows, components)
    : new THREE.Float32BufferAttribute(flat(rows), components);
const zeroMorph = (positions: number[][] | Float32Array): number[][] | Float32Array =>
  positions instanceof Float32Array
    ? new Float32Array(positions.length)
    : positions.map(() => [0, 0, 0]);
const bytes = (base64: string) => Uint8Array.from(atob(base64), c => c.charCodeAt(0));
function sourceJson(bytes: Uint8Array): any {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const glb = bytes.length >= 20 && view.getUint32(0, true) === 0x46546c67;
  const start = glb ? 20 : 0;
  const end = glb ? 20 + view.getUint32(12, true) : bytes.length;
  return JSON.parse(new TextDecoder().decode(bytes.subarray(start, end)));
}
const filter = (v: number | null, min: boolean): THREE.TextureFilter => ({
  9728: THREE.NearestFilter, 9729: THREE.LinearFilter,
  9984: THREE.NearestMipmapNearestFilter, 9985: THREE.LinearMipmapNearestFilter,
  9986: THREE.NearestMipmapLinearFilter, 9987: THREE.LinearMipmapLinearFilter
})[v ?? (min ? 9987 : 9729)] ?? THREE.LinearFilter;
const wrap = (v: number) => v === 33071 ? THREE.ClampToEdgeWrapping : v === 33648 ? THREE.MirroredRepeatWrapping : THREE.RepeatWrapping;
const knownExtensions = new Set([
  'KHR_mesh_quantization', 'EXT_mesh_gpu_instancing', 'EXT_texture_webp', 'EXT_texture_avif',
  'KHR_texture_transform', 'KHR_materials_unlit', 'KHR_materials_emissive_strength',
  'KHR_materials_clearcoat', 'KHR_materials_sheen', 'KHR_materials_iridescence',
  'KHR_materials_specular', 'KHR_materials_ior', 'KHR_materials_transmission',
  'KHR_materials_volume', 'KHR_materials_dispersion', 'KHR_materials_anisotropy',
  'EXT_materials_bump', 'KHR_lights_punctual',
]);
function applyMetadata(target: {userData: Record<string, any>}, metadata: any, node = false): void {
  if (!metadata) return;
  if (node && metadata.name) target.userData.name = metadata.name;
  if (metadata.extras && typeof metadata.extras === 'object') Object.assign(target.userData, metadata.extras);
  for (const [name, value] of Object.entries(metadata.extensions ?? {})) {
    if (knownExtensions.has(name)) continue;
    (target.userData.gltfExtensions ??= {})[name] = value;
  }
}

async function texturesOf(data: any): Promise<THREE.Texture[]> {
  const images = await Promise.all(data.images.map(async (entry: any) => {
    const blob = new Blob([bytes(entry.base64)], { type: entry.mime });
    return createImageBitmap(blob, { premultiplyAlpha: 'none' });
  }));
  const cache = new Map<string, THREE.Texture>();
  return data.textures.map((entry: any, index: number) => {
    // GLTFLoader caches a texture by image source and sampler. Materials
    // receive that same object until texCoord or a transform clones it;
    // a later sRGB use can therefore change earlier material maps too.
    const source = data.rawImages?.[entry.source];
    const key = `${source ? (source.uri || source.bufferView) : entry.source}:${data.rawTextures?.[index]?.sampler ?? JSON.stringify(entry.sampler)}`;
    const cached = cache.get(key);
    if (cached) return cached;
    const tex = new THREE.Texture(entry.source == null ? undefined : images[entry.source]);
    tex.name = data.metadata?.textures?.[index]?.name || data.metadata?.images?.[entry.source]?.name || '';
    tex.flipY = false;
    tex.magFilter = filter(entry.sampler.mag, false) as THREE.MagnificationTextureFilter;
    tex.minFilter = filter(entry.sampler.min, true) as THREE.MinificationTextureFilter;
    tex.wrapS = wrap(entry.sampler.wrapS);
    tex.wrapT = wrap(entry.sampler.wrapT);
    tex.needsUpdate = entry.source != null;
    if (entry.source != null) {
      applyMetadata(tex, data.metadata?.images?.[entry.source]);
      tex.userData.mimeType = data.images[entry.source].mime;
    }
    cache.set(key, tex);
    return tex;
  });
}

function geometry(p: any, metadata?: any): THREE.BufferGeometry {
  const g = new THREE.BufferGeometry();
  applyMetadata(g, metadata);
  g.setAttribute('position', floatAttribute(p.positions, 3));
  if (p.normals) g.setAttribute('normal', floatAttribute(p.normals, 3));
  if (p.uv) g.setAttribute('uv', floatAttribute(p.uv, 2));
  if (p.uv1) g.setAttribute('uv1', floatAttribute(p.uv1, 2));
  if (p.uv2) g.setAttribute('uv2', floatAttribute(p.uv2, 2));
  if (p.uv3) g.setAttribute('uv3', floatAttribute(p.uv3, 2));
  if (p.colors) {
    let colors: number[] | number[][] | Float32Array = p.colors;
    if (p.colorComponents === 3) {
      if (colors instanceof Float32Array) {
        const rgb = new Float32Array(colors.length / 4 * 3);
        for (let i = 0, j = 0; i < colors.length; i += 4, j += 3) {
          rgb[j] = colors[i]!; rgb[j + 1] = colors[i + 1]!; rgb[j + 2] = colors[i + 2]!;
        }
        colors = rgb;
      } else {
        colors = flat(colors).filter((_: number, i: number) => i % 4 !== 3);
      }
    }
    g.setAttribute('color', floatAttribute(colors, p.colorComponents));
  }
  if (p.tangents) g.setAttribute('tangent', floatAttribute(p.tangents, 4));
  if (p.joints) g.setAttribute('skinIndex', p.joints instanceof Uint32Array
    ? new THREE.BufferAttribute(Uint16Array.from(p.joints), 4)
    : new THREE.Uint16BufferAttribute(flat(p.joints), 4));
  if (p.weights) g.setAttribute('skinWeight', floatAttribute(p.weights, 4));
  if (p.indices) {
    if (p.indices instanceof Uint32Array) {
      // WebGL2 reserves 65535 for primitive restart (three r180 utils).
      const wide = p.indices.some((value: number) => value >= 65535);
      g.setIndex(new THREE.BufferAttribute(wide ? p.indices : Uint16Array.from(p.indices), 1));
    } else g.setIndex(p.indices);
  }
  if (p.accessorBounds) {
    g.boundingBox = new THREE.Box3(new THREE.Vector3(...p.accessorBounds[0]), new THREE.Vector3(...p.accessorBounds[1]));
    g.boundingSphere = g.boundingBox.getBoundingSphere(new THREE.Sphere());
  }
  const targets = p.targets ?? [];
  if (targets.some((t: any) => t.position)) g.morphAttributes.position = targets.map((t: any) => floatAttribute(t.position ?? zeroMorph(p.positions), 3));
  if (targets.some((t: any) => t.normal)) g.morphAttributes.normal = targets.map((t: any) => floatAttribute(t.normal ?? zeroMorph(p.positions), 3));
  g.morphTargetsRelative = true;
  return g;
}

function material(data: any, p: any, textures: THREE.Texture[], kind: string): THREE.Material {
  const m = p.material == null ? null : data.materials[p.material];
  const raw = p.material == null ? null : data.rawMaterials?.[p.material];
  const pbr = raw?.pbrMetallicRoughness;
  const ext = raw?.extensions ?? m?.extensions ?? {};
  const physical = m && Object.keys(ext).some(x => /KHR_materials_(clearcoat|sheen|transmission|ior|specular|iridescence|anisotropy|dispersion|volume)|EXT_materials_bump/.test(x));
  const baseColor = pbr?.baseColorFactor ?? m?.baseColor ?? [1, 1, 1, 1];
  const common = {
    color: new THREE.Color(baseColor[0], baseColor[1], baseColor[2]),
    opacity: baseColor[3] ?? 1,
    transparent: (raw?.alphaMode ?? m?.alphaMode) === 'BLEND',
    alphaTest: (raw?.alphaMode ?? m?.alphaMode) === 'MASK' ? raw?.alphaCutoff ?? m?.alphaCutoff ?? .5 : 0,
    side: (raw?.doubleSided ?? m?.doubleSided) ? THREE.DoubleSide : THREE.FrontSide,
    vertexColors: !!p.colors,
    flatShading: !p.normals,
  };
  let out: THREE.Material;
  if (p.topology === 'Points') out = new THREE.PointsMaterial({ ...common, size: 1, sizeAttenuation: false });
  else if (p.topology.startsWith('Line')) out = new THREE.LineBasicMaterial(common);
  else if (m?.unlit) out = new THREE.MeshBasicMaterial(common);
  else {
    const emissive = raw?.emissiveFactor ?? m?.emissive ?? [0, 0, 0];
    const params = { ...common, metalness: pbr?.metallicFactor ?? m?.metallic ?? 1,
      roughness: pbr?.roughnessFactor ?? m?.roughness ?? 1,
      emissive: new THREE.Color(emissive[0], emissive[1], emissive[2]),
      emissiveIntensity: ext.KHR_materials_emissive_strength?.emissiveStrength ?? m?.emissiveStrength ?? 1 };
    out = physical ? new THREE.MeshPhysicalMaterial(params) : new THREE.MeshStandardMaterial(params);
  }
  out.name = raw?.name ?? m?.name ?? '';
  applyMetadata(out, p.material == null ? null : data.metadata?.materials?.[p.material]);
  const assign = (property: string, ref: any, srgb = false) => {
    const index = ref?.texture ?? ref?.index;
    if (index == null || !textures[index]) return;
    let texture = textures[index]!;
    const transform = ref.extensions?.KHR_texture_transform ?? ref.transform;
    if ((ref.texCoord ?? 0) > 0) {
      texture = texture.clone();
      texture.channel = ref.texCoord;
    }
    if (transform) {
      if ((transform.texCoord !== undefined && transform.texCoord !== texture.channel) || transform.offset !== undefined ||
          transform.scale !== undefined || transform.rotation !== undefined) {
        texture = texture.clone();
        if (transform.texCoord !== undefined) texture.channel = transform.texCoord;
        if (transform.offset !== undefined) texture.offset.fromArray(transform.offset);
        if (transform.scale !== undefined) texture.repeat.fromArray(transform.scale);
        if (transform.rotation !== undefined) texture.rotation = transform.rotation;
      }
    }
    if (srgb) texture.colorSpace = THREE.SRGBColorSpace;
    texture.needsUpdate = true;
    (out as any)[property] = texture;
  };
  assign('map', pbr?.baseColorTexture ?? m?.baseColorTexture, true);
  if (!(out instanceof THREE.MeshBasicMaterial) && !(out instanceof THREE.LineBasicMaterial) && !(out instanceof THREE.PointsMaterial)) {
    assign('metalnessMap', pbr?.metallicRoughnessTexture ?? m?.metallicRoughnessTexture);
    assign('roughnessMap', pbr?.metallicRoughnessTexture ?? m?.metallicRoughnessTexture);
    assign('normalMap', raw?.normalTexture ?? m?.normalTexture);
    assign('aoMap', raw?.occlusionTexture ?? m?.occlusionTexture);
    assign('emissiveMap', raw?.emissiveTexture ?? m?.emissiveTexture, true);
    const standard = out as THREE.MeshStandardMaterial;
    if (raw?.normalTexture ?? m?.normalTexture) standard.normalScale.setScalar(raw?.normalTexture?.scale ?? m?.normalScale ?? 1);
    if (raw?.occlusionTexture ?? m?.occlusionTexture) standard.aoMapIntensity = raw?.occlusionTexture?.strength ?? m?.occlusionStrength ?? 1;
    const physicalMat = out as THREE.MeshPhysicalMaterial;
    const cc = ext.KHR_materials_clearcoat;
    if (cc) {
      physicalMat.clearcoat = cc.clearcoatFactor ?? 0;
      physicalMat.clearcoatRoughness = cc.clearcoatRoughnessFactor ?? 0;
      assign('clearcoatMap', cc.clearcoatTexture);
      assign('clearcoatRoughnessMap', cc.clearcoatRoughnessTexture);
      assign('clearcoatNormalMap', cc.clearcoatNormalTexture);
      if (cc.clearcoatNormalTexture?.scale != null) physicalMat.clearcoatNormalScale.setScalar(cc.clearcoatNormalTexture.scale);
    }
    const sh = ext.KHR_materials_sheen;
    if (sh) { physicalMat.sheen = 1; physicalMat.sheenColor = new THREE.Color(...(sh.sheenColorFactor ?? [0, 0, 0])); physicalMat.sheenRoughness = sh.sheenRoughnessFactor ?? 0; assign('sheenColorMap', sh.sheenColorTexture, true); assign('sheenRoughnessMap', sh.sheenRoughnessTexture); }
    const tr = ext.KHR_materials_transmission;
    if (tr) { physicalMat.transmission = tr.transmissionFactor ?? 0; assign('transmissionMap', tr.transmissionTexture); }
    const io = ext.KHR_materials_ior; if (io) physicalMat.ior = io.ior ?? 1.5;
    const sp = ext.KHR_materials_specular;
    if (sp) { physicalMat.specularIntensity = sp.specularFactor ?? 1; physicalMat.specularColor = new THREE.Color(...(sp.specularColorFactor ?? [1, 1, 1])); assign('specularIntensityMap', sp.specularTexture); assign('specularColorMap', sp.specularColorTexture, true); }
    const ir = ext.KHR_materials_iridescence;
    if (ir) {
      physicalMat.iridescence = ir.iridescenceFactor ?? 0;
      physicalMat.iridescenceIOR = ir.iridescenceIor ?? 1.3;
      physicalMat.iridescenceThicknessRange = [ir.iridescenceThicknessMinimum ?? 100, ir.iridescenceThicknessMaximum ?? 400];
      assign('iridescenceMap', ir.iridescenceTexture);
      assign('iridescenceThicknessMap', ir.iridescenceThicknessTexture);
    }
    const vo = ext.KHR_materials_volume;
    if (vo) { physicalMat.thickness = vo.thicknessFactor ?? 0; physicalMat.attenuationDistance = vo.attenuationDistance ?? Infinity; physicalMat.attenuationColor = new THREE.Color(...(vo.attenuationColor ?? [1, 1, 1])); assign('thicknessMap', vo.thicknessTexture); }
    const an = ext.KHR_materials_anisotropy;
    if (an) { physicalMat.anisotropy = an.anisotropyStrength ?? 0; physicalMat.anisotropyRotation = an.anisotropyRotation ?? 0; assign('anisotropyMap', an.anisotropyTexture); }
    const di = ext.KHR_materials_dispersion;
    if (di) physicalMat.dispersion = di.dispersion ?? 0;
    const bu = ext.EXT_materials_bump;
    if (bu) { physicalMat.bumpScale = bu.bumpFactor ?? 1; assign('bumpMap', bu.bumpTexture); }
    // GLTFLoader clones materials for primitives without a TANGENT attribute
    // and flips both normal-map Y scales, even when a map is not assigned.
    if (!p.tangents && kind !== 'InstancedMesh') {
      standard.normalScale.y *= -1;
      if (out instanceof THREE.MeshPhysicalMaterial) physicalMat.clearcoatNormalScale.y *= -1;
    }
  }
  return out;
}

function attributeKey(attributes: Record<string, number> = {}): string {
  return Object.keys(attributes).sort().map(key => `${key}:${attributes[key]};`).join('');
}
function primitiveKey(data: any, mesh: number, primitive: number): string {
  const raw = data.rawMeshes?.[mesh]?.primitives?.[primitive];
  if (!raw) return `${mesh}:${primitive}`;
  let key = `${raw.indices}:${attributeKey(raw.attributes)}:${raw.mode}`;
  for (const target of raw.targets ?? []) key += `:${attributeKey(target)}`;
  return key;
}

function makeObject(o: any, data: any, tex: THREE.Texture[], geometries: Map<string, THREE.BufferGeometry>, materials: Map<string, THREE.Material>): THREE.Object3D {
  const primitive = o.primitive ? data.meshes[o.primitive[0]].primitives[o.primitive[1]] : null;
  const meshMetadata = o.primitive ? data.metadata?.meshes?.[o.primitive[0]] : null;
  const primitiveMetadata = o.primitive ? meshMetadata?.primitives?.[o.primitive[1]] : null;
  const geometryKey = o.primitive ? primitiveKey(data, o.primitive[0], o.primitive[1]) : '';
  let g = primitive ? geometries.get(geometryKey) : null;
  if (primitive && !g) { g = geometry(primitive, primitiveMetadata); geometries.set(geometryKey, g); }
  const materialKey = primitive ? `${primitive.material ?? 'default'}:${primitive.topology.startsWith('Line') ? 'Line' : primitive.topology === 'Points' ? 'Points' : 'Mesh'}:${!!primitive.colors}:${!!primitive.normals}:${!!primitive.tangents}:${o.kind === 'InstancedMesh'}` : '';
  let m = primitive ? materials.get(materialKey) : null;
  if (primitive && !m) { m = material(data, primitive, tex, o.kind); materials.set(materialKey, m); }
  let obj: THREE.Object3D;
  switch (o.kind) {
    case 'Bone': obj = new THREE.Bone(); break;
    case 'Mesh': obj = new THREE.Mesh(g!, m!); break;
    case 'SkinnedMesh': obj = new THREE.SkinnedMesh(g!, m!); break;
    case 'InstancedMesh': {
      const matrices = data.instances?.[o.instances] ?? [];
      const inst = new THREE.InstancedMesh(g!, m!, matrices.length);
      matrices.forEach((v: number[], i: number) => inst.setMatrixAt(i, new THREE.Matrix4().fromArray(v)));
      obj = inst; break;
    }
    case 'Points': obj = new THREE.Points(g!, m!); break;
    case 'LineSegments': obj = new THREE.LineSegments(g!, m!); break;
    case 'LineLoop': obj = new THREE.LineLoop(g!, m!); break;
    case 'LineStrip': obj = new THREE.Line(g!, m!); break;
    case 'DirectionalLight': obj = new THREE.DirectionalLight(); break;
    case 'PointLight': obj = new THREE.PointLight(); break;
    case 'SpotLight': obj = new THREE.SpotLight(); break;
    case 'PerspectiveCamera': obj = new THREE.PerspectiveCamera(); break;
    case 'OrthographicCamera': obj = new THREE.OrthographicCamera(); break;
    case 'Group': obj = new THREE.Group(); break;
    default: obj = new THREE.Object3D();
  }
  obj.name = o.name;
  if (o.primitive) {
    applyMetadata(obj, meshMetadata?.meta);
    applyMetadata(obj, primitiveMetadata);
  }
  if (o.camera != null) applyMetadata(obj, data.metadata?.cameras?.[o.camera]);
  if (o.light != null) applyMetadata(obj, data.metadata?.lights?.[o.light]);
  if (o.node != null) {
    const sourceNode = data.metadata?.nodes?.[o.node];
    const meshIndex = data.nodeMeshes?.[o.node];
    if (meshIndex != null && !o.primitive) applyMetadata(obj, data.metadata?.meshes?.[meshIndex]?.meta);
    applyMetadata(obj, sourceNode, true);
  }
  if (o.trs) {
    obj.position.fromArray(o.trs.translation);
    obj.quaternion.fromArray(o.trs.rotation);
    obj.scale.fromArray(o.trs.scale);
    obj.updateMatrix();
  } else {
    obj.matrix.fromArray(o.matrix);
    obj.matrix.decompose(obj.position, obj.quaternion, obj.scale);
  }
  if (o.light != null) {
    const light = data.lights[o.light];
    const l = obj as THREE.Light;
    l.color.setRGB(...(light.color as [number, number, number]));
    l.intensity = light.intensity;
    if (obj instanceof THREE.PointLight || obj instanceof THREE.SpotLight) obj.distance = light.range ?? 0;
    if (obj instanceof THREE.SpotLight) {
      obj.angle = light.spot[1];
      obj.penumbra = 1 - light.spot[0] / light.spot[1];
    }
  }
  if (o.camera != null) {
    const camera = data.cameras[o.camera];
    if (obj instanceof THREE.PerspectiveCamera) {
      obj.fov = THREE.MathUtils.radToDeg(camera.yfov);
      obj.aspect = camera.aspect ?? 1;
      obj.near = camera.znear;
      obj.far = camera.zfar ?? Infinity;
      obj.updateProjectionMatrix();
    } else if (obj instanceof THREE.OrthographicCamera) {
      obj.left = -camera.xmag; obj.right = camera.xmag;
      obj.top = camera.ymag; obj.bottom = -camera.ymag;
      obj.near = camera.znear; obj.far = camera.zfar;
      obj.updateProjectionMatrix();
    }
  }
  if (g && (g.morphAttributes.position?.length || g.morphAttributes.normal?.length) && 'morphTargetInfluences' in obj) {
    const influences = (obj as THREE.Mesh).morphTargetInfluences;
    const mesh = data.meshes[o.primitive[0]];
    const targetNames = data.metadata?.meshes?.[o.primitive[0]]?.meta?.extras?.targetNames;
    if (Array.isArray(targetNames) && influences?.length === targetNames.length)
      (obj as THREE.Mesh).morphTargetDictionary = Object.fromEntries(targetNames.map((name: string, index: number) => [name, index]));
    const weights = o.node == null ? null : data.nodeWeights?.[o.node];
    (weights ?? mesh.weights).forEach((w: number, i: number) => { if (influences) influences[i] = w; });
  }
  return obj;
}

class GltfCubicSplineInterpolant extends THREE.Interpolant {
  copySampleValue_(index: number): any {
    const stride = this.valueSize, offset = index * stride * 3 + stride;
    for (let i = 0; i < stride; i++) this.resultBuffer[i] = this.sampleValues[offset + i];
    return this.resultBuffer;
  }
  interpolate_(i1: number, t0: number, t: number, t1: number): any {
    const stride = this.valueSize, values = this.sampleValues, result = this.resultBuffer;
    const dt = t1 - t0, p = (t - t0) / dt, p2 = p * p, p3 = p2 * p;
    const s2 = -2 * p3 + 3 * p2, s3 = p3 - p2;
    const s0 = 1 - s2, s1 = s3 - p2 + p;
    const a = (i1 - 1) * stride * 3, b = i1 * stride * 3;
    for (let i = 0; i < stride; i++) result[i] =
      s0 * values[a + stride + i] + s1 * values[a + 2 * stride + i] * dt +
      s2 * values[b + stride + i] + s3 * values[b + i] * dt;
    return result;
  }
}
class GltfCubicSplineQuaternionInterpolant extends GltfCubicSplineInterpolant {
  override interpolate_(i1: number, t0: number, t: number, t1: number): any {
    const result = super.interpolate_(i1, t0, t, t1);
    new THREE.Quaternion().fromArray(result).normalize().toArray(result);
    return result;
  }
}

function animations(data: any, objects: THREE.Object3D[]): THREE.AnimationClip[] {
  return data.clips.map((clip: any, clipIndex: number) => {
    const tracks: THREE.KeyframeTrack[] = [];
    for (const ch of clip.channels) {
      const index = data.nodeObjects[ch.node];
      if (index == null) continue;
      const targets: THREE.Object3D[] = [];
      if (ch.path === 'weights') objects[index]!.traverse(o => { if ((o as THREE.Mesh).morphTargetInfluences) targets.push(o); });
      else targets.push(objects[index]!);
      for (const target of targets) {
        const prop = ({translation:'position',rotation:'quaternion',scale:'scale',weights:'morphTargetInfluences'} as Record<string,string>)[ch.path];
        const name = `${target.name || target.uuid}.${prop}`;
        const Track = ch.path === 'rotation' ? THREE.QuaternionKeyframeTrack : THREE.NumberKeyframeTrack;
        const Klass = ch.path === 'translation' || ch.path === 'scale' || (ch.path === 'weights' && ch.size > 1)
          ? THREE.VectorKeyframeTrack : Track;
        // Cubic-spline keys store [in tangent, value, out tangent]. The custom
        // interpolant below retains all three parts and Hermite timing.
        const track = new Klass(name, ch.times, ch.values);
        if (ch.interpolation === 'STEP') track.setInterpolation(THREE.InterpolateDiscrete);
        if (ch.interpolation === 'CUBICSPLINE') {
          (track as any).createInterpolant = function(result: any) {
            const Interpolant = ch.path === 'rotation' ? GltfCubicSplineQuaternionInterpolant : GltfCubicSplineInterpolant;
            return new Interpolant(this.times, this.values, this.getValueSize() / 3, result);
          };
          (track as any).createInterpolant.isInterpolantFactoryMethodGLTFCubicSpline = true;
        }
        tracks.push(track);
      }
    }
    const result = new THREE.AnimationClip(clip.name || `animation_${clipIndex}`, clip.duration, tracks);
    applyMetadata(result, data.metadata?.animations?.[clipIndex]);
    return result;
  });
}

export async function object3dFromEngine(data: any, source?: Uint8Array): Promise<{object3d: THREE.Group; animations: THREE.AnimationClip[]; vertices: number; triangles: number}> {
  if (source) {
    const raw = sourceJson(source);
    data = { ...data, rawMaterials: raw.materials ?? [], rawTextures: raw.textures ?? [], rawImages: raw.images ?? [], rawMeshes: raw.meshes ?? [] };
  }
  const tex = await texturesOf(data);
  const geometries = new Map<string, THREE.BufferGeometry>();
  const materials = new Map<string, THREE.Material>();
  const objects: THREE.Object3D[] = data.objects.map((o: any) => makeObject(o, data, tex, geometries, materials));
  data.objects.forEach((o: any, i: number) => { if (o.parent != null) objects[o.parent]!.add(objects[i]!); });
  data.nodeWeights?.forEach((weights: number[] | null, node: number) => {
    const root = objects[data.nodeObjects[node]];
    if (!root || !weights) return;
    root.traverse(o => {
      const influences = (o as THREE.Mesh).morphTargetInfluences;
      weights.forEach((w, i) => { if (influences && i < influences.length) influences[i] = w; });
    });
  });
  const scene = objects[0] as THREE.Group;
  applyMetadata(scene, data.metadata?.scene);
  scene.updateMatrixWorld(true);
  data.objects.forEach((o: any, i: number) => {
    if (o.skin == null || !(objects[i] instanceof THREE.SkinnedMesh)) return;
    const skin = data.skins[o.skin];
    const bones = skin.joints.map((node: number) => objects[data.nodeObjects[node]] as THREE.Bone);
    const inverses = skin.inverseBind.map((m: number[]) => new THREE.Matrix4().fromArray(m));
    const mesh = objects[i] as THREE.SkinnedMesh;
    mesh.bind(new THREE.Skeleton(bones, inverses), new THREE.Matrix4());
    // GLTFLoader has updated the mesh's world transform when it binds the
    // skeleton. Box3 uses the attached mesh's inverse before a render tick.
    mesh.bindMatrixInverse.copy(mesh.matrixWorld).invert();
  });
  // Today's loadGltf normalizes the loaded Three graph with Box3.setFromObject.
  // Run that same measurement over geometry supplied by the engine: morph and
  // skin bounds can differ from an accessor-only box by a few percent.
  const bounds = new THREE.Box3().setFromObject(scene);
  const center = data.skins.length
    ? new THREE.Vector3(...data.facts.center)
    : bounds.getCenter(new THREE.Vector3());
  const radius = data.skins.length
    ? data.facts.radius
    : bounds.getSize(new THREE.Vector3()).length() / 2;
  if (!Number.isFinite(radius) || radius < 1e-10) throw new Error('GLB model has no measurable size');
  const normalization = new THREE.Group();
  normalization.scale.setScalar(1 / radius);
  scene.position.sub(center);
  normalization.add(scene);
  const root = new THREE.Group(); root.add(normalization);
  return { object3d: root, animations: animations(data, objects), vertices: data.facts.vertices, triangles: data.facts.triangles };
}
