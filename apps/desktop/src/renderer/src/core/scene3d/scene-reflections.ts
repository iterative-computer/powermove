import * as THREE from 'three';

type ViewCamera=THREE.PerspectiveCamera|THREE.OrthographicCamera;
type ReflectionMaterial=THREE.MeshStandardMaterial;
type Binding={mode:{value:number};cube:{value:THREE.Texture|null};plane:{value:THREE.Texture|null};matrix:{value:THREE.Matrix4};lod:{value:number}};
type Capture={root:THREE.Object3D;target:THREE.WebGLRenderTarget|THREE.WebGLCubeRenderTarget;plane:THREE.Plane|null;key:string};

/** A genuine flat mesh, including imported/rotated planes. */
export function reflectionPlane(mesh:THREE.Mesh,cameraPosition:THREE.Vector3):THREE.Plane|null {
  const geometry=mesh.geometry;if(!geometry.boundingBox)geometry.computeBoundingBox();
  const box=geometry.boundingBox;if(!box || box.isEmpty())return null;
  const size=box.getSize(new THREE.Vector3()),extent=Math.max(size.x,size.y,size.z);
  if(extent<=1e-8)return null;
  const axis=size.x<=size.y && size.x<=size.z?0:size.y<=size.z?1:2;
  if(size.getComponent(axis)>extent*1e-5)return null;
  const normal=new THREE.Vector3().setComponent(axis,1).applyNormalMatrix(new THREE.Matrix3().getNormalMatrix(mesh.matrixWorld));
  const point=box.getCenter(new THREE.Vector3()).applyMatrix4(mesh.matrixWorld);
  const plane=new THREE.Plane().setFromNormalAndCoplanarPoint(normal,point);
  if(plane.distanceToPoint(cameraPosition)<0)plane.negate();
  return plane;
}

/** Mirror both perspective and orthographic cameras, clipping geometry behind
 * the mirror. The projection matrix keeps the original view's pixel density. */
export function reflectedCamera(camera:ViewCamera,plane:THREE.Plane):{camera:ViewCamera;matrix:THREE.Matrix4} {
  camera.updateMatrixWorld(true);
  const position=camera.getWorldPosition(new THREE.Vector3());
  const forward=camera.getWorldDirection(new THREE.Vector3());
  const mirror=(point:THREE.Vector3)=>point.addScaledVector(plane.normal,-2*plane.distanceToPoint(point));
  const reflected=camera.clone();reflected.parent=null;reflected.matrixAutoUpdate=true;reflected.matrixWorldAutoUpdate=true;
  reflected.position.copy(mirror(position.clone()));
  reflected.up.set(0,1,0).transformDirection(camera.matrixWorld).reflect(plane.normal);
  reflected.lookAt(mirror(position.clone().add(forward)));reflected.updateMatrixWorld(true);
  reflected.projectionMatrix.copy(camera.projectionMatrix);
  const matrix=new THREE.Matrix4().set(.5,0,0,.5,0,.5,0,.5,0,0,.5,.5,0,0,0,1)
    .multiply(reflected.projectionMatrix).multiply(reflected.matrixWorldInverse);
  const local=plane.clone().applyMatrix4(reflected.matrixWorldInverse);
  const clip=new THREE.Vector4(local.normal.x,local.normal.y,local.normal.z,local.constant);
  const inverse=reflected.projectionMatrix.clone().invert();
  const q=new THREE.Vector4(Math.sign(clip.x),Math.sign(clip.y),1,1).applyMatrix4(inverse);
  const denominator=clip.dot(q);
  if(Math.abs(denominator)>1e-8){
    clip.multiplyScalar(2/denominator);
    const p=reflected.projectionMatrix.elements;
    p[2]=clip.x-p[3]!;p[6]=clip.y-p[7]!;p[10]=clip.z-p[11]!;p[14]=clip.w-p[15]!;
  }
  reflected.projectionMatrixInverse.copy(reflected.projectionMatrix).invert();
  return {camera:reflected,matrix};
}

const fragment=/* glsl */`
uniform float pmReflectionMode;
uniform samplerCube pmReflectionCube;
uniform sampler2D pmReflectionPlane;
uniform float pmReflectionMaxLod;
varying vec4 vPmReflectionCoord;
vec4 pmSceneReflection(vec3 direction,float roughness) {
  // Microfacet roughness gives an angular footprint, rather than a fraction
  // of the mip count; moderate roughness must not still look like a mirror.
  float lod=clamp(pmReflectionMaxLod+log2(max(roughness*roughness,0.000001)),0.0,pmReflectionMaxLod);
  if(pmReflectionMode<1.5)return textureCubeLodEXT(pmReflectionCube,direction,lod);
  vec3 coord=vPmReflectionCoord.xyz/vPmReflectionCoord.w;
  if(vPmReflectionCoord.w<=0.0 || coord.x<0.0 || coord.x>1.0 || coord.y<0.0 || coord.y>1.0)return vec4(0.0);
  return texture2DLodEXT(pmReflectionPlane,coord.xy,lod);
}
`;
const lighting=/* glsl */`
#if defined( RE_IndirectSpecular )
if(pmReflectionMode>0.5){
  vec3 pmDirection=inverseTransformDirection(normalize(mix(reflect(-geometryViewDir,geometryNormal),geometryNormal,material.roughness*material.roughness)),viewMatrix);
  vec4 pmReflection=pmSceneReflection(pmDirection,material.roughness);
  radiance=mix(radiance,pmReflection.rgb/max(pmReflection.a,0.0001),clamp(pmReflection.a,0.0,1.0));
  #ifdef USE_CLEARCOAT
  vec3 pmCoatDirection=inverseTransformDirection(reflect(-geometryViewDir,geometryClearcoatNormal),viewMatrix);
  vec4 pmCoatReflection=pmSceneReflection(pmCoatDirection,material.clearcoatRoughness);
  clearcoatRadiance=mix(clearcoatRadiance,pmCoatReflection.rgb/max(pmCoatReflection.a,0.0001),clamp(pmCoatReflection.a,0.0,1.0));
  #endif
}
#endif
`;

/** One-bounce scene reflections. Captures keep the HDR environment as their
 * background and replace only specular radiance, preserving diffuse lighting. */
export class SceneReflections {
  private captures=new Map<string,Capture>();
  private bindings=new Map<ReflectionMaterial,Binding>();
  private warmup:THREE.WebGLRenderTarget|null=null;
  private shadowKey:string|null=null;
  private depthMaterial=new THREE.MeshBasicMaterial({colorWrite:false,depthWrite:false});
  private capturing=false;

  update(renderer:THREE.WebGLRenderer,scene:THREE.Scene,objects:Map<string,THREE.Object3D>,camera:ViewCamera,
    revision:number,width:number,height:number,depthIds?:Set<string>):void {
    if(this.capturing)return;
    scene.updateMatrixWorld(true);camera.updateMatrixWorld(true);
    for(const binding of this.bindings.values()){binding.mode.value=0;binding.cube.value=null;binding.plane.value=null;}
    const meshes:THREE.Mesh[]=[];
    scene.traverseVisible(object=>{const mesh=object as THREE.Mesh;if(mesh.isMesh && (objects.get(mesh.userData.sceneId)?.userData.layerOpacity ?? 1)>0 && (!depthIds || depthIds.has(mesh.userData.sceneId)))meshes.push(mesh);});
    const owners=new Set(meshes.map(mesh=>mesh.userData.sceneId));
    const frustum=new THREE.Frustum().setFromProjectionMatrix(new THREE.Matrix4().multiplyMatrices(camera.projectionMatrix,camera.matrixWorldInverse));
    const eye=camera.getWorldPosition(new THREE.Vector3()),active=new Set<string>();
    const allBounds=new THREE.Box3();for(const mesh of meshes)allBounds.expandByObject(mesh);
    const groups:string[]=[];
    for(const [id,root] of objects){
      if(owners.size<2 || !owners.has(id) || !root.visible)continue;
      const owned=meshes.filter(mesh=>mesh.userData.sceneId===id);
      const materials=[...new Set(owned.flatMap(mesh=>Array.isArray(mesh.material)?mesh.material:[mesh.material]))]
        .filter((material):material is ReflectionMaterial=>(material as ReflectionMaterial).isMeshStandardMaterial && material.visible && material.opacity>0 && (material as ReflectionMaterial).roughness<.9);
      if(!materials.length)continue;
      const bounds=new THREE.Box3();for(const mesh of owned)bounds.expandByObject(mesh);
      if(bounds.isEmpty() || !frustum.intersectsBox(bounds))continue;
      const plane=owned.length===1?reflectionPlane(owned[0]!,eye):null;
      const size=plane?0:this.cubeSize(bounds,camera,width,height,renderer.capabilities.maxCubemapSize);
      let capture=this.captures.get(id);
      if(capture && (capture.root!==root || !!capture.plane!==!!plane || (!plane && capture.target.width!==size))){capture.target.dispose();this.captures.delete(id);capture=undefined;}
      if(!capture){
        const options:THREE.RenderTargetOptions={type:THREE.HalfFloatType,minFilter:THREE.LinearMipmapLinearFilter,magFilter:THREE.LinearFilter,generateMipmaps:true,depthBuffer:true};
        const target=plane?new THREE.WebGLRenderTarget(width,height,options):new THREE.WebGLCubeRenderTarget(size,options);
        if(plane)target.samples=Math.min(4,renderer.capabilities.maxSamples);
        target.texture.colorSpace=THREE.LinearSRGBColorSpace;
        capture={root,target,plane,key:''};this.captures.set(id,capture);
      }
      if(plane && (capture.target.width!==width || capture.target.height!==height)){capture.target.setSize(width,height);capture.key='';}
      capture.plane=plane;active.add(id);
      const key=`${revision}:${depthIds?[...depthIds].sort().join(','):'all'}:${plane?camera.matrixWorld.elements+':'+camera.projectionMatrix.elements:''}`;
      if(capture.key!==key)groups.push(id);
      for(const material of materials){
        const binding=this.bind(material);
        if(plane){binding.plane.value=capture.target.texture;binding.matrix.value.copy(reflectedCamera(camera,plane).matrix);binding.mode.value=2;}
        else{binding.cube.value=capture.target.texture;binding.mode.value=1;}
        binding.lod.value=Math.log2(Math.max(capture.target.width,capture.target.height));
      }
    }
    for(const [id,capture] of this.captures)if(!active.has(id)){capture.target.dispose();this.captures.delete(id);}
    // Removed/replaced mesh materials must not be retained by the capture cache.
    const liveMaterials=new Set<THREE.Material>();
    scene.traverse(object=>{const mesh=object as THREE.Mesh;if(mesh.isMesh)for(const material of Array.isArray(mesh.material)?mesh.material:[mesh.material])liveMaterials.add(material);});
    for(const material of this.bindings.keys())if(!liveMaterials.has(material))this.bindings.delete(material);
    if(!groups.length)return;
    const enabled=new Map([...this.bindings.values()].map(binding=>[binding,binding.mode.value]));
    for(const binding of this.bindings.values())binding.mode.value=0;
    const visibility=new Map<THREE.Mesh,boolean>();
    const materials=new Map<THREE.Material,{opacity:number;transparent:boolean;depthWrite:boolean}>();
    scene.traverse(object=>{const mesh=object as THREE.Mesh;if(mesh.isMesh){
      visibility.set(mesh,mesh.visible);
      const opacity=objects.get(mesh.userData.sceneId)?.userData.layerOpacity ?? 1;
      if(opacity<=0 || depthIds && !depthIds.has(mesh.userData.sceneId))mesh.visible=false;
      if(opacity<1)for(const material of Array.isArray(mesh.material)?mesh.material:[mesh.material]){
        if(materials.has(material))continue;
        materials.set(material,{opacity:material.opacity,transparent:material.transparent,depthWrite:material.depthWrite});
        material.opacity*=Math.max(0,opacity);material.depthWrite=false;
        if(!material.transparent){material.transparent=true;material.needsUpdate=true;}
      }
    }});
    const target=renderer.getRenderTarget(),face=renderer.getActiveCubeFace(),mip=renderer.getActiveMipmapLevel();
    const viewport=renderer.getViewport(new THREE.Vector4()),scissor=renderer.getScissor(new THREE.Vector4()),scissorTest=renderer.getScissorTest();
    const clearColor=renderer.getClearColor(new THREE.Color()).clone(),clearAlpha=renderer.getClearAlpha();
    const autoClear=renderer.autoClear,shadowUpdate=renderer.shadowMap.autoUpdate,xr=renderer.xr.enabled;
    const background=scene.background,rotation=scene.backgroundRotation.clone(),intensity=scene.backgroundIntensity,override=scene.overrideMaterial;
    this.capturing=true;
    try{
      renderer.autoClear=true;renderer.xr.enabled=false;renderer.setScissorTest(false);
      const shadowKey=`${revision}:${depthIds?[...depthIds].sort().join(','):'all'}`;
      if(renderer.shadowMap.enabled && shadowKey!==this.shadowKey){
        this.warmup ??= new THREE.WebGLRenderTarget(1,1);
        scene.overrideMaterial=this.depthMaterial;renderer.setRenderTarget(this.warmup);
        renderer.shadowMap.autoUpdate=true;renderer.render(scene,camera);scene.overrideMaterial=override;
        this.shadowKey=shadowKey;
      }
      renderer.shadowMap.autoUpdate=false;
      scene.background=scene.environment ?? background;
      if(scene.environment){scene.backgroundRotation.copy(scene.environmentRotation);scene.backgroundIntensity=scene.environmentIntensity;}
      renderer.setClearColor(0,0);
      for(const id of groups){
        const capture=this.captures.get(id)!;
        const mipmaps=capture.target.texture.generateMipmaps;
        for(const mesh of meshes)if(mesh.userData.sceneId===id)mesh.visible=false;
        try{
          if(capture.plane){
            const mirror=reflectedCamera(camera,capture.plane);
            renderer.setRenderTarget(capture.target);renderer.state.buffers.depth.setMask(true);renderer.clear();
            renderer.render(scene,mirror.camera);
          }else{
            const bounds=new THREE.Box3();for(const mesh of meshes)if(mesh.userData.sceneId===id)bounds.expandByObject(mesh);
            const near=Math.max(.001,bounds.getSize(new THREE.Vector3()).length()*.0001);
            const far=Math.max(near*100,allBounds.getSize(new THREE.Vector3()).length()*4);
            const cube=new THREE.CubeCamera(near,far,capture.target as THREE.WebGLCubeRenderTarget);
            cube.position.copy(bounds.getCenter(new THREE.Vector3()));cube.update(renderer,scene);
          }
          capture.key=`${revision}:${depthIds?[...depthIds].sort().join(','):'all'}:${capture.plane?camera.matrixWorld.elements+':'+camera.projectionMatrix.elements:''}`;
        }finally{capture.target.texture.generateMipmaps=mipmaps;for(const mesh of meshes)if(mesh.userData.sceneId===id)mesh.visible=visibility.get(mesh)!;}
      }
    }finally{
      for(const [mesh,visible] of visibility)mesh.visible=visible;
      for(const [material,state] of materials){if(material.transparent!==state.transparent)material.needsUpdate=true;Object.assign(material,state);}
      for(const [binding,mode] of enabled)binding.mode.value=mode;
      scene.background=background;scene.backgroundRotation.copy(rotation);scene.backgroundIntensity=intensity;scene.overrideMaterial=override;
      renderer.autoClear=autoClear;renderer.shadowMap.autoUpdate=shadowUpdate;renderer.xr.enabled=xr;
      renderer.setRenderTarget(target,face,mip);renderer.setViewport(viewport);renderer.setScissor(scissor);renderer.setScissorTest(scissorTest);renderer.setClearColor(clearColor,clearAlpha);
      this.capturing=false;
    }
  }

  private cubeSize(bounds:THREE.Box3,camera:ViewCamera,width:number,height:number,limit:number):number {
    let minX=1,maxX=-1,minY=1,maxY=-1;
    for(const x of [bounds.min.x,bounds.max.x])for(const y of [bounds.min.y,bounds.max.y])for(const z of [bounds.min.z,bounds.max.z]){
      const p=new THREE.Vector3(x,y,z).project(camera);minX=Math.min(minX,p.x);maxX=Math.max(maxX,p.x);minY=Math.min(minY,p.y);maxY=Math.max(maxY,p.y);
    }
    const pixels=Math.max((Math.min(1,maxX)-Math.max(-1,minX))*width/2,(Math.min(1,maxY)-Math.max(-1,minY))*height/2);
    return Math.min(limit,THREE.MathUtils.ceilPowerOfTwo(Math.max(32,pixels/2)));
  }

  private bind(material:ReflectionMaterial):Binding {
    const existing=this.bindings.get(material);if(existing)return existing;
    const binding:Binding={mode:{value:0},cube:{value:null},plane:{value:null},matrix:{value:new THREE.Matrix4()},lod:{value:0}};
    const previous=material.onBeforeCompile,key=material.customProgramCacheKey();
    material.onBeforeCompile=function(shader,renderer){
      previous.call(this,shader,renderer);
      Object.assign(shader.uniforms,{pmReflectionMode:binding.mode,pmReflectionCube:binding.cube,pmReflectionPlane:binding.plane,pmReflectionMatrix:binding.matrix,pmReflectionMaxLod:binding.lod});
      shader.vertexShader='uniform mat4 pmReflectionMatrix;\nvarying vec4 vPmReflectionCoord;\n'+shader.vertexShader;
      shader.vertexShader=shader.vertexShader.replace('#include <worldpos_vertex>',`#include <worldpos_vertex>
        vec4 pmWorld=vec4(transformed,1.0);
        #ifdef USE_BATCHING
        pmWorld=batchingMatrix*pmWorld;
        #endif
        #ifdef USE_INSTANCING
        pmWorld=instanceMatrix*pmWorld;
        #endif
        vPmReflectionCoord=pmReflectionMatrix*modelMatrix*pmWorld;`);
      shader.fragmentShader=fragment+'\n'+shader.fragmentShader;
      shader.fragmentShader=shader.fragmentShader.replace('#include <lights_fragment_end>',lighting+'\n#include <lights_fragment_end>');
    };
    material.customProgramCacheKey=()=>`${key}:pm-scene-reflections-v1`;material.needsUpdate=true;
    this.bindings.set(material,binding);return binding;
  }

  dispose():void {
    for(const capture of this.captures.values())capture.target.dispose();this.captures.clear();
    this.bindings.clear();this.warmup?.dispose();this.warmup=null;this.depthMaterial.dispose();
  }
}
