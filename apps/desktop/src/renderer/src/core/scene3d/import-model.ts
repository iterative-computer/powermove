import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

/** Self-contained glTF only: imports must never fetch untrusted remote dependencies. */
export async function loadGltf(blob:Blob):Promise<{object3d:THREE.Group;animations:THREE.AnimationClip[];vertices:number;triangles:number}> {
  if(blob.size>128*1024*1024)throw new Error('GLB models must be smaller than 128 MB');
  const data=await blob.arrayBuffer();
  const bytes=new Uint8Array(data),view=new DataView(data);
  let json:any;
  if(bytes.length>=20 && view.getUint32(0,true)===0x46546c67) {
    if(view.getUint32(4,true)!==2 || view.getUint32(8,true)!==data.byteLength)throw new Error('Invalid GLB container');
    const length=view.getUint32(12,true);
    if(view.getUint32(16,true)!==0x4e4f534a || 20+length>bytes.length)throw new Error('Invalid GLB scene chunk');
    json=JSON.parse(new TextDecoder().decode(bytes.subarray(20,20+length)));
  } else json=JSON.parse(new TextDecoder().decode(data));
  for(const dependency of [...(json.buffers || []),...(json.images || [])]) {
    if(dependency.uri && !/^data:(?:application\/octet-stream|application\/gltf-buffer|image\/(?:png|jpeg|webp));base64,[a-z0-9+/=\s]+$/i.test(dependency.uri))
      throw new Error('Use a self-contained GLB with embedded textures and geometry. External glTF files are not loaded.');
  }
  let predictedVertices=0;
  for(const accessor of json.accessors || []) {
    if(!Number.isSafeInteger(accessor.count)||accessor.count<0||accessor.count>1_500_000)throw new Error('Invalid or oversized glTF geometry');
  }
  for(const mesh of json.meshes || [])for(const primitive of mesh.primitives || [])
    predictedVertices+=json.accessors?.[primitive.attributes?.POSITION]?.count || 0;
  if(predictedVertices>1_000_000 || (json.nodes?.length || 0)>4096)throw new Error('GLB exceeds the scene complexity budget');
  const manager=new THREE.LoadingManager(),embedded=new Map<string,string>();
  manager.setURLModifier(url=>{
    if(url.startsWith('blob:'))return url;
    if(!url.startsWith('data:'))throw new Error('External model resources are not allowed');
    let blobUrl=embedded.get(url);
    if(!blobUrl){
      const match=/^data:([^;]+);base64,(.*)$/s.exec(url);
      if(!match)throw new Error('Invalid embedded model resource');
      const binary=atob(match[2]!),bytes=Uint8Array.from(binary,c=>c.charCodeAt(0));
      blobUrl=URL.createObjectURL(new Blob([bytes],{type:match[1]}));embedded.set(url,blobUrl);
    }
    return blobUrl;
  });
  const loader=new GLTFLoader(manager);
  let result;
  try{result=await loader.parseAsync(data,'');}
  finally{for(const url of embedded.values())URL.revokeObjectURL(url);}
  const scene=result.scene;
  let vertices=0,triangles=0;
  scene.traverse((object:any)=>{
    if(!object.isMesh)return;
    const geometry=object.geometry;
    vertices+=geometry.attributes.position?.count || 0;
    triangles+=(geometry.index?.count || geometry.attributes.position?.count || 0)/3;
  });
  if(!vertices || triangles>500_000){disposeModel(scene);throw new Error('GLB has no supported geometry or exceeds 500,000 triangles');}
  const bounds=new THREE.Box3().setFromObject(scene),center=bounds.getCenter(new THREE.Vector3());
  const radius=bounds.getSize(new THREE.Vector3()).length()/2;
  if(!Number.isFinite(radius)||radius<1e-10){disposeModel(scene);throw new Error('GLB model has no measurable size');}
  const normalization=new THREE.Group(),root=new THREE.Group();
  normalization.add(scene);normalization.scale.setScalar(1/radius);scene.position.sub(center);
  root.add(normalization);
  return {object3d:root,animations:result.animations,vertices,triangles:Math.ceil(triangles)};
}
export function disposeModel(root:THREE.Object3D):void {
  const textures=new Set<THREE.Texture>(),geometries=new Set<THREE.BufferGeometry>(),materials=new Set<THREE.Material>();
  root.traverse((o:any)=>{if(o.geometry)geometries.add(o.geometry);
    for(const m of Array.isArray(o.material)?o.material:[o.material])if(m){materials.add(m);for(const v of Object.values<any>(m))if(v?.isTexture)textures.add(v);}});
  for(const g of geometries)g.dispose();for(const m of materials)m.dispose();
  for(const t of textures){t.dispose();t.image?.close?.();}
}
