import * as THREE from 'three';
import { describe, expect, it } from 'vitest';
import { buildBvh } from './bvh';
import { extractScene } from './extract';

const env = { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 };

function leafTriangles(nodes: ArrayBuffer): number {
  const u = new Uint32Array(nodes);
  let total = 0;
  const stack = [0];
  while (stack.length) {
    const w = stack.pop()!;
    for (const side of [0, 1]) {
      const index = u[w * 16 + side * 8 + 3]!, count = u[w * 16 + side * 8 + 7]!;
      if (count === 0xffffffff) continue;
      if (count === 0) stack.push(index); else total += count;
    }
  }
  return total;
}

describe('Studio BVH', () => {
  it('references every triangle exactly once', () => {
    const n = 4000, corners = new Float32Array(n * 9);
    let s = 7;
    const r = () => ((s = (s * 1103515245 + 12345) >>> 0) / 2 ** 32);
    for (let t = 0; t < n; t++) { const c = [r() * 50, r() * 20, r() * 30]; for (let k = 0; k < 3; k++) for (let a = 0; a < 3; a++) corners[t * 9 + k * 3 + a] = c[a]! + r(); }
    const { nodes, order } = buildBvh(corners);
    expect(leafTriangles(nodes)).toBe(n);
    expect(new Set(order).size).toBe(n);
  });
  it('builds a single leaf for a tiny scene', () => {
    const { nodes } = buildBvh(new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]));
    expect(nodes.byteLength).toBe(64);
    expect(new Uint32Array(nodes)[15]).toBe(0xffffffff);
  });
});

describe('Studio scene extraction', () => {
  it('bakes meshes to world space with their materials, maps and object ids', () => {
    const scene = new THREE.Scene();
    const box = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 2), new THREE.MeshStandardMaterial({ color: '#ff0000', metalness: 1, roughness: 0.25 }));
    box.position.set(10, 0, 0); box.userData.sceneId = 'box';
    const hidden = new THREE.Mesh(new THREE.SphereGeometry(1), new THREE.MeshStandardMaterial()); hidden.userData.sceneId = 'other';
    scene.add(box, hidden);
    const s = extractScene(scene, id => id === 'box', env);
    expect(s.triangles).toBe(12);
    expect(s.objects).toEqual(['box']);
    expect(s.bounds.min.x).toBeCloseTo(9);
    expect(s.bounds.max.x).toBeCloseTo(11);
    // base rgb (linear red), metallic, then roughness.
    expect(s.materials[0]).toBeCloseTo(1);
    expect(s.materials[3]).toBe(1);
    expect(s.materials[4]).toBeCloseTo(0.25);
    // No maps: -1 layers.
    expect([...s.materials.slice(16, 20)]).toEqual([-1, -1, -1, -1]);
    expect([...s.materials.slice(20, 24)]).toEqual([0, 0, 0, 0]);
    expect(s.materials.length).toBe(24);
  });

  it('turns three lights into Studio lights', () => {
    const scene = new THREE.Scene();
    const sun = new THREE.DirectionalLight(0xffffff, 2); sun.position.set(0, 10, 0); sun.target.position.set(0, 0, 0);
    const spot = new THREE.SpotLight(0xffffff, 5, 0, Math.PI / 6, 0.5); spot.position.set(0, 4, 0); spot.target.position.set(0, 0, 0);
    scene.add(sun, sun.target, spot, spot.target, new THREE.HemisphereLight());
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.MeshStandardMaterial()); mesh.userData.sceneId = 'p'; scene.add(mesh);
    const s = extractScene(scene, () => true, env);
    expect(s.lightCount).toBe(2);
    // Sun: direction toward the light, kind 3, intensity as irradiance.
    expect([...s.sdata.slice(0, 4)].map(v => Math.round(v * 1000) / 1000)).toEqual([0, 1, 0, 3]);
    // Irradiance 2 spread over the 1.5° sun disc's solid angle.
    expect(s.sdata[12]).toBeCloseTo(2 / (2 * Math.PI * (1 - Math.cos(THREE.MathUtils.degToRad(1.5)))), 0);
    // Spot: kind 2, axis stored pointing back at the light, cos outer < cos inner.
    expect(s.sdata[16 + 3]).toBe(2);
    expect(s.sdata[16 + 9]).toBeCloseTo(1);
    expect(s.sdata[16 + 11]).toBeCloseTo(Math.cos(Math.PI / 6));
    expect(s.sdata[16 + 15]).toBeGreaterThan(s.sdata[16 + 11]!);
  });

  it('keeps each mesh BVH while models move, and stores world-to-model transforms', () => {
    const scene = new THREE.Scene();
    const box = new THREE.Mesh(new THREE.BoxGeometry(1, 1, 1), new THREE.MeshStandardMaterial()); box.userData.sceneId = 'box';
    scene.add(box);
    const first = extractScene(scene, () => true, env);
    box.position.set(5, 2, -3); box.rotation.y = 0.7;
    const moved = extractScene(scene, () => true, env);
    expect(moved.geometryKey).toBe(first.geometryKey);
    expect(moved.positions).toBe(first.positions);        // the same cached geometry
    expect(moved.instanceCount).toBe(1);
    expect(moved.bounds.getCenter(new THREE.Vector3()).x).toBeCloseTo(5);
    // Instance 0 (after the lights, none here): world→object rows map the box centre to its origin.
    const o = moved.lightCount * 16, row = (r: number) => [...moved.sdata.slice(o + r * 4, o + r * 4 + 4)];
    const world = new THREE.Vector3(5, 2, -3);
    const local = [0, 1, 2].map(r => { const [a, b, c, d] = row(r); return a! * world.x + b! * world.y + c! * world.z + d!; });
    for (const v of local) expect(v).toBeCloseTo(0, 5);
    // n0.w holds the instance's BVH root, just after the top level.
    expect(moved.sdata[o + 15]).toBe(moved.tlasNodes.byteLength / 64);
    // Editing the geometry makes a new key.
    box.geometry.getAttribute('position').needsUpdate = true;
    expect(extractScene(scene, () => true, env).geometryKey).not.toBe(first.geometryKey);
  });

  it('skips hidden and fully transparent objects', () => {
    const scene = new THREE.Scene();
    const a = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial()); a.userData.sceneId = 'a'; a.visible = false;
    const b = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial({ transparent: true, opacity: 0 })); b.userData.sceneId = 'b';
    scene.add(a, b);
    expect(extractScene(scene, () => true, env).triangles).toBe(0);
  });
});
