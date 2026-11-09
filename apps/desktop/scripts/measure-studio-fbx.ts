/** Reproducible CPU skin + BVH preparation measurement; outputs stay outside Git. */
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import * as THREE from 'three';
import { StudioHostManager } from '../src/main/studio-native/manager';
import { loadGltf, disposeModel } from '../src/renderer/src/core/scene3d/import-model';
import { gather } from '../src/renderer/src/core/studio/gather';
import { finishBlas } from '../src/renderer/src/core/studio/blas';

const directory='/tmp/pm-shots/studio3d/laneK';
const host=new StudioHostManager({executable:path.join(process.env.CARGO_TARGET_DIR ?? path.join(homedir(),'Developer/pm-target'),'release/pm-studio-host')});
try {
  const bytes=await readFile(new URL('../../../engine/hosts/studio-electron/fixtures/tiny-animated.fbx',import.meta.url));
  const reply=await host.request({op:'importFbx'},new Uint8Array(bytes));
  const loaded=await loadGltf(new Blob([new Uint8Array(reply.binary)]));
  let mesh!:THREE.SkinnedMesh;
  loaded.object3d.traverse(object=>{if((object as THREE.SkinnedMesh).isSkinnedMesh)mesh=object as THREE.SkinnedMesh;});
  const source=mesh.geometry,geometry=new THREE.BufferGeometry(),copies=10_000;
  for(const [name,attribute] of Object.entries(source.attributes)) {
    const array=new Float32Array(attribute.array.length*copies);
    for(let copy=0;copy<copies;copy++){
      array.set(attribute.array,copy*attribute.array.length);
      if(name==='position')for(let vertex=0;vertex<3;vertex++){array[copy*9+vertex*3]!+=(copy%100)*2.1;array[copy*9+vertex*3+1]!+=Math.floor(copy/100)*1.1;}
    }
    geometry.setAttribute(name,new THREE.BufferAttribute(array,attribute.itemSize));
  }
  mesh.geometry=geometry;
  const mixer=new THREE.AnimationMixer(loaded.object3d);mixer.clipAction(loaded.animations[0]!).play();
  const samples:number[]=[];
  for(let frame=0;frame<65;frame++){
    const start=performance.now();mixer.setTime(frame/30);loaded.object3d.updateMatrixWorld(true);mesh.skeleton.update();
    const data=gather(mesh,geometry,[0],0);finishBlas(data.corners,data.shading);
    if(frame>=5)samples.push(performance.now()-start);
  }
  samples.sort((a,b)=>a-b);
  await mkdir(directory,{recursive:true});
  await writeFile(`${directory}/fbx-skin-metrics.json`,JSON.stringify({import:reply.result,triangles:copies,vertices:copies*3,frames:samples.length,medianMs:samples[Math.floor(samples.length/2)],p95Ms:samples[Math.floor(samples.length*.95)],path:'CPU skin evaluation and per-pose BVH build, excluding render/IPC'},null,2));
  mesh.geometry=source;geometry.dispose();disposeModel(loaded.object3d);
} finally {host.dispose();}
