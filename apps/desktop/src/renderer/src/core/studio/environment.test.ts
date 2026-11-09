import { describe, it, expect } from 'vitest';
import { packWorld, worldSun, bakeSky } from './environment';
describe('Studio environments', () => {
  it('normalizes an HDR environment including its polar texels', () => {
    const p = packWorld(
      {
        hdri: {
          width: 8,
          height: 4,
          pixels: Array.from({ length: 32 }, (_, i) => ({ x: i === 0 ? 10000 : 1, y: 1, z: 1 })),
        },
      },
      [],
    );
    let integral = 0;
    for (let i = 0; i < 32; i++) {
      const y = Math.floor(i / 8),
        solid = ((2 * Math.PI) / 8) * (Math.cos((Math.PI * y) / 4) - Math.cos((Math.PI * (y + 1)) / 4));
      integral += p[(6 + 32 + i) * 4 + 1]! * solid;
    }
    expect(integral).toBeCloseTo(1, 6);
    expect(p[(6 + 63) * 4]).toBe(1);
  });
  it('keeps the solar disc small and night sun disabled', () => {
    const sun = worldSun({ sky: {} });
    expect(sun[4]).toBeGreaterThan(0.9999);
    expect(worldSun({ sky: { elevation: -5 } })).toEqual([]);
    const sky = bakeSky({}, 8, 4);
    expect(sky.pixels).toHaveLength(32);
    expect(sky.pixels.every((p) => p.x >= 0 && p.y >= 0 && p.z >= 0)).toBe(true);
  });
  it('packs a finite balanced tree for hundreds of lights', () => {
    const lights = Array.from({ length: 200 }, (_, i) => [i, 5, 0, 1, 0.1, 0, 0, 0, 0, 0, 0, 0, 10, 5, 1, 0]).flat();
    const p = packWorld({}, lights),
      tree = p[19]!;
    expect(tree).toBe(206);
    expect(p.length / 4 - tree).toBe(3 * 399);
    expect([...p].every(Number.isFinite)).toBe(true);
  });
});
import * as THREE from 'three';
import { generateScatter, sampleTerrain, instanceMatrix } from './environment-geometry';
import { extractScene } from './extract';
it('extracts 100000 copies with one portable BLAS', () => {
  const graph = new THREE.Scene(),
    geometry = new THREE.BoxGeometry(1, 1, 1),
    material = new THREE.MeshStandardMaterial(),
    mesh = new THREE.InstancedMesh(geometry, material, 100000);
  mesh.userData.sceneId = 'trees';
  mesh.castShadow = true;
  mesh.receiveShadow = true;
  const matrix = new THREE.Matrix4();
  for (let i = 0; i < mesh.count; i++) {
    matrix.makeTranslation(i % 400, 0, Math.floor(i / 400));
    mesh.setMatrixAt(i, matrix);
  }
  graph.add(mesh);
  const scene = extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
  expect(scene.triangles).toBe(12);
  expect(scene.instanceCount).toBe(100000);
  expect(scene.positions.byteLength).toBe(12 * 48);
  // A new capacity must rebase BLAS links, even when the prototype is unchanged.
  mesh.count = 5;
  const smaller = extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
  expect(smaller.geometryKey).not.toBe(scene.geometryKey);
  expect(smaller.sdata[15]).toBe(6);
});
it('keeps scatter deterministic and matches a flat image heightfield', () => {
  const source = { objects: [{}], terrains: [{ resolution: 2, heights: Array(9).fill(0.5), amplitude: 10 }] } as any;
  const config = { count: 1000, terrain: 0, exclusions: [{ center: { x: 0, y: 0, z: 0 }, radius: 10 }] };
  const a = generateScatter(config, source);
  expect(a).toEqual(generateScatter(config, source));
  expect(a.length).toBeGreaterThan(900);
  expect(a.every((i) => i.position!.y === 5)).toBe(true);
  expect(sampleTerrain(source.terrains[0], 0, 0).height).toBe(5);
  const m = instanceMatrix({ object: 0, rotation: { x: 20, y: 40, z: 60 }, scale: { x: 2, y: 3, z: 4 } });
  expect(m.determinant()).toBeCloseTo(24, 6);
});
import { compileSceneDescription, describePreparedScene, viewFromDescription } from './scene-description';
import type { StudioView } from './gpu';
it('roundtrips a world, layer flags and visibility without adding a second sun', () => {
  const graph = new THREE.Scene(),
    mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  mesh.userData.sceneId = 'box';
  graph.add(mesh);
  const prepared = extractScene(graph, () => true, { sky: new THREE.Color(), ground: new THREE.Color(), intensity: 0 });
  const view: StudioView = {
    camera: {
      position: [0, 0, 5],
      forward: [0, 0, -1],
      right: [1, 0, 0],
      up: [0, 1, 0],
      tanY: 0.5,
      aspect: 1,
      lensRadius: 0,
      focus: 5,
    },
    exposure: 1,
    backdrop: [0, 0, 0],
    backdropOpaque: true,
    shadowCatcher: false,
    shadows: true,
    floorY: 0,
    environment: { zenith: [0, 0, 0], horizon: [0, 0, 0], ground: [0, 0, 0] },
  };
  const doc = describePreparedScene(prepared, view, []);
  doc.scene.world = { sky: { elevation: 45 } };
  doc.scene.meters_per_unit = 1;
  doc.scene.camera.focal_mm = 50;
  doc.scene.camera.f_stop = 4;
  expect(viewFromDescription(doc, 1).camera.lensRadius).toBeCloseTo(0.00625, 7);
  doc.object_ids[0]!.primary_visible = false;
  doc.scene.instances = [
    { object: 0, layer: 0, position: { x: 1, y: 0, z: 0 } },
    { object: 0, position: { x: -1, y: 0, z: 0 } },
  ];
  const first = compileSceneDescription(doc);
  expect(first.scene.instanceCount).toBe(2);
  expect(first.scene.lightCount).toBe(1);
  expect(first.scene.triangles).toBe(12);
  const offset = first.scene.lightCount * 16;
  expect([first.scene.sdata[offset + 19], first.scene.sdata[offset + 36 + 19]]).toContain(2);
  expect([first.scene.sdata[offset + 23], first.scene.sdata[offset + 36 + 23]]).toEqual([1, 1]);
  const roundtrip = describePreparedScene(first.scene, view, []);
  expect(roundtrip.object_ids.every((o) => o.primary_visible === false)).toBe(true);
  expect(roundtrip.scene.meters_per_unit).toBe(1);
  expect(roundtrip.scene.extra_lights).toHaveLength(0);
  expect(compileSceneDescription(roundtrip).scene.lightCount).toBe(1);
});
import { displaceMesh } from './displacement';
it('subdivides shared edges and displaces real geometry', () => {
  const source = {
    positions: [
      { x: 0, y: 0, z: 0 },
      { x: 1, y: 0, z: 0 },
      { x: 0, y: 0, z: 1 },
    ],
    normals: Array.from({ length: 3 }, () => ({ x: 0, y: 1, z: 0 })),
    uvs: [
      [0, 0],
      [1, 0],
      [0, 1],
    ] as [number, number][],
    indices: [[0, 2, 1]] as [number, number, number][],
  };
  const mesh = displaceMesh(source, {
    object: 0,
    width: 2,
    height: 2,
    values: [1, 1, 1, 1],
    strength: 2,
    subdivision: 2,
  });
  expect(mesh.indices).toHaveLength(16);
  expect(mesh.positions).toHaveLength(15);
  expect(mesh.positions.every((p) => p.y === 1)).toBe(true);
  expect(mesh.normals.every((n) => n.y === 1)).toBe(true);
});
