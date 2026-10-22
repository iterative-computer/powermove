import * as THREE from 'three';

type Case = 'uvUnused' | 'uvUsed' | 'uvUsedIndexedTail' | 'uv1Unused' | 'uv1Used' |
  'uv2Unused' | 'uv2Used' | 'uv3Unused' | 'uv3Used' |
  'uvDepthNoMap' | 'uvDepthMapOpaque' | 'uvDepthMapAlphaTest' |
  'uvDistanceMapOpaque' | 'normalDistance' | 'colorDistance' |
  'normalUnlit' | 'normalLit' | 'normalLitNoLights' | 'normalLitAmbientOnly' | 'normalOverride' |
  'normalDepth' | 'colorDepth' |
  'tangentUnused' | 'tangentUsed' | 'shortMorph' | 'badIndex' |
  'normalOnlyMorph' | 'colorOnlyMorph' |
  'basicSkinnedNormal' | 'basicSkinnedShortNormal' | 'basicEnvShortNormal' |
  'standardFlatShortNormal' | 'standardSkinnedFlatShortNormal' |
  'maskOverrideShortNormal' | 'maskOverrideSkinnedShortNormal' | 'maskOverrideSkinnedNormal';

function run(): Record<Case, { rgb: number[]; error: number; exception: string | null;
  activeAttributes: string[]; png: string }> {
  const canvas = document.createElement('canvas');
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true });
  renderer.setSize(32, 32, false);
  renderer.setClearColor(0x000000, 1);
  const gl = renderer.getContext();
  const scene = new THREE.Scene();
  const ambient = new THREE.AmbientLight(0xffffff, 1);
  scene.add(ambient);
  const directional = new THREE.DirectionalLight(0xffffff, 1);
  directional.position.set(0, 0, 3);
  scene.add(directional);
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, .1, 10);
  camera.position.z = 3;
  camera.lookAt(0, 0, 0);
  const tex = (bytes: number[]) => {
    const t = new THREE.DataTexture(new Uint8Array(bytes), 1, 1, THREE.RGBAFormat);
    t.needsUpdate = true;
    return t;
  };
  const white = tex([255, 255, 255, 255]);
  white.colorSpace = THREE.SRGBColorSpace;
  const flatNormal = tex([128, 128, 255, 255]);
  const out = {} as Record<Case, { rgb: number[]; error: number; exception: string | null;
    activeAttributes: string[]; png: string }>;
  const cases: Case[] = ['uvUnused', 'uvUsed', 'uvUsedIndexedTail', 'uv1Unused', 'uv1Used',
    'uv2Unused', 'uv2Used', 'uv3Unused', 'uv3Used',
    'uvDepthNoMap', 'uvDepthMapOpaque', 'uvDepthMapAlphaTest',
    'uvDistanceMapOpaque', 'normalDistance', 'colorDistance',
    'normalUnlit', 'normalLit',
    'normalLitNoLights', 'normalLitAmbientOnly', 'normalOverride',
    'normalDepth', 'colorDepth',
    'tangentUnused', 'tangentUsed', 'shortMorph', 'badIndex', 'normalOnlyMorph', 'colorOnlyMorph',
    'basicSkinnedNormal', 'basicSkinnedShortNormal', 'basicEnvShortNormal',
    'standardFlatShortNormal', 'standardSkinnedFlatShortNormal',
    'maskOverrideShortNormal', 'maskOverrideSkinnedShortNormal', 'maskOverrideSkinnedNormal'];
  // An equirect env map: WebGLCubeMaps converts it, and the Basic program
  // gets USE_ENVMAP.
  const env = new THREE.DataTexture(new Uint8Array(8 * 4 * 4).fill(255), 8, 4, THREE.RGBAFormat);
  env.needsUpdate = true;
  env.mapping = THREE.EquirectangularReflectionMapping;
  // overlay.ts's outline mask override (MeshBasicMaterial, DoubleSide).
  const mask = new THREE.MeshBasicMaterial({ color: 0xff0000, side: THREE.DoubleSide });
  for (const name of cases) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute('position', new THREE.Float32BufferAttribute(
      name === 'uvUsedIndexedTail'
        ? [-1, -1, 0, 1, -1, 0, 0, 1, 0, 2, 2, 0]
        : [-1, -1, 0, 1, -1, 0, 0, 1, 0], 3));
    geometry.setIndex(name === 'badIndex' ? [0, 1, 7] : [0, 1, 2]);
    if (name.startsWith('uv')) {
      const channel = Number(name.match(/^uv([123])/)?.[1] ?? 0);
      geometry.setAttribute(channel ? `uv${channel}` : 'uv',
        new THREE.Float32BufferAttribute(name === 'uvUsedIndexedTail'
          ? [0, 0, 1, 0, .5, 1] : [0, 0, 1, 0], 2));
    }
    const skinned = name.includes('Skinned');
    const fullNormal = name === 'basicSkinnedNormal' || name === 'maskOverrideSkinnedNormal';
    if (name.startsWith('normal') || name.includes('Normal')) geometry.setAttribute('normal',
      new THREE.Float32BufferAttribute(fullNormal ? [0, 0, 1, 0, 0, 1, 0, 0, 1] : [0, 0, 1, 0, 0, 1], 3));
    if (skinned) {
      geometry.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(new Array(12).fill(0), 4));
      geometry.setAttribute('skinWeight', new THREE.Float32BufferAttribute([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0], 4));
    }
    if (name === 'colorDepth' || name === 'colorDistance') geometry.setAttribute('color',
      new THREE.Float32BufferAttribute([1, 0, 0, 1, 0, 0], 3));
    if (name.startsWith('tangent')) {
      geometry.setAttribute('normal', new THREE.Float32BufferAttribute([
        0, 0, 1, 0, 0, 1, 0, 0, 1,
      ], 3));
      geometry.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, .5, 1], 2));
      geometry.setAttribute('tangent', new THREE.Float32BufferAttribute([
        1, 0, 0, 1, 1, 0, 0, 1,
      ], 4));
    }
    if (name === 'shortMorph') {
      geometry.morphAttributes.position = [new THREE.Float32BufferAttribute([
        0, 0, 0, 0, 0, 0,
      ], 3)];
      geometry.morphTargetsRelative = true;
    }
    if (name === 'normalOnlyMorph' || name === 'colorOnlyMorph') {
      geometry.morphAttributes[name === 'normalOnlyMorph' ? 'normal' : 'color'] = [
        new THREE.Float32BufferAttribute([0, 0, 0, 0, 0, 0, 0, 0, 0], 3),
      ];
      geometry.morphTargetsRelative = true;
    }
    const material = name.includes('Distance')
      ? new THREE.MeshDistanceMaterial({ side: THREE.DoubleSide,
        map: name === 'uvDistanceMapOpaque' ? white : null,
        vertexColors: name === 'colorDistance' })
      : name.startsWith('uvDepth') || name === 'normalDepth' || name === 'colorDepth'
      ? new THREE.MeshDepthMaterial({ side: THREE.DoubleSide, depthPacking: THREE.RGBADepthPacking,
        map: name === 'uvDepthMapOpaque' || name === 'uvDepthMapAlphaTest' ? white : null,
        alphaTest: name === 'uvDepthMapAlphaTest' ? .5 : 0,
        vertexColors: name === 'colorDepth' })
      : name.startsWith('normalLit') || name.startsWith('tangent') || name.startsWith('standard') ||
        name.startsWith('maskOverride')
      ? new THREE.MeshStandardMaterial({ color: 0xff0000, side: THREE.DoubleSide,
        flatShading: name.startsWith('standard'),
        normalMap: name === 'tangentUsed' ? flatNormal : null })
      : new THREE.MeshBasicMaterial({ color: 0xff0000, side: THREE.DoubleSide,
        envMap: name === 'basicEnvShortNormal' ? env : null,
        map: name.startsWith('uv') && (name.endsWith('Used') || name === 'uvUsedIndexedTail') ? white : null });
    if (name.startsWith('uv')) white.channel = Number(name.match(/^uv([123])/)?.[1] ?? 0);
    if (material instanceof THREE.MeshDistanceMaterial) {
      // WebGLShadowMap normally installs this per-material light before
      // WebGLMaterials refreshes the distance uniforms.
      const light = new THREE.PointLight();
      light.position.z = 3;
      light.shadow.camera.near = .1;
      light.shadow.camera.far = 10;
      light.updateMatrixWorld();
      (renderer.properties.get(material) as any).light = light;
    }
    let mesh: THREE.Mesh;
    if (skinned) {
      const bone = new THREE.Bone();
      const skin = new THREE.SkinnedMesh(geometry, material);
      skin.add(bone);
      skin.bind(new THREE.Skeleton([bone]));
      mesh = skin;
    } else {
      mesh = new THREE.Mesh(geometry, material);
    }
    scene.overrideMaterial = name.startsWith('maskOverride') ? mask : null;
    if (name === 'normalOverride') mesh.material = new THREE.MeshStandardMaterial({
      color: 0xff0000, side: THREE.DoubleSide,
    });
    ambient.visible = name !== 'normalLitNoLights';
    directional.visible = name !== 'normalLitNoLights' && name !== 'normalLitAmbientOnly';
    scene.add(mesh);
    while (gl.getError()) {};
    let exception: string | null = null;
    try {
      renderer.render(scene, camera);
    } catch (error) {
      exception = String(error);
    }
    const error = gl.getError();
    const active = (renderer.properties.get(scene.overrideMaterial ?? mesh.material) as any)
      .currentProgram?.getAttributes?.();
    const pixel = new Uint8Array(4);
    gl.readPixels(16, 16, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    out[name] = { rgb: [...pixel.slice(0, 3)], error, exception,
      activeAttributes: Object.keys(active ?? {}).sort(),
      png: canvas.toDataURL('image/png') };
    scene.remove(mesh);
    geometry.dispose();
    if (mesh.material !== material) mesh.material.dispose();
    material.dispose();
  }
  scene.overrideMaterial = null;
  white.dispose(); flatNormal.dispose(); env.dispose(); mask.dispose(); renderer.dispose();
  return out;
}

(window as any).__gltfShortAttributeProof = run;
