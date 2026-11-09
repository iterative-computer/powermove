import {afterEach,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {compositionTextureFrame,clearCompositionTextures} from './composition-textures';
import {SceneRuntime} from './runtime';
import {parseScene} from './schema';

afterEach(()=>vi.unstubAllGlobals());

function editor(type='solid') {
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({w:64,h:64,fps:30,dur:10});
  const compId=PM.Comps.create({name:'Material',w:64,h:64,fps:30,dur:10},{open:false});
  const comp=PM.proj.comps[compId],paint=PM.mkLayer(type);paint.id='paint';paint.from=0;paint.dur=10;
  comp.layers.push(paint);
  const receiver={id:'receiver',from:0,d:{}};
  PM.proj.layers.push(receiver);
  PM.assets.get=(id:string)=>PM.assets.map.get(id);
  const capture=vi.fn(()=>new Uint8Array(64*64*4).fill(255));
  PM.GL={renderMaterialComposition:capture};
  return {PM,comp,paint,receiver,capture,map:{compositionId:compId}};
}

it('captures a static material once across playback and recaptures edits, assets, fonts and reopened projects',()=>{
  const {PM,paint,receiver,capture,map}=editor();
  const first=compositionTextureFrame(PM,receiver,map,0)!;
  for(let frame=1;frame<120;frame++)expect(compositionTextureFrame(PM,receiver,map,frame/30)).toBe(first);
  expect(capture).toHaveBeenCalledOnce();
  for(const invalidate of [()=>{paint.d.color='#112233';PM.touch();},()=>PM.bus.emit('assets'),()=>PM.bus.emit('fonts'),
    ()=>{PM.proj=PM.deserialize(PM.serialize());},()=>{PM.GL.gl={};}]){
    const before=capture.mock.calls.length;invalidate();
    expect(compositionTextureFrame(PM,receiver,map,1)).not.toBe(first);
    expect(capture).toHaveBeenCalledTimes(before+1);
  }
  clearCompositionTextures(PM);
});

it('keeps source pixels when only the receiving model or camera is edited',()=>{
  const {PM,receiver,capture,map}=editor();
  const first=compositionTextureFrame(PM,receiver,map,0);
  for(let step=0;step<30;step++){
    receiver.d={rotation:step};PM.touch();PM.bus.emit('project');
    expect(compositionTextureFrame(PM,receiver,map,step/30)).toBe(first);
  }
  expect(capture).toHaveBeenCalledOnce();clearCompositionTextures(PM);
});

it('does not reuse static pixels across layer entrances or exits',()=>{
  const {PM,paint,receiver,map,capture}=editor();paint.from=1;paint.dur=1;
  const before=compositionTextureFrame(PM,receiver,map,.5);
  expect(compositionTextureFrame(PM,receiver,map,.75)).toBe(before);
  const during=compositionTextureFrame(PM,receiver,map,1.25);
  expect(during).not.toBe(before);expect(compositionTextureFrame(PM,receiver,map,1.5)).toBe(during);
  expect(compositionTextureFrame(PM,receiver,map,2.5)).not.toBe(during);
  expect(capture).toHaveBeenCalledTimes(3);clearCompositionTextures(PM);
});

it.each(['keys','expression','video','effect','text animator','precomp'])('keeps %s sources on their exact frame clock',kind=>{
  const {PM,paint,receiver,map,capture}=editor();
  if(kind==='keys')paint.d.color={v:'#FF0000',kf:[PM.KF(0,'#FF0000'),PM.KF(2,'#0000FF')],expr:null};
  if(kind==='expression')paint.d.color={v:'#FF0000',kf:[],expr:'time'};
  if(kind==='video')paint.type='video';
  if(kind==='effect')paint.fx=[{type:'noise',on:true,p:{}}];
  if(kind==='text animator'){paint.type='text';paint.d.animators=[{id:'anim'}];}
  if(kind==='precomp')paint.type='precomp';
  const first=compositionTextureFrame(PM,receiver,map,.5);
  expect(compositionTextureFrame(PM,receiver,map,.501)).toBe(first);
  expect(compositionTextureFrame(PM,receiver,map,.6)).not.toBe(first);
  expect(capture).toHaveBeenCalledTimes(2);clearCompositionTextures(PM);
});

it('retains immutable image sources, observes replacement/close, and never freezes mutable canvases',()=>{
  class Bitmap {width=64;height=64;}
  vi.stubGlobal('ImageBitmap',Bitmap);
  const {PM,paint,receiver,map,capture}=editor('image');paint.d.asset='image';
  const asset={kind:'image',el:new Bitmap()};PM.assets.map.set('image',asset);
  const first=compositionTextureFrame(PM,receiver,map,0);
  expect(compositionTextureFrame(PM,receiver,map,1)).toBe(first);
  asset.el=new Bitmap();expect(compositionTextureFrame(PM,receiver,map,1)).not.toBe(first);
  asset.el.width=0;const closed=compositionTextureFrame(PM,receiver,map,2);
  expect(compositionTextureFrame(PM,receiver,map,3)).not.toBe(closed);
  asset.el={width:64,height:64};const mutable=compositionTextureFrame(PM,receiver,map,4);
  expect(compositionTextureFrame(PM,receiver,map,5)).not.toBe(mutable);
  expect(capture).toHaveBeenCalledTimes(6);clearCompositionTextures(PM);
});

it.each(['composition','image'])('shares %s textures across imported meshes and slots, preserving UV variants and disposal',kind=>{
  const model=new THREE.Group(),geometry=new THREE.PlaneGeometry();
  for(let index=0;index<12;index++){
    const material=new THREE.MeshStandardMaterial();material.userData.sourceMaterialIndex=index;
    if(index===11){material.map=new THREE.Texture();material.map.repeat.set(2,3);}
    model.add(new THREE.Mesh(geometry,material));
  }
  const frame={pixels:new Uint8Array(16).fill(255),width:2,height:2,key:'first'};
  const capture=vi.fn(()=>frame),image={kind:'image',el:{width:2,height:2}},asset={object3d:model};
  const map=kind==='composition'?{compositionId:'source'}:'image';
  const raw=parseScene({objects:[{id:'model',source:{assetId:'model'},useSourceMaterials:true,material:{maps:{color:map,emissive:map}}}]});
  const runtime=new SceneRuntime(),sync=()=>runtime.sync(raw,id=>id==='model'?asset:image,p=>p.v,0,true,undefined,capture);
  const maps=()=>{
    const out:THREE.Texture[]=[];runtime.objects.get('model')!.traverse((child:any)=>{
      if(child.isMesh)out.push(child.material.map,child.material.emissiveMap);
    });return out;
  };
  sync();const textures=maps();
  // Null original maps share, and the one explicit, transformed UV map stays separate.
  expect(new Set(textures).size).toBe(2);
  expect(textures.at(-2)!.repeat.toArray()).toEqual([2,3]);
  expect(textures[0]!.repeat.toArray()).toEqual([1,1]);
  if(kind==='composition')expect(capture).toHaveBeenCalledOnce();
  const version=textures[0]!.version;sync();expect(textures[0]!.version).toBe(version);
  const disposals=[...new Set(textures)].map(texture=>vi.spyOn(texture,'dispose'));
  raw.objects[0]!.material.maps={};sync();for(const dispose of disposals)expect(dispose).toHaveBeenCalledOnce();
  runtime.dispose();geometry.dispose();model.traverse((child:any)=>{child.material?.map?.dispose();child.material?.dispose();});
});

it('keeps different composition sources separate on a generated material',()=>{
  const runtime=new SceneRuntime(),raw=parseScene({objects:[{id:'model',source:{primitive:'box'},material:{maps:{color:{compositionId:'red'},emissive:{compositionId:'blue'}}}}]});
  const red={pixels:new Uint8Array([255,0,0,255]),width:1,height:1,key:'red'},blue={...red,pixels:new Uint8Array([0,0,255,255]),key:'blue'};
  runtime.sync(raw,()=>null,p=>p.v,0,true,undefined,(_id,map)=>typeof map==='object' && 'compositionId' in map && map.compositionId==='red'?red:blue);
  const material=(runtime.objects.get('model') as THREE.Mesh).material as THREE.MeshStandardMaterial;
  expect(material.map!.image.data).toBe(red.pixels);expect(material.emissiveMap!.image.data).toBe(blue.pixels);
  runtime.dispose();
});

it('retains static group textures but observes descendant edits, animation and external parent rigs',()=>{
  const {PM,comp,paint,receiver,capture}=editor();
  const group=PM.mkLayer('group');group.id='artwork';group.on=false;paint.group=group.id;
  PM.proj.layers.push(group,paint);comp.layers=[];PM.touch();
  const map={layerId:group.id},first=compositionTextureFrame(PM,receiver,map,0);
  expect(first).not.toBeNull();expect(compositionTextureFrame(PM,receiver,map,1)).toBe(first);expect(capture).toHaveBeenCalledOnce();
  paint.d.color='#00FF00';PM.touch();const edited=compositionTextureFrame(PM,receiver,map,1);
  expect(edited).not.toBe(first);
  paint.p['position.x'].kf=[PM.KF(0,0),PM.KF(2,100)];PM.touch();
  expect(compositionTextureFrame(PM,receiver,map,2)).not.toBe(compositionTextureFrame(PM,receiver,map,1));
  paint.p['position.x'].kf=[];
  const rig=PM.mkLayer('null');rig.id='rig';rig.p.rotation.kf=[PM.KF(0,0),PM.KF(2,90)];paint.parent=rig.id;PM.proj.layers.push(rig);PM.touch();
  expect(compositionTextureFrame(PM,receiver,map,2)).not.toBe(compositionTextureFrame(PM,receiver,map,1));
  clearCompositionTextures(PM);
});
