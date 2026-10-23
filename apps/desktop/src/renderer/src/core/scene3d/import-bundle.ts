import * as THREE from 'three';
import type { MTLLoader as MaterialLoader } from 'three/addons/loaders/MTLLoader.js';
import { parseObj } from '../../kernel/obj';
import { disposeModel } from './import-model';
import { object3dFromEngine } from './compat-object3d';

const leaf=(path:string)=>decodeURIComponent(path).replaceAll('\\','/').split('/').pop()!.toLowerCase();

/** Give the Rust OBJ parser Chromium's decoded texture pixels. Today's
 * TextureLoader uses an HTML image before GLTFExporter draws to a canvas. */
async function chromiumTextureBytes(file:File):Promise<Uint8Array> {
  if(!/^image\/(png|jpeg|webp)$/.test(file.type)&&!/\.(png|jpe?g|webp)$/i.test(file.name))
    return new Uint8Array(await file.arrayBuffer());
  const url=URL.createObjectURL(file);
  try {
    const image=new Image();image.src=url;
    try { await image.decode(); } catch { return new Uint8Array(await file.arrayBuffer()); }
    const canvas=document.createElement('canvas');canvas.width=image.naturalWidth;canvas.height=image.naturalHeight;
    const context=canvas.getContext('2d');
    if(!context)return new Uint8Array(await file.arrayBuffer());
    context.drawImage(image,0,0);
    const png=await new Promise<Blob|null>(resolve=>canvas.toBlob(resolve,'image/png'));
    return png?new Uint8Array(await png.arrayBuffer()):new Uint8Array(await file.arrayBuffer());
  } finally { URL.revokeObjectURL(url); }
}

/** The engine's render materials use f32 factors. For durable bundles,
 * GLTFExporter historically wrote the source MTL's f64 factors and OBJ names.
 * Restore those values from the engine packer's GLB before Chromium encodes. */
async function restoreExporterInputs(root:THREE.Object3D,scene:any,glb:Uint8Array,files:File[]):Promise<void> {
  const view=new DataView(glb.buffer,glb.byteOffset,glb.byteLength);
  if(glb.length<20||view.getUint32(0,true)!==0x46546c67||view.getUint32(16,true)!==0x4e4f534a)
    throw new Error('The engine returned an invalid model bundle');
  const raw=JSON.parse(new TextDecoder().decode(glb.subarray(20,20+view.getUint32(12,true))));
  const original=scene.objects as any[];
  const bumpJobs:Promise<void>[]=[];
  const groupedMeshes:THREE.Object3D[]=[];
  const sourceImages=new Map<number,Promise<HTMLImageElement>>();
  const selectedImage=(index:number):Promise<HTMLImageElement>=>{
    const prior=sourceImages.get(index);if(prior)return prior;
    const file=files[index];
    if(!file)throw new Error('The engine bundle texture source is missing');
    const job=(async()=>{
      const url=URL.createObjectURL(file);
      try { const image=new Image();image.src=url;await image.decode();return image; }
      finally { URL.revokeObjectURL(url); }
    })();
    sourceImages.set(index,job);
    return job;
  };
  const restoreTexture=(ref:any,texture:THREE.Texture|null)=>{
    const imageIndex=raw.textures?.[ref?.index]?.source;
    const fileIndex=raw.images?.[imageIndex]?.extras?.__pmSourceFile;
    if(fileIndex==null||!texture)return;
    bumpJobs.push(selectedImage(fileIndex).then(image=>{
      // TextureLoader's HTML image is the input GLTFExporter drew before
      // packing. The intermediate GLB image was vertically flipped and
      // decoded again, which changes JPEG/WebP pixels by one color value.
      texture.image=image;
      texture.flipY=true;
      texture.userData={};
      texture.needsUpdate=true;
    }));
  };
  // Object 0 is the synthetic AuxScene, outside the exported OBJ root.
  let objectIndex=1;
  root.traverse((object:any)=>{
    const source=original[objectIndex++];
    if(!source)throw new Error('The engine bundle hierarchy is incomplete');
    if(object.userData?.name)object.name=object.userData.name;
    object.userData={};
    if(!source.primitive){
      const meshIndex=source.node==null?null:raw.nodes?.[source.node]?.mesh;
      if(meshIndex!=null&&raw.meshes?.[meshIndex]?.primitives?.length>1)groupedMeshes.push(object);
      return;
    }
    const [meshIndex,primitiveIndex]=source.primitive;
    const definition=raw.materials?.[raw.meshes?.[meshIndex]?.primitives?.[primitiveIndex]?.material];
    const material=object.material as THREE.MeshStandardMaterial|undefined;
    if(!definition||!material)return;
    const pbr=definition.pbrMetallicRoughness??{};
    if(pbr.baseColorFactor){
      material.color.setRGB(pbr.baseColorFactor[0],pbr.baseColorFactor[1],pbr.baseColorFactor[2]);
      material.opacity=pbr.baseColorFactor[3]??1;
    }
    if(pbr.metallicFactor!=null)material.metalness=pbr.metallicFactor;
    if(pbr.roughnessFactor!=null)material.roughness=pbr.roughnessFactor;
    if(definition.emissiveFactor)material.emissive.setRGB(definition.emissiveFactor[0],definition.emissiveFactor[1],definition.emissiveFactor[2]);
    restoreTexture(pbr.baseColorTexture,material.map);
    restoreTexture(definition.emissiveTexture,material.emissiveMap);
    const bump=definition.extras?.__pmBumpSource;
    if(bump){
      const file=files[bump.file];
      if(!file)throw new Error('The engine bundle bump source is missing');
      bumpJobs.push((async()=>{
        const url=URL.createObjectURL(file);
        try {
          const image=new Image();image.src=url;await image.decode();
          const input=new THREE.Texture(image);
          const old=material.normalMap;
          material.normalMap=bumpNormals(input,bump.strength);
          old?.dispose();
        } finally { URL.revokeObjectURL(url); }
      })());
    }
    material.userData={};
    object.geometry.userData={};
  });
  if(objectIndex!==original.length)throw new Error('The engine bundle hierarchy is incomplete');
  await Promise.all(bumpJobs);
  for(const group of groupedMeshes){
    const members=group.children as THREE.Mesh[];
    if(!members.length||members.some(mesh=>!mesh.isMesh||!mesh.geometry.index||Array.isArray(mesh.material)))
      throw new Error('The engine bundle material groups are incomplete');
    const base=members[0]!.geometry;
    const geometry=base.clone(),indices:number[]=[];
    geometry.clearGroups();
    for(const [slot,member] of members.entries()){
      const attributes=Object.keys(base.attributes);
      if(attributes.length!==Object.keys(member.geometry.attributes).length||attributes.some(key=>{
        const left=base.getAttribute(key),right=member.geometry.getAttribute(key);
        return !right||left.itemSize!==right.itemSize||left.count!==right.count||
          Array.from(left.array).some((value,i)=>value!==right.array[i]);
      }))throw new Error('The engine bundle material geometry differs between groups');
      const start=indices.length;
      for(const index of member.geometry.index!.array)indices.push(index);
      geometry.addGroup(start,indices.length-start,slot);
    }
    geometry.setIndex(indices);
    const mesh=new THREE.Mesh(geometry,members.map(member=>member.material as THREE.Material));
    mesh.name=group.name;
    mesh.position.copy(group.position);mesh.quaternion.copy(group.quaternion);mesh.scale.copy(group.scale);
    mesh.matrix.copy(group.matrix);mesh.matrixAutoUpdate=group.matrixAutoUpdate;
    mesh.userData=group.userData;
    const parent=group.parent;
    if(!parent)throw new Error('The engine bundle material group has no parent');
    const at=parent.children.indexOf(group);
    parent.remove(group);parent.add(mesh);
    parent.children.splice(parent.children.indexOf(mesh),1);
    parent.children.splice(at,0,mesh);
    for(const member of members)member.geometry.dispose();
  }
}

/** Convert a user-selected OBJ/MTL/image bundle to one durable, portable GLB.
 * No sibling file is read implicitly and no URL from model source is fetched. */
export async function prepareModelImport(files:File[],PM?:any):Promise<File> {
  if(!files.length||files.length>128)throw new Error('Choose a model and its material/texture files');
  if(files.some(f=>!(f instanceof File))||files.reduce((s,f)=>s+f.size,0)>128*1024*1024)throw new Error('Model files must be smaller than 128 MB in total');
  if(PM?.EngineDoc?.enabled){
    const importer=PM.EngineDoc.api?.prepareModelImport;
    if(typeof importer!=='function')throw new Error('The engine model importer is unavailable');
    const bundle=files.length>1&&files.some(file=>/\.obj$/i.test(file.name));
    const selected=await Promise.all(files.map(async file=>({name:file.name,mime:file.type,bytes:bundle?await chromiumTextureBytes(file):new Uint8Array(await file.arrayBuffer())})));
    const packed=importer.call(PM.EngineDoc.api,selected);
    if('unchanged' in packed)return files[packed.unchanged]!;
    const model=files.find(file=>/\.obj$/i.test(file.name));
    if(!model)throw new Error('Choose one model at a time with its material and texture files');
    const scene=PM.EngineDoc.api.importModel(packed.bytes);
    const adapted=await object3dFromEngine(scene,packed.bytes);
    let exported:THREE.Object3D|undefined;
    try {
      // The engine packs the scene and geometry. Today's GLTFExporter owns the
      // final Chromium image encoding and GLB byte layout. The input to it is
      // the OBJ group inside the engine's AuxScene, before normalization.
      exported=adapted.object3d.children[0]?.children[0]?.children[0];
      if(!exported)throw new Error('Could not package model materials');
      await restoreExporterInputs(exported,scene,packed.bytes,files);
      exported.removeFromParent();
      const data=await new GLTFExporter().parseAsync(exported,{binary:true,onlyVisible:false});
      if(!(data instanceof ArrayBuffer))throw new Error('Could not package model materials');
      return new File([data],model.name.replace(/\.obj$/i,'.glb'),{type:'model/gltf-binary'});
    } finally { if(exported)disposeModel(exported);else disposeModel(adapted.object3d); }
  }
  const models=files.filter(f=>/\.(obj|glb|gltf)$/i.test(f.name));
  if(models.length!==1)throw new Error('Choose one model at a time with its material and texture files');
  const model=models[0]!;
  if(!/\.obj$/i.test(model.name)||files.length===1)return model;
  const source=await model.text();parseObj(source); // Validate geometry before Three allocates it.
  const materials=files.filter(f=>/\.mtl$/i.test(f.name));
  if(!materials.length)throw new Error('Include the OBJ material (.mtl) file with its textures');
  const byName=new Map<string,File>();
  for(const file of files){const key=leaf(file.name);if(byName.has(key))throw new Error(`Duplicate file name: ${file.name}`);byName.set(key,file);}
  // Ordinary editor startup and single-file models never need the OBJ material
  // converter. Load it only after the selected bundle has passed validation.
  const [{OBJLoader},{MTLLoader},{GLTFExporter}]=await Promise.all([
    import('three/addons/loaders/OBJLoader.js'),
    import('three/addons/loaders/MTLLoader.js'),
    import('three/addons/exporters/GLTFExporter.js')
  ]);
  const urls=new Map<string,string>(),manager=new THREE.LoadingManager(),failures:string[]=[];
  let settle:(()=>void)|undefined;
  const loaded=new Promise<void>(resolve=>{settle=resolve;});
  manager.onLoad=()=>settle?.();manager.onError=url=>failures.push(url);
  manager.setURLModifier(url=>{
    if(url.startsWith('blob:'))return url;
    if(/^(?:[a-z]+:|\/\/)/i.test(url))throw new Error('Model texture references must be selected local files');
    const file=byName.get(leaf(url));
    if(!file||!/^image\/(png|jpeg|webp)$/.test(file.type)&&!/\.(png|jpe?g|webp)$/i.test(file.name))throw new Error(`Select the missing texture file: ${url}`);
    let local=urls.get(file.name);if(!local){local=URL.createObjectURL(file);urls.set(file.name,local);}return local;
  });
  let root:THREE.Group|undefined,creator:ReturnType<MaterialLoader['parse']>|undefined;
  try {
    const text=(await Promise.all(materials.map(f=>f.text()))).join('\n');
    creator=new MTLLoader(manager).parse(text,'');creator.preload();
    if(urls.size)await loaded;
    if(failures.length)throw new Error('A selected texture could not be decoded');
    root=new OBJLoader(manager).setMaterials(creator).parse(source);
    root.traverse((child:any)=>{
      if(!child.isMesh)return;
      const convert=(old:THREE.MeshPhongMaterial)=>{
        const material=new THREE.MeshStandardMaterial({color:old.color,emissive:old.emissive,
          opacity:old.opacity,transparent:old.transparent,side:THREE.DoubleSide,
          map:old.map,normalMap:old.normalMap,emissiveMap:old.emissiveMap,
          roughness:THREE.MathUtils.clamp(Math.sqrt(2/(old.shininess+2)),.02,1),metalness:0});
        if(old.bumpMap&&!material.normalMap)material.normalMap=bumpNormals(old.bumpMap,old.bumpScale);
        old.dispose();return material;
      };
      child.material=Array.isArray(child.material)?child.material.map(convert):convert(child.material);
    });
    const data=await new GLTFExporter().parseAsync(root,{binary:true,onlyVisible:false});
    if(!(data instanceof ArrayBuffer))throw new Error('Could not package model materials');
    return new File([data],model.name.replace(/\.obj$/i,'.glb'),{type:'model/gltf-binary'});
  } finally {
    for(const url of urls.values())URL.revokeObjectURL(url);
    if(root)disposeModel(root);
    if(creator)for(const material of Object.values(creator.materials)){
      for(const value of Object.values(material))if(value instanceof THREE.Texture)value.dispose();
      material.dispose();
    }
  }
}

/** glTF has normal maps, so turn OBJ's scalar bump map into a tangent normal map. */
function bumpNormals(texture:THREE.Texture,strength:number):THREE.Texture {
  const image=texture.image,width=Math.min(2048,image.width),height=Math.min(2048,image.height);
  const canvas=document.createElement('canvas');canvas.width=width;canvas.height=height;
  const ctx=canvas.getContext('2d')!;ctx.drawImage(image,0,0,width,height);
  const pixels=ctx.getImageData(0,0,width,height),input=pixels.data.slice();
  const sample=(x:number,y:number)=>{const i=(((y+height)%height)*width+(x+width)%width)*4;return (input[i]!+input[i+1]!+input[i+2]!)/765;};
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const n=new THREE.Vector3((sample(x-1,y)-sample(x+1,y))*strength,(sample(x,y-1)-sample(x,y+1))*strength,1).normalize(),i=(y*width+x)*4;
    pixels.data[i]=(n.x*.5+.5)*255;pixels.data[i+1]=(n.y*.5+.5)*255;pixels.data[i+2]=(n.z*.5+.5)*255;pixels.data[i+3]=255;
  }
  ctx.putImageData(pixels,0,0);const map=new THREE.CanvasTexture(canvas);map.flipY=texture.flipY;return map;
}
