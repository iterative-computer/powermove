import { describe, it, expect } from 'vitest';
import { NATURE_SPECIES, natureMesh, makeNature, selectNatureLod } from './nature';
import { bakeTerrain, terrainMeshes, terrainResolution, sampleTerrain } from './environment-geometry';
describe('natural environment content', () => {
    it('repeats every seeded species, with smaller distant meshes and unit normals', () => {
        for (const species of NATURE_SPECIES) {
            const a = natureMesh(species, 41), b = natureMesh(species, 41), c = natureMesh(species, 42), far = natureMesh(species, 41, 2);
            expect(a.positions).toEqual(b.positions);
            expect(a.positions).not.toEqual(c.positions);
            expect(a.indices.length).toBeGreaterThan(far.indices.length * 2);
            expect(a.colors).toHaveLength(a.positions.length);
            expect(a.normals.every(n => Math.abs(Math.hypot(n.x, n.y, n.z) - 1) < 1e-5)).toBe(true);
        }
    });
    it('grounds rocks and welds their seam normals at every detail level',()=>{
      for(let lod=0;lod<3;lod++){const mesh=natureMesh('rock',81,lod),segments=[16,10,6][lod]!,rings=[10,6,4][lod]!;
        expect(Math.min(...mesh.positions.map(p=>p.y))).toBe(0);
        for(let row=0;row<=rings;row++){const a=row*(segments+1),b=a+segments;expect(mesh.positions[a]).toEqual(mesh.positions[b]);expect(mesh.normals[a]).toEqual(mesh.normals[b]);}
      }
    });
    it('keeps authored roughness and metallic values on blended ground',()=>{
      const t={resolution:2,amplitude:0,natural:true,blend:.2,layers:[{material:1,min_height:-1000,max_height:1000,min_slope:0,max_slope:90}]};
      const mesh=terrainMeshes(t,[{x:.1,y:.1,z:.1},{x:.4,y:.4,z:.4}],[{roughness:.9,metallic:0},{roughness:.23,metallic:.6}])[0]!.mesh;
      expect(mesh.uvs.every(uv=>Math.abs(uv[0]-.23)<1e-6 && Math.abs(uv[1]-.6)<1e-6)).toBe(true);
    });
    it('selects full leaves, clusters and sparse distant cards from angular size', () => {
        expect([10, 100, 1000].map(d => selectNatureLod(8, d))).toEqual([0, 1, 2]);
        const model = makeNature('birch', 7);
        expect(model.children.map(c => c.visible)).toEqual([true, false, false]);
        for (const mesh of model.children as any[]) {
            expect(mesh.material[1].userData.studioProcedural).toBe(4);
            expect(mesh.geometry.getAttribute('color')).toBeDefined();
            mesh.geometry.dispose();
        }
        for (const material of (model.children[0] as any).material)
            material.dispose();
    });
    it('erodes one grid for both the smooth surface and attached scatter', () => {
        const t = { resolution: 16, size: [8, 8] as [
                number,
                number
            ], amplitude: 5, frequency: .8, seed: 41, erosion: 20, natural: true, blend: .2 };
        const baked = bakeTerrain(t), mesh = terrainMeshes(baked)[0]!.mesh;
        expect(bakeTerrain(t)).toBe(baked);
        expect(mesh.colors).toHaveLength(mesh.positions.length);
        for (const p of mesh.positions)
            expect(sampleTerrain(baked, p.x, p.z).height).toBeCloseTo(p.y, 5);
        expect(terrainResolution({ ...t, adaptive: true }, { position: { x: 0, y: 5, z: 10 }, fov: 45 })).toBeGreaterThan(terrainResolution({ ...t, adaptive: true }, { position: { x: 0, y: 5000, z: 5000 }, fov: 45 }));
    });
});
