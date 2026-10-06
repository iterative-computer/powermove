import * as THREE from 'three';
import { RectAreaLightUniformsLib } from 'three/addons/lights/RectAreaLightUniformsLib.js';
let areaLightsReady=false;
import {materialPreview} from './materials';
import { clone as cloneSkinned } from 'three/addons/utils/SkeletonUtils.js';
import { parseScene, type Scene3D, type SceneObject, type SceneSource } from './schema';

export type SceneEvaluator = (property: any,path: string) => any;
export type SceneAssetGetter = (id: string) => any;
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
    for(const group of m.groups || [])g.addGroup(group.start,group.count,group.materialIndex);
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

function disposeTree(object: THREE.Object3D): void {
  const geometries = new Set<THREE.BufferGeometry>(), materials = new Set<THREE.Material>();
  object.traverse((child: any)=>{
    if (child.geometry) geometries.add(child.geometry);
    for (const m of Array.isArray(child.material) ? child.material : [child.material]) if (m) materials.add(m);
  });
  for (const g of geometries) g.dispose();
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
  private ambient = new THREE.HemisphereLight();
  private parsed?: Scene3D;
  private raw: any;
  private evaluation: SceneEvaluator = prop=>prop.v;
  private getter: SceneAssetGetter = ()=>null;
  private mixers = new Map<string,{mixer:THREE.AnimationMixer;clips:THREE.AnimationClip[]}>();
  warnings: string[] = [];
  revision = 0;

  constructor() { this.scene.add(this.ambient); }

  private texture(id: string|undefined,colorSpace: boolean): THREE.Texture|null {
    if (!id) return null;
    const asset = this.getter(id),source = asset?.el || asset?.bitmap;
    if (!source || asset.kind !== 'image') { this.warnings.push(`Missing texture: ${id}`);return null; }
    const key = `${id}:${colorSpace}`;
    let entry = this.textures.get(key);
    if (!entry || entry.source !== source) {
      entry?.texture.dispose();
      const texture = new THREE.Texture(source);
      texture.colorSpace = colorSpace ? THREE.SRGBColorSpace : THREE.NoColorSpace;
      texture.needsUpdate = true;texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
      entry = {source,texture};this.textures.set(key,entry);
    }
    return entry.texture;
  }

  private makeObject(node: SceneObject): THREE.Object3D {
    if ('assetId' in node.source) {
      const asset = this.getter(node.source.assetId);
      if (asset?.object3d) {
        const object = cloneSkinned(asset.object3d);
        // Clones own geometry/material lifetime, never the asset store's resources.
        object.traverse((c:any)=> { if (c.geometry) c.geometry=c.geometry.clone();
          if (c.material) c.material=Array.isArray(c.material) ? c.material.map((m:any)=>m.clone()) : c.material.clone(); });
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

  sync(raw: Scene3D,assets: SceneAssetGetter = this.getter,evaluate: SceneEvaluator = this.evaluation,time = 0,alreadyValidated=false,animationTime?:(id:string)=>number): void {
    if (this.raw !== raw) { this.parsed=alreadyValidated?raw:parseScene(raw);this.raw=raw; }
    // Channel edits mutate the original scene; retain those live channels.
    const data = raw as Scene3D;
    const validated = this.parsed!;
    this.getter=assets;this.evaluation=evaluate;this.warnings=[];
    const keep = new Set([...validated.objects,...validated.lights].map(o=>o.id));
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
      this.traverseOwned(object,(c:any)=>{
        c.userData.sceneId=node.id;
        if (!c.isMesh) return;
        c.castShadow=node.castShadow;c.receiveShadow=node.receiveShadow;
        if(c.userData.missing)return;
        const definitions=node.slots?.length?node.slots.map(s=>s.material):[node.material];
        if(node.useSourceMaterials && !node.slots?.length)return;
        const existing=Array.isArray(c.material)?c.material:[c.material];
        const list=definitions.map((definition,index)=>{
          const preview=materialPreview(definition),m=values(preview.p,`o.${node.id}.m`),maps=definition.maps;
          const material=existing[index] instanceof THREE.MeshStandardMaterial?existing[index]:new THREE.MeshStandardMaterial();
          material.name=node.slots?.[index]?.name || definition.shader?.name || 'Surface';
          material.color.set(m.color);material.roughness=m.roughness;material.metalness=m.metalness;
          material.emissive.set(m.emissive);material.emissiveIntensity=m.emissiveIntensity;
          const transparent=m.opacity<1,side=definition.doubleSided?THREE.DoubleSide:THREE.FrontSide;
          if(transparent!==material.transparent||side!==material.side)material.needsUpdate=true;
          material.opacity=m.opacity;material.transparent=transparent;material.depthWrite=!transparent;material.side=side;
          for(const [key,slot,isColor] of [['color','map',true],['normal','normalMap',false],['roughness','roughnessMap',false],['metalness','metalnessMap',false],['emissive','emissiveMap',true],['ao','aoMap',false]] as const){
            const texture=this.texture(maps[key],isColor);
            const placement=definition.placement,placementKey=texture&&placement?JSON.stringify([maps[key],placement]):null;
            const previous=material[slot];
            if(placementKey&&placement){
              if(material.userData[slot+'Placement']!==placementKey){
                if(previous?.userData.scenePlacement)previous.dispose();
                const copy=texture!.clone();copy.repeat.set(placement.scaleX,placement.scaleY);copy.offset.set(placement.offsetX,placement.offsetY);copy.rotation=radians(placement.rotation);copy.userData.scenePlacement=true;copy.needsUpdate=true;material[slot]=copy;material.needsUpdate=true;material.userData[slot+'Placement']=placementKey;
              }
            }else if(previous!==texture){if(previous?.userData.scenePlacement)previous.dispose();material[slot]=texture;material.needsUpdate=true;delete material.userData[slot+'Placement'];}
          }
          return material;
        });
        for(const material of existing)if(!list.includes(material))material.dispose();
        c.material=node.slots?.length?list:list[0];
      });
      this.mixers.get(node.id)?.mixer.setTime(Math.max(0,animationTime?.(node.id) ?? time));
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
        if(node.type==='area'&&!areaLightsReady){RectAreaLightUniformsLib.init();areaLightsReady=true;}
        object=node.type==='sun'?new THREE.DirectionalLight():node.type==='spot'?new THREE.SpotLight():node.type==='area'?new THREE.RectAreaLight():new THREE.PointLight();
        object.userData.lightType=node.type;object.userData.sceneId=node.id;this.objects.set(node.id,object);this.scene.add(object);
        if ('target' in object) this.scene.add((object as THREE.DirectionalLight).target);
      }
      const v=values(node.p,`light.${node.id}`);
      object.name=node.name;object.position.set(v.x,v.y,v.z);object.color.set(v.color);object.intensity=v.intensity;
      object.visible=v.visible!==false;
      const light=object as any;
      light.castShadow=node.castShadow && data.environment.shadows && shadowLights++<4;
      if (light.target) light.target.position.set(v.targetX,v.targetY,v.targetZ);
      // Area lights face their target and have a real size; they cast no native shadows.
      if (light.isRectAreaLight) { light.width=v.width;light.height=v.height;light.castShadow=false;light.lookAt(v.targetX,v.targetY,v.targetZ); }
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
    this.scene.background=data.environment.background?new THREE.Color(data.environment.background):null;
    if (this.renderer) { this.renderer.toneMappingExposure=environment.exposure;this.renderer.shadowMap.enabled=data.environment.shadows; }
    this.scene.updateMatrixWorld(true);this.camera.updateMatrixWorld(true);this.revision++;
    const textureKeys=new Set(data.objects.flatMap(o=>[o.material,...(o.slots||[]).map(s=>s.material)].flatMap(m=>Object.entries(m.maps).map(([slot,id])=>`${id}:${slot==='color'||slot==='emissive'}`))));
    for(const [key,entry] of this.textures)if(!textureKeys.has(key)){entry.texture.dispose();this.textures.delete(key);}
  }

  resizeCamera(w:number,h:number,camera=this.camera): void {
    const aspect=w/Math.max(1,h);
    if (camera instanceof THREE.PerspectiveCamera) camera.aspect=aspect;
    else { camera.left=-3*aspect;camera.right=3*aspect;camera.top=3;camera.bottom=-3; }
    camera.updateProjectionMatrix();camera.updateMatrixWorld();
  }
  /** `look` is the editor's Wireframe shading and X-ray; output never sets it. */
  render(w:number,h:number,objectId?:string,depthIds?:Set<string>,background=true,camera=this.camera,look:{wireframe?:boolean;xray?:boolean}={}): HTMLCanvasElement {
    const saved:[any,{wireframe:boolean;transparent:boolean;opacity:number;depthWrite:boolean}][]=[];
    if(look.wireframe||look.xray)this.scene.traverse((child:any)=>{
      if(!child.isMesh)return;
      for(const material of Array.isArray(child.material)?child.material:[child.material]){
        if(!material||saved.some(([m])=>m===material))continue;
        saved.push([material,{wireframe:!!material.wireframe,transparent:material.transparent,opacity:material.opacity,depthWrite:material.depthWrite}]);
        if(look.wireframe&&'wireframe' in material)material.wireframe=true;
        // X-ray: models become see-through and stop hiding one another.
        if(look.xray){material.transparent=true;material.opacity*=.45;material.depthWrite=false;}
      }
    });
    try{return this.renderPass(w,h,objectId,depthIds,background,camera);}
    finally{for(const [material,old] of saved)Object.assign(material,old);}
  }
  private renderPass(w:number,h:number,objectId:string|undefined,depthIds:Set<string>|undefined,background:boolean,camera:THREE.PerspectiveCamera|THREE.OrthographicCamera): HTMLCanvasElement {
    if (!this.renderer) {
      // Output passes share one GPU context; a project can contain many 3D layers.
      outputRenderer ??= new THREE.WebGLRenderer({alpha:true,antialias:true,preserveDrawingBuffer:true,premultipliedAlpha:true});
      this.renderer=outputRenderer;outputOwners.add(this);
      this.renderer.outputColorSpace=THREE.SRGBColorSpace;this.renderer.toneMapping=THREE.ACESFilmicToneMapping;
      this.renderer.shadowMap.type=THREE.PCFSoftShadowMap;
    }
    const width=Math.max(1,Math.min(8192,Math.round(w))),height=Math.max(1,Math.min(8192,Math.round(h)));
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
      const saved=new Map<THREE.Material,{colorWrite:boolean;depthWrite:boolean;visible:boolean}>();
      const originalBackground=this.scene.background,originalAutoClear=this.renderer.autoClear;
      const autoShadow=this.renderer.shadowMap.autoUpdate;
      const depthKey=`${this.revision}:${width}:${height}:${camera.matrixWorld.elements}:${camera.projectionMatrix.elements}:${[...(depthIds || this.objects.keys())].sort().join(',')}`;
      const visit=(phase:'depth'|'color')=>{
        this.scene.traverse((child:any)=>{
          if(!child.isMesh)return;
          const id=child.userData.sceneId;
          for(const material of Array.isArray(child.material)?child.material:[child.material]){
            if(!saved.has(material))saved.set(material,{colorWrite:material.colorWrite,depthWrite:material.depthWrite,visible:material.visible});
            const old=saved.get(material)!;
            material.colorWrite=phase==='color' && id===objectId && old.colorWrite;
            material.visible=old.visible && (!depthIds || depthIds.has(id)) && (phase==='depth' || id===objectId);
            const data=this.objects.get(id)?.userData;
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
    this.scene.updateMatrixWorld(true);camera.updateMatrixWorld(true);
    const ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2(x,y),camera);
    for (const hit of ray.intersectObjects([...this.objects.values()],true)) {
      if (hit.object.visible && hit.object.userData.sceneId && (!ids || ids.has(hit.object.userData.sceneId))) {
        const materials=(hit.object as THREE.Mesh).material;
        if(materials && (Array.isArray(materials)?materials:[materials]).every(m=>m.opacity<=0))continue;
        let visible=true,parent:THREE.Object3D|null=hit.object;
        while(parent){if(!parent.visible)visible=false;parent=parent.parent;}
        if (visible) return hit.object.userData.sceneId;
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
    for(const {texture} of this.textures.values())texture.dispose();this.textures.clear();
    outputOwners.delete(this);
    if(outputOwners.size===0&&outputRenderer){outputRenderer.dispose();outputRenderer.forceContextLoss();outputRenderer=null;}
    this.renderer=null;
  }
}
