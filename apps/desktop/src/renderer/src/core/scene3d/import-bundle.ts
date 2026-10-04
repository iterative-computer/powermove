import * as THREE from 'three';
import { OBJLoader } from 'three/addons/loaders/OBJLoader.js';
import { MTLLoader } from 'three/addons/loaders/MTLLoader.js';
import { GLTFExporter } from 'three/addons/exporters/GLTFExporter.js';
import { parseObj } from '../../kernel/obj';
import { disposeModel } from './import-model';

const leaf=(path:string)=>decodeURIComponent(path).replaceAll('\\','/').split('/').pop()!.toLowerCase();

/** Convert a user-selected OBJ/MTL/image bundle to one durable, portable GLB.
 * No sibling file is read implicitly and no URL from model source is fetched. */
export async function prepareModelImport(files:File[]):Promise<File> {
  if(!files.length||files.length>128)throw new Error('Choose a model and its material/texture files');
  if(files.some(f=>!(f instanceof File))||files.reduce((s,f)=>s+f.size,0)>128*1024*1024)throw new Error('Model files must be smaller than 128 MB in total');
  const models=files.filter(f=>/\.(obj|glb|gltf)$/i.test(f.name));
  if(models.length!==1)throw new Error('Choose one model at a time with its material and texture files');
  const model=models[0]!;
  if(!/\.obj$/i.test(model.name)||files.length===1)return model;
  const source=await model.text();parseObj(source); // Validate geometry before Three allocates it.
  const materials=files.filter(f=>/\.mtl$/i.test(f.name));
  if(!materials.length)throw new Error('Include the OBJ material (.mtl) file with its textures');
  const byName=new Map<string,File>();
  for(const file of files){const key=leaf(file.name);if(byName.has(key))throw new Error(`Duplicate file name: ${file.name}`);byName.set(key,file);}
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
  let root:THREE.Group|undefined,creator:ReturnType<MTLLoader['parse']>|undefined;
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
