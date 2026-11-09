import {expect,it} from 'vitest';
import * as THREE from 'three';
import {prepareEnvironment,type TerrainPart,type ScatterPart,type ScatterJob} from './environment-preparation';
import {generateScatter,instanceMatrix,terrainMeshes} from '../studio/environment-geometry';
import {geometryPayload} from '../studio/gather';
it('worker terrain retains the seeded positions, normals, colors and triangle order',()=>{
 const settings={size:[12,12] as [number,number],resolution:16,amplitude:2,frequency:.1,octaves:3,seed:41,ridged:false,heights:[],material:0,layers:[],erosion:3,natural:true,detail:.01,blend:.1,adaptive:false,water_level:-2,snow_level:2},palette=[{x:.3,y:.4,z:.2}],materials=[{roughness:.8,metallic:0}];
 const expected=terrainMeshes(settings,palette,materials),actual=prepareEnvironment({kind:'terrain',settings,palette,materials}) as TerrainPart[];
 expect(actual).toHaveLength(expected.length);
 actual.forEach((part,i)=>{const e=expected[i]!.mesh;expect(part.positions).toEqual(new Float32Array(e.positions.flatMap(v=>[v.x,v.y,v.z])));expect(part.normals).toEqual(new Float32Array(e.normals.flatMap(v=>[v.x,v.y,v.z])));expect(part.indices).toEqual(new Uint32Array(e.indices.flat()));expect(part.colors).toEqual(new Float32Array(e.colors!.flatMap(v=>[v.x,v.y,v.z])));});
});
it('worker scatter retains exact matrices with parent, prototype rotation and scale',()=>{
 const g=new THREE.PlaneGeometry(10,10,2,2);g.rotateX(-Math.PI/2);const identity=new THREE.Matrix4(),inverse=new THREE.Matrix4().makeTranslation(-2,-1,3),rotation=new THREE.Matrix4().makeRotationY(.4),relative=new THREE.Matrix4().makeTranslation(0,.3,0),scale=new THREE.Vector3(1.1,.9,1.2);
 const settings={objects:[0],surface:0,count:180,density:1,seed:71,scale:[.8,1.2],rotation:[0,360],min_height:-10,max_height:10,min_slope:0,max_slope:90};
 const input:ScatterJob={kind:'scatter',settings,surfaces:[{geometry:geometryPayload(g,[],0),matrix:identity.toArray()}],inverse:inverse.toArray(),rotation:rotation.toArray(),scale:scale.toArray(),prototypes:[{relative:relative.toArray()}],diameter:0,camera:[0,2,8]};
 const pos=g.getAttribute('position'),normal=g.getAttribute('normal'),mesh={positions:Array.from({length:pos.count},(_,i)=>({x:pos.getX(i),y:pos.getY(i),z:pos.getZ(i)})),normals:Array.from({length:normal.count},(_,i)=>({x:normal.getX(i),y:normal.getY(i),z:normal.getZ(i)})),uvs:Array.from({length:pos.count},()=>[0,0]),indices:Array.from({length:g.index!.count/3},(_,i)=>[0,1,2].map(j=>g.index!.getX(i*3+j)))};
 const placements=generateScatter(settings as any,{objects:[{mesh,material:0}]} as any),expected=new Float32Array(placements.length*16);
 placements.forEach((p,i)=>inverse.clone().multiply(instanceMatrix(p).multiply(rotation).scale(scale).multiply(relative)).toArray(expected,i*16));
 const actual=prepareEnvironment(input) as ScatterPart[];expect(actual[0]!.matrices).toEqual(expected);g.dispose();
});
