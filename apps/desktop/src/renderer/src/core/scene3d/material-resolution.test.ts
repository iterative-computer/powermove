import {expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {materialTextureSize,projectedMaterialSize} from './material-resolution';
import {materialCompositionSample,compositionTextureFrame,clearCompositionTextures} from './composition-textures';
import {SceneRuntime} from './runtime';
import {parseScene} from './schema';

it('retains native image/comp dimensions and increases density to match the screen, bounded by the capture budget and device',()=>{
  expect(materialTextureSize(4096,2048)).toEqual({width:4096,height:2048});
  expect(materialTextureSize(200,100,{width:2048,height:1024})).toEqual({width:2048,height:1024});
  expect(materialTextureSize(4096,2048,{width:100,height:100})).toEqual({width:4096,height:2048});
  expect(materialTextureSize(16000,8000,undefined,8192)).toEqual({width:4096,height:2048});
  expect(materialTextureSize(200,100,{width:16384,height:16384},4096)).toEqual({width:4096,height:2048});
});

it('preserves an image layer bitmap even when its layout dimensions are smaller',()=>{
  const source={id:'image',type:'image',from:0,dur:10,d:{asset:'photo',w:320,h:160}};
  const receiver={id:'receiver'},PM:any={proj:{id:'root',compId:'comp',w:1920,h:1080,fps:30,layers:[source,receiver],comps:{}},
    assets:{get:()=>({kind:'image',el:{width:4096,height:2048}})},GL:{bounds:()=>({w:320,h:160}),maxTextureSize:8192}};
  expect(materialCompositionSample(PM,receiver,{layerId:'image'},0)).toMatchObject({width:4096,height:2048});
  expect(materialCompositionSample(PM,receiver,{layerId:'image'},0,{width:4096,height:2048})).toMatchObject({width:4096,height:2048});
});

function screen() {
  const geometry=new THREE.PlaneGeometry(2,2),texture=new THREE.Texture(),material=new THREE.MeshBasicMaterial({map:texture});
  const mesh=new THREE.Mesh(geometry,material),camera=new THREE.OrthographicCamera(-1,1,1,-1,.1,100);
  camera.position.z=3;camera.lookAt(0,0,0);camera.updateProjectionMatrix();camera.updateMatrixWorld(true);mesh.updateMatrixWorld(true);
  return {geometry,texture,material,mesh,camera};
}

it('requests more detail for camera zoom, larger exports, cropped UVs and texture transforms without rescanning vertices',()=>{
  const {geometry,texture,material,mesh,camera}=screen();
  const uv=geometry.getAttribute('uv'),read=vi.spyOn(uv,'getX');
  expect(projectedMaterialSize(mesh,texture,1024,1024,camera)).toEqual({width:1024,height:1024});
  const reads=read.mock.calls.length;
  expect(projectedMaterialSize(mesh,texture,2048,2048,camera)).toEqual({width:2048,height:2048});
  expect(read.mock.calls.length).toBe(reads);
  camera.zoom=2;camera.updateProjectionMatrix();
  expect(projectedMaterialSize(mesh,texture,1024,1024,camera)).toEqual({width:2048,height:2048});
  for(let i=0;i<uv.count;i++)uv.setX(i,uv.getX(i)*.5);uv.needsUpdate=true;
  expect(projectedMaterialSize(mesh,texture,1024,1024,camera)).toEqual({width:4096,height:2048});
  texture.repeat.x=.5;
  expect(projectedMaterialSize(mesh,texture,1024,1024,camera)).toEqual({width:8192,height:2048});
  geometry.dispose();texture.dispose();material.dispose();
});

it('upgrades the shared material before rendering and retains its density across animation and camera motion',()=>{
  const {geometry,material,mesh,camera}=screen();material.userData.sourceMaterialIndex=0;
  const runtime=new SceneRuntime(),raw=parseScene({objects:[{id:'screen',source:{assetId:'screen'},useSourceMaterials:true,
    material:{maps:{color:{compositionId:'artwork'},emissive:{compositionId:'artwork'}}}}]});
  const capture=vi.fn((_id:string,_map:any,minimum?:{width:number;height:number})=>{
    const size=materialTextureSize(64,64,minimum,4096);return {...size,pixels:new Uint8Array(4),key:JSON.stringify(size)};
  });
  const asset={object3d:mesh},sync=(time=0)=>runtime.sync(raw,()=>asset,p=>p.v,time,true,undefined,capture);
  const rendered=()=> (runtime.objects.get('screen') as THREE.Mesh).material as THREE.MeshBasicMaterial;
  sync();const small=rendered().map!,dispose=vi.spyOn(small,'dispose');
  // Imported materials may share their color texture with data slots.
  (rendered() as any).normalMap=small;(rendered() as any).roughnessMap=small;
  runtime.prepareMaterialTextures(2048,2048,camera,4096);
  expect(rendered().map!.image.width).toBe(2048);expect(dispose).toHaveBeenCalledOnce();
  expect(rendered().map!.generateMipmaps).toBe(true);expect(rendered().map!.anisotropy).toBe(16);
  expect((rendered() as any).normalMap).toBe(rendered().map);expect((rendered() as any).roughnessMap).toBe(rendered().map);
  const high=rendered().map;
  runtime.prepareMaterialTextures(1900,1900,camera,4096);expect(rendered().map).toBe(high);expect(capture).toHaveBeenCalledTimes(2);
  camera.zoom=2;camera.updateProjectionMatrix();runtime.prepareMaterialTextures(2048,2048,camera,4096);
  expect(rendered().map!.image.width).toBe(4096);
  sync(1);expect(capture.mock.calls.at(-1)?.[2]).toEqual({width:4096,height:4096});
  camera.zoom=.25;camera.updateProjectionMatrix();runtime.prepareMaterialTextures(2048,2048,camera,4096);
  sync(2);expect(capture.mock.calls.at(-1)?.[2]).toEqual({width:512,height:512});
  runtime.dispose();geometry.dispose();material.map!.dispose();material.dispose();
});

it('bounds material frame memory by bytes instead of retaining 32 large frames',()=>{
  const comp={id:'source',w:128,h:128,fps:30,dur:10,layers:[{id:'video',type:'video',from:0,dur:10,d:{}}]},receiver={id:'receiver',from:0,d:{}};
  const capture=vi.fn(()=>new Uint8Array(128*128*4));
  const PM:any={proj:{id:'root',compId:'root',layers:[receiver],comps:{source:comp}},GL:{maxTextureSize:8192,renderMaterialComposition:capture},Memory:{budget:()=>65536}};
  const map={compositionId:'source'};
  const first=compositionTextureFrame(PM,receiver,map,0);expect(compositionTextureFrame(PM,receiver,map,.001)).toBe(first);
  compositionTextureFrame(PM,receiver,map,1);
  expect(compositionTextureFrame(PM,receiver,map,0)).not.toBe(first);expect(capture).toHaveBeenCalledTimes(3);
  clearCompositionTextures(PM);
});

it('shares a sharper cached frame with material slots requesting fewer texels',()=>{
  const comp={id:'source',w:64,h:64,fps:30,dur:10,layers:[]},receiver={id:'receiver',from:0,d:{}};
  const capture=vi.fn((_comp:any,_time:number,width:number,height:number)=>new Uint8Array(width*height*4));
  const PM:any={proj:{id:'root',compId:'root',layers:[receiver],comps:{source:comp}},GL:{maxTextureSize:8192,renderMaterialComposition:capture}};
  const map={compositionId:'source'},high=compositionTextureFrame(PM,receiver,map,0,undefined,{width:512,height:512});
  expect(compositionTextureFrame(PM,receiver,map,1,undefined,{width:256,height:256})).toBe(high);
  expect(compositionTextureFrame(PM,receiver,map,2)).toBe(high);expect(capture).toHaveBeenCalledOnce();
  clearCompositionTextures(PM);
});

it('caps extreme demands while preserving aspect',()=>{expect(materialTextureSize(200,100,{width:1e12,height:1e12},16384)).toEqual({width:4096,height:2048});});
