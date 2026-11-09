import {describe,it,expect,vi} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS,compositionScene,layer3DProperties} from './layers';
import {listEligibleMaterialLayers,materialLayerLinks,validateCompositionMaps} from './material-maps';
import {compositionTextureFrame,materialCompositionSample} from './composition-textures';
import {readMaterialProperties,setAmbientResponse} from './material-properties';
import {parseScene,sceneProperties} from './schema';
import {SceneRuntime} from './runtime';
import {remapLayerInputs} from '../effect-inputs';
import {sourceTime} from '../../legacy/core/retiming';
import {videoClipsAt} from '../../legacy/core/video-timeline';
import {makeScene3DAPI} from './api';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Film',w:320,h:180,fps:30,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  PM.assets.get=(id:string)=>PM.assets.map.get(id);
  PM.assets.map.set('screen',{kind:'model',materials:[{index:0,name:'Screen'},{index:1,name:'Frame'}]});
  expect(editScene(PM,{operation:'add_object',object:{id:'model',source:{assetId:'screen'},useSourceMaterials:true}}).ok).toBe(true);
  const add=(id:string,type='solid')=>{const l=PM.mkLayer(type,{name:id});l.id=id;PM.proj.layers.push(l);return l;};
  const source=add('paint');
  const set=(map:any,slot='color',materialIndex:any=0)=>editScene(PM,{operation:'set_material_map',target:'model',slot:slot as any,map,materialIndex});
  const props=(patch:any,index:any=0)=>editScene(PM,{operation:'set_material_properties',target:'model',materialIndex:index,patch});
  return {PM,add,source,set,props};
}

describe('layers as materials',()=>{
  it('checks drag-over without mutation and returns the same eligibility/cycle errors as drop',()=>{
    const {PM,source,add}=editor(),api=makeScene3DAPI(PM,vi.fn()),before=PM.serialize();
    expect(api.canAssignMaterialMap('model',0,'color',{layerId:'paint'})).toEqual({ok:true});
    expect(PM.serialize()).toBe(before);
    const check=(map:any,slot:any='color',index:any=0)=>{
      const result=api.canAssignMaterialMap('model',index,slot,map);
      expect(result.ok).toBe(false);
      expect(api.setMaterialMap('model',index,slot,map)).toMatchObject({ok:false,message:result.reason});
    };
    check({layerId:'model'});check({layerId:'missing'});check({layerId:'paint'},'normal');check({layerId:'paint'},'color',99);
    add('null','null');check({layerId:'null'});
    source.fx=[{inputs:{provider:{layerId:'model'}}}];check({layerId:'paint'});source.fx=[];
    const sub=PM.Comps.create({name:'Loop'},{open:false});PM.proj.comps[sub].layers.push({id:'nested',type:'precomp',d:{comp:PM.proj.compId}});
    check({compositionId:sub});
    PM.L('model').d.data.object.source={mesh:{positions:[0,0,0,1,0,0,0,1,0]}};check({layerId:'paint'});
    expect(api.canAssignMaterialMap('model',null,'color',null)).toEqual({ok:true});
  });
  it('groups same-comp sources, includes hidden artwork, excludes nonvisual layers and receiver',()=>{
    const {PM,add}=editor();
    for(const type of ['precomp','group','image','video','text','shape','adjustment','null','audio'])add(type,type);
    PM.L('paint').on=false;
    const groups=listEligibleMaterialLayers(PM,'model');
    expect(groups.map(g=>g.group)).toEqual(['Comps','Groups','Footage','Solids','Text & Shapes']);
    expect(groups.flatMap(g=>g.layers.map(l=>l.id))).toEqual(['precomp','group','image','video','paint','text','shape']);
    const other=PM.Comps.create({name:'Other'},{open:false});PM.proj.comps[other].layers.push({...PM.mkLayer('solid'),id:'other-paint'});
    expect(groups.flatMap(g=>g.layers.map(l=>l.id))).not.toContain('other-paint');
  });
  it('rejects receiver, invalid source, cross-comp, nested and material/effect cycles atomically',()=>{
    const {PM,source,set,add}=editor();
    expect(set({layerId:'model'})).toMatchObject({ok:false,message:expect.stringContaining('cycle')});
    expect(set({layerId:'missing'}).ok).toBe(false);
    add('null','null');expect(set({layerId:'null'}).ok).toBe(false);
    const root=PM.proj.compId,sub=PM.Comps.create({name:'Sub'},{open:false});
    const nested=add('nested','precomp');nested.d.comp=sub;
    PM.proj.comps[sub].layers.push({id:'loop',type:'precomp',d:{comp:root}});
    expect(set({layerId:'nested'})).toMatchObject({ok:false,message:expect.stringContaining('cycle')});
    PM.proj.comps[sub].layers=[];
    PM.proj.comps[sub].layers.push({id:'inner',type:'extension',d:{data:{object:{material:{maps:{color:{compositionId:root}}}}}}});
    expect(set({layerId:'nested'}).ok).toBe(false);
    PM.proj.comps[sub].layers=[];
    source.fx=[{inputs:{provider:{layerId:'model',stage:'effects'}}}];
    expect(set({layerId:'paint'}).ok).toBe(false);source.fx=[];
    expect(set({layerId:'paint'},'normal').ok).toBe(false);
    const before=PM.serialize(),object=JSON.parse(JSON.stringify(PM.L('model').d.data.object));
    object.sourceMaterials[0]={color:{layerId:'model'}};
    expect(PM.Edit.apply([{type:'set_layer',target:'model',patch:{name:'bad'}},{type:'set_content',target:'model',patch:{data:{object}}}]).ok).toBe(false);
    expect(PM.serialize()).toBe(before);
  });
  it('keeps stable links through rename/reorder/save, receiver duplication, source deletion and history',()=>{
    const {PM,set}=editor();expect(set({layerId:'paint'}).ok).toBe(true);
    expect(PM.Edit.apply({type:'set_layer',target:'paint',patch:{name:'Renamed'}}).ok).toBe(true);
    PM.proj.layers.reverse();PM.replaceProject(PM.deserialize(PM.serialize()));
    expect(materialLayerLinks(PM,'model')).toMatchObject([{layerId:'paint',status:'ready'}]);
    const duplicate=editScene(PM,{operation:'duplicate',target:'model'});expect(duplicate.ok).toBe(true);
    const copy=duplicate.ok?(duplicate.data.result as any).id:'';
    expect(materialLayerLinks(PM,copy)).toMatchObject([{layerId:'paint',status:'ready'}]);
    expect(PM.Edit.apply({type:'delete_layers',targets:['paint']}).ok).toBe(true);
    expect(materialLayerLinks(PM,'model')).toMatchObject([{layerId:'paint',status:'missing'}]);
    PM.hist.undo();expect(materialLayerLinks(PM,'model')[0]?.status).toBe('ready');
    PM.hist.redo();expect(materialLayerLinks(PM,'model')[0]?.status).toBe('missing');
  });
  it('remaps comp duplication and batch copies; cross-comp receiver-only copies are flagged',()=>{
    const {PM,set}=editor();expect(set({layerId:'paint'}).ok).toBe(true);
    const copied=PM.Comps.duplicate(PM.proj.compId),comp=PM.proj.comps[copied],model=comp.layers.find((l:any)=>l.d?.data?.object),paint=comp.layers.find((l:any)=>l.type==='solid');
    expect(model.d.data.object.sourceMaterials[0].color).toEqual({layerId:paint.id});
    expect(materialLayerLinks(PM,model.id)[0]?.status).toBe('ready');
    const receiver=PM.cloneLayer(PM.L('model'));remapLayerInputs(receiver,new Map([['paint','pasted-source']]));
    expect(receiver.d.data.object.sourceMaterials[0].color).toEqual({layerId:'pasted-source'});
    comp.layers.push(receiver);expect(materialLayerLinks(PM,receiver.id)[0]?.status).toBe('missing');
    expect(()=>validateCompositionMaps(PM.proj)).not.toThrow();
  });
  it('samples layer space on composition time, preserves hidden source and shares frame cache',()=>{
    const {PM,source,set}=editor();source.on=false;source.from=.5;source.dur=2;source.d.w=80;source.d.h=40;
    PM.L('model').from=1;PM.L('model').d.speed=4;
    expect(set({layerId:'paint'}).ok).toBe(true);
    PM.GL.renderMaterialComposition=vi.fn(()=>new Uint8Array(80*40*4));
    const a=compositionTextureFrame(PM,PM.L('model'),{layerId:'paint'},1)!;
    expect(PM.GL.renderMaterialComposition.mock.calls[0].slice(1)).toEqual([1,80,40,1,'paint']);
    expect(compositionTextureFrame(PM,PM.L('model'),{layerId:'paint'},1.001)).toBe(a);
    expect(materialCompositionSample(PM,PM.L('model'),{layerId:'paint'},1.05)?.time).toBeCloseTo(31/30);
    source.d.w=4096;source.d.h=2048;expect(materialCompositionSample(PM,PM.L('model'),{layerId:'paint'},1)).toMatchObject({width:4096,height:2048});
    PM.proj.layers=PM.proj.layers.filter((l:any)=>l.id!=='paint');
    expect(compositionTextureFrame(PM,PM.L('model'),{layerId:'paint'},1)).toBeNull();
  });
  it('prepares a hidden video source with its own clock and supports plain/keyframed remaps',()=>{
    const {PM,add,set}=editor(),video=add('movie','video');video.on=false;video.from=.5;video.dur=2;video.d.asset='movie-asset';video.d.speed=2;
    PM.assets.map.set('movie-asset',{kind:'video',el:{},dur:3});PM.touch();
    expect(set({layerId:'movie'}).ok).toBe(true);
    const clip=videoClipsAt(PM,1).find(c=>c.layer.id==='movie');
    expect(clip).toMatchObject({id:'material-layer:movie/movie',time:1,at:1});
    video.d.timeRemap=true;video.d.sourceTime=.25;expect(sourceTime(PM,video,1)).toBe(.25);
    video.d.sourceTime=PM.P(.5);video.d.sourceTime.kf=[PM.KF(0,0),PM.KF(1,2)];PM.touch();
    expect(sourceTime(PM,video,1)).toBeCloseTo(1);
    expect(videoClipsAt(PM,.25).filter(c=>c.layer.id==='movie')).toEqual([]);
  });
  it('keeps old project defaults and supports primitive, lathe, extrude and UV mesh receivers',()=>{
    expect(parseScene({objects:[{id:'old',source:{primitive:'box'}}]}).objects[0]!.materialProperties).toEqual({});
    const {PM,set}=editor();
    for(const [index,source] of [{primitive:'sphere'},{lathe:[[0,0],[1,1]]},{extrude:[[0,0],[1,0],[0,1]],depth:1},{mesh:{positions:[0,0,0,1,0,0,0,1,0],uvs:[0,0,1,0,0,1]}}].entries()){
      const id=`primitive${index}`;expect(editScene(PM,{operation:'add_object',object:{id,source}}).ok).toBe(true);
      expect(editScene(PM,{operation:'set_material_map',target:id,map:{layerId:'paint'},materialIndex:0,slot:'color'}).ok).toBe(true);
    }
    expect(editScene(PM,{operation:'add_object',object:{id:'noUV',source:{mesh:{positions:[0,0,0,1,0,0,0,1,0]}}}}).ok).toBe(true);
    expect(listEligibleMaterialLayers(PM,'noUV')).toEqual([]);
    expect(editScene(PM,{operation:'set_material_map',target:'noUV',map:{layerId:'paint'},slot:'color'}).ok).toBe(false);
    expect(set({layerId:'paint'}).ok).toBe(true);
  });
});

describe('intrinsic material properties',()=>{
  it('validates, serializes, animates through normal paths and undoes per-slot overrides',()=>{
    const {PM,props}=editor();
    expect(props({opacity:55,roughness:.2,metallic:.7,emissionColor:'#FF0000',emissionIntensity:2,ambientResponse:35}).ok).toBe(true);
    expect(readMaterialProperties(PM,'model',0)?.values).toMatchObject({opacity:55,metallic:.7,ambientResponse:35});
    expect(layer3DProperties(PM.L('model')).map(p=>p.key)).toContain('mi.0.opacity');
    expect(PM.Edit.apply({type:'replace_keyframes',target:'model',path:'mi.0.opacity',keyframes:[{time:0,value:20},{time:1,value:80}]}).ok).toBe(true);
    expect(readMaterialProperties(PM,'model',0,.5)?.values.opacity).toBeCloseTo(50);
    expect(compositionScene(PM,.5).objects[0]!.sourceMaterialProperties[0]!.opacity!.v).toBeCloseTo(50);
    const before=PM.serialize();expect(props({opacity:101,metallic:2}).ok).toBe(false);expect(PM.serialize()).toBe(before);
    expect(props({emissionColor:'#xyzxyz'}).ok).toBe(false);
    expect(PM.Edit.apply({type:'replace_keyframes',target:'model',path:'mi.0.opacity',keyframes:[{time:0,value:101}]}).ok).toBe(false);
    PM.replaceProject(PM.deserialize(PM.serialize()));expect(readMaterialProperties(PM,'model',0,.5)?.values.opacity).toBeCloseTo(50);
    expect(props({opacity:null}).ok).toBe(true);expect(readMaterialProperties(PM,'model',0)?.properties.opacity).toBeUndefined();
    PM.hist.undo();expect(readMaterialProperties(PM,'model',0,.5)?.values.opacity).toBeCloseTo(50);
    expect(props({roughness:{v:.3,kf:[{t:0,v:.1},{t:1,v:.9}],expr:null}},null).ok).toBe(true);
    expect(PM.findProp(PM.L('model'),'mi.roughness')).toBeTruthy();
    expect(props({emissionColor:{v:'#FF0000',kf:[{t:0,v:'#FF0000'},{t:1,v:'#0000FF'}]}}).ok).toBe(true);
    expect(readMaterialProperties(PM,'model',0,.5)?.values.emissionColor.toLowerCase()).toBe('#800080');
    const duplicate=editScene(PM,{operation:'duplicate',target:'model'});expect(duplicate.ok).toBe(true);
    const copy=PM.L(duplicate.ok?(duplicate.data.result as any).id:'');
    expect(sceneProperties(copy).flatMap(p=>p.prop.kf.map((k:any)=>k.i))).not.toEqual(sceneProperties(PM.L('model')).flatMap(p=>p.prop.kf.map((k:any)=>k.i)));
  });
  it('applies per-material and whole-model properties, keeps source defaults and restores maps on missing links',()=>{
    const original=new THREE.MeshStandardMaterial({roughness:.87,metalness:.23,opacity:.9,emissive:'#112233',emissiveIntensity:3});original.userData.sourceMaterialIndex=0;
    const model=new THREE.Mesh(new THREE.PlaneGeometry(),original),runtime=new SceneRuntime();
    const raw=parseScene({objects:[{id:'model',source:{assetId:'screen'},useSourceMaterials:true,material:{maps:{color:{layerId:'paint'}}},materialProperties:{roughness:.4},sourceMaterialProperties:{0:{opacity:30,metallic:.8,ambientResponse:0}}}]});
    const frame={pixels:new Uint8Array([255,0,0,255]),width:1,height:1,key:'a'};let available=true;
    const asset={object3d:model},sync=()=>runtime.sync(raw,()=>asset,p=>p.v,0,true,undefined,()=>available?frame:null);
    sync();let material:any;runtime.objects.get('model')!.traverse((mesh:any)=>{if(mesh.isMesh)material=mesh.material;});
    expect(material.roughness).toBe(.4);expect(material.metalness).toBe(.8);expect(material.opacity).toBe(.3);expect(material.transparent).toBe(true);
    expect(material.emissiveIntensity).toBe(3);expect(material.userData.materialAmbientUniform.value).toBe(0);
    expect(material.map.image.data).toBe(frame.pixels);available=false;sync();expect(material.map).toBeNull();
    raw.objects[0]!.materialProperties={};raw.objects[0]!.sourceMaterialProperties={};sync();
    expect(material.roughness).toBe(.87);expect(material.metalness).toBe(.23);expect(material.opacity).toBe(.9);expect(material.userData.materialAmbientUniform.value).toBe(1);
    expect(original.roughness).toBe(.87);expect(original.map).toBeNull();runtime.dispose();model.geometry.dispose();original.dispose();
  });
  it('only scales ambient lighting in the shader and updates the uniform without recompilation',()=>{
    const material=new THREE.MeshStandardMaterial();setAmbientResponse(material,.25);
    const shader:any={uniforms:{},fragmentShader:'#include <lights_fragment_begin>'};material.onBeforeCompile(shader,{} as any);
    expect(shader.fragmentShader).toContain('getHemisphereLightIrradiance( hemisphereLights[ i ], geometryNormal ) * pmAmbientResponse');
    expect(shader.fragmentShader).toContain('getDirectionalLightInfo');
    expect(shader.uniforms.pmAmbientResponse.value).toBe(.25);const version=material.version;setAmbientResponse(material,.5);
    expect(material.version).toBe(version);expect(shader.uniforms.pmAmbientResponse.value).toBe(.5);material.dispose();
  });
  it('exposes link and intrinsic APIs and publishes changes for edits and history replay',()=>{
    const {PM}=editor(),emit=vi.fn(),api=makeScene3DAPI(PM,emit),listener=vi.fn(),subscription=api.onMaterialsChange(listener);
    expect(api.listEligibleSourceLayers('model').flatMap(g=>g.layers.map(l=>l.id))).toContain('paint');
    expect(api.setMaterialMap('model',0,'color',{layerId:'paint'}).ok).toBe(true);
    expect(api.materialLinks('model')[0]?.status).toBe('ready');expect(listener).toHaveBeenCalled();
    expect(emit).toHaveBeenCalledWith('scene3d:materials',{layerIds:['model']});
    expect(api.setMaterialProperties('model',0,{opacity:25}).ok).toBe(true);expect(api.getMaterialProperties('model',0)?.values.opacity).toBe(25);
    listener.mockClear();PM.hist.undo();expect(listener).toHaveBeenCalled();subscription.dispose();listener.mockClear();
    PM.hist.redo();expect(listener).not.toHaveBeenCalled();
  });
});

it('uses nested groups and prevents feedback through any descendant, including membership edits',()=>{
  const {PM,add,source,set}=editor();
  const group=add('artwork','group'),nested=add('nested-group','group');nested.group=group.id;source.group=nested.id;group.on=false;PM.touch();
  expect(set({layerId:group.id}).ok).toBe(true);
  expect(materialLayerLinks(PM,'model')).toMatchObject([{layerId:group.id,status:'ready'}]);
  const before=PM.serialize();
  expect(PM.Edit.apply({type:'move_to_group',targets:['model'],group:nested.id})).toMatchObject({ok:false,message:expect.stringContaining('cycle')});
  expect(PM.serialize()).toBe(before);
  PM.L(source.id).fx=[{inputs:{provider:{layerId:'model'}}}];
  expect(set({layerId:group.id})).toMatchObject({ok:false,message:expect.stringContaining('cycle')});
});

it('prepares visible videos inside hidden group sources with their composition clock',()=>{
  const {PM,add,set}=editor(),group=add('artwork','group'),nested=add('nested','group'),video=add('movie','video'),hidden=add('hidden','video');
  group.on=false;nested.group=group.id;video.group=nested.id;hidden.group=nested.id;hidden.on=false;
  video.from=.5;video.dur=2;video.d.asset=hidden.d.asset='movie-asset';video.d.speed=2;
  PM.assets.map.set('movie-asset',{kind:'video',el:{},dur:3});PM.touch();
  expect(set({layerId:group.id}).ok).toBe(true);
  const materialClips=videoClipsAt(PM,1).filter(c=>c.id.startsWith('material-layer:'));
  expect(materialClips).toHaveLength(1);expect(materialClips[0]).toMatchObject({id:'material-layer:artwork/movie',time:1,at:1});
  nested.on=false;PM.touch();expect(videoClipsAt(PM,1).filter(c=>c.id.startsWith('material-layer:'))).toHaveLength(0);
});
