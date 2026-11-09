/** Texture-free nature geometry. Keep the seeded construction in step with
 * engine/crates/pm-studio/src/nature.rs; the renderers share the resulting buffers. */
import * as THREE from 'three';
import { natureHash, terrainNoise } from './environment-geometry';
import type { StudioMesh, StudioVec3 } from '../scene3d/studio-backend';
export const NATURE_SPECIES = ['conifer', 'broadleaf', 'birch', 'bush', 'grass', 'rock'] as const;
export type NatureSpecies = typeof NATURE_SPECIES[number];
export const NATURE_NAMES: Record<NatureSpecies, string> = { conifer: 'Pine', broadleaf: 'Oak', birch: 'Birch', bush: 'Bush', grass: 'Grass', rock: 'Rock' };
const TAU = Math.PI * 2, V = (x = 0, y = 0, z = 0) => new THREE.Vector3(x, y, z), to = (p: THREE.Vector3) => ({ x: p.x, y: p.y, z: p.z });
export function selectNatureLod(diameter: number, distance: number): number { const angular = diameter / Math.max(.001, distance); return angular >= .09 ? 0 : angular >= .02 ? 1 : 2; }
class Builder {
    mesh: StudioMesh = { positions: [], normals: [], uvs: [], indices: [], colors: [], triangle_materials: [] };
    constructor(private state: number) { }
    rnd() { this.state = natureHash(this.state); return (this.state >>> 8) / 16777216; }
    vertex(p: THREE.Vector3, n: THREE.Vector3, uv: [
        number,
        number
    ], tint: number) { const i = this.mesh.positions.length; this.mesh.positions.push(to(p)); this.mesh.normals.push(to(n.clone().normalize())); this.mesh.uvs.push(uv); this.mesh.colors!.push({ x: tint, y: tint, z: tint }); return i; }
    tri(a: number, b: number, c: number, material: number) { this.mesh.indices.push([a, b, c]); this.mesh.triangle_materials!.push(material); }
    branch(a: THREE.Vector3, b: THREE.Vector3, radius: number, end: number, segments: number) {
        const axis = b.clone().sub(a).normalize(), side = axis.clone().cross(Math.abs(axis.y) < .9 ? V(0, 1, 0) : V(1, 0, 0)).normalize(), up = axis.clone().cross(side).normalize(), start = this.mesh.positions.length;
        for (let j = 0; j <= 1; j++)
            for (let k = 0; k < segments; k++) {
                const angle = TAU * k / segments, n = side.clone().multiplyScalar(Math.cos(angle)).addScaledVector(up, Math.sin(angle));
                this.vertex((j === 0 ? a : b).clone().addScaledVector(n, j === 0 ? radius : end), n, [k / segments, j], .8 + .2 * Math.abs(Math.cos(angle * 3)));
            }
        for (let k = 0; k < segments; k++) {
            const q = (k + 1) % segments;
            this.tri(start + k, start + q, start + k + segments, 0);
            this.tri(start + q, start + q + segments, start + k + segments, 0);
        }
    }
    leaf(p: THREE.Vector3, length: number, width: number, yaw: number, pitch: number) {
        p = p.clone();
        p.y = Math.max(p.y, -Math.min(0, length * Math.sin(pitch)) + .02);
        const axis = V(Math.cos(yaw) * Math.cos(pitch), Math.sin(pitch), Math.sin(yaw) * Math.cos(pitch)), side = V(-Math.sin(yaw), 0, Math.cos(yaw)), n = side.clone().cross(axis).normalize(), tint = .68 + this.rnd() * .32, start = this.mesh.positions.length;
        for (const [u, v, fold] of [[.5, 0, 0], [0, .48, 0], [.5, .55, .12], [1, .48, 0], [.5, 1, 0]] as [
            number,
            number,
            number
        ][]) {
            this.vertex(p.clone().addScaledVector(axis, v * length).addScaledVector(side, (u - .5) * width).addScaledVector(n, length * fold), n.clone().addScaledVector(side, (u - .5) * .35).normalize(), [u, v], tint);
        }
        for (const [a, b, c] of [[0, 1, 2], [1, 4, 2], [4, 3, 2], [3, 0, 2]])
            this.tri(start + a!, start + b!, start + c!, 1);
    }
    spray(p: THREE.Vector3, radius: number, count: number, leafSize: number) {
        for (let i = 0; i < count; i++) {
            const yaw = this.rnd() * TAU, z = this.rnd() * 2 - 1, ring = Math.sqrt(1 - z * z), spread = radius * Math.cbrt(this.rnd()), q = p.clone().addScaledVector(V(ring * Math.cos(yaw), z, ring * Math.sin(yaw)), spread), pitch = (this.rnd() - .5) * 1.8, length = leafSize * (.65 + this.rnd() * .7);
            this.leaf(q, length, length * .48, yaw, pitch);
        }
    }
}
export function natureMesh(species: NatureSpecies, seed = 1, level = 0): StudioMesh {
    const lod = Math.max(0, Math.min(2, Math.round(level))), b = new Builder(seed), segments = [7, 5, 4][lod]!;
    if (species === 'rock') {
        const rings = [10, 6, 4][lod]!, segs = [16, 10, 6][lod]!, stretch = V(.8 + b.rnd() * .8, .45 + b.rnd() * .45, .7 + b.rnd() * .5), m = b.mesh;
        for (let r = 0; r <= rings; r++)
            for (let s = 0; s <= segs; s++) {
                const phi = Math.PI * r / rings, th = TAU * s / segs, p = V(Math.sin(phi) * Math.cos(th), Math.cos(phi), Math.sin(phi) * Math.sin(th));
                if(r===0 || r===rings)p.set(0,p.y,0);
                const d = 1 + terrainNoise(p.x * 2.7 + p.y * .6, p.z * 2.7, seed) * .22 + terrainNoise(p.y * 6, p.x * 5 + p.z * 3, seed ^ 73) * .08;
                p.set(p.x * stretch.x * d, Math.max(0, p.y * d + 1) * stretch.y, p.z * stretch.z * d);
                m.positions.push(to(p));
                m.normals.push({ x: 0, y: 0, z: 0 });
                m.uvs.push([s / segs, r / rings]);
                m.colors!.push({ x: .72 + .23 * Math.abs(terrainNoise(p.x * 3, p.z * 3, seed)), y: .72 + .23 * Math.abs(terrainNoise(p.x * 3, p.z * 3, seed)), z: .72 + .23 * Math.abs(terrainNoise(p.x * 3, p.z * 3, seed)) });
            }
        const stride = segs + 1;
        for (let r = 0; r < rings; r++)
            for (let s = 0; s < segs; s++) {
                const a = r * stride + s, c = a + stride;
                m.indices.push([a, a + 1, c], [a + 1, c + 1, c]);
            }
        for(let row=0;row<=rings;row++)m.positions[row*stride+segs]={...m.positions[row*stride]!};
        const floor=Math.min(...m.positions.map(p=>p.y));for(const p of m.positions)p.y-=floor;
        const normal = m.normals.map(() => V());
        for (const t of m.indices) {
            const [a, c, d] = t.map(i => V(m.positions[i]!.x, m.positions[i]!.y, m.positions[i]!.z)), n = c!.sub(a!).cross(d!.sub(a!));
            for (const i of t)
                normal[i]!.add(n);
        }
        for(let row=1;row<rings;row++){const a=row*stride,c=a+segs,n=normal[a]!.clone().add(normal[c]!);normal[a]=n;normal[c]=n.clone();}
        for(let col=0;col<=segs;col++){normal[col]=V(0,1,0);normal[rings*stride+col]=V(0,-1,0);}
        m.normals = normal.map(n => to(n.length() > 1e-8 ? n.normalize() : V(0, 1, 0)));
        return m;
    }
    if (species === 'grass') {
        for (let i = 0; i < [28, 12, 5][lod]!; i++) {
            const yaw = b.rnd() * TAU, root = V((b.rnd() - .5) * .3, 0, (b.rnd() - .5) * .3), height = .22 + b.rnd() * .4, bend = .1 + b.rnd() * .16, side = V(-Math.sin(yaw), 0, Math.cos(yaw)), forward = V(Math.cos(yaw), 0, Math.sin(yaw)), start = b.mesh.positions.length;
            for (let j = 0; j <= 3; j++) {
                const t = j / 3, center = root.clone().add(V(0, height * t, 0)).addScaledVector(forward, bend * t * t), width = .014 * (1 - t) + .0005;
                for (const [u, sign] of [[0, -1], [1, 1]])
                    b.vertex(center.clone().addScaledVector(side, width * sign!), forward, [u!, t], .7 + t * .25);
            }
            for (let j = 0; j < 3; j++) {
                const a = start + j * 2;
                b.tri(a, a + 1, a + 2, 1);
                b.tri(a + 1, a + 3, a + 2, 1);
            }
        }
        return b.mesh;
    }
    if (species === 'conifer') {
        const height = 7 + b.rnd() * 1.6, lean = V((b.rnd() - .5) * .35, 0, (b.rnd() - .5) * .35), n = [9, 6, 3][lod]!;
        for (let j = 0; j < n; j++) {
            const t = j / n, u = (j + 1) / n;
            b.branch(V(0, t * height, 0).addScaledVector(lean, t * t), V(0, u * height, 0).addScaledVector(lean, u * u), .17 * (1 - t) + .012, .17 * (1 - u) + .012, segments);
        }
        const tiers = [15, 10, 6][lod]!, arms = [7, 5, 3][lod]!;
        for (let tier = 0; tier < tiers; tier++) {
            const t = tier / tiers, y = height * (.12 + .82 * t) + (b.rnd() - .5) * height / tiers * .55, radius = 1.75 * (1 - t) ** .8 + .10;
            for (let arm = 0; arm < arms; arm++) {
                const yaw = arm / arms * TAU + tier * 2.399 + (b.rnd() - .5) * .5, r = radius * (.7 + b.rnd() * .4), a = V(lean.x * t, y, lean.z * t), end = a.clone().add(V(Math.cos(yaw) * r, -.10 * r, Math.sin(yaw) * r));
                if (lod < 2)
                    b.branch(a, end, .035 * (1 - t) + .006, .004, segments);
                const shoots = [5, 3, 1][lod]!;
                for (let j = 0; j < shoots; j++) {
                    const u = (j + .5) / shoots, p = a.clone().multiplyScalar(1 - u).addScaledVector(end, u).add(V(0, .1 + u * .18, 0));
                    b.spray(p, [.38, .44, .65][lod]! * (1 - t * .5), [18, 8, 4][lod]!, [.28, .46, .85][lod]! * (1 - t * .4));
                }
            }
        }
    }
    else {
        const bush = species === 'bush', birch = species === 'birch', height = bush ? 1.4 : birch ? 8.2 : 6.4, width = bush ? 1 : birch ? 1.6 : 2.5, trunk = height * (bush ? .3 : .6), lean = V((b.rnd() - .5) * height * .12, trunk, (b.rnd() - .5) * height * .12);
        b.branch(V(), lean, bush ? .06 : birch ? .12 : .25, .025, segments);
        const arms = [18, 12, 7][lod]!;
        for (let i = 0; i < arms; i++) {
            const yaw = i * 2.399 + b.rnd() * .7, t = (i + .5) / arms, y = height * (.36 + .55 * t), r = width * (.55 + .45 * Math.sin(t * Math.PI)) * (.75 + b.rnd() * .35), a = lean.clone().multiplyScalar(.35 + .6 * t), end = V(Math.cos(yaw) * r + lean.x, y, Math.sin(yaw) * r + lean.z);
            if (lod < 2)
                b.branch(a, end, (bush ? .025 : .055) * (1 - t * .6), .008, segments);
            const clusters = [5, 3, 1][lod]!;
            for (let j = 0; j < clusters; j++) {
                const u = .45 + j / clusters * .55, p = a.clone().multiplyScalar(1 - u).addScaledVector(end, u), rad = bush ? .34 : .58, leaf = bush ? .14 : birch ? .16 : .22;
                b.spray(p, rad * [1, 1.15, 1.5][lod]!, [26, 12, 6][lod]!, leaf * [1, 1.9, 4.5][lod]!);
            }
        }
    }
    return b.mesh;
}
export function natureMaterials(species: NatureSpecies): THREE.MeshPhysicalMaterial[] {
    const bark = species === 'birch' ? [.66, .64, .55] : species === 'rock' ? [.34, .31, .26] : [.19, .105, .046], leaf = species === 'conifer' ? [.085, .18, .066] : species === 'birch' ? [.23, .34, .055] : species === 'grass' ? [.23, .30, .065] : [.15, .26, .046];
    return [bark, leaf].map((rgb, i) => { const m = new THREE.MeshPhysicalMaterial({ color: new THREE.Color(...rgb as [
            number,
            number,
            number
        ]), roughness: i === 0 ? .87 : .72, metalness: 0, side: THREE.DoubleSide, vertexColors: true, specularIntensity: i === 0 ? 1 : .25 }); m.userData.sourceMaterialIndex = i; m.userData.studioProcedural = i === 1 ? (species === 'grass' ? 5 : 4) : species === 'birch' ? 6 : species === 'rock' ? 2 : 3; return m; });
}
export function natureGeometry(mesh: StudioMesh): THREE.BufferGeometry {
    const g = new THREE.BufferGeometry(), flat = (p: StudioVec3[]) => p.flatMap(v => [v.x, v.y, v.z]);
    g.setAttribute('position', new THREE.Float32BufferAttribute(flat(mesh.positions), 3));
    g.setAttribute('normal', new THREE.Float32BufferAttribute(flat(mesh.normals), 3));
    g.setAttribute('uv', new THREE.Float32BufferAttribute(mesh.uvs.flat(), 2));
    if (mesh.colors)
        g.setAttribute('color', new THREE.Float32BufferAttribute(flat(mesh.colors), 3));
    g.setIndex(mesh.indices.flat());
    // Contiguous runs keep material groups small, even with thousands of leaves.
    const faces = mesh.triangle_materials;
    let start = 0;
    for (let i = 1; i <= (faces?.length ?? 0); i++)
        if (i === faces!.length || faces![i] !== faces![start]) {
            g.addGroup(start * 3, (i - start) * 3, faces![start]);
            start = i;
        }
    return g;
}
export function makeNature(species: NatureSpecies, seed: number): THREE.Group {
    const group = new THREE.Group(), materials = natureMaterials(species);
    for (let lod = 0; lod < 3; lod++) {
        const mesh = new THREE.Mesh(natureGeometry(natureMesh(species, seed, lod)), materials);
        mesh.visible = lod === 0;
        mesh.userData.studioNatureLevel = lod;
        group.add(mesh);
    }
    group.userData.studioNature = true;
    return group;
}
