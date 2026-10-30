import * as THREE from 'three';
import { disposeModel } from './import-model';

/*
 * CAD import: STEP, IGES and BREP become a GLB (parts, names and colors
 * kept) that then imports like any other model. OpenCascade tessellates in a
 * worker; tolerances suit close-up product shots.
 */

export const CAD_FILE = /\.(step|stp|iges|igs|brep)$/i;

/** Surface tolerance as a fraction of the model's size, and the largest angle between facets (radians). */
const TESSELLATION = { linearUnit: 'millimeter', linearDeflectionType: 'bounding_box_ratio', linearDeflection: 0.0006, angularDeflection: 0.25 };

type CadMesh = { name: string; color: number[] | null; faces: { first: number; last: number; color: number[] | null }[]; position: Float32Array; normal: Float32Array | null; index: Uint32Array };
type CadNode = { name: string; meshes: number[]; children: CadNode[] };

function tessellate(format: 'step' | 'iges' | 'brep', bytes: Uint8Array): Promise<{ root: CadNode; meshes: CadMesh[] }> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./cad.worker.ts', import.meta.url), { type: 'module' });
    worker.onmessage = (e: MessageEvent) => {
      worker.terminate();
      if (e.data.ok) resolve(e.data); else reject(new Error(e.data.message));
    };
    worker.onerror = (e) => { worker.terminate(); reject(new Error(e.message || 'The CAD reader failed')); };
    worker.postMessage({ format, bytes, params: TESSELLATION }, [bytes.buffer]);
  });
}

const srgb = (c: number[] | null | undefined) => (c ? new THREE.Color().setRGB(c[0]!, c[1]!, c[2]!, THREE.SRGBColorSpace) : new THREE.Color(0.75, 0.76, 0.78));
const key = (c: number[] | null | undefined) => (c ? c.map(v => v.toFixed(3)).join(',') : '-');

/** One part: faces that carry their own color become material groups. */
function part(m: CadMesh): THREE.Mesh {
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(m.position, 3));
  if (m.normal) g.setAttribute('normal', new THREE.BufferAttribute(m.normal, 3));
  g.setIndex(new THREE.BufferAttribute(m.index, 1));
  if (!m.normal) g.computeVertexNormals();
  const materials: THREE.MeshStandardMaterial[] = [];
  const byColor = new Map<string, number>();
  const material = (c: number[] | null) => {
    const k = key(c);
    let i = byColor.get(k);
    if (i === undefined) {
      i = materials.length;
      byColor.set(k, i);
      // A neutral satin finish: Studio's finishes replace it.
      materials.push(new THREE.MeshStandardMaterial({ color: srgb(c), roughness: 0.45, metalness: 0 }));
    }
    return i;
  };
  const faces = m.faces.filter(f => f.color);
  if (!faces.length) return new THREE.Mesh(g, materials[material(m.color)] ?? new THREE.MeshStandardMaterial());
  // brep_faces: triangle ranges; consecutive faces of one color share a group.
  let start = 0, current = -1;
  const triangles = m.index.length / 3;
  const ranges: { first: number; last: number; mat: number }[] = [];
  for (const f of m.faces) ranges.push({ first: f.first, last: f.last, mat: material(f.color ?? m.color) });
  ranges.sort((a, b) => a.first - b.first);
  for (const r of ranges) {
    if (r.mat !== current) {
      if (current >= 0 && r.first > start) g.addGroup(start * 3, (r.first - start) * 3, current);
      start = r.first; current = r.mat;
    }
  }
  if (current >= 0) g.addGroup(start * 3, (triangles - start) * 3, current);
  return new THREE.Mesh(g, materials);
}

/** Converts a STEP, IGES or BREP file to a GLB with the same parts. */
export async function cadToGlb(file: File): Promise<File> {
  const ext = file.name.toLowerCase().split('.').pop()!;
  const format = ext === 'brep' ? 'brep' : ext === 'igs' || ext === 'iges' ? 'iges' : 'step';
  const { root, meshes } = await tessellate(format, new Uint8Array(await file.arrayBuffer()));
  if (!meshes.length) throw new Error('The CAD file has no solid geometry');
  const group = new THREE.Group();
  const build = (node: CadNode, parent: THREE.Object3D) => {
    const o = new THREE.Group();
    o.name = node.name || '';
    for (const i of node.meshes) {
      const mesh = part(meshes[i]!);
      mesh.name = meshes[i]!.name || node.name || `part${i}`;
      o.add(mesh);
    }
    for (const child of node.children) build(child, o);
    parent.add(o);
  };
  build(root, group);
  // CAD is Z-up in millimetres; Powermove is Y-up (scale is normalized on import).
  group.rotation.x = -Math.PI / 2;
  try {
    const { GLTFExporter } = await import('three/addons/exporters/GLTFExporter.js');
    const data = await new GLTFExporter().parseAsync(group, { binary: true, onlyVisible: false });
    if (!(data instanceof ArrayBuffer)) throw new Error('Could not convert the CAD model');
    return new File([data], file.name.replace(CAD_FILE, '.glb'), { type: 'model/gltf-binary' });
  } finally {
    disposeModel(group);
  }
}
