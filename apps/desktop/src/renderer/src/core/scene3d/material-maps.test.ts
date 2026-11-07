import {readFileSync} from 'node:fs';
import {describe,it,expect,vi} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS,layer3DAssetIds} from './layers';
import {listModelMaterials,listValidMaterialCompositions,validateCompositionMaps} from './material-maps';
import {compositionTextureFrame,materialCompositionSample} from './composition-textures';
import {loadGltf,disposeModel} from './import-model';
import {screenGlb} from './__fixtures__/screen-glb';
import {SceneRuntime} from './runtime';
import {parseScene,sceneAssetIds} from './schema';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Film',w:320,h:180,fps:30,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  PM.assets.get=(id:string)=>PM.assets.map.get(id);
  PM.assets.map.set('screen',{kind:'model',materials:[{index:0,name:'Screen'},{index:1,name:'Screen'}]});
  expect(editScene(PM,{operation:'add_object',object:{id:'model',source:{assetId:'screen'},useSourceMaterials:true}}).ok).toBe(true);
  const source=PM.Comps.create({name:'Animation',w:128,h:64,fps:24,dur:2},{open:false});
  return {PM,source,set:(map:any,slot='emissive',materialIndex:any=0)=>editScene(PM,{operation:'set_material_map',target:'model',slot:slot as any,map,materialIndex})};
}

describe('composition material maps',()=>{
  it('retains original glTF material names and indices including duplicate display names',async()=>{
    const fixture=readFileSync(new URL('./__fixtures__/screen.glb',import.meta.url));
    expect([...fixture]).toEqual([...screenGlb()]);
    const loaded=await loadGltf(new Blob([new Uint8Array(fixture)]));
    expect(loaded.materials).toEqual([{index:0,name:'Screen'}]);
    loaded.object3d.traverse((mesh:any)=>{if(mesh.isMesh)expect(mesh.material.userData).toMatchObject({sourceMaterialIndex:0,sourceMaterialName:'Screen'});});
    disposeModel(loaded.object3d);
    const {PM}=editor();expect(listModelMaterials(PM,'model').map(m=>[m.index,m.name])).toEqual([[0,'Screen'],[1,'Screen']]);
  });
  it('saves, reopens, renames, clears and replays undo/redo using durable composition IDs',()=>{
    const {PM,source,set}=editor();expect(set({compositionId:source}).ok).toBe(true);
    expect(PM.L('model').d.data.object.sourceMaterials[0]).toEqual({emissive:{compositionId:source}});
    PM.hist.undo();expect(PM.L('model').d.data.object.sourceMaterials).toEqual({});
    PM.hist.redo();expect(PM.L('model').d.data.object.sourceMaterials[0].emissive).toEqual({compositionId:source});
    PM.Comps.rename(source,'New name');
    const saved=PM.serialize();PM.replaceProject(PM.deserialize(saved));
    expect(listValidMaterialCompositions(PM,'model').find(c=>c.id===source)?.name).toBe('New name');
    expect(PM.L('model').d.data.object.sourceMaterials[0].emissive).toEqual({compositionId:source});
    expect(set('paint','color',1).ok).toBe(true);
    expect(layer3DAssetIds(PM.L('model'))).toEqual(['screen','paint']);
    expect(set(null).ok).toBe(true);expect(PM.L('model').d.data.object.sourceMaterials[0]).toBeUndefined();
    PM.hist.undo();expect(PM.L('model').d.data.object.sourceMaterials[0].emissive).toEqual({compositionId:source});
    const scene=parseScene({objects:[{id:'model',source:{primitive:'box'},material:{maps:{color:{compositionId:source}}},sourceMaterials:{0:{color:'paint'}}}]});
    expect(sceneAssetIds(scene)).toEqual(['paint']);
  });
  it('rejects self, ancestor, indirect material/precomp cycles and atomic command bypasses',()=>{
    const {PM,source,set}=editor(),root=PM.proj.compId;
    expect(set({compositionId:root})).toMatchObject({ok:false,message:expect.stringContaining('cycle')});
    PM.proj.comps[source].layers.push({id:'nested',type:'precomp',d:{comp:root}});
    expect(listValidMaterialCompositions(PM,'model')).toEqual([]);
    expect(set({compositionId:source}).ok).toBe(false);
    PM.proj.comps[source].layers=[];
    expect(set({compositionId:source}).ok).toBe(true);
    PM.Comps.open(source);expect(PM.Comps.canNest(root)).toBe(false);expect(PM.Comps.addToTimeline(root)).toBeNull();
    expect(editScene(PM,{operation:'add_object',object:{id:'inner',source:{primitive:'box'},material:{maps:{color:{compositionId:root}}}}}).ok).toBe(false);
    PM.Comps.open(root);const before=PM.serialize();
    const object=JSON.parse(JSON.stringify(PM.L('model').d.data.object));object.sourceMaterials[0].emissive={compositionId:root};
    expect(PM.Edit.apply([{type:'set_layer',target:'model',patch:{name:'Must roll back'}},{type:'set_content',target:'model',patch:{data:{object}}}]).ok).toBe(false);
    expect(PM.serialize()).toBe(before);expect(()=>validateCompositionMaps(PM.proj)).not.toThrow();
    expect(set({compositionId:source},'normal').ok).toBe(false);
    expect(set({compositionId:'missing'}).ok).toBe(false);
  });
  it('samples source time and frame rate, bounds resolution, and reuses unchanged source frames',()=>{
    const {PM,source}=editor(),layer=PM.L('model'),map={compositionId:source};layer.from=.5;
    expect(PM.Edit.apply({type:'set_content',target:'model',patch:{speed:2,trim:.25}}).ok).toBe(true);
    const capture=vi.fn((..._args:any[])=>new Uint8Array(128*64*4));PM.GL.renderMaterialComposition=capture;
    const first=compositionTextureFrame(PM,layer,map,.5)!;
    expect(capture.mock.calls[0]?.slice(1,4)).toEqual([.25,128,64]);
    expect(compositionTextureFrame(PM,layer,map,.501)).toBe(first);expect(capture).toHaveBeenCalledOnce();
    expect(compositionTextureFrame(PM,layer,map,.6)).not.toBe(first);expect(capture).toHaveBeenCalledTimes(2);
    expect(PM.Edit.apply({type:'set_content',target:'model',patch:{timeRemap:true,sourceTime:1.5}}).ok).toBe(true);
    expect(materialCompositionSample(PM,layer,map,.5)?.time).toBe(1.5);
    PM.proj.comps[source].w=4096;PM.proj.comps[source].h=2048;
    expect(materialCompositionSample(PM,layer,map,.5)).toMatchObject({width:1024,height:512});
    expect(materialCompositionSample(PM,layer,{compositionId:PM.proj.compId},.5)).toBeNull();
    PM.Comps.remove(source);expect(compositionTextureFrame(PM,layer,map,1)).toBeNull();
  });
  it('supports unlit glTF screens without making them depend on scene lights',()=>{
    const original=new THREE.MeshBasicMaterial({color:'#223344'});original.userData.sourceMaterialIndex=0;
    const model=new THREE.Mesh(new THREE.PlaneGeometry(),original),runtime=new SceneRuntime();
    const raw=parseScene({objects:[{id:'model',source:{assetId:'screen'},useSourceMaterials:true,sourceMaterials:{0:{emissive:{compositionId:'comp'}}}}]});
    const frame={pixels:new Uint8Array([255,0,0,255]),width:1,height:1,key:'frame'};
    const sync=()=>runtime.sync(raw,()=>({object3d:model}),p=>p.v,0,true,undefined,()=>frame);
    sync();let rendered:any;runtime.objects.get('model')!.traverse((mesh:any)=>{if(mesh.isMesh)rendered=mesh.material;});
    expect(rendered.isMeshBasicMaterial).toBe(true);expect(rendered.map.image.data).toBe(frame.pixels);
    expect(rendered.color.getHexString()).toBe('ffffff');expect(original.map).toBeNull();
    delete raw.objects[0]!.sourceMaterials[0];sync();
    runtime.objects.get('model')!.traverse((mesh:any)=>{if(mesh.isMesh){expect(mesh.material.map).toBeNull();expect(mesh.material.color.getHexString()).toBe('223344');}});
    runtime.dispose();model.geometry.dispose();original.dispose();
  });
  it('restores original source materials on clear or missing composition without mutating the imported model',async()=>{
    const loaded=await loadGltf(new Blob([screenGlb()])),runtime=new SceneRuntime();
    loaded.object3d.traverse((mesh:any)=>{if(mesh.isMesh){mesh.material.map=new THREE.Texture();mesh.material.map.channel=1;mesh.material.map.repeat.set(2,3);}});
    let available=true;
    const frame={pixels:new Uint8Array([255,0,0,255]),width:1,height:1,key:'frame'};
    const raw=parseScene({objects:[{id:'model',source:{assetId:'screen'},useSourceMaterials:true,sourceMaterials:{0:{color:{compositionId:'comp'},emissive:{compositionId:'comp'}}}}]});
    const sync=()=>runtime.sync(raw,()=>loaded,prop=>prop.v,0,true,undefined,()=>available?frame:null);
    sync();let material:any;runtime.objects.get('model')!.traverse((m:any)=>{if(m.isMesh)material=m.material;});
    expect(material.map.channel).toBe(1);expect(material.map.repeat.toArray()).toEqual([2,3]);
    expect(material.emissiveMap.channel).toBe(0);
    expect(material.emissiveMap.image.data).toBe(frame.pixels);expect(material.emissiveMap.flipY).toBe(false);
    expect(material.emissiveMap.colorSpace).toBe(THREE.SRGBColorSpace);expect(material.emissive.getHexString()).toBe('ffffff');
    const texture=material.emissiveMap,version=texture.version;sync();expect(texture.version).toBe(version);
    available=false;sync();expect(material.emissiveMap).toBeNull();expect(material.emissive.getHexString()).toBe('000000');
    available=true;sync();delete raw.objects[0]!.sourceMaterials[0];sync();expect(material.emissiveMap).toBeNull();
    loaded.object3d.traverse((mesh:any)=>{if(mesh.isMesh)expect(mesh.material.emissiveMap).toBeNull();});
    runtime.dispose();disposeModel(loaded.object3d);
  });
});
