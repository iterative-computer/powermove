import { afterEach, expect, it } from 'vitest';
import old from './__fixtures__/pre-studio.json';
import early from './__fixtures__/early-studio.json';
import { makePM } from '../../legacy/__tests__/make-pm';
import { migrateInherited3D } from '../../legacy/core/switch-3d-migration';
import { convertLegacyScene } from './migration';
import { createScene, SCENE3D_DEFINITION } from './schema';
import { LAYER3D_DEFINITIONS, compositionScene } from './layers';
import { compositionRuntime, disposeSceneRuntimes } from './service';

const editors:any[]=[];
afterEach(()=>{for(const PM of editors.splice(0))disposeSceneRuntimes(PM);});
it.each([['pre-Studio',old],['early Studio',early]] as const)('%s fixture keeps camera, materials, switches and exact conversion history',(_label,fixture)=>{
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  editors.push(PM);PM.proj=PM.mkProject({name:'Migration',dur:3,fps:24});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}
    :id===SCENE3D_DEFINITION?{id,label:'3D Scene',version:1,params:[],defaults:{scene:createScene()},renderer:{kind:'scene3d'}}:null;
  PM.proj.layers=fixture.layers.map(source=>({...PM.mkLayer(source.type as any),...source}));
  const legacy=PM.mkLayer('extension');legacy.id='legacy';legacy.from=.25;legacy.dur=2;
  legacy.d={definition:SCENE3D_DEFINITION,version:1,w:640,h:360,params:{},data:{scene:structuredClone(fixture.scene)}};
  PM.proj.layers.push(legacy);PM.proj.switch3DVersion=fixture.switch3DVersion;PM.proj.scene3dDefaultsVersion=fixture.scene3dDefaultsVersion;
  PM.proj=PM.deserialize(PM.serialize());
  expect(PM.L('artwork').threeD).toBe(fixture.switch3DVersion<2);
  const before=JSON.stringify(PM.proj.layers);
  expect(convertLegacyScene(PM,'legacy').ok).toBe(true);
  const data=compositionScene(PM,1),world=compositionRuntime(PM,1),pose=world.camera.matrixWorld.elements.slice();
  const model=data.objects.find(o=>o.id==='legacy')!;
  const source:any=fixture.scene;
  expect(model.castShadow).toBe(source.objects[0]!.castShadow ?? true);
  expect(data.camera.p.x!.v).toBe(fixture.scene.camera.p.x);
  expect(data.camera.p.fStop!.v).toBe(source.camera.p.fStop ?? 0);
  for(const [key,value] of Object.entries(source.objects[0]?.material?.p || {}))expect(model.material.p[key]!.v).toBe(value);
  expect(data.camera.p.irisAspect!.v).toBe(1);expect(model.world).toBeUndefined();
  expect(PM.L('legacy').from).toBe(.25);expect(PM.L('legacy').dur).toBe(2);
  const converted=JSON.stringify(PM.proj.layers);
  PM.hist.undo();expect(JSON.stringify(PM.proj.layers)).toBe(before);
  PM.hist.redo();expect(JSON.stringify(PM.proj.layers)).toBe(converted);
  PM.proj=PM.deserialize(PM.serialize());PM.touch();
  expect(compositionScene(PM,1)).toEqual(data);
  expect(compositionRuntime(PM,1).camera.matrixWorld.elements).toEqual(pose);
  expect(migrateInherited3D(structuredClone(PM.proj))).toEqual(PM.proj);
});
it('does not downgrade a future switch migration version or reinterpret its explicit switches',()=>{
  const project={switch3DVersion:3,layers:[{id:'group',type:'group',threeD:true},{id:'art',type:'solid',group:'group',threeD:false}]};
  expect(migrateInherited3D(structuredClone(project))).toEqual(project);
});
it('keeps scatter references when conversion preserves the original container layer ID',()=>{
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  editors.push(PM);PM.proj=PM.mkProject();PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}
    :id===SCENE3D_DEFINITION?{id,label:'Scene',version:1,params:[],defaults:{scene:createScene()},renderer:{kind:'scene3d'}}:null;
  const layer=PM.mkLayer('extension');layer.id='legacy';
  layer.d={definition:SCENE3D_DEFINITION,version:1,params:{},data:{scene:{objects:[
    {id:'prototype',source:{primitive:'box'}},
    {id:'ground',source:{primitive:'plane'},terrain:{p:{resolution:2}}},
    {id:'copies',source:{primitive:'plane'},scatter:{model:'prototype',surface:'ground',p:{count:3}}}
  ]}}};PM.proj.layers.push(layer);
  const before=JSON.stringify(PM.proj.layers);
  const result=convertLegacyScene(PM,'legacy');expect(result.ok,result.message).toBe(true);
  const scatter=compositionScene(PM).objects.find(o=>o.scatter)!.scatter!;
  expect(scatter.model).toBe('legacy');expect(PM.L(scatter.surface)).toBeTruthy();
  const instances=compositionRuntime(PM).objects.get(compositionScene(PM).objects.find(o=>o.scatter)!.id)!.children as any[];
  expect(instances.reduce((n,m)=>n+(m.count || 0),0)).toBe(3);
  PM.hist.undo();expect(JSON.stringify(PM.proj.layers)).toBe(before);
  PM.hist.redo();expect(compositionScene(PM).objects.find(o=>o.scatter)!.scatter!.model).toBe('legacy');
});
