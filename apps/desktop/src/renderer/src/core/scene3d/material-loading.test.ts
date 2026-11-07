// @vitest-environment happy-dom
import {it,expect,vi} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {editScene} from './operations';
import {compositionRuntime,disposeSceneRuntimes} from './service';
import {LAYER3D_DEFINITIONS} from './layers';

it.each(['shape','generated','imported','unlit'] as const)('%s retains its material while layer/composition video frames load and seek',kind=>{
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({w:320,h:180,dur:3,fps:30});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  PM.assets.get=(id:string)=>PM.assets.map.get(id);
  if(kind==='shape'){
    expect(PM.Edit.apply({type:'add_layer',id:'receiver',layerType:'shape',content:{w:80,h:50,color:'#FF6600'}}).ok).toBe(true);
    expect(editScene(PM,{operation:'set_extrusion',target:'receiver',patch:{enabled:true,depth:20}}).ok).toBe(true);
  }else{
    const original=kind==='unlit'?new THREE.MeshBasicMaterial({color:'#FF6600'}):new THREE.MeshStandardMaterial({color:'#FF6600'});
    original.userData.sourceMaterialIndex=0;
    PM.assets.map.set('model',{kind:'model',object3d:new THREE.Mesh(new THREE.BoxGeometry(),original),materials:[{index:0,name:'Original'}]});
    expect(editScene(PM,{operation:'add_object',object:{id:'receiver',source:kind==='generated'?{primitive:'box'}:{assetId:'model'},useSourceMaterials:kind!=='generated',material:{p:{color:'#FF6600'}}}}).ok).toBe(true);
  }
  PM.Edit.apply({type:'add_layer',id:'video',layerType:'video',visible:false,content:{asset:'video-asset',w:8,h:8}});
  const comp=PM.Comps.create({name:'Video comp',w:8,h:8},{open:false});
  PM.proj.comps[comp].layers.push({...PM.mkLayer('video'),id:'nested-video',d:{asset:'video-asset',w:8,h:8}});
  let pixels:Uint8Array<ArrayBuffer>|null=null;
  const capture=vi.fn(()=>pixels);
  PM.GL={bounds:()=>({w:8,h:8}),renderMaterialComposition:capture};
  const material=()=>{
    const runtime=compositionRuntime(PM),root=runtime.objects.get('receiver')!;
    let result:any;root.traverse((node:any)=>{if(node.material)result=Array.isArray(node.material)?node.material[0]:node.material;});
    expect(root.visible).toBe(true);expect(result.opacity).toBeGreaterThan(0);return result;
  };
  for(const map of [{layerId:'video'},{compositionId:comp}]){
    expect(editScene(PM,{operation:'set_material_map',target:'receiver',materialIndex:0,slot:'color',map}).ok).toBe(true);
    const old=material().map;
    expect(compositionRuntime(PM).materialsPending).toBe(true);
    // Every waiting paint has a live mesh and retains the last map/color.
    for(let i=0;i<4;i++)expect(material().map).toBe(old);
    pixels=new Uint8Array(8*8*4).fill(255);
    const loaded=material().map;expect(loaded).toBeInstanceOf(THREE.DataTexture);
    expect(compositionRuntime(PM).materialsPending).toBe(false);
    // A new seek is not cached as a transparent frame, and retries at the same time.
    PM.time+=.1;pixels=null;expect(material().map).toBe(loaded);
    expect(compositionRuntime(PM).materialsPending).toBe(true);
    const previous=loaded.image.data;
    pixels=new Uint8Array(8*8*4).fill(128);for(let i=3;i<pixels.length;i+=4)pixels[i]=255;
    expect(material().map.image.data).not.toBe(previous);
    expect(compositionRuntime(PM).materialsPending).toBe(false);
    // Intentional transparent output (e.g. source outside its in/out) is ready.
    PM.time+=.1;pixels=new Uint8Array(8*8*4);expect(material().map.image.data).toBe(pixels);
    expect(compositionRuntime(PM).materialsPending).toBe(false);
    pixels=null;PM.time+=.1;
  }
  const oldTexture=material().map,dispose=vi.spyOn(oldTexture,'dispose');
  PM.proj.comps[comp].w=16;PM.proj.comps[comp].h=4;PM.touch();
  pixels=new Uint8Array(16*4*4).fill(255);
  const resized=material().map;
  expect(resized).not.toBe(oldTexture);expect(dispose).toHaveBeenCalledOnce();
  expect(resized.image).toMatchObject({width:16,height:4});
  // Deleting a source restores the actual original, rather than keeping stale pixels.
  PM.Comps.remove(comp);expect(material().map).toBeNull();
  expect(compositionRuntime(PM).materialsPending).toBe(false);
  expect(capture.mock.calls.length).toBeGreaterThan(10);
  disposeSceneRuntimes(PM);
});
