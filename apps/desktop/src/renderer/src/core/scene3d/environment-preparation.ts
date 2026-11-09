import * as THREE from 'three';
import {generateScatter,instanceMatrix,terrainMeshes,bakeTerrain} from '../studio/environment-geometry';
import {selectNatureLod} from '../studio/nature';
import type {GeometryPayload} from '../studio/gather';

export type TerrainPart={material:number;positions:Float32Array;normals:Float32Array;uvs:Float32Array;colors?:Float32Array;indices:Uint32Array};
export type TerrainJob={kind:'terrain';settings:any;palette:any[];materials:{roughness:number;metallic:number}[]};
export type ScatterJob={kind:'scatter';settings:any;surfaces:{geometry:GeometryPayload;matrix:number[]}[];
 inverse:number[];rotation:number[];scale:number[];prototypes:{relative:number[];level?:number}[];diameter:number;camera:number[]};
export type EnvironmentJob=TerrainJob|ScatterJob;
export type ScatterPart={prototype:number;matrices:Float32Array};

/** Identical seeded geometry/placement math, with no DOM or editor ownership. */
export function prepareEnvironment(job:EnvironmentJob):TerrainPart[]|ScatterPart[]{
 if(job.kind==='terrain')return terrainMeshes(job.settings.bake_resolution!==undefined?bakeTerrain(job.settings,job.settings.bake_resolution):job.settings,job.palette,job.materials).map(part=>({material:part.material,
  positions:new Float32Array(part.mesh.positions.flatMap(v=>[v.x,v.y,v.z])),normals:new Float32Array(part.mesh.normals.flatMap(v=>[v.x,v.y,v.z])),
  uvs:new Float32Array(part.mesh.uvs.flat()),colors:part.mesh.colors?new Float32Array(part.mesh.colors.flatMap(v=>[v.x,v.y,v.z])):undefined,indices:new Uint32Array(part.mesh.indices.flat())}));
 const positions:any[]=[],normals:any[]=[],indices:any[]=[],uvs:any[]=[];
 const vector=new THREE.Vector3(),matrix=new THREE.Matrix4();
 for(const surface of job.surfaces){
  matrix.fromArray(surface.matrix);const g=new THREE.BufferGeometry();for(const [name,a] of Object.entries(surface.geometry.attributes))g.setAttribute(name,new THREE.BufferAttribute(a.array,a.itemSize,a.normalized));
  const position=g.getAttribute('position'),normal=g.getAttribute('normal'),offset=positions.length;
  for(let i=0;i<position.count;i++){vector.fromBufferAttribute(position,i).applyMatrix4(matrix);positions.push({x:vector.x,y:vector.y,z:vector.z});if(normal)vector.fromBufferAttribute(normal,i).transformDirection(matrix);else vector.set(0,1,0);normals.push({x:vector.x,y:vector.y,z:vector.z});uvs.push([0,0]);}
  const index=surface.geometry.index;for(let i=0;i<(index?.length??position.count);i+=3)indices.push([0,1,2].map(j=>offset+(index?index[i+j]!:i+j)));
 }
 const placements=generateScatter(job.settings,{objects:[{mesh:{positions,normals,indices,uvs},material:0}]} as any);
 const camera=new THREE.Vector3().fromArray(job.camera),selected=job.diameter?placements.map(p=>selectNatureLod(job.diameter*(p.scale?.x??1),camera.distanceTo(vector.set(p.position!.x,p.position!.y,p.position!.z)))):[];
 const inverse=new THREE.Matrix4().fromArray(job.inverse),rotation=new THREE.Matrix4().fromArray(job.rotation),scale=new THREE.Vector3().fromArray(job.scale);
 return job.prototypes.map((prototype,index)=>{
  const chosen=prototype.level===undefined?placements:placements.filter((_,i)=>selected[i]===prototype.level),matrices=new Float32Array(chosen.length*16),relative=new THREE.Matrix4().fromArray(prototype.relative);
  chosen.forEach((placement,i)=>{const m=instanceMatrix(placement).multiply(rotation).scale(scale).multiply(relative);inverse.clone().multiply(m).toArray(matrices,i*16);});
  return {prototype:index,matrices};
 });
}
