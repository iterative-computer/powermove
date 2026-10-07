import * as THREE from 'three';
import {animatedPart} from './recipe-preview';
import { world3D,parent3D,depthOrderedLayers } from '../../legacy/core/space-3d';
import { SceneRuntime } from './runtime';
import { createScene,parseScene,SCENE3D_DEFINITION,type Scene3D } from './schema';
import { compositionScene, layer3DRole,layer3DAssetIds,isModelGroup } from './layers';

const runtimes = new WeakMap<object,Map<string,SceneRuntime>>();
const legacyScenes = new WeakMap<object,{signature:string;scene:Scene3D}>();
const compositionCache=new WeakMap<SceneRuntime,{comp:any;time:number;revision:number;assets:any;references:Map<string,any>}>();
export function compositionRuntime(PM:any,time=PM.time || 0,composition?:any):SceneRuntime {
  let pool=runtimes.get(PM);if(!pool){pool=new Map();runtimes.set(PM,pool);}
  const comp=composition || PM._renderComposition3D || PM.curComp?.() || PM.proj,key=`composition:${comp.id || 'root'}`;
  let runtime=pool.get(key);if(!runtime){runtime=new SceneRuntime();pool.set(key,runtime);}
  const cached=compositionCache.get(runtime),revision=PM._scene3dRevision || 0;
  const references=new Map<string,any>(comp.layers.flatMap((l:any)=>layer3DAssetIds(l)).map((id:string)=>[id,PM.assets?.get?.(id)]));
  const assetsChanged=!cached || cached.references.size!==references.size || [...references].some(([id,asset])=>cached.references.get(id)!==asset);
  if(assetsChanged || !cached || cached.comp!==comp || cached.time!==time || cached.revision!==revision || cached.assets!==PM.proj.assets){
    const starts=new Map<string,number>(comp.layers.map((l:any)=>[l.id,l.from]));
    runtime.sync(compositionScene(PM,time,comp),(id:string)=>PM.assets?.get?.(id),prop=>prop.v,time,true,id=>time-(starts.get(id) || 0));
    const models=comp.layers.filter((l:any)=>layer3DRole(l)==='object');
    // Use exactly the native layer hierarchy, orientation and anchor matrices.
    // Flattening the evaluated world avoids applying parent transforms twice.
    for(const l of models){const object=runtime.objects.get(l.id);if(!object)continue;
      runtime.scene.add(object);object.matrixAutoUpdate=false;object.matrix.fromArray(world3D(PM,l,time));
      const part=animatedPart(PM,l,time);if(part){const delta=new THREE.Vector3(part.base.x!-part.prior.x!,part.base.y!-part.prior.y!,part.base.z!-part.prior.z!);const parent=new THREE.Matrix4().fromArray(parent3D(PM,l,time));const origin=new THREE.Vector3().applyMatrix4(parent);delta.applyMatrix4(parent).sub(origin);object.matrix.elements[12]!+=delta.x;object.matrix.elements[13]!+=delta.y;object.matrix.elements[14]!+=delta.z;}

      object.matrix.decompose(object.position,object.quaternion,object.scale);object.rotation.order='ZYX';
      object.userData.layerOpacity=PM.worldOpacity?PM.worldOpacity(l,time):1;
      object.userData.layerOccludes=!(l.masks || []).some((mask:any)=>typeof mask.on==='object'?PM.evP(l,mask.on,time,`m.${mask.id}.on`)!==false:mask.on!==false);
    }
    for(const l of comp.layers.filter((l:any)=>layer3DRole(l)==='light' || layer3DRole(l)==='camera')){
      const role=layer3DRole(l),node=role==='light'?runtime.objects.get(l.id):comp.layers.find((c:any)=>layer3DRole(c)==='camera' && PM.active(c,time))===l?runtime.camera:null;
      if(!node || !role)continue;
      node.position.setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,l,time)));
      const p=l.d.data[role].p,ev=(key:string)=>PM.evP(l,p[key],time,`${role}.${key}`);
      const parent=new THREE.Matrix4().fromArray(parent3D(PM,l,time));
      const target=new THREE.Vector3(ev('targetX'),ev('targetY'),ev('targetZ')).applyMatrix4(parent);
      if(role==='camera'){node.up.set(0,1,0).transformDirection(new THREE.Matrix4().fromArray(world3D(PM,l,time)));node.lookAt(target);node.updateMatrixWorld(true);}
      else if((node as THREE.DirectionalLight).target)(node as THREE.DirectionalLight).target.position.copy(target);
      else if((node as THREE.RectAreaLight).isRectAreaLight)node.lookAt(target);
    }
    runtime.scene.updateMatrixWorld(true);
    compositionCache.set(runtime,{comp,time,revision,assets:PM.proj.assets,references});
  }
  runtime.resizeCamera(comp.w,comp.h);
  return runtime;
}
/** 2D layers remain compositing barriers between independent model depth groups. */
export function compositionDepthIds(PM:any,layer:any,time:number):Set<string> {
  // A flattened render pass retains styled group boundaries; plain group
  // headers are organizational and must never split the shared depth buffer.
  const source=(PM._renderComposition3D || PM.curComp?.() || PM.proj).layers;
  const layers=PM._renderComposition3DPass || source.filter((l:any)=>l.type!=='group');
  const index=layers.indexOf(layer),ids=new Set<string>(),solo=source.some((l:any)=>l.solo);
  const visit=(i:number)=>{const l=layers[i];if(!l)return false;if(!PM.active(l,time))return true;
    const role=layer3DRole(l);if(!role)return false;
    if(role==='object' && (!solo || l.solo || (PM.groupAncestors?.(l,layers) || []).some((g:any)=>g.solo)) && (!PM.worldOpacity || PM.worldOpacity(l,time)>.001))ids.add(l.id);
    return true;};
  for(let i=index;i>=0 && visit(i);i--){}
  for(let i=index+1;i<layers.length && visit(i);i++){}
  return ids;
}
/** Real geometry bounds in the group's local world-unit space, not projected pixels. */
export function modelGroupBounds(PM:any,group:any,time:number):any {
  if(!isModelGroup(PM,group))return null;
  const runtime=compositionRuntime(PM,time),box=new THREE.Box3();
  const inverse=new THREE.Matrix4().fromArray(world3D(PM,group,time)).invert();
  for(const child of (PM.curComp?.() || PM.proj).layers){
    if(!(PM.groupAncestors?.(child) || []).some((g:any)=>g.id===group.id) || !PM.active(child,time))continue;
    const object=layer3DRole(child)==='object'?runtime.objects.get(child.id):null;
    if(object)box.union(new THREE.Box3().setFromObject(object).applyMatrix4(inverse));
    else if(layer3DRole(child))box.expandByPoint(new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,child,time))).applyMatrix4(inverse));
  }
  if(box.isEmpty())return null;
  return {x0:box.min.x,y0:box.min.y,z0:box.min.z,x1:box.max.x,y1:box.max.y,z1:box.max.z,
    w:box.max.x-box.min.x,h:box.max.y-box.min.y,ax:0,ay:0};
}
/** Transparent models sort in the shared camera's depth, keeping every 2D stack boundary. */
export function compositionOrderedLayers(PM:any,layers:any[],time:number,camera?:THREE.Camera):any[] {
  if(!layers.some(layer3DRole))return depthOrderedLayers(PM,layers,time);
  const world=compositionRuntime(PM,time),result:any[]=[],position=new THREE.Vector3();
  for(let start=0;start<layers.length;){
    const modelBlock=!!layer3DRole(layers[start]);let end=start+1;
    while(end<layers.length && !!layer3DRole(layers[end])===modelBlock)end++;
    const block=layers.slice(start,end);
    if(modelBlock){
      const depth=(l:any)=>{const object=world.objects.get(l.id);return object?object.getWorldPosition(position).applyMatrix4((camera||world.camera).matrixWorldInverse).z:0;};
      block.sort((a,b)=>depth(b)-depth(a));result.push(...block);
    }else result.push(...depthOrderedLayers(PM,block,time));
    start=end;
  }
  return result;
}
export function sceneForLayer(PM:any,layer:any,time:number):Scene3D|null {
  if(layer.d?.definition===SCENE3D_DEFINITION)return layer.d.data?.scene || null;
  const definition=PM.layerDefinition?.(layer.d?.definition);
  if(definition?.renderer?.kind!=='mesh')return null;
  const get=(key:string,fallback:any)=> {
    const prop=layer.d.params?.[key];return prop?PM.evP(layer,prop,time,`x.${key}`):fallback;
  };
  const values={yaw:get('yaw',28),pitch:get('pitch',18),distance:get('distance',4.2),
    rx:get('rotationX',0),ry:get('rotationY',25),rz:get('rotationZ',0),size:get('size',1.35),
    roughness:get('roughness',.25),metalness:get('metalness',.7),color:get('objectColor','#C7C4FF'),
    lightColor:get('lightColor','#FFB36B'),background:get('background','#0C0D12'),
    assetId:layer.d.data?.[definition.renderer.assetField],autoRotate:get('autoRotate',false),speed:get('speed',25)};
  if(values.autoRotate)values.ry+=(time-layer.from)*values.speed;
  const signature=JSON.stringify(values),cached=legacyScenes.get(layer);
  if(cached?.signature===signature)return cached.scene;
  if(!values.assetId)return null;
  const yaw=values.yaw*Math.PI/180,pitch=values.pitch*Math.PI/180,dist=values.distance;
  const scene=parseScene({ ...createScene(),objects:[{id:'model',name:layer.name,source:{assetId:values.assetId},
    p:{rx:values.rx,ry:values.ry,rz:values.rz,sx:values.size,sy:values.size,sz:values.size},
    material:{p:{color:values.color,roughness:values.roughness,metalness:values.metalness}}}],
    lights:[{id:'key',name:'Key',type:'sun',p:{color:values.lightColor}}],
    camera:{p:{x:dist*Math.cos(pitch)*Math.sin(yaw),y:dist*Math.sin(pitch),z:dist*Math.cos(pitch)*Math.cos(yaw),fov:58.109208}},
    environment:{background:values.background,p:{ambient:.4}} });
  legacyScenes.set(layer,{signature,scene});return scene;
}
export function sceneRuntime(PM:any,layer:any,time=PM.time || 0):SceneRuntime|null {
  const scene=sceneForLayer(PM,layer,time);if(!scene)return null;
  let pool=runtimes.get(PM);if(!pool){pool=new Map();runtimes.set(PM,pool);}
  let runtime=pool.get(layer.id);if(!runtime){runtime=new SceneRuntime();pool.set(layer.id,runtime);}
  // Retain geometry across edits and frame changes; refresh only evaluated channels.
  runtime.sync(scene,(id:string)=>PM.assets?.get?.(id), (prop:any,path:string)=>PM.evP?PM.evP(layer,prop,time,path):prop.v,time-layer.from);
  return runtime;
}
export function pruneSceneRuntimes(PM:any):void {
  const pool=runtimes.get(PM);if(!pool)return;
  const all=PM.ProjectIndex?.allLayers?.() || [...(PM.proj?.layers || []),...Object.values<any>(PM.proj?.comps || {}).flatMap(c=>c.layers || [])];
  const keep=new Set(all.filter((l:any)=>l.type==='extension').map((l:any)=>l.id));
  for(const [id,runtime] of pool)if(!id.startsWith('composition:')&&!keep.has(id)){runtime.dispose();pool.delete(id);}
}
export function disposeSceneRuntimes(PM:any):void {
  const pool=runtimes.get(PM);for(const runtime of pool?.values() || [])runtime.dispose();pool?.clear();
}
