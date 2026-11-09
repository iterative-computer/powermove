import * as THREE from 'three';
import { channelValues } from './environment-data';
import { terrainMeshes,generateScatter,instanceMatrix,bakeTerrain,terrainResolution } from '../studio/environment-geometry';
import { packWorld,worldSun,type StudioWorld } from '../studio/environment';
import {selectNatureLod} from '../studio/nature';
import type { StudioScene } from '../studio/extract';
import type { Scene3D,SceneObject } from './schema';
import {prepareEnvironmentInWorker} from './environment-worker';
import {prepareEnvironment,type TerrainJob,type TerrainPart,type ScatterPart,type ScatterJob} from './environment-preparation';
import {geometryPayload} from '../studio/gather';
import {reportLayerFailure} from './layer-diagnostic';

const rgb=(color:string)=>{const c=new THREE.Color(color);return {x:c.r,y:c.g,z:c.b};};
const imageCache=new WeakMap<object,ReturnType<typeof environmentImage>>();
const worldPixelCache=new WeakMap<object,{width:number;height:number;pixels:{x:number;y:number;z:number}[]}>();
const sourceIdentities=new WeakMap<object,number>();let nextSourceIdentity=1;
export function environmentImageKey(image:ReturnType<typeof environmentImage>):unknown{const key=image?.key;if(!key || typeof key!=='object')return key;if(!sourceIdentities.has(key))sourceIdentities.set(key,nextSourceIdentity++);return sourceIdentities.get(key);}
const xyz=(v:THREE.Vector3)=>({x:v.x,y:v.y,z:v.z});
/** Convert durable image/layer/comp sources before crossing the render boundary. */
export function environmentImage(source:any,id:string,assets:(id:string)=>any,composition:(id:string,source:any)=>any):{width:number;height:number;pixels:any;linear:boolean;key:any}|null{
  if(!source)return null;
  if(typeof source==='object'){
    const frame=composition(id,source);return frame && frame!=='loading'?{width:frame.width,height:frame.height,pixels:frame.pixels,linear:false,key:frame.key}:null;
  }
  const asset=assets(source);
  if(asset?.environmentMap){const m=asset.environmentMap;return {width:m.width,height:m.height,pixels:m.pixels,linear:true,key:m};}
  const image=asset?.bitmap || asset?.el;
  if(!image)return null;
  const cached=imageCache.get(image);if(cached)return cached;
  const width=image.width || image.videoWidth,height=image.height || image.videoHeight;
  if(!width || !height)return null;
  if(image.data)return {width,height,pixels:image.data,linear:image.data instanceof Float32Array,key:image};
  const canvas=new OffscreenCanvas(Math.min(2048,width),Math.min(1024,height));
  canvas.getContext('2d')!.drawImage(image,0,0,canvas.width,canvas.height);
  const result={width:canvas.width,height:canvas.height,pixels:canvas.getContext('2d')!.getImageData(0,0,canvas.width,canvas.height).data,linear:false,key:image};imageCache.set(image,result);return result;
}
const linear=(v:number)=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4;
export function worldForNode(node:any,image:ReturnType<typeof environmentImage>):StudioWorld{
  const p=channelValues(node.p),world:StudioWorld={};
  if(node.mode==='sky')world.sky={elevation:p.elevation,azimuth:p.azimuth+p.rotation,turbidity:p.haze,ground_albedo:rgb(p.groundColor ?? '#7C7C7C'),intensity:p.intensity,sun_intensity:p.sunIntensity,sun_radius:p.sunRadius,overcast:p.overcast,visible:p.visible};
  if(node.mode==='hdri' && image){
    let mapped=worldPixelCache.get(image.pixels);
    if(!mapped){
      const width=Math.min(2048,image.width),height=Math.min(1024,image.height),pixels=[];
      if(image.linear && typeof image.pixels[0]==='object' && width===image.width && height===image.height)mapped={width,height,pixels:image.pixels};
      else{
        const stride=image.pixels.length/(image.width*image.height);
        for(let y=0;y<height;y++)for(let x=0;x<width;x++){
          const at=(Math.floor(y*image.height/height)*image.width+Math.floor(x*image.width/width))*stride;
          const read=(k:number)=>image.linear?image.pixels[at+k]:linear(image.pixels[at+k]/255);
          pixels.push({x:read(0),y:read(1),z:read(2)});
        }
        mapped={width,height,pixels};
      }
      worldPixelCache.set(image.pixels,mapped);
    }
    const {width,height,pixels}=mapped;
    world.hdri={width,height,pixels,intensity:p.intensity,rotation:p.rotation,visible:p.visible};
  }
  world.atmosphere={density:p.fogDensity,color:rgb(p.fogColor),anisotropy:p.fogAnisotropy,base_height:p.fogBaseHeight,height_falloff:p.fogHeightFalloff,distance:p.fogDistance};
  world.adaptive={enabled:p.adaptive,min_samples:Math.round(p.minSamples),relative_error:p.relativeError,absolute_error:p.absoluteError};
  return world;
}
type TerrainSettings={size:[number,number];resolution:number;amplitude:number;frequency:number;octaves:number;seed:number;ridged:boolean;detail:number;erosion:number;adaptive:boolean;blend:number;natural:boolean;water_level:number;snow_level:number;heights:number[];material:number;layers:{material:number;min_height:number;max_height:number;min_slope:number;max_slope:number}[];center?:{x:number;y:number;z:number}};
const terrainSettingsCache=new WeakMap<object,{key:string;heights:any;result:any}>();
export function terrainSettings(node:any,image:ReturnType<typeof environmentImage>,camera?:{position:{x:number;y:number;z:number};fov:number},defer=false){
  const p=channelValues(node.p),resolution=Math.round(p.resolution),heights:number[]=[];
  const selected=terrainResolution({size:[p.width,p.depth],resolution,adaptive:p.adaptive},camera);
  const key=JSON.stringify([p,node.regions,environmentImageKey(image),selected,defer]);
  const cached=terrainSettingsCache.get(node);if(cached?.key===key && cached.heights===node.heights)return cached.result as TerrainSettings;
  if(image){
    const stride=image.pixels.length/(image.width*image.height);
    for(let y=0;y<=resolution;y++)for(let x=0;x<=resolution;x++){
      const at=(Math.min(image.height-1,Math.round(y/resolution*(image.height-1)))*image.width+Math.min(image.width-1,Math.round(x/resolution*(image.width-1))))*stride;
      const read=(k:number)=>typeof image.pixels[at]==='object'?image.pixels[at][['x','y','z'][k]!]:image.pixels[at+k]/(image.linear?1:255);
      heights.push(Math.max(0,Math.min(1,.2126*read(0)+.7152*read(1)+.0722*read(2))));
    }
  }
  const settings={size:[p.width,p.depth] as [number,number],resolution,amplitude:p.amplitude,frequency:p.frequency,octaves:Math.round(p.octaves),seed:Math.round(p.seed),ridged:p.ridged,detail:p.detail??0,erosion:Math.round(p.erosion??0),adaptive:p.adaptive??false,blend:p.blend??0,natural:p.natural??false,water_level:p.waterLevel??-100000,snow_level:p.snowLevel??100000,heights:image?heights:node.heights,
    material:0,layers:node.regions.map((region:any,i:number)=>{const p=channelValues(region.p);return {material:i+1,min_height:p.minHeight,max_height:p.maxHeight,min_slope:p.minSlope,max_slope:p.maxSlope};})};
  const result=(defer?(settings.adaptive||settings.erosion?{...settings,bake_resolution:selected}:settings):settings.adaptive || settings.erosion?bakeTerrain(settings,selected):settings) as TerrainSettings;
  terrainSettingsCache.set(node,{key,heights:node.heights,result});return result;
}
export function terrainJob(node:SceneObject,settings:ReturnType<typeof terrainSettings>):TerrainJob{
  const palette=[rgb(node.material.p.color!.v),...node.terrain!.regions.map(r=>rgb(channelValues(r.p).color))];
  return {kind:'terrain',settings,palette,materials:[{roughness:node.material.p.roughness!.v,metallic:node.material.p.metalness!.v},...node.terrain!.regions.map(r=>{const p=channelValues(r.p);return {roughness:p.roughness,metallic:p.metalness};})]};
}
export function makeTerrain(node:SceneObject,settings:ReturnType<typeof terrainSettings>,prepared?:TerrainPart[]):THREE.Group{
  const group=new THREE.Group();
  for(const part of prepared??prepareEnvironment(terrainJob(node,settings)) as TerrainPart[]){
    const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.BufferAttribute(part.positions,3));
    g.setAttribute('normal',new THREE.BufferAttribute(part.normals,3));g.setAttribute('uv',new THREE.BufferAttribute(part.uvs,2));if(part.colors)g.setAttribute('color',new THREE.BufferAttribute(part.colors,3));g.setIndex(new THREE.BufferAttribute(part.indices,1));
    const material=new THREE.MeshPhysicalMaterial(),region=node.terrain!.regions[part.material-1];
    if(region){const p=channelValues(region.p);material.color.set(p.color);material.roughness=p.roughness;material.metalness=p.metalness;}
    if(part.colors){material.color.setRGB(1,1,1);material.vertexColors=true;material.metalness=0;material.roughness=.88;material.userData.studioProcedural=1;}
    material.userData.sourceMaterialIndex=part.material;
    const mesh=new THREE.Mesh(g,material);mesh.userData.studioNaturalGround=!!part.colors;mesh.userData.terrainRegion=region?channelValues(region.p):null;mesh.userData.sceneId=node.id;mesh.castShadow=node.castShadow;mesh.receiveShadow=node.receiveShadow;group.add(mesh);
  }
  return group;
}
export function clearScatterChildren(object:THREE.Object3D):void{for(const child of [...object.children]){object.remove(child);(child as any).dispose?.();if(child.userData.studioOwnsMaterial)for(const material of Array.isArray((child as any).material)?(child as any).material:[(child as any).material])material.dispose();if(child.userData.studioOwnsGeometry)(child as any).geometry.dispose();}}
function prototypeMeshes(root:THREE.Object3D,visit:(mesh:any)=>void):void{const walk=(object:THREE.Object3D,visible:boolean)=>{const show=visible && (object===root || object.visible);if(!show)return;visit(object);for(const child of object.children)walk(child,show);};walk(root,true);}
function prepareScatter(runtime:any,node:SceneObject,object:THREE.Object3D,source:THREE.Object3D,surface:THREE.Object3D,p:any,signature:string):boolean{
 if(typeof Worker==='undefined')return false;
 const surfaces:THREE.Mesh[]=[],prototypes:THREE.Mesh[]=[];
 prototypeMeshes(surface,(mesh:any)=>{if(mesh.isMesh)surfaces.push(mesh);});
 // Posed surfaces keep the established evaluated-vertex path until their
 // pose buffers are available to workers; never scatter over a rest pose.
 if(surfaces.some((mesh:any)=>mesh.isSkinnedMesh||mesh.morphTargetInfluences?.some((v:number)=>v!==0)))return false;
 if(source.userData.studioNature){for(const child of source.children)if((child as any).isMesh)prototypes.push(child as THREE.Mesh);}
 else prototypeMeshes(source,(mesh:any)=>{if(mesh.isMesh)prototypes.push(mesh);});
 const inverse=object.matrixWorld.clone().invert(),sourceInverse=source.matrixWorld.clone().invert(),scale=source.getWorldScale(new THREE.Vector3());
 let diameter=0;
 if(source.userData.studioNature&&prototypes[0]){const g=prototypes[0].geometry;if(!g.boundingBox)g.computeBoundingBox();const size=g.boundingBox!.getSize(new THREE.Vector3());diameter=Math.max(size.x,size.y,size.z)*Math.max(Math.abs(scale.x),Math.abs(scale.y),Math.abs(scale.z));}
 const input:ScatterJob={kind:'scatter',settings:{objects:[0],surface:0,count:Math.round(p.count),density:p.density,seed:Math.round(p.seed),scale:[p.scaleMin,p.scaleMax],rotation:[p.rotationMin,p.rotationMax],min_height:p.minHeight,max_height:p.maxHeight,min_slope:p.minSlope,max_slope:p.maxSlope},
  surfaces:surfaces.map(mesh=>({geometry:geometryPayload(mesh.geometry,[],0),matrix:mesh.matrixWorld.toArray()})),inverse:inverse.toArray(),rotation:new THREE.Matrix4().makeRotationFromQuaternion(source.getWorldQuaternion(new THREE.Quaternion())).toArray(),scale:scale.toArray(),
  prototypes:prototypes.map(mesh=>({relative:sourceInverse.clone().multiply(mesh.matrixWorld).toArray(),level:mesh.userData.studioNatureLevel})),diameter,camera:runtime.camera.position.toArray()};
 const generation=runtime.preparationGeneration;runtime.geometryPending++;
 void prepareEnvironmentInWorker<ScatterPart[]>(input,object).then(parts=>{
  if(runtime.objects.get(node.id)!==object||scatterCache.get(object)!==signature)return;
  clearScatterChildren(object);
  const options=node.materialOptions?channelValues(node.materialOptions.p):null;
  for(const part of parts){if(!part.matrices.length)continue;const mesh=prototypes[part.prototype]!;
   const material=options?(Array.isArray(mesh.material)?mesh.material.map((m:any)=>m.clone()):mesh.material.clone()):mesh.material;
   if(options)for(const m of Array.isArray(material)?material:[material]){m.userData.studioAcceptsLights=options.acceptsLights;m.userData.studioReflections=options.appearsInReflections;}
   const instanced=new THREE.InstancedMesh(mesh.geometry,material,part.matrices.length/16);instanced.instanceMatrix.array.set(part.matrices);instanced.instanceMatrix.needsUpdate=true;
   instanced.userData.sceneId=node.id;instanced.userData.studioSharedPrototype=true;instanced.userData.studioOwnsMaterial=!!options;instanced.userData.studioPrimaryVisible=options?.castsShadows!=='only';instanced.userData.studioPart=mesh.userData.studioPart;
   instanced.castShadow=options?options.castsShadows!=='off':node.castShadow&&mesh.castShadow;instanced.receiveShadow=options?options.acceptsShadows:node.receiveShadow&&mesh.receiveShadow;object.add(instanced);
  }
  runtime.scene.updateMatrixWorld(true);runtime.revision++;
 },error=>{if(runtime.objects.get(node.id)===object&&scatterCache.get(object)===signature)reportLayerFailure(runtime,node.id,'scatter preparation',error);})
 .finally(()=>{if(runtime.preparationGeneration===generation){runtime.geometryPending--;runtime.onGeometryReady();}});
 return true;
}
const scatterCache=new WeakMap<object,string>();
const scatterPlacements=new WeakMap<object,{signature:string;placements:ReturnType<typeof generateScatter>}>();
/** Called after main has installed its world matrices, so surfaces and prototypes
 * share the same parent/anchor semantics as ordinary models. */
export function syncEnvironmentGraph(runtime:any,data:Scene3D,assets:(id:string)=>any,composition:(id:string,source:any)=>any):void{
  const worldNode=data.objects.find(o=>o.world),environmentLights=data.lights.filter(l=>l.type==='environment');
  const environment=worldNode?.world ?? environmentLights[0]?.world;
  const owner=worldNode?.id ?? environmentLights[0]?.id ?? '';
  const world:StudioWorld=environment?worldForNode(environment,environmentImage(environment.source,owner,assets,composition)):{};
  if(!worldNode && environmentLights[0]){const intensity=channelValues(environmentLights[0].p).intensity;if(world.sky)world.sky.intensity=(world.sky.intensity??1)*intensity;if(world.hdri)world.hdri.intensity=(world.hdri.intensity??1)*intensity;}
  const volumes=[];
  for(const node of data.objects){
    const object=runtime.objects.get(node.id);if(!object)continue;
    if(node.fog){
      const p=channelValues(node.fog.p),center=new THREE.Vector3().setFromMatrixPosition(object.matrixWorld),scale=new THREE.Vector3();object.matrixWorld.decompose(new THREE.Vector3(),new THREE.Quaternion(),scale);
      // The engine contract uses axis-aligned box half extents and sphere radius.
      const size=p.shape==='sphere'?new THREE.Vector3(p.width/2,p.width/2,p.width/2).multiplyScalar(Math.max(Math.abs(scale.x),Math.abs(scale.y),Math.abs(scale.z))):new THREE.Vector3(p.width/2,p.height/2,p.depth/2).multiply(scale);
      size.set(Math.abs(size.x),Math.abs(size.y),Math.abs(size.z));
      if(p.shape==='box'){const e=object.matrixWorld.elements;size.set(Math.abs(e[0])*p.width/2+Math.abs(e[4])*p.height/2+Math.abs(e[8])*p.depth/2,Math.abs(e[1])*p.width/2+Math.abs(e[5])*p.height/2+Math.abs(e[9])*p.depth/2,Math.abs(e[2])*p.width/2+Math.abs(e[6])*p.height/2+Math.abs(e[10])*p.depth/2);}
      volumes.push({shape:p.shape,center:xyz(center),size:xyz(size),density:p.density,color:rgb(p.color),anisotropy:p.anisotropy});
    }
    if(!node.scatter)continue;
    const source=runtime.objects.get(node.scatter.model),surface=runtime.objects.get(node.scatter.surface),p=channelValues(node.scatter.p);
    if(!source || !surface){clearScatterChildren(object);scatterCache.delete(object);scatterPlacements.delete(object);runtime.warnings.push('Scatter model or surface is unavailable');continue;}
    if(surface.userData.studioTerrainPending)continue;
    const geometryKey=(root:THREE.Object3D)=>{const keys:any[]=[];prototypeMeshes(root,(mesh:any)=>{if(mesh.isMesh)keys.push(mesh.geometry.uuid,mesh.geometry.getAttribute('position')?.version,mesh.geometry.index?.version,mesh.matrixWorld.elements,mesh.morphTargetInfluences,mesh.skeleton?Array.from(mesh.skeleton.boneMatrices):null,(Array.isArray(mesh.material)?mesh.material:[mesh.material]).map((m:any)=>m.uuid));});return keys;};
    const placementSignature=JSON.stringify([p,node.materialOptions,node.castShadow,node.receiveShadow,source.matrixWorld.elements,surface.matrixWorld.elements,source.uuid,surface.uuid,geometryKey(source),geometryKey(surface)]);
    const camera=runtime.camera.position as THREE.Vector3;
    const signature=placementSignature+(source.userData.studioNature?`:${Math.floor(camera.x/8)},${Math.floor(camera.y/8)},${Math.floor(camera.z/8)}`:'');
    if(scatterCache.get(object)===signature)continue;
    scatterCache.set(object,signature);
    if(prepareScatter(runtime,node,object,source,surface,p,signature))continue;
    clearScatterChildren(object);
    let cached=scatterPlacements.get(object);
    if(!cached || cached.signature!==placementSignature){
    const positions:any[]=[],normals:any[]=[],indices:any[]=[],uvs:any[]=[];
    prototypeMeshes(surface,(mesh:any)=>{if(!mesh.isMesh)return;const g=mesh.geometry,position=g.getAttribute('position'),normal=g.getAttribute('normal'),offset=positions.length;
      for(let i=0;i<position.count;i++){positions.push(xyz((mesh.getVertexPosition?mesh.getVertexPosition(i,new THREE.Vector3()):new THREE.Vector3().fromBufferAttribute(position,i)).applyMatrix4(mesh.matrixWorld)));normals.push(xyz(normal?new THREE.Vector3().fromBufferAttribute(normal,i).transformDirection(mesh.matrixWorld):new THREE.Vector3(0,1,0)));uvs.push([0,0]);}
      const index=g.getIndex();for(let i=0;i<(index?.count??position.count);i+=3)indices.push([0,1,2].map(j=>offset+(index?index.getX(i+j):i+j)));
    });
    const placements=generateScatter({objects:[0],surface:0,count:Math.round(p.count),density:p.density,seed:Math.round(p.seed),scale:[p.scaleMin,p.scaleMax],rotation:[p.rotationMin,p.rotationMax],min_height:p.minHeight,max_height:p.maxHeight,min_slope:p.minSlope,max_slope:p.maxSlope},{objects:[{mesh:{positions,normals,indices,uvs},material:0}]} as any);
    cached={signature:placementSignature,placements};scatterPlacements.set(object,cached);
    }
    const placements=cached.placements;
    const inverse=object.matrixWorld.clone().invert(),sourceInverse=source.matrixWorld.clone().invert();
    const prototypes:THREE.Mesh[]=[];
    if(source.userData.studioNature){for(const child of source.children)if((child as any).isMesh)prototypes.push(child as THREE.Mesh);}
    else prototypeMeshes(source,(mesh:any)=>{if(mesh.isMesh)prototypes.push(mesh);});
    // Partition transforms only. All levels retain their own resident BLAS.
    let selected:number[]=[];
    if(source.userData.studioNature){
      const g=prototypes[0]!.geometry;if(!g.boundingBox)g.computeBoundingBox();const size=g.boundingBox!.getSize(new THREE.Vector3()),scale=source.getWorldScale(new THREE.Vector3()),diameter=Math.max(size.x,size.y,size.z)*Math.max(Math.abs(scale.x),Math.abs(scale.y),Math.abs(scale.z));
      selected=placements.map(p=>selectNatureLod(diameter*(p.scale?.x??1),camera.distanceTo(new THREE.Vector3(p.position!.x,p.position!.y,p.position!.z))));
    }
    for(const mesh of prototypes){const level=mesh.userData.studioNatureLevel;
      const chosen=level===undefined?placements:placements.filter((_,i)=>selected[i]===level);if(!chosen.length)continue;
      const options=node.materialOptions?channelValues(node.materialOptions.p):null;
      const material=options?(Array.isArray(mesh.material)?mesh.material.map((m:any)=>m.clone()):mesh.material.clone()):mesh.material;
      if(options)for(const m of Array.isArray(material)?material:[material]){m.userData.studioAcceptsLights=options.acceptsLights;m.userData.studioReflections=options.appearsInReflections;}
      const instanced=new THREE.InstancedMesh(mesh.geometry,material,chosen.length);
      instanced.userData.sceneId=node.id;instanced.userData.studioSharedPrototype=true;instanced.userData.studioOwnsMaterial=!!options;instanced.userData.studioPrimaryVisible=options?.castsShadows!=='only';instanced.userData.studioPart=mesh.userData.studioPart;instanced.castShadow=options?options.castsShadows!=='off':node.castShadow && mesh.castShadow;instanced.receiveShadow=options?options.acceptsShadows:node.receiveShadow && mesh.receiveShadow;
      const relative=sourceInverse.clone().multiply(mesh.matrixWorld),sourceScale=source.getWorldScale(new THREE.Vector3()),sourceRotation=new THREE.Matrix4().makeRotationFromQuaternion(source.getWorldQuaternion(new THREE.Quaternion()));
      chosen.forEach((placement,i)=>{const m=instanceMatrix(placement).multiply(sourceRotation).scale(sourceScale).multiply(relative);instanced.setMatrixAt(i,inverse.clone().multiply(m));});
      instanced.instanceMatrix.needsUpdate=true;object.add(instanced);
    }
  }
  if(volumes.length)world.volumes=volumes;
  runtime.studioWorld=environment || volumes.length?world:undefined;
  runtime.metersPerUnit=environment?channelValues(environment.p).metersPerUnit:.1;
  runtime.scene.updateMatrixWorld(true);
}
const worldTrailers=new Map<string,Float32Array>();
const worldBases=new WeakMap<StudioScene,{sdata:Float32Array;lightCount:number;key?:string}>();
const pixelIds=new WeakMap<object,number>();let nextPixelId=1;
/** Append exactly the same world/light trailer consumed by both native and WebGPU. */
export function attachStudioWorld(scene:StudioScene,world:StudioWorld|undefined,metersPerUnit=.1):void{
  let base=worldBases.get(scene);if(!base){base={sdata:scene.sdata,lightCount:scene.lightCount};worldBases.set(scene,base);}
  scene.metersPerUnit=metersPerUnit;scene.world=world;
  if(!world){scene.sdata=base.sdata;scene.lightCount=base.lightCount;base.key=undefined;return;}
  const lights=Array.from(base.sdata.subarray(0,base.lightCount*16));lights.push(...worldSun(world));
  const pixels=world.hdri?.pixels;if(pixels && !pixelIds.has(pixels))pixelIds.set(pixels,nextPixelId++);
  const key=JSON.stringify([{...world,hdri:world.hdri?{...world.hdri,pixels:pixelIds.get(pixels!)}:null},lights]);
  if(base.key===key)return;base.key=key;
  let trailer=worldTrailers.get(key);if(!trailer){trailer=packWorld(world,lights);worldTrailers.set(key,trailer);while(worldTrailers.size>4 || [...worldTrailers.values()].reduce((n,v)=>n+v.byteLength,0)>96*1024*1024){const oldest=worldTrailers.keys().next().value!;if(oldest===key)break;worldTrailers.delete(oldest);}}
  const instances=base.sdata.subarray(base.lightCount*16,base.lightCount*16+scene.instanceCount*36);
  const packed=new Float32Array(lights.length+instances.length+trailer.length);packed.set(lights);packed.set(instances,lights.length);packed.set(trailer,lights.length+instances.length);
  scene.sdata=packed;scene.lightCount=lights.length/16;
}
