import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {meshFromGeometry,weldMesh,meshEdges,transformPoints,deletePoints,mergePoints,extrudeRegion,pointPosition} from './editmesh';

const box=()=>meshFromGeometry(new THREE.BoxGeometry(1,1,1));
const top=(mesh:ReturnType<typeof box>,weld:ReturnType<typeof weldMesh>)=>new Set(weld.points.map((_,p)=>p).filter(p=>pointPosition(mesh,weld,p).y>.4));

describe('Edit Mode meshes',()=>{
  it('welds a box’s split vertices into eight points and twelve edges plus face diagonals',()=>{
    const mesh=box(),weld=weldMesh(mesh);
    expect(mesh.positions.length/3).toBe(24);expect(weld.points).toHaveLength(8);
    expect(weld.points.every(list=>list.length===3)).toBe(true);
    expect(meshEdges(mesh,weld)).toHaveLength(18);
  });
  it('moves every split copy of a point together and keeps UVs',()=>{
    const mesh=box(),weld=weldMesh(mesh),selected=top(mesh,weld);
    const moved=transformPoints(mesh,weld,selected,new THREE.Matrix4().makeTranslation(0,1,0));
    const after=weldMesh(moved);expect(after.points).toHaveLength(8);
    expect(Math.max(...after.points.map((_,p)=>pointPosition(moved,after,p).y))).toBeCloseTo(1.5);
    expect(moved.uvs).toEqual(mesh.uvs);
  });
  it('deletes points with their faces and merges points to their centre',()=>{
    const mesh=box(),weld=weldMesh(mesh),selected=top(mesh,weld);
    const deleted=deletePoints(mesh,weld,new Set([[...selected][0]!]));
    expect(weldMesh(deleted).points).toHaveLength(7);expect(deleted.indices.length).toBe(36-4*3);
    const merged=mergePoints(mesh,weld,selected),w=weldMesh(merged);
    expect(w.points).toHaveLength(5);expect(merged.indices.length).toBe(36-6-12);
  });
  it('extrudes the top face upward as a region with four side quads',()=>{
    const mesh=box(),weld=weldMesh(mesh),selected=top(mesh,weld);
    const result=extrudeRegion(mesh,weld,selected)!;
    expect(result.normal.y).toBeCloseTo(1);
    expect(result.mesh.indices.length).toBe(36+4*6);
    const raised=transformPoints(result.mesh,weldMesh(result.mesh),new Set(),new THREE.Matrix4());
    expect(raised.positions.length).toBeGreaterThan(mesh.positions.length);
    expect(result.weld.points).toHaveLength(12);expect(weldMesh(result.mesh).points).toHaveLength(8);
    // Moving only the new vertices leaves a closed box one unit taller.
    const moved=result.mesh.positions.slice();for(const v of result.vertices)moved[v*3+1]=moved[v*3+1]!+1;
    expect(weldMesh({...result.mesh,positions:moved}).points).toHaveLength(12);
  });
  it('extrudes selected edges into quads when no face is fully selected',()=>{
    const mesh=meshFromGeometry(new THREE.PlaneGeometry(1,1)),weld=weldMesh(mesh);
    const right=new Set(weld.points.map((_,p)=>p).filter(p=>pointPosition(mesh,weld,p).x>.4));
    const result=extrudeRegion(mesh,weld,right)!;expect(result.mesh.indices.length).toBe(6+6);expect(result.vertices).toHaveLength(2);
    expect(extrudeRegion(mesh,weld,new Set([[...right][0]!]))).toBeNull();
  });
});
