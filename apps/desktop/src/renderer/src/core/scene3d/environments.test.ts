import {describe,it,expect} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {LAYER3D_DEFINITIONS,compositionScene,layer3DAssetIds} from './layers';
import {parseScene,sceneProperties,createScene} from './schema';
import {editScene} from './operations';
import {compositionRuntime} from './service';
import {extractScene} from '../studio/extract';
import {attachStudioWorld,worldForNode,terrainSettings} from './environment-render';
import {remapMaterialLayerLinks} from './material-maps';
import {worldSchema,materialOptionsSchema} from './environment-data';
import {describePreparedScene,compileSceneDescription} from '../studio/scene-description';
import {viewOf,cameraOf} from '../studio';
function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');PM.proj=PM.mkProject({name:'Environment',dur:5,fps:30});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  return PM;
}
function edit(PM:any,args:any){const r=editScene(PM,args,{origin:'agent'});expect(r.ok,r.message).toBe(true);return r;}
describe('environment documents and guarded agent edits',()=>{
  it('keeps old documents valid without adding world layers',()=>{
    const old=parseScene({objects:[{id:'old',source:{primitive:'box'},castShadow:false,receiveShadow:false}]});
    expect(old.objects[0]!.world).toBeUndefined();expect(old.objects[0]!.materialOptions).toBeUndefined();expect(old.camera.p.irisAspect!.v).toBe(1);
    const PM=editor();edit(PM,{operation:'create',scene:old});expect(compositionScene(PM).objects[0]!.castShadow).toBe(false);
  });
  it('creates one composition World, animates channels, and survives save/load and undo',()=>{
    const PM=editor();edit(PM,{operation:'set_world',id:'world',world:{preset:'golden-hour',p:{fogDensity:.01}}});
    expect(PM.L('world').d.data.object.world.p.elevation.v).toBe(6);
    expect(sceneProperties(PM.L('world')).map(p=>p.key)).toContain('world.fogDensity');
    edit(PM,{operation:'set_world',patch:{p:{elevation:{v:6,kf:[{t:0,v:6},{t:2,v:20}]}}}});
    expect(compositionScene(PM,1).objects[0]!.world!.p.elevation!.v).toBeCloseTo(13);
    const snapshot=JSON.parse(PM.serialize());PM.proj=PM.deserialize(JSON.stringify(snapshot));expect(compositionScene(PM,1).objects.find(o=>o.world)!.world!.p.elevation!.v).toBeCloseTo(13);
    edit(PM,{operation:'set_world',patch:{p:{intensity:2}}});PM.hist.undo();expect(PM.L('world').d.data.object.world.p.intensity.v).toBe(1);
    expect(editScene(PM,{operation:'add_object',object:{source:{primitive:'plane'},world:{}}}).ok).toBe(false);
  });
  it('honors locks and protected world channels, rejecting invalid edits atomically',()=>{
    const PM=editor();edit(PM,{operation:'set_world',id:'world'});const layer=PM.L('world');layer.locked_intent['world.intensity']={by:'human'};
    const before=PM.serialize();expect(editScene(PM,{operation:'set_world',patch:{p:{intensity:8}}},{origin:'agent'}).ok).toBe(false);expect(PM.serialize()).toBe(before);
    expect(editScene(PM,{operation:'set_world',patch:{p:{haze:99}}}).ok).toBe(false);expect(PM.serialize()).toBe(before);
    PM.L('world').lock=true;expect(editScene(PM,{operation:'set_world',patch:{p:{haze:3}}}).ok).toBe(false);
  });
  it('rejects stale revisions, source cycles and invalid terrain/scatter bounds',()=>{
    const PM=editor();edit(PM,{operation:'set_world',id:'world'});
    const revision=PM.proj.revision;edit(PM,{operation:'set_world',patch:{preset:'night'}});
    expect(PM.L('world').d.data.object.world.p.elevation.v).toBe(-20);
    const before=PM.serialize();expect(editScene(PM,{operation:'set_world',patch:{p:{intensity:2}}},{origin:'agent',baseRevision:revision}).ok).toBe(false);expect(PM.serialize()).toBe(before);
    expect(editScene(PM,{operation:'set_world',patch:{mode:'hdri',source:{compositionId:PM.proj.compId || PM.proj.id}}}).ok).toBe(false);expect(PM.serialize()).toBe(before);
    expect(editScene(PM,{operation:'add_terrain',terrain:{p:{resolution:8},heights:[0,1,0]}}).ok).toBe(false);
    expect(editScene(PM,{operation:'add_terrain',terrain:{regions:[{p:{minSlope:60,maxSlope:20}}]}}).ok).toBe(false);
  });
  it('creates fog, terrain regions and deterministic scatter with real layer references',()=>{
    const PM=editor();edit(PM,{operation:'add_object',object:{id:'tree',source:{primitive:'cone',parameters:{height:2,radius:.3}}}});
    edit(PM,{operation:'add_terrain',id:'ground',terrain:{p:{resolution:8,amplitude:0,width:10,depth:10},regions:[{p:{minSlope:0,maxSlope:20,color:'#44AA22'}}]}});
    edit(PM,{operation:'add_fog',id:'mist',fog:{p:{shape:'sphere',density:.03,color:'#FFEEDD'}}});
    edit(PM,{operation:'add_scatter',id:'grove',scatter:{model:'tree',surface:'ground',p:{count:32,seed:123,scaleMin:.4,scaleMax:.8,maxSlope:90}}});
    const runtime=compositionRuntime(PM,0),instances:any[]=[];runtime.objects.get('grove')!.traverse((o:any)=>{if(o.isInstancedMesh)instances.push(o);});
    expect(instances).toHaveLength(1);expect(instances[0].count).toBe(32);
    const matrices=Array.from(instances[0].instanceMatrix.array);edit(PM,{operation:'update_scatter',target:'grove',patch:{p:{count:16}}});
    PM.hist.undo();const repeated=compositionRuntime(PM,0).objects.get('grove')!.children[0] as THREE.InstancedMesh;
    expect(Array.from(repeated.instanceMatrix.array)).toEqual(matrices);
    expect((runtime as any).studioWorld.volumes[0].density).toBe(.03);
    expect(sceneProperties(PM.L('ground')).map(p=>p.key)).toContain('terrain.region0.color');
    expect(editScene(PM,{operation:'add_scatter',scatter:{model:'missing',surface:'ground'}}).ok).toBe(false);
  });
  it('keeps hidden prototypes usable and removes missing-source copies without stale geometry',()=>{
    const PM=editor();edit(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'}}});edit(PM,{operation:'add_terrain',id:'ground',terrain:{p:{resolution:2,amplitude:0}}});
    edit(PM,{operation:'add_scatter',id:'copies',scatter:{model:'model',surface:'ground',p:{count:4}}});
    PM.proj.layers.sort((a:any,b:any)=>a.id==='model'?-1:b.id==='model'?1:0);PM.touch();
    PM.Edit.apply({type:'set_layer',target:'model',patch:{visible:false}});
    let runtime=compositionRuntime(PM);expect(runtime.objects.get('model')!.visible).toBe(false);expect((runtime.objects.get('copies')!.children[0] as THREE.InstancedMesh).count).toBe(4);
    expect(editScene(PM,{operation:'update_scatter',target:'copies',patch:{model:'copies'}}).ok).toBe(false);
    edit(PM,{operation:'remove',target:'model'});runtime=compositionRuntime(PM);expect(runtime.objects.get('copies')!.children).toHaveLength(0);PM.hist.undo();runtime=compositionRuntime(PM);expect(runtime.objects.get('copies')!.children).toHaveLength(1);
  });
  it('creates complete environment scenes with remapped scatter references in one edit',()=>{
    const PM=editor();edit(PM,{operation:'create',scene:{objects:[{id:'model',source:{primitive:'box'}},{id:'ground',source:{primitive:'plane'},terrain:{p:{resolution:2}}},{id:'copies',source:{primitive:'plane'},scatter:{model:'model',surface:'ground',p:{count:4}}}]}});
    const scene=compositionScene(PM),scatter=scene.objects.find(o=>o.scatter)!.scatter!;expect(scene.objects.some(o=>o.id===scatter.model)).toBe(true);expect(scene.objects.some(o=>o.id===scatter.surface)).toBe(true);
  });
  it('remaps scatter and environment layer sources when a composition is copied',()=>{
    const layer={d:{data:{object:{source:{primitive:'plane'},scatter:{model:'model',surface:'ground'},world:{source:{layerId:'sky'}}}}}};
    remapMaterialLayerLinks(layer,new Map([['model','new_model'],['ground','new_ground'],['sky','new_sky']]));
    expect(layer.d.data.object.scatter).toEqual({model:'new_model',surface:'new_ground'});expect(layer.d.data.object.world.source).toEqual({layerId:'new_sky'});
  });
  it('transports world, visibility policies, ambient lights, and iris through extraction',()=>{
    const PM=editor();edit(PM,{operation:'set_world',id:'world',world:{preset:'blue-hour',p:{metersPerUnit:1}}});
    edit(PM,{operation:'add_object',object:{id:'hero',source:{primitive:'sphere'}}});
    edit(PM,{operation:'set_material_options',target:'hero',patch:{castsShadows:'only',acceptsShadows:false,acceptsLights:false,appearsInReflections:false}});
    edit(PM,{operation:'add_light',light:{id:'fill',type:'ambient',p:{intensity:.3}}});
    edit(PM,{operation:'add_light',light:{id:'parallel',type:'parallel'}});
    edit(PM,{operation:'add_camera',id:'camera',camera:{p:{fStop:2,irisShape:6,irisRotation:30,irisRoundness:.2,irisAspect:1.4}}});
    const runtime=compositionRuntime(PM),scene=extractScene(runtime.scene,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});
    attachStudioWorld(scene,(runtime as any).studioWorld,(runtime as any).metersPerUnit);
    expect(scene.world!.sky!.elevation).toBe(-5);expect(scene.metersPerUnit).toBe(1);
    expect(scene.instancePolicies![0]!.primaryVisible).toBe(false);
    expect(scene.materials[25]! & 32).toBe(32);expect(scene.materials[25]! & 8).toBe(8);expect(scene.materials[25]! & 2).toBe(0);
    const view=viewOf(runtime,cameraOf(runtime.camera,64,64)!,scene.bounds);expect(view.camera.iris).toEqual([6,Math.PI/6,.2,1.4]);
    const description=describePreparedScene(scene,view,[]),compiled=compileSceneDescription(description);
    expect(compiled.scene.world!.sky!.elevation).toBe(-5);expect(compiled.scene.materials[25]! & 32).toBe(32);
  });
  it('keeps environment source references durable and supports image/layer heightfields',()=>{
    const PM=editor();edit(PM,{operation:'set_world',id:'world',world:{mode:'hdri',source:'sky_exr'}});
    expect(layer3DAssetIds(PM.L('world'))).toContain('sky_exr');
    const node=worldSchema.parse({mode:'hdri',p:{intensity:2,rotation:90,visible:false}});
    const world=worldForNode(node,{width:2,height:1,pixels:new Float32Array([12,4,1,.5,.2,.1]),linear:true,key:'hdr'});
    expect(world.hdri!.pixels[0]!.x).toBe(12);expect(world.hdri!.visible).toBe(false);
    const terrain=parseScene({objects:[{id:'terrain',source:{primitive:'plane'},terrain:{p:{resolution:2}}}]}).objects[0]!.terrain!;
    const settings=terrainSettings(terrain,{width:2,height:2,pixels:new Uint8Array([0,0,0,255,255,255,255,255,128,128,128,255,64,64,64,255]),linear:false,key:'height'});
    expect(settings.heights).toHaveLength(9);expect(settings.heights[0]).toBe(0);expect(settings.heights[2]).toBeCloseTo(1);
  });
  it('adds keyframeable Material Options to ordinary 3D artwork',()=>{
    const PM=editor();PM.Edit.apply({type:'add_layer',id:'card',layerType:'solid'});PM.Edit.apply({type:'set_layer',target:'card',patch:{threeD:true}});
    edit(PM,{operation:'set_material_options',target:'card',patch:{acceptsLights:true,castsShadows:'on'}});
    expect(PM.findProp(PM.L('card'),'materialOptions.acceptsLights').v).toBe(true);
    expect(PM.Edit.apply({type:'replace_keyframes',target:'card',path:'materialOptions.acceptsLights',keyframes:[{time:0,value:true},{time:1,value:false,hold:true}]}).ok).toBe(true);
    expect(editScene(PM,{operation:'set_material_options',target:'card',patch:{typo:true}}).ok).toBe(false);
  });
});
