import { buildExtrusionGeometry } from './extrusion-geometry';
import { EXTRUSION_SURFACES } from './extrusion-data';
import type { ExtrusionMeshSample } from './extrusion-render';
import { applyMaterialProperties } from './material-properties';
import { applyParts, captureRestPose, restoreParts } from './model-parts';
import * as THREE from 'three';
import { clone as cloneSkinned } from 'three/addons/utils/SkeletonUtils.js';
import { type MaterialMap } from './material-maps';
import type { CompositionTextureFrame } from './composition-textures';
import { projectedMaterialSize, type MaterialTextureSize } from './material-resolution';
import { parseScene, type Scene3D, type SceneObject, type SceneSource } from './schema';

export type SceneEvaluator = (property: any,path: string) => any;
export type SceneCompositionGetter = (objectId:string,map:MaterialMap,minimum?:MaterialTextureSize)=>CompositionTextureFrame|null|'loading';
export type SceneAssetGetter = (id: string) => any;
type MaterialSource = {id:MaterialMap;objectId:string;imported:boolean;binding:string;original?:THREE.Texture|null;nativeArtwork:boolean;minimum?:MaterialTextureSize};
const radians = THREE.MathUtils.degToRad;
let outputRenderer:THREE.WebGLRenderer|null=null;
let outputDepth:{owner:SceneRuntime;key:string}|null=null;
const outputOwners=new Set<SceneRuntime>();
const sourceSignatures=new WeakMap<object,string>();
const signature=(source:SceneSource)=>{let key=sourceSignatures.get(source);if(!key){key=JSON.stringify(source);sourceSignatures.set(source,key);}return key;};
export function primitiveGeometry(source: SceneSource): THREE.BufferGeometry {
  if ('mesh' in source) {
    const m = source.mesh,g = new THREE.BufferGeometry();
    g.setAttribute('position',new THREE.Float32BufferAttribute(m.positions,3));
    if (m.indices) g.setIndex(m.indices);
    if (m.normals) g.setAttribute('normal',new THREE.Float32BufferAttribute(m.normals,3)); else g.computeVertexNormals();
    if (m.uvs) g.setAttribute('uv',new THREE.Float32BufferAttribute(m.uvs,2));
    return g;
  }
  if ('lathe' in source) return new THREE.LatheGeometry(source.lathe.map(v=>new THREE.Vector2(...v)),source.segments || 48);
  if ('extrude' in source) {
    const shape = new THREE.Shape(source.extrude.map(v=>new THREE.Vector2(...v)));
    return new THREE.ExtrudeGeometry(shape,{depth:source.depth,steps:1,bevelEnabled:!!source.bevel,
      bevelSegments:3,bevelThickness:source.bevel || 0,bevelSize:source.bevel || 0});
  }
  if ('assetId' in source) throw new Error('Asset geometry must be supplied by the asset store');
  const p = source.parameters || {},s=p.segments || 48;
  switch (source.primitive) {
    case 'box': return new THREE.BoxGeometry(p.width || 1,p.height || 1,p.depth || 1);
    case 'sphere': return new THREE.SphereGeometry(p.radius || .5,s,Math.max(3,Math.floor(s/2)));
    case 'plane': return new THREE.PlaneGeometry(p.width || 4,p.height || 4);
    case 'cone': return new THREE.ConeGeometry(p.radius || .5,p.height || 1,s);
    case 'cylinder': return new THREE.CylinderGeometry(p.radiusTop ?? p.radius ?? .5,p.radiusBottom ?? p.radius ?? .5,p.height || 1,s);
    case 'torus': return new THREE.TorusGeometry(p.radius || .6,p.tube || .18,Math.max(8,Math.floor(s/2)),s);
    case 'capsule': return new THREE.CapsuleGeometry(p.radius || .3,p.height || .7,8,s);
    case 'icosahedron': return new THREE.IcosahedronGeometry(p.radius || .5,p.detail ?? 2);
  }
}

const cachedExtrusionGeometries=new WeakSet<THREE.BufferGeometry>();
function disposeTree(object: THREE.Object3D): void {
  const geometries = new Set<THREE.BufferGeometry>(), materials = new Set<THREE.Material>();
  object.traverse((child: any)=>{
    if (child.geometry) geometries.add(child.geometry);
    for (const m of Array.isArray(child.material) ? child.material : [child.material]) if (m) materials.add(m);
  });
  for (const g of geometries) if(!cachedExtrusionGeometries.has(g))g.dispose();
  for (const m of materials) m.dispose();
}

/** Owns the shared graph used for output, ray picking and the editor viewport. */
export class SceneRuntime {
  readonly scene = new THREE.Scene();
  camera: THREE.PerspectiveCamera|THREE.OrthographicCamera = new THREE.PerspectiveCamera();
  readonly objects = new Map<string,THREE.Object3D>();
  private renderer: THREE.WebGLRenderer|null = null;
  private sources = new Map<string,string>();
  private assetRefs = new Map<string,any>();
  private textures = new Map<string,{source:any;texture:THREE.Texture}>();
  private usedTextures=new Set<string>();
  private materialBindings=new Map<string,{key:string;texture:THREE.Texture}>();
  private usedBindings=new Set<string>();
  private compositionFrames=new Map<string,ReturnType<SceneCompositionGetter>>();
  private materialSources=new Map<string,MaterialSource>();
  private extrusionGeometries=new Map<string,{geometry:THREE.BufferGeometry;bytes:number}>();
  private extrusionGeometryBytes=0;
  private compositionGetter:SceneCompositionGetter=()=>null;
  private ambient = new THREE.HemisphereLight();
  private parsed?: Scene3D;
  private raw: any;
  private evaluation: SceneEvaluator = prop=>prop.v;
  private getter: SceneAssetGetter = ()=>null;
  private mixers = new Map<string,{mixer:THREE.AnimationMixer;clips:THREE.AnimationClip[]}>();
  warnings: string[] = [];
  revision = 0;
  /** Loading captures must be retried even when project/time are unchanged. */
  materialsPending = false;

  constructor() { this.scene.add(this.ambient); }

  private texture(id: MaterialMap|undefined,colorSpace: boolean,objectId:string,imported=false,binding='',original?:THREE.Texture|null,nativeArtwork=false): THREE.Texture|null {
    if (!id) return null;
    // UV transforms belong to a material, but identical sources and sampling
    // settings can use one GPU texture across all its meshes/material slots.
    const sampling=original?[original.channel,original.wrapS,original.wrapT,original.offset.toArray(),original.repeat.toArray(),
      original.center.toArray(),original.rotation,original.matrixAutoUpdate,original.matrix.toArray()]:null;
    const bindingKey=JSON.stringify([objectId,imported,binding,nativeArtwork,sampling]);this.usedBindings.add(bindingKey);
    if(typeof id==='object'){
      const key=JSON.stringify(['comp',objectId,id,imported,nativeArtwork,sampling]);
      const request={id,objectId,imported,binding,original,nativeArtwork,minimum:this.materialSources.get(key)?.minimum};
      this.materialSources.set(key,request);
      const captureKey=JSON.stringify([objectId,id,request.minimum]);
      if(!this.compositionFrames.has(captureKey))this.compositionFrames.set(captureKey,this.compositionGetter(objectId,id,request.minimum));
      const frame=this.compositionFrames.get(captureKey)!;
      if(frame==='loading'){
        this.materialsPending=true;
        const previous=this.materialBindings.get(bindingKey);
        if(previous)this.usedTextures.add(previous.key);
        return previous?.texture ?? null;
      }
      if(!frame){this.materialBindings.delete(bindingKey);this.warnings.push(`Missing material texture: ${JSON.stringify(id)}`);return null;}
      this.usedTextures.add(key);
      let entry=this.textures.get(key);
      if(!entry || entry.texture.image.width!==frame.width || entry.texture.image.height!==frame.height){
        // GPU data-texture storage has fixed dimensions after its first upload.
        entry?.texture.dispose();
        const texture=new THREE.DataTexture(frame.pixels,frame.width,frame.height,THREE.RGBAFormat);
        texture.colorSpace=THREE.SRGBColorSpace;texture.flipY=!imported && !nativeArtwork;
        texture.generateMipmaps=true;texture.minFilter=THREE.LinearMipmapLinearFilter;texture.magFilter=THREE.LinearFilter;texture.anisotropy=16;
        entry={source:null,texture};this.textures.set(key,entry);
      }
      if(entry.source!==frame){entry.texture.image={data:frame.pixels,width:frame.width,height:frame.height};entry.texture.needsUpdate=true;entry.texture.userData.materialHasAlpha='layerId' in id && frame.hasAlpha;entry.source=frame;}
      this.materialBindings.set(bindingKey,{key,texture:entry.texture});
      this.textureCoordinates(entry.texture,original);return entry.texture;
    }
    this.materialBindings.delete(bindingKey);
    const asset = this.getter(id),source = asset?.el || asset?.bitmap;
    if (!source || asset.kind !== 'image') { this.warnings.push(`Missing texture: ${id}`);return null; }
    const key = JSON.stringify(['image',id,colorSpace,imported,nativeArtwork,sampling]);this.usedTextures.add(key);
    let entry = this.textures.get(key);
    if (!entry || entry.source !== source) {
      entry?.texture.dispose();
      const texture = new THREE.Texture(source);
      texture.anisotropy=16;
      texture.colorSpace = colorSpace ? THREE.SRGBColorSpace : THREE.NoColorSpace;
      texture.flipY=!imported && !nativeArtwork;texture.needsUpdate = true;texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
      entry = {source,texture};this.textures.set(key,entry);
    }
    this.textureCoordinates(entry.texture,original);return entry.texture;
  }

  private textureCoordinates(texture:THREE.Texture,original?:THREE.Texture|null):void {
    if(!original)return;
    if(texture.wrapS!==original.wrapS || texture.wrapT!==original.wrapT)texture.needsUpdate=true;
    texture.channel=original.channel;texture.wrapS=original.wrapS;texture.wrapT=original.wrapT;
    texture.offset.copy(original.offset);texture.repeat.copy(original.repeat);texture.center.copy(original.center);
    texture.rotation=original.rotation;texture.matrixAutoUpdate=original.matrixAutoUpdate;texture.matrix.copy(original.matrix);
  }

  private makeObject(node: SceneObject): THREE.Object3D {
    if ('assetId' in node.source) {
      const asset = this.getter(node.source.assetId);
      if (asset?.object3d) {
        const object = cloneSkinned(asset.object3d);
        // Clones own geometry/material lifetime, never the asset store's resources.
        object.traverse((c:any)=> { if (c.geometry) c.geometry=c.geometry.clone();
          if (c.material) c.material=Array.isArray(c.material) ? c.material.map((m:any)=>m.clone()) : c.material.clone(); });
        captureRestPose(object);
        if (asset.animations?.length) {
          const mixer = new THREE.AnimationMixer(object);
          for (const clip of asset.animations) mixer.clipAction(clip).play();
          this.mixers.set(node.id,{mixer,clips:asset.animations});
        }
        return object;
      }
      if (asset?.mesh) {
        const g = new THREE.BufferGeometry();
        g.setAttribute('position',new THREE.BufferAttribute(asset.mesh.positions.slice(),3));
        g.setAttribute('normal',new THREE.BufferAttribute(asset.mesh.normals.slice(),3));
        g.setAttribute('uv',new THREE.BufferAttribute(asset.mesh.texcoords.slice(),2));
        return new THREE.Mesh(g,new THREE.MeshStandardMaterial());
      }
      this.warnings.push(`Missing model: ${node.source.assetId}`);
      const placeholder = new THREE.Mesh(new THREE.BoxGeometry(1,1,1),new THREE.MeshBasicMaterial({color:0xff6600,wireframe:true}));
      placeholder.userData.missing = true; return placeholder;
    }
    return new THREE.Mesh(primitiveGeometry(node.source),new THREE.MeshStandardMaterial());
  }

  sync(raw: Scene3D,assets: SceneAssetGetter = this.getter,evaluate: SceneEvaluator = this.evaluation,time = 0,alreadyValidated=false,animationTime?:(id:string)=>number,compositionGetter:SceneCompositionGetter=()=>null,extrusions:ExtrusionMeshSample[]=[]): void {
    if (this.raw !== raw) { this.parsed=alreadyValidated?raw:parseScene(raw);this.raw=raw; }
    // Channel edits mutate the original scene; retain those live channels.
    const data = raw as Scene3D;
    const validated = this.parsed!;
    this.getter=assets;this.evaluation=evaluate;this.warnings=[];this.compositionGetter=compositionGetter;this.usedTextures.clear();this.usedBindings.clear();this.compositionFrames.clear();this.materialsPending=false;
    const keep = new Set([...validated.objects,...validated.lights,...extrusions].map(o=>o.id));
    for (const [id,object] of this.objects) if (!keep.has(id)) this.remove(id,object);
    const values = (p:any,prefix:string) => Object.fromEntries(Object.entries<any>(p || {}).map(([k,v])=>{
      const value=evaluate(v,`${prefix}.${k}`);
      // Expressions can be invalid while the user is typing; keep the last stored value usable.
      return [k,typeof v.v==='number'?Number.isFinite(value)?value:v.v:typeof value===typeof v.v?value:v.v];
    }));
    let importedVertices=0;
    for(const node of data.objects)if('assetId' in node.source)importedVertices+=assets(node.source.assetId)?.vertices || 0;
    if(importedVertices>2_000_000)throw new Error('Imported scene exceeds two million vertices; simplify or split the models');
    for (const node of data.objects) {
      const source = signature(node.source)+':'+node.useSourceMaterials,asset='assetId' in node.source ? assets(node.source.assetId) : null;
      let object = this.objects.get(node.id);
      if (!object || this.sources.get(node.id)!==source || this.assetRefs.get(node.id)!==asset) {
        if (object) this.remove(node.id,object);
        object=this.makeObject(node);this.objects.set(node.id,object);this.sources.set(node.id,source);this.assetRefs.set(node.id,asset);
        this.scene.add(object);
      }
      object.name=node.name;object.userData.sceneId=node.id;
      const v = values(node.p,`o.${node.id}`);
      object.position.set(v.x,v.y,v.z);object.rotation.set(radians(v.rx),radians(v.ry),radians(v.rz),'XYZ');
      object.scale.set(v.sx,v.sy,v.sz);object.visible=v.visible !== false;
      const m = values(node.material.p,`o.${node.id}.m`), maps=node.material.maps;
      const intrinsic=values(node.materialProperties,`o.${node.id}.mi`);
      const properties=(index:number)=>({...intrinsic,...values(node.sourceMaterialProperties?.[index],`o.${node.id}.mi.${index}`)});
      this.traverseOwned(object,(c:any)=>{
        c.userData.sceneId=node.id;
        if (!c.isMesh) return;
        c.castShadow=node.castShadow;c.receiveShadow=node.receiveShadow;
        if (c.userData.missing) return;
        if(node.useSourceMaterials){
          for(const material of Array.isArray(c.material)?c.material:[c.material]){
            if(material.isMeshBasicMaterial){
              // glTF unlit screens already emit their full colour. Treat an
              // emissive override as that same self-lit map without relighting it.
              const original=material.userData.originalMapState ||= {map:material.map,color:material.color.clone(),opacity:material.opacity,transparent:material.transparent,depthWrite:material.depthWrite};
              const index=material.userData.sourceMaterialIndex,overrides={...maps,...node.sourceMaterials?.[index]};
              const map=overrides.emissive || overrides.color;
              const texture=map?this.texture(map,true,node.id,true,`${index}:unlit`,original.map):null;
              if(material.map!==(texture || original.map)){material.map=texture || original.map;material.needsUpdate=true;}
              material.color.copy(texture?new THREE.Color(1,1,1):original.color);
              applyMaterialProperties(material,properties(index),original);
              continue;
            }
            if(!material.isMeshStandardMaterial)continue;
            // Capture only original slot state; clear/delete restores the model exactly.
            const original=material.userData.originalMapState ||= {map:material.map,normalMap:material.normalMap,
              roughnessMap:material.roughnessMap,metalnessMap:material.metalnessMap,emissiveMap:material.emissiveMap,aoMap:material.aoMap,
              color:material.color.clone(),emissive:material.emissive.clone(),emissiveIntensity:material.emissiveIntensity,roughness:material.roughness,metalness:material.metalness,opacity:material.opacity,transparent:material.transparent,depthWrite:material.depthWrite};
            const overrides={...maps,...node.sourceMaterials?.[material.userData.sourceMaterialIndex]};
            for(const [key,slot,isColor] of [['color','map',true],['normal','normalMap',false],['roughness','roughnessMap',false],
              ['metalness','metalnessMap',false],['emissive','emissiveMap',true],['ao','aoMap',false]] as const){
              const texture=overrides[key]?this.texture(overrides[key],isColor,node.id,true,`${material.userData.sourceMaterialIndex}:${key}`,original[slot]):null;
              const next=texture || original[slot];
              if(material[slot]!==next){material[slot]=next;material.needsUpdate=true;}
              if(key==='color')material.color.copy(texture?new THREE.Color(1,1,1):original.color);
              if(key==='emissive'){
                material.emissive.copy(texture?new THREE.Color(1,1,1):original.emissive);
                material.emissiveIntensity=texture?Math.max(1,original.emissiveIntensity):original.emissiveIntensity;
              }
            }
            applyMaterialProperties(material,properties(material.userData.sourceMaterialIndex),original);
          }
          return;
        }
        if (!(c.material instanceof THREE.MeshStandardMaterial) || Array.isArray(c.material)) {
          for (const material of Array.isArray(c.material)?c.material:[c.material]) material.dispose();
          c.material=new THREE.MeshStandardMaterial();
        }
        const material=c.material as THREE.MeshStandardMaterial;
        material.color.set(m.color);material.roughness=m.roughness;material.metalness=m.metalness;
        material.emissive.set(m.emissive);material.emissiveIntensity=m.emissiveIntensity;
        const transparent=m.opacity<1,side=node.material.doubleSided?THREE.DoubleSide:THREE.FrontSide;
        if (transparent!==material.transparent || side!==material.side) material.needsUpdate=true;
        material.opacity=m.opacity;material.transparent=transparent;material.depthWrite=!transparent;material.side=side;
        for (const [key,slot,isColor] of [['color','map',true],['normal','normalMap',false],['roughness','roughnessMap',false],
          ['metalness','metalnessMap',false],['emissive','emissiveMap',true],['ao','aoMap',false]] as const) {
          const texture=this.texture(node.sourceMaterials?.[0]?.[key] || maps[key],isColor,node.id,false,key);
          if (material[slot]!==texture) { material[slot]=texture;material.needsUpdate=true; }
        }
        applyMaterialProperties(material,properties(0),{opacity:m.opacity,transparent:m.opacity<1,depthWrite:m.opacity>=1,roughness:m.roughness,metalness:m.metalness});
      });
      // Part offsets sit on top of the file's pose and its own animation.
      restoreParts(object);
      this.mixers.get(node.id)?.mixer.setTime(Math.max(0,animationTime?.(node.id) ?? time));
      const parts=(node as any).parts as Record<string,{p:any}>|undefined;
      applyParts(object,parts?Object.fromEntries(Object.entries(parts).map(([key,part])=>[key,values(part.p,`o.${node.id}.pt.${key}`)])):{},parts as any);
    }
    let nativeVertices=0;
    for(const sample of extrusions){
      this.syncExtrusion(sample);nativeVertices+=(this.objects.get(sample.id) as THREE.Mesh).geometry.getAttribute('position').count;
      if(nativeVertices>500000)throw new Error('Native 3D artwork exceeds the 500,000 vertex composition budget; reduce geometry quality');
    }
    // Attach after every object exists, independently of document ordering.
    for (const node of data.objects) {
      const object=this.objects.get(node.id)!,parent=node.parent?this.objects.get(node.parent)!:this.scene;
      if (object.parent!==parent) parent.add(object);
    }
    let shadowLights=0;
    for (const node of data.lights) {
      let object=this.objects.get(node.id) as THREE.Light|undefined;
      if (!object || object.userData.lightType!==node.type) {
        if (object) this.remove(node.id,object);
        object=node.type==='sun'?new THREE.DirectionalLight():node.type==='spot'?new THREE.SpotLight():new THREE.PointLight();
        object.userData.lightType=node.type;object.userData.sceneId=node.id;this.objects.set(node.id,object);this.scene.add(object);
        if ('target' in object) this.scene.add((object as THREE.DirectionalLight).target);
      }
      const v=values(node.p,`light.${node.id}`);
      object.name=node.name;object.position.set(v.x,v.y,v.z);object.color.set(v.color);object.intensity=v.intensity;
      object.visible=v.visible!==false;
      const light=object as any;
      light.castShadow=node.castShadow && data.environment.shadows && shadowLights++<4;
      if (light.target) light.target.position.set(v.targetX,v.targetY,v.targetZ);
      if ('distance' in light) { light.distance=v.distance;light.decay=v.decay; }
      if ('angle' in light) { light.angle=radians(v.angle);light.penumbra=v.penumbra; }
      if (light.shadow) {
        light.shadow.mapSize.set(1024,1024);light.shadow.normalBias=.025;light.shadow.bias=-.0002;
        if (light.shadow.camera.isOrthographicCamera) Object.assign(light.shadow.camera,{left:-10,right:10,top:10,bottom:-10,near:.01,far:200});
        light.shadow.camera.updateProjectionMatrix();
      }
    }
    if ((data.camera.projection==='orthographic')!==(this.camera instanceof THREE.OrthographicCamera))
      this.camera=data.camera.projection==='orthographic'?new THREE.OrthographicCamera():new THREE.PerspectiveCamera();
    const c=values(data.camera.p,'camera');
    this.camera.position.set(c.x,c.y,c.z);this.camera.up.set(0,1,0);this.camera.lookAt(c.targetX,c.targetY,c.targetZ);
    this.camera.near=Math.max(.0001,c.near);this.camera.far=Math.max(this.camera.near+.01,c.far);this.camera.zoom=Math.max(.001,c.zoom);
    if (this.camera instanceof THREE.PerspectiveCamera) this.camera.fov=THREE.MathUtils.clamp(c.fov,1,150);
    this.camera.updateProjectionMatrix();
    const environment=values(data.environment.p,'environment');
    this.ambient.color.set(environment.ambientColor);this.ambient.groundColor.set(environment.ambientColor).multiplyScalar(.35);
    this.ambient.intensity=environment.ambient;
    const environmentAsset=data.environment.map?assets(data.environment.map):null;
    this.scene.environment=environmentAsset?.environmentTexture ?? null;
    this.scene.environmentIntensity=environment.intensity ?? 1;
    this.scene.environmentRotation.set(0,radians(environment.rotation ?? 0),0);
    if(data.environment.map && !this.scene.environment)this.warnings.push('Environment image is missing or still loading');
    this.scene.background=data.environment.background?new THREE.Color(data.environment.background):null;
    if (this.renderer) { this.renderer.toneMappingExposure=environment.exposure;this.renderer.shadowMap.enabled=data.environment.shadows; }
    this.scene.updateMatrixWorld(true);this.camera.updateMatrixWorld(true);this.revision++;
    for(const [key,entry] of this.textures)if(!this.usedTextures.has(key)){entry.texture.dispose();this.textures.delete(key);}
    for(const key of this.materialSources.keys())if(!this.usedTextures.has(key))this.materialSources.delete(key);
    for(const key of this.materialBindings.keys())if(!this.usedBindings.has(key))this.materialBindings.delete(key);
    this.compositionFrames.clear();
  }

  private syncExtrusion(sample:ExtrusionMeshSample):void {
    let mesh=this.objects.get(sample.id) as THREE.Mesh|undefined;
    if(!mesh || this.sources.get(sample.id)!==sample.key){
      if(mesh)this.remove(sample.id,mesh);
      const cacheKey=`${sample.id}:${sample.key}`;
      let cached=this.extrusionGeometries.get(cacheKey);
      if(!cached){
        const geometry=buildExtrusionGeometry(sample.outlines,sample.options);
        cachedExtrusionGeometries.add(geometry);
        const bytes=Object.values(geometry.attributes).reduce((sum,attribute)=>sum+attribute.array.byteLength,0);
        cached={geometry,bytes};this.extrusionGeometries.set(cacheKey,cached);this.extrusionGeometryBytes+=bytes;
        while(this.extrusionGeometries.size>32 || this.extrusionGeometryBytes>64*1024*1024){
          const [key,entry]=this.extrusionGeometries.entries().next().value!;
          if(key===cacheKey)break;
          this.extrusionGeometries.delete(key);this.extrusionGeometryBytes-=entry.bytes;
          const inUse=[...this.objects.values()].some(object=>(object as THREE.Mesh).geometry===entry.geometry);
          cachedExtrusionGeometries.delete(entry.geometry);if(!inUse)entry.geometry.dispose();
        }
      }else{this.extrusionGeometries.delete(cacheKey);this.extrusionGeometries.set(cacheKey,cached);}
      mesh=new THREE.Mesh(cached.geometry,EXTRUSION_SURFACES.map(()=>sample.plane?new THREE.MeshBasicMaterial({side:THREE.DoubleSide,transparent:true,alphaTest:.001,toneMapped:false}):new THREE.MeshStandardMaterial({vertexColors:true,roughness:.4,metalness:0})));
      this.objects.set(sample.id,mesh);this.sources.set(sample.id,sample.key);this.scene.add(mesh);
    }
    mesh.name=sample.name;mesh.userData.sceneId=sample.id;mesh.userData.layerOpacity=sample.opacity;mesh.userData.layerOccludes=true;
    mesh.matrixAutoUpdate=false;mesh.matrix.copy(sample.matrix);mesh.matrix.decompose(mesh.position,mesh.quaternion,mesh.scale);
    mesh.castShadow=true;mesh.receiveShadow=true;
    if(sample.plane){
      const key=`plane:${sample.id}`,frame=sample.plane;this.usedTextures.add(key);
      let entry=this.textures.get(key);
      if(!entry){const texture=new THREE.DataTexture(frame.pixels,frame.width,frame.height,THREE.RGBAFormat);texture.colorSpace=THREE.SRGBColorSpace;texture.flipY=false;texture.minFilter=texture.magFilter=THREE.LinearFilter;entry={texture,source:null};this.textures.set(key,entry);}
      if(entry.source!==frame){entry.texture.image={data:frame.pixels,width:frame.width,height:frame.height};entry.texture.needsUpdate=true;entry.source=frame;}
      const material=(mesh.material as THREE.MeshBasicMaterial[])[0]!;if(material.map!==entry.texture){material.map=entry.texture;material.needsUpdate=true;}
      mesh.userData.nativePlane=true;mesh.castShadow=false;return;
    }
    if(mesh.userData.fillColor!==sample.color){
      const color=new THREE.Color(sample.color),attribute=mesh.geometry.getAttribute('color');
      for(const index of mesh.geometry.userData.defaultColorVertices || [])attribute.setXYZ(index,color.r,color.g,color.b);
      attribute.needsUpdate=true;mesh.userData.fillColor=sample.color;
    }
    const materials=mesh.material as THREE.MeshStandardMaterial[];
    EXTRUSION_SURFACES.forEach((surface,index)=>{
      const material=materials[index]!,data=sample.surfaces[surface],maps=data?.maps || {},values=data?.values || {};
      material.color.set(values.color || '#FFFFFF');material.emissive.set('#000000');material.emissiveIntensity=0;
      let mapped=false;
      for(const [key,slot,isColor] of [['color','map',true],['normal','normalMap',false],['roughness','roughnessMap',false],['metalness','metalnessMap',false],['emissive','emissiveMap',true],['ao','aoMap',false]] as const){
        const texture=this.texture(maps[key],isColor,sample.id,false,`${surface}:${key}`,undefined,true);
        if(material[slot]!==texture){material[slot]=texture;material.needsUpdate=true;}
        if(key==='color' && texture){material.color.set('#FFFFFF');mapped=true;}
        if(key==='emissive' && texture){material.emissive.set('#FFFFFF');material.emissiveIntensity=1;}
      }
      const vertexColors=!mapped && !values.color;
      if(material.vertexColors!==vertexColors){material.vertexColors=vertexColors;material.needsUpdate=true;}
      applyMaterialProperties(material,values,{opacity:1,transparent:vertexColors && mesh!.geometry.userData.hasVertexAlpha,depthWrite:!vertexColors || !mesh!.geometry.userData.hasVertexAlpha,roughness:.4,metalness:0});
    });
  }

  resizeCamera(w:number,h:number,camera=this.camera): void {
    const aspect=w/Math.max(1,h);
    if (camera instanceof THREE.PerspectiveCamera) camera.aspect=aspect;
    else { camera.left=-3*aspect;camera.right=3*aspect;camera.top=3;camera.bottom=-3; }
    camera.updateProjectionMatrix();camera.updateMatrixWorld();
  }
  /** Resolve material detail after native transforms and the final output
   * camera are known. This also covers high-resolution exports and free view. */
  prepareMaterialTextures(w:number,h:number,camera=this.camera,limit=8192):void {
    if(!this.materialSources.size)return;
    this.scene.updateMatrixWorld(true);camera.updateMatrixWorld(true);
    try{
      for(const [key,request] of this.materialSources){
        const object=this.objects.get(request.objectId),previous=this.textures.get(key)?.texture;
        if(!object || !previous)continue;
        const demand=projectedMaterialSize(object,previous,w,h,camera);if(!demand)continue;
        const smaller=request.minimum && demand.width<=request.minimum.width/4 && demand.height<=request.minimum.height/4;
        const minimum={width:Math.min(limit,Math.max(demand.width,smaller?0:request.minimum?.width || 0)),height:Math.min(limit,Math.max(demand.height,smaller?0:request.minimum?.height || 0))};
        if(!smaller && request.minimum && minimum.width<=request.minimum.width && minimum.height<=request.minimum.height)continue;
        request.minimum=minimum;
        if(previous.image.width>=minimum.width && previous.image.height>=minimum.height)continue;
        const next=this.texture(request.id,true,request.objectId,request.imported,request.binding,request.original,request.nativeArtwork);
        if(!next || next===previous)continue;
        this.scene.traverse((child:any)=>{
          if(!child.isMesh)return;
          for(const material of Array.isArray(child.material)?child.material:[child.material]){
            for(const slot of Object.keys(material))if(material[slot]===previous){material[slot]=next;material.needsUpdate=true;}
            if(material.map===next && next.userData.materialHasAlpha){material.transparent=true;material.depthWrite=false;}
          }
        });
        for(const binding of this.materialBindings.values())if(binding.key===key)binding.texture=next;
      }
    }finally{this.compositionFrames.clear();}
  }
  render(w:number,h:number,objectId?:string,depthIds?:Set<string>,background=true,camera=this.camera): HTMLCanvasElement {
    if (!this.renderer) {
      // Output passes share one GPU context; a project can contain many 3D layers.
      outputRenderer ??= new THREE.WebGLRenderer({alpha:true,antialias:true,preserveDrawingBuffer:true,premultipliedAlpha:true});
      this.renderer=outputRenderer;outputOwners.add(this);
      this.renderer.outputColorSpace=THREE.SRGBColorSpace;this.renderer.toneMapping=THREE.ACESFilmicToneMapping;
      this.renderer.shadowMap.type=THREE.PCFSoftShadowMap;
    }
    const width=Math.max(1,Math.min(8192,Math.round(w))),height=Math.max(1,Math.min(8192,Math.round(h)));
    this.resizeCamera(width,height,camera);
    this.prepareMaterialTextures(width,height,camera,this.renderer.capabilities.maxTextureSize);
    // A material composition can render its own 3D scene using this shared
    // renderer. Restore the receiving pass's dimensions after that capture.
    if(this.renderer.domElement.width!==width || this.renderer.domElement.height!==height){
      this.renderer.setSize(width,height,false);outputDepth=null;
    }
    this.resizeCamera(width,height,camera);
    const e=this.raw?.environment;
    this.renderer.toneMappingExposure=e?this.evaluation(e.p.exposure,'environment.exposure'):1;
    this.renderer.shadowMap.enabled=e?.shadows!==false;
    this.renderer.setClearColor(0,0);
    if(!objectId){outputDepth=null;this.renderer.render(this.scene,camera);}
    else {
      // A shared depth prepass lets ordinary timeline layers keep their own effects,
      // opacity and masks while model intersections and shared shadows remain correct.
      const saved=new Map<THREE.Material,{colorWrite:boolean;depthWrite:boolean;visible:boolean;alphaTest:number}>();
      const originalBackground=this.scene.background,originalAutoClear=this.renderer.autoClear;
      const autoShadow=this.renderer.shadowMap.autoUpdate;
      const depthKey=`${this.revision}:${width}:${height}:${camera.matrixWorld.elements}:${camera.projectionMatrix.elements}:${[...(depthIds || this.objects.keys())].sort().join(',')}`;
      const visit=(phase:'depth'|'color')=>{
        this.scene.traverse((child:any)=>{
          if(!child.isMesh)return;
          const id=child.userData.sceneId;
          for(const material of Array.isArray(child.material)?child.material:[child.material]){
            if(!saved.has(material))saved.set(material,{colorWrite:material.colorWrite,depthWrite:material.depthWrite,visible:material.visible,alphaTest:material.alphaTest});
            const old=saved.get(material)!;
            material.colorWrite=phase==='color' && id===objectId && old.colorWrite;
            material.visible=old.visible && (!depthIds || depthIds.has(id)) && (phase==='depth' || id===objectId);
            const data=this.objects.get(id)?.userData;
            if(data?.nativePlane)material.alphaTest=phase==='depth'?.999:old.alphaTest;
            material.depthWrite=phase==='depth' && old.depthWrite && (data?.layerOpacity ?? 1)>=.999 && data?.layerOccludes!==false;
          }
        });
      };
      try {
        this.scene.background=null;
        if(outputDepth?.owner!==this || outputDepth.key!==depthKey){
          this.renderer.autoClear=true;this.renderer.shadowMap.autoUpdate=true;
          visit('depth');this.renderer.render(this.scene,camera);
          outputDepth={owner:this,key:depthKey};
        }
        // Retain shared depth between model layers; clear only the color surface.
        this.renderer.setClearColor(background && originalBackground instanceof THREE.Color?originalBackground:0,
          background && originalBackground instanceof THREE.Color?1:0);
        this.renderer.clear(true,false,false);
        this.renderer.autoClear=false;this.renderer.shadowMap.autoUpdate=false;
        visit('color');this.renderer.render(this.scene,camera);
      } finally {
        for(const [material,old] of saved)Object.assign(material,old);
        this.scene.background=originalBackground;this.renderer.autoClear=originalAutoClear;this.renderer.shadowMap.autoUpdate=autoShadow;
      }
    }
    return this.renderer.domElement;
  }
  pick(x:number,y:number,ids?:Set<string>,camera=this.camera): string|null {
    return this.pickHit(x,y,ids,camera)?.id ?? null;
  }
  /** The nearest visible mesh under a normalized device point, and its layer. */
  pickHit(x:number,y:number,ids?:Set<string>,camera=this.camera): {id:string;object:THREE.Object3D;point:THREE.Vector3}|null {
    this.scene.updateMatrixWorld(true);camera.updateMatrixWorld(true);
    const ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2(x,y),camera);
    for (const hit of ray.intersectObjects([...this.objects.values()],true)) {
      if (hit.object.visible && hit.object.userData.sceneId && (!ids || ids.has(hit.object.userData.sceneId))) {
        const materials=(hit.object as THREE.Mesh).material;
        if(materials && (Array.isArray(materials)?materials:[materials]).every(m=>m.opacity<=0))continue;
        let visible=true,parent:THREE.Object3D|null=hit.object;
        while(parent){if(!parent.visible)visible=false;parent=parent.parent;}
        if (visible) return {id:hit.object.userData.sceneId,object:hit.object,point:hit.point};
      }
    }
    return null;
  }
  bounds(w:number,h:number,ids?:string[],camera=this.camera): {x0:number;y0:number;x1:number;y1:number;w:number;h:number;ax:number;ay:number}|null {
    this.resizeCamera(w,h,camera);
    const box=this.worldBounds(ids);if(box.isEmpty())return null;
    const corners=[];
    for (const x of [box.min.x,box.max.x]) for(const y of [box.min.y,box.max.y])for(const z of [box.min.z,box.max.z]) {
      const v=new THREE.Vector3(x,y,z).project(camera);if(v.z<1)corners.push(v);
    }
    if(!corners.length)return null;
    const x0=Math.max(0,Math.min(...corners.map(v=>(v.x+1)*w/2))),x1=Math.min(w,Math.max(...corners.map(v=>(v.x+1)*w/2)));
    const y0=Math.max(0,Math.min(...corners.map(v=>(1-v.y)*h/2))),y1=Math.min(h,Math.max(...corners.map(v=>(1-v.y)*h/2)));
    return x1>x0&&y1>y0?{x0,y0,x1,y1,w:x1-x0,h:y1-y0,ax:0,ay:0}:null;
  }
  worldBounds(ids?:string[]): THREE.Box3 {
    this.scene.updateMatrixWorld(true);const box=new THREE.Box3();
    for(const [id,object] of this.objects)if(object.visible&&(!ids||ids.includes(id))&&!(object as THREE.Light).isLight)
      object.traverseVisible((child:any)=>{
        if(!child.isMesh)return;
        let parent=child.parent;while(parent){if(!parent.visible)return;parent=parent.parent;}
        if(!child.geometry.boundingBox)child.geometry.computeBoundingBox();
        if(child.geometry.boundingBox)box.union(child.geometry.boundingBox.clone().applyMatrix4(child.matrixWorld));
      });
    return box;
  }
  /** Tight evaluated mesh bounds for tools. Unlike navigation's inexpensive
   * boxes, this includes part poses, morph targets and skinned vertices. */
  geometryBounds(ids:readonly string[],basis=new THREE.Matrix4()):THREE.Box3 {
    this.scene.updateMatrixWorld(true);
    const box=new THREE.Box3(),point=new THREE.Vector3();
    for(const id of new Set(ids)){
      const object=this.objects.get(id);
      if(!object)throw new Error(`Geometry is unavailable for ${id}`);
      if(object.userData.missing)throw new Error(`Model ${id} is still loading or missing`);
      object.traverseVisible((child:any)=>{
        if(!child.isMesh)return;
        child.skeleton?.update();
        const matrix=basis.clone().multiply(child.matrixWorld),positions=child.geometry.getAttribute('position');
        for(let i=0;i<(positions?.count || 0);i++)box.expandByPoint(child.getVertexPosition(i,point).applyMatrix4(matrix));
      });
    }
    return box;
  }
  private traverseOwned(object:THREE.Object3D,visit:(child:THREE.Object3D)=>void):void {
    const walk=(child:THREE.Object3D)=>{visit(child);for(const nested of child.children)
      if(this.objects.get(nested.userData.sceneId)!==nested)walk(nested);};
    walk(object);
  }
  private remove(id:string,object:THREE.Object3D):void {
    // Child scene objects own their geometry/materials separately from this model.
    for(const child of [...object.children])if(this.objects.get(child.userData.sceneId)===child)this.scene.add(child);
    object.removeFromParent();
    const light=object as any;if(light.target)light.target.removeFromParent();if(light.shadow)light.shadow.dispose();
    disposeTree(object);this.objects.delete(id);this.sources.delete(id);this.assetRefs.delete(id);
    const mixer=this.mixers.get(id)?.mixer;if(mixer){mixer.stopAllAction();mixer.uncacheRoot(object);}this.mixers.delete(id);
  }
  dispose():void {
    for(const [id,object] of this.objects)this.remove(id,object);
    for(const {texture} of this.textures.values())texture.dispose();this.textures.clear();this.materialBindings.clear();this.compositionFrames.clear();this.materialSources.clear();
    for(const {geometry} of this.extrusionGeometries.values()){cachedExtrusionGeometries.delete(geometry);geometry.dispose();}this.extrusionGeometries.clear();this.extrusionGeometryBytes=0;
    outputOwners.delete(this);
    if(outputOwners.size===0&&outputRenderer){outputRenderer.dispose();outputRenderer.forceContextLoss();outputRenderer=null;}
    this.renderer=null;
  }
}
