import * as THREE from 'three';

/*
 * Mesh data for Edit Mode. Rendered geometry splits vertices along hard
 * edges and UV seams; Blender edits the underlying points. `weldMesh` groups
 * coincident vertices into points, and every operation acts on points while
 * keeping each split vertex's own UVs, so seams and hard edges survive.
 */

export type EditMesh={positions:number[];indices:number[];uvs?:number[]};
export type Weld={vertexPoint:Int32Array;points:number[][];key:string};

export function meshFromGeometry(geometry:THREE.BufferGeometry):EditMesh {
  const position=geometry.getAttribute('position'),uv=geometry.getAttribute('uv');
  const positions=Array.from(position.array as ArrayLike<number>,v=>Math.round(v*1e6)/1e6);
  const indices=geometry.index?Array.from(geometry.index.array as ArrayLike<number>):Array.from({length:position.count},(_,i)=>i);
  return {positions,indices,...(uv&&uv.count===position.count?{uvs:Array.from(uv.array as ArrayLike<number>)}:{})};
}
export const topologyKey=(mesh:EditMesh)=>`${mesh.positions.length}:${mesh.indices.length}:${mesh.indices.slice(0,24).join(',')}:${mesh.indices.slice(-24).join(',')}`;
export function weldMesh(mesh:EditMesh,epsilon=1e-5):Weld {
  const count=mesh.positions.length/3,vertexPoint=new Int32Array(count),points:number[][]=[],lookup=new Map<string,number>();
  for(let i=0;i<count;i++){
    const key=[0,1,2].map(k=>Math.round(mesh.positions[i*3+k]!/epsilon)).join(',');
    let point=lookup.get(key);if(point===undefined){point=points.length;points.push([]);lookup.set(key,point);}
    points[point]!.push(i);vertexPoint[i]=point;
  }
  return {vertexPoint,points,key:topologyKey(mesh)};
}
export const pointPosition=(mesh:EditMesh,weld:Weld,point:number)=>{const i=weld.points[point]![0]!;return new THREE.Vector3(mesh.positions[i*3],mesh.positions[i*3+1],mesh.positions[i*3+2]);};
/** Unique edges between points, each with whether it lies on a face. */
export function meshEdges(mesh:EditMesh,weld:Weld):[number,number][] {
  const seen=new Set<string>(),out:[number,number][]=[];
  for(let f=0;f<mesh.indices.length;f+=3)for(let k=0;k<3;k++){
    const a=weld.vertexPoint[mesh.indices[f+k]!]!,b=weld.vertexPoint[mesh.indices[f+(k+1)%3]!]!;if(a===b)continue;
    const key=a<b?`${a},${b}`:`${b},${a}`;if(!seen.has(key)){seen.add(key);out.push([a,b]);}
  }
  return out;
}
/** Move the selected points by a local-space matrix, starting from `base`. */
export function transformPoints(base:EditMesh,weld:Weld,selected:ReadonlySet<number>,matrix:THREE.Matrix4):EditMesh {
  const positions=base.positions.slice(),v=new THREE.Vector3();
  for(const point of selected)for(const i of weld.points[point]??[]){
    v.set(base.positions[i*3]!,base.positions[i*3+1]!,base.positions[i*3+2]!).applyMatrix4(matrix);
    positions[i*3]=Math.round(v.x*1e6)/1e6;positions[i*3+1]=Math.round(v.y*1e6)/1e6;positions[i*3+2]=Math.round(v.z*1e6)/1e6;
  }
  return {...base,positions};
}
/** Drop unused vertices and renumber. */
function compact(mesh:EditMesh):EditMesh {
  const used=new Int32Array(mesh.positions.length/3).fill(-1),positions:number[]=[],uvs:number[]=[];
  const indices=mesh.indices.map(i=>{
    if(used[i]===-1){used[i]=positions.length/3;positions.push(mesh.positions[i*3]!,mesh.positions[i*3+1]!,mesh.positions[i*3+2]!);if(mesh.uvs)uvs.push(mesh.uvs[i*2]!,mesh.uvs[i*2+1]!);}
    return used[i]!;
  });
  return {positions,indices,...(mesh.uvs?{uvs}:{})};
}
/** X › Vertices: remove the selected points and every face that uses one. */
export function deletePoints(mesh:EditMesh,weld:Weld,selected:ReadonlySet<number>):EditMesh {
  const indices:number[]=[];
  for(let f=0;f<mesh.indices.length;f+=3){
    const face=mesh.indices.slice(f,f+3);
    if(!face.some(i=>selected.has(weld.vertexPoint[i]!)))indices.push(...face);
  }
  return compact({...mesh,indices});
}
/** M › At Center: collapse the selected points to their centre and drop faces that degenerate. */
export function mergePoints(mesh:EditMesh,weld:Weld,selected:ReadonlySet<number>):EditMesh {
  if(selected.size<2)return mesh;
  const center=new THREE.Vector3();for(const point of selected)center.add(pointPosition(mesh,weld,point));center.divideScalar(selected.size);
  const positions=mesh.positions.slice();
  for(const point of selected)for(const i of weld.points[point]!){positions[i*3]=center.x;positions[i*3+1]=center.y;positions[i*3+2]=center.z;}
  const merged=(i:number)=>selected.has(weld.vertexPoint[i]!)?-1:weld.vertexPoint[i]!;
  const indices:number[]=[];
  for(let f=0;f<mesh.indices.length;f+=3){
    const face=mesh.indices.slice(f,f+3),keys=face.map(merged);
    if(new Set(keys).size===3)indices.push(...face);
  }
  return compact({...mesh,positions,indices});
}
/**
 * E: extrude the selected region. Faces whose points are all selected move to
 * new vertices; each boundary edge of that region gains a side quad. With no
 * selected faces, selected edges extrude into quads. Returns the new points'
 * vertex indices (to select) and the region's average local normal.
 */
export function extrudeRegion(mesh:EditMesh,weld:Weld,selected:ReadonlySet<number>):{mesh:EditMesh;vertices:number[];normal:THREE.Vector3;weld:Weld}|null {
  const point=(i:number)=>weld.vertexPoint[i]!,positions=mesh.positions.slice(),uvs=mesh.uvs?.slice(),indices=mesh.indices.slice();
  const region:number[]=[];
  for(let f=0;f<indices.length;f+=3)if([0,1,2].every(k=>selected.has(point(indices[f+k]!))))region.push(f);
  const duplicate=new Map<number,number>();
  const copy=(i:number)=>{
    let next=duplicate.get(i);if(next!==undefined)return next;
    next=positions.length/3;positions.push(positions[i*3]!,positions[i*3+1]!,positions[i*3+2]!);if(uvs)uvs.push(uvs[i*2]!,uvs[i*2+1]!);
    duplicate.set(i,next);return next;
  };
  const normal=new THREE.Vector3(),a=new THREE.Vector3(),b=new THREE.Vector3(),c=new THREE.Vector3();
  const sides:[number,number][]=[];
  if(region.length){
    // Boundary edges belong to exactly one region face (counted between points, across seams).
    const counts=new Map<string,number>();
    const key=(x:number,y:number)=>{const p=point(x),q=point(y);return p<q?`${p},${q}`:`${q},${p}`;};
    for(const f of region)for(let k=0;k<3;k++){const id=key(indices[f+k]!,indices[f+(k+1)%3]!);counts.set(id,(counts.get(id)??0)+1);}
    for(const f of region){
      const face=[indices[f]!,indices[f+1]!,indices[f+2]!];
      a.fromArray(positions,face[0]!*3);b.fromArray(positions,face[1]!*3);c.fromArray(positions,face[2]!*3);
      normal.add(b.clone().sub(a).cross(c.clone().sub(a)));
      for(let k=0;k<3;k++)if(counts.get(key(face[k]!,face[(k+1)%3]!))===1)sides.push([face[k]!,face[(k+1)%3]!]);
    }
    for(const f of region)for(let k=0;k<3;k++)indices[f+k]=copy(indices[f+k]!);
  }else{
    // Edge extrude: every edge whose two points are selected becomes a quad.
    const seen=new Set<string>();
    for(let f=0;f<indices.length;f+=3)for(let k=0;k<3;k++){
      const x=indices[f+k]!,y=indices[f+(k+1)%3]!,p=point(x),q=point(y);
      if(p===q||!selected.has(p)||!selected.has(q))continue;
      const id=p<q?`${p},${q}`:`${q},${p}`;if(seen.has(id))continue;seen.add(id);sides.push([x,y]);
      a.fromArray(positions,indices[f]!*3);b.fromArray(positions,indices[f+1]!*3);c.fromArray(positions,indices[f+2]!*3);
      normal.add(b.clone().sub(a).cross(c.clone().sub(a)));
    }
    if(!sides.length)return null;
  }
  for(const [x,y] of sides){const nx=copy(x),ny=copy(y);indices.push(x,y,ny,x,ny,nx);}
  if(normal.lengthSq()<1e-12)normal.set(0,1,0);
  const next={positions,indices,...(uvs?{uvs}:{})};
  // The copies start where the originals are; give them their own points instead of welding them back.
  const vertexPoint=new Int32Array(positions.length/3),points=weld.points.map(list=>[...list]),fresh=new Map<number,number>();
  vertexPoint.set(weld.vertexPoint);
  for(const [old,copyIndex] of duplicate){
    const source=weld.vertexPoint[old]!;let target=fresh.get(source);
    if(target===undefined){target=points.length;points.push([]);fresh.set(source,target);}
    points[target]!.push(copyIndex);vertexPoint[copyIndex]=target;
  }
  return {mesh:next,vertices:[...duplicate.values()],normal:normal.normalize(),weld:{vertexPoint,points,key:topologyKey(next)}};
}
