import * as THREE from 'three';
import { STUDIO_TRIANGLE_FLOATS } from './layout';

/** Normals follow the same morph and skin pose as the sampled vertex. */
function posedNormal(mesh:THREE.Mesh,g:THREE.BufferGeometry,index:number,base:THREE.BufferAttribute):THREE.Vector3 {
 const normal=new THREE.Vector3().fromBufferAttribute(base,index),original=normal.clone();
 const morphs=g.morphAttributes.normal;
 morphs?.forEach((attribute,i)=>{const weight=mesh.morphTargetInfluences?.[i] ?? 0;if(!weight)return;const delta=new THREE.Vector3().fromBufferAttribute(attribute,index);if(!g.morphTargetsRelative)delta.sub(original);normal.addScaledVector(delta,weight);});
 const skinned=mesh as THREE.SkinnedMesh;
 if(skinned.isSkinnedMesh){
  const indices=g.getAttribute('skinIndex'),weights=g.getAttribute('skinWeight'),matrix=new THREE.Matrix4();matrix.elements.fill(0);
  const bone=new THREE.Matrix4();
  for(let i=0;i<4;i++){const weight=weights.getComponent(index,i);if(!weight)continue;bone.fromArray(skinned.skeleton.boneMatrices,indices.getComponent(index,i)*16);for(let j=0;j<16;j++)matrix.elements[j]!+=bone.elements[j]!*weight;}
  matrix.premultiply(skinned.bindMatrixInverse).multiply(skinned.bindMatrix);normal.applyMatrix3(new THREE.Matrix3().getNormalMatrix(matrix));
 }
 return normal.normalize();
}

/**
 * A mesh's triangles as flat arrays: corners (9 floats each, model space) and
 * shading (36 floats each: three normals, uvs, material, object, tangent).
 */
export function gather(mesh: THREE.Mesh, g: THREE.BufferGeometry, mi: number[], oi: number): { corners: Float32Array; shading: Float32Array; bounds: THREE.Box3 } {
  const posAttr = g.getAttribute('position') as THREE.BufferAttribute, nrm = g.getAttribute('normal') as THREE.BufferAttribute | undefined, uvA = g.getAttribute('uv') as THREE.BufferAttribute | undefined;
  const secondary=g.getAttribute('uv1');
  const index = g.getIndex();
  const vertexCount = index ? index.count : posAttr.count;
  const groups = Array.isArray(mesh.material) && g.groups.length ? g.groups : [{ start: 0, count: vertexCount, materialIndex: 0 }];
  let count = 0;
  for (const group of groups) {
    if ((mi[group.materialIndex ?? 0] ?? -1) < 0) continue;
    const end = Math.min(group.start + group.count, vertexCount);
    count += Math.max(0, Math.floor((end - group.start) / 3));
  }
  const corners = new Float32Array(count * 9), shading = new Float32Array(count * STUDIO_TRIANGLE_FLOATS);
  let lx = Infinity, ly = Infinity, lz = Infinity, hx = -Infinity, hy = -Infinity, hz = -Infinity;
  let t = 0;
  const p = new Float64Array(9), uv = new Float64Array(6),uv1=new Float64Array(6);
  for (const group of groups) {
    const m = mi[group.materialIndex ?? 0] ?? -1;
    if (m < 0) continue;
    const end = Math.min(group.start + group.count, vertexCount);
    for (let k = group.start; k + 2 < end; k += 3, t++) {
      const c = t * 9, s = t * STUDIO_TRIANGLE_FLOATS;
      for (let j = 0; j < 3; j++) {
        const v = index ? index.getX(k + j) : k + j;
        if(!Number.isInteger(v) || v<0 || v>=posAttr.count)throw new Error('Mesh index is outside its vertex buffer');
        const deformed=(mesh as THREE.SkinnedMesh).isSkinnedMesh || mesh.morphTargetInfluences?.some(v=>v!==0);
        const point=deformed?mesh.getVertexPosition(v,new THREE.Vector3()):null;
        const x = point?.x ?? posAttr.getX(v), y = point?.y ?? posAttr.getY(v), z = point?.z ?? posAttr.getZ(v);
        if(![x,y,z].every(v=>Number.isFinite(Math.fround(v))))throw new Error('Mesh contains non-finite positions');
        p[j * 3] = x; p[j * 3 + 1] = y; p[j * 3 + 2] = z;
        corners[c + j * 3] = x; corners[c + j * 3 + 1] = y; corners[c + j * 3 + 2] = z;
        if (x < lx) lx = x; if (y < ly) ly = y; if (z < lz) lz = z;
        if (x > hx) hx = x; if (y > hy) hy = y; if (z > hz) hz = z;
        if (nrm) {
          const posed=(mesh as THREE.SkinnedMesh).isSkinnedMesh || g.morphAttributes.normal?.length?posedNormal(mesh,g,v,nrm):null;
          const nx = posed?.x ?? nrm.getX(v), ny = posed?.y ?? nrm.getY(v), nz = posed?.z ?? nrm.getZ(v), l = Math.hypot(nx, ny, nz) || 1;
          if(![nx,ny,nz].every(Number.isFinite))throw new Error('Mesh contains non-finite normals');
          shading[s + j * 4] = nx / l; shading[s + j * 4 + 1] = ny / l; shading[s + j * 4 + 2] = nz / l;
        }
        uv[j * 2] = uvA ? uvA.getX(v) : 0; uv[j * 2 + 1] = uvA ? uvA.getY(v) : 0;
        if(![uv[j*2],uv[j*2+1]].every(v=>Number.isFinite(Math.fround(v!))))throw new Error('Mesh contains non-finite texture coordinates');
        uv1[j*2]=secondary?secondary.getX(v):uv[j*2]!;uv1[j*2+1]=secondary?secondary.getY(v):uv[j*2+1]!;
        if(![uv1[j*2],uv1[j*2+1]].every(v=>Number.isFinite(Math.fround(v!))))throw new Error('Mesh contains non-finite secondary texture coordinates');
      }
      const ax = p[3]! - p[0]!, ay = p[4]! - p[1]!, az = p[5]! - p[2]!, bx = p[6]! - p[0]!, by = p[7]! - p[1]!, bz = p[8]! - p[2]!;
      if (!nrm || [0,1,2].some(j=>Math.hypot(...shading.subarray(s+j*4,s+j*4+3))<1e-8)) {
        // Faceted normals.
        const nx = ay * bz - az * by, ny = az * bx - ax * bz, nz = ax * by - ay * bx, l = Math.hypot(nx, ny, nz) || 1;
        for (let j = 0; j < 3; j++) { shading[s + j * 4] = nx / l; shading[s + j * 4 + 1] = ny / l; shading[s + j * 4 + 2] = nz / l; }
      }
      const color=g.getAttribute('color');
      if(color){const words=new Uint32Array(shading.buffer);for(let j=0;j<3;j++){
        const v=index?index.getX(k+j):k+j;
        const rgba=[color.getX(v),color.getY(v),color.getZ(v),color.itemSize>3?color.getW(v):1];
        if(!rgba.every(Number.isFinite))throw new Error('Mesh contains non-finite vertex colors');
        const byte=(n:number)=>Math.round(Math.max(0,Math.min(1,n))*255);
        words[s+j*4+3]=(byte(rgba[0]!) | byte(rgba[1]!)<<8 | byte(rgba[2]!)<<16 | byte(rgba[3]!)<<24)>>>0;
      }}
      shading.set([uv[0]!, uv[1]!, uv[2]!, uv[3]!, uv[4]!, uv[5]!, m, oi], s + 12);
      shading.set([uv1[0]!,uv1[1]!,uv1[2]!,uv1[3]!,uv1[4]!,uv1[5]!,secondary?1:0,0],s+24);
      // Tangents drive normal maps and brushed (anisotropic) highlights.
      for(const [coords,offset,exists] of [[uv,20,!!uvA],[uv1,32,!!secondary || !!uvA]] as const)if(exists){
        const d1u = coords[2]! - coords[0]!, d1v = coords[3]! - coords[1]!, d2u = coords[4]! - coords[0]!, d2v = coords[5]! - coords[1]!;
        const r = d1u * d2v - d2u * d1v;
        if (Math.abs(r) < 1e-12) { shading[s + offset+3] = 1; continue; }
        const f = 1 / r;
        const tx = f * (d2v * ax - d1v * bx), ty = f * (d2v * ay - d1v * by), tz = f * (d2v * az - d1v * bz);
        const qx = f * (d1u * bx - d2u * ax), qy = f * (d1u * by - d2u * ay), qz = f * (d1u * bz - d2u * az);
        // Handedness: (e1 × e2) × t against the bitangent.
        const ex = ay * bz - az * by, ey = az * bx - ax * bz, ez = ax * by - ay * bx;
        const cx = ey * tz - ez * ty, cy = ez * tx - ex * tz, cz = ex * ty - ey * tx;
        const handed = cx * qx + cy * qy + cz * qz < 0 ? -1 : 1;
        const len = Math.hypot(tx, ty, tz) || 1;
        shading[s + offset] = tx / len; shading[s + offset+1] = ty / len; shading[s + offset+2] = tz / len; shading[s + offset+3] = handed;
      }
    }
  }
  const bounds = count ? new THREE.Box3(new THREE.Vector3(lx, ly, lz), new THREE.Vector3(hx, hy, hz)) : new THREE.Box3();
  return { corners: corners.subarray(0, t * 9), shading: shading.subarray(0, t * STUDIO_TRIANGLE_FLOATS), bounds };
}

export type GeometryPayload={attributes:Record<string,{array:THREE.TypedArray;itemSize:number;normalized:boolean}>;index:Uint32Array|null;groups:{start:number;count:number;materialIndex?:number}[];materials:number[];object:number};
/** Own copies are transferred; imported asset buffers remain available to the editor. */
export function geometryPayload(g:THREE.BufferGeometry,materials:number[],object:number):GeometryPayload {
  const attributes:GeometryPayload['attributes']={};
  for(const name of ['position','normal','uv','uv1','color']){
    const a=g.getAttribute(name);if(!a)continue;
    if((a as THREE.InterleavedBufferAttribute).isInterleavedBufferAttribute){
      const array=new Float32Array(a.count*a.itemSize);for(let v=0;v<a.count;v++)for(let c=0;c<a.itemSize;c++)array[v*a.itemSize+c]=a.getComponent(v,c);
      attributes[name]={array,itemSize:a.itemSize,normalized:false};
    }else attributes[name]={array:a.array.slice() as THREE.TypedArray,itemSize:a.itemSize,normalized:a.normalized};
  }
  return {attributes,index:g.getIndex()?new Uint32Array(g.getIndex()!.array):null,groups:g.groups.map(x=>({...x})),materials,object};
}
export function gatherPayload(input:GeometryPayload){
  const g=new THREE.BufferGeometry();for(const [name,a] of Object.entries(input.attributes))g.setAttribute(name,new THREE.BufferAttribute(a.array,a.itemSize,a.normalized));
  if(input.index)g.setIndex(new THREE.BufferAttribute(input.index,1));g.groups=input.groups;
  const mesh=new THREE.Mesh(g,input.materials.map(()=>new THREE.MeshStandardMaterial()));
  return gather(mesh,g,input.materials,input.object);
}
