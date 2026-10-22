import * as THREE from 'three';

export const materialKinds = ['Basic', 'Lambert', 'Phong', 'Standard', 'Physical'] as const;
export type Kind = typeof materialKinds[number];
export type Flags = { skinning: boolean; morphNormals: boolean; envMap: boolean; flatShading: boolean; normalMap: boolean };
export type Proof = { kind: Kind; flags: Flags; short: boolean; normalMapType: 'tangent' | 'object';
  tangents: boolean; activeAttributes: string[]; defines: string[]; rgba: number[]; error: number;
  exception: string | null; covered: number; png: string };

export function captureNormalMatrix(): Record<string, Proof> {
  const renderer = new THREE.WebGLRenderer({ antialias: false, preserveDrawingBuffer: true });
  renderer.setSize(32, 32, false);
  renderer.setClearColor(0, 0);
  const gl = renderer.getContext();
  const scene = new THREE.Scene();
  scene.add(new THREE.AmbientLight(0xffffff, 1));
  const light = new THREE.DirectionalLight(0xffffff, 1); light.position.z = 3; scene.add(light);
  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, .1, 10); camera.position.z = 3;
  const env = new THREE.DataTexture(new Uint8Array(8 * 4 * 4).fill(255), 8, 4, THREE.RGBAFormat);
  env.mapping = THREE.EquirectangularReflectionMapping; env.needsUpdate = true;
  const normalMap = new THREE.DataTexture(new Uint8Array([128, 128, 255, 255]), 1, 1, THREE.RGBAFormat);
  normalMap.needsUpdate = true;
  const out: Record<string, Proof> = {};
  try {
    for (const kind of materialKinds) for (let mask = 0; mask < 32; mask++) {
      const flags: Flags = { skinning: !!(mask & 1), morphNormals: !!(mask & 2),
        envMap: !!(mask & 4), flatShading: !!(mask & 8), normalMap: !!(mask & 16) };
      // Cover normal-map object space and both presence/absence of tangents,
      // in addition to all five requested boolean define combinations.
      const variants = flags.normalMap ? ['tangent', 'tangent-basis', 'object'] as const : ['tangent'] as const;
      for (const variant of variants) for (const short of [false, true]) {
        const geometry = new THREE.BufferGeometry();
        geometry.setAttribute('position', new THREE.Float32BufferAttribute([-1, -1, 0, 1, -1, 0, 0, 1, 0], 3));
        geometry.setIndex([0, 1, 2]);
        geometry.setAttribute('normal', new THREE.Float32BufferAttribute([0, 0, 1, 0, 0, 1, ...(short ? [] : [0, 0, 1])], 3));
        geometry.setAttribute('uv', new THREE.Float32BufferAttribute([0, 0, 1, 0, .5, 1], 2));
        if (variant === 'tangent-basis') geometry.setAttribute('tangent',
          new THREE.Float32BufferAttribute([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1], 4));
        if (flags.skinning) {
          geometry.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(new Array(12).fill(0), 4));
          geometry.setAttribute('skinWeight', new THREE.Float32BufferAttribute([1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0], 4));
        }
        if (flags.morphNormals) {
          // POSITION accompanies NORMAL so r180's morph packer is valid.
          geometry.morphAttributes.position = [new THREE.Float32BufferAttribute(new Array(9).fill(0), 3)];
          geometry.morphAttributes.normal = [new THREE.Float32BufferAttribute(new Array(9).fill(0), 3)];
          geometry.morphTargetsRelative = true;
        }
        const constructors = { Basic: THREE.MeshBasicMaterial, Lambert: THREE.MeshLambertMaterial,
          Phong: THREE.MeshPhongMaterial, Standard: THREE.MeshStandardMaterial, Physical: THREE.MeshPhysicalMaterial };
        const material = new constructors[kind]({ color: 0xff0000, side: THREE.DoubleSide });
        // WebGLPrograms reads these fields, including defines a material's
        // public constructor doesn't expose (Basic normalMap/flatShading).
        Object.assign(material, { flatShading: flags.flatShading,
          envMap: flags.envMap ? env : null, normalMap: flags.normalMap ? normalMap : null,
          normalMapType: variant === 'object' ? THREE.ObjectSpaceNormalMap : THREE.TangentSpaceNormalMap });
        let mesh: THREE.Mesh;
        if (flags.skinning) {
          const bone = new THREE.Bone(); const skin = new THREE.SkinnedMesh(geometry, material);
          skin.add(bone); skin.bind(new THREE.Skeleton([bone])); mesh = skin;
        } else mesh = new THREE.Mesh(geometry, material);
        if (flags.morphNormals) mesh.morphTargetInfluences![0] = .25;
        scene.add(mesh);
        let exception: string | null = null;
        try {
          while (gl.getError()) { /* drain previous draw only */ }
          renderer.render(scene, camera);
        } catch (error) { exception = String(error); }
        const error = gl.getError();
        const program = (renderer.properties.get(material) as any).currentProgram;
        const attributes = program?.getAttributes?.() ?? {};
        const source = program?.vertexShader ? gl.getShaderSource(program.vertexShader) ?? '' : '';
        const rgba = new Uint8Array(4); gl.readPixels(16, 16, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
        const pixels = new Uint8Array(32 * 32 * 4); gl.readPixels(0, 0, 32, 32, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
        let covered = 0; for (let i = 3; i < pixels.length; i += 4) if (pixels[i]) covered++;
        const key = `${kind}-${mask.toString().padStart(2, '0')}-${variant}-${short ? 'short' : 'full'}`;
        out[key] = { kind, flags, short, normalMapType: variant === 'object' ? 'object' : 'tangent',
          tangents: variant === 'tangent-basis', activeAttributes: Object.keys(attributes).sort(),
          defines: source.split('\n').filter(line => /^#define (USE_|FLAT_SHADED|STANDARD|PHYSICAL)/.test(line)),
          rgba: [...rgba], error, exception, covered, png: renderer.domElement.toDataURL('image/png') };
        scene.remove(mesh); geometry.dispose(); material.dispose();
        if (mesh instanceof THREE.SkinnedMesh) mesh.skeleton.dispose();
      }
    }
  } finally {
    normalMap.dispose(); env.dispose(); renderer.dispose(); renderer.forceContextLoss();
  }
  return out;
}
(window as any).__shortNormalMatrix = captureNormalMatrix;
