import { describe,expect,it } from 'vitest';
import * as THREE from 'three';
import { createScene,createObject,parseScene,sceneProperties,SCENE3D_DEFINITION,sceneAssetIds } from './schema';
import { LAYER3D_DEFINITIONS,compositionScene,layer3DAssetIds,parseLayer3DData } from './layers';
import { SceneRuntime,primitiveGeometry } from './runtime';
import { compositionRuntime,compositionDepthIds,compositionOrderedLayers } from './service';
import { convertLegacyScene } from './migration';
import { editScene } from './operations';
import { makePM } from '../../legacy/__tests__/make-pm';
import { world3D,parent3D } from '../../legacy/core/space-3d';
import { sceneFromComp } from './space';

function editor() {
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'3D',fps:30,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:id===SCENE3D_DEFINITION?{id,label:'3D Scene',version:1,params:[],defaults:{scene:createScene()},renderer:{kind:'scene3d'}}:null;
  return PM;
}

describe('individual 3D layers and guarded edits',()=>{
  it('groups models around their world-space center without changing their pose',()=>{
    const PM=editor();
    expect(editScene(PM,{operation:'add_object',object:{id:'a',source:{primitive:'box'},p:{x:900,z:-100}}}).ok).toBe(true);
    expect(editScene(PM,{operation:'add_object',object:{id:'b',source:{primitive:'box'},p:{x:1100,z:100}}}).ok).toBe(true);
    PM.GL.bounds=()=>({x0:800,x1:1100,y0:400,y1:700});
    const before=['a','b'].map(id=>world3D(PM,PM.L(id),0));
    const grouped=PM.Edit.apply({type:'group_layers',targets:['a','b']});expect(grouped.ok,grouped.message).toBe(true);
    const group=PM.proj.layers.find((l:any)=>l.type==='group');
    expect(group.threeD).toBe(true);
    ['x','y','z'].forEach((axis,i)=>expect(group.p[`anchor.${axis}`].v).toBeCloseTo([1000,540,0][i]!));
    ['a','b'].forEach((id,i)=>expect(world3D(PM,PM.L(id),0)).toEqual(before[i]));
    expect(PM.Edit.apply({type:'set_property',target:group.id,path:'rotation.y',value:90}).ok).toBe(true);
    const a=world3D(PM,PM.L('a'),0),b=world3D(PM,PM.L('b'),0);
    // A quarter turn about the group's anchor swings each model around it.
    expect(a[12]).toBeCloseTo(900);expect(a[14]).toBeCloseTo(100);
    expect(b[12]).toBeCloseTo(1100);expect(b[14]).toBeCloseTo(-100);
    PM.hist.undo();PM.hist.undo();expect(PM.L(group.id)).toBeNull();
    ['a','b'].forEach((id,i)=>expect(world3D(PM,PM.L(id),0)).toEqual(before[i]));
  });
  it('pins camera layers to the top of the stack, outside groups',()=>{
    const PM=editor();
    editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'}}});
    expect(editScene(PM,{operation:'add_camera',id:'camera'}).ok).toBe(true);
    editScene(PM,{operation:'add_object',object:{id:'top',source:{primitive:'box'}}});
    expect(PM.proj.layers[0].id).toBe('camera');
    expect(PM.Edit.apply({type:'reorder_layer',target:'model',index:0}).ok).toBe(true);
    expect(PM.proj.layers.map((l:any)=>l.id).slice(0,2)).toEqual(['camera','model']);
    // Grouping a camera parents it to the group instead, so rigs still move it.
    const grouped=PM.Edit.apply({type:'group_layers',targets:['model','camera']});expect(grouped.ok,grouped.message).toBe(true);
    const group=PM.proj.layers.find((l:any)=>l.type==='group');
    expect(PM.proj.layers[0].id).toBe('camera');expect(PM.L('camera').group).toBeNull();expect(PM.L('camera').parent).toBe(group.id);
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);expect(PM.L('camera').parent).toBeNull();
  });
  it('keeps shared depth across ordinary group headers and preserves 2D barriers',()=>{
    const PM=editor();editScene(PM,{operation:'add_object',object:{id:'a',source:{primitive:'box'}}});editScene(PM,{operation:'add_object',object:{id:'b',source:{primitive:'box'}}});
    const a=PM.L('a'),b=PM.L('b'),g=PM.groupLayers(['a']);
    expect(compositionDepthIds(PM,a,0)).toEqual(new Set(['a','b']));
    const barrier=PM.mkLayer('solid');PM.proj.layers=[g,a,barrier,b];PM.touch();
    expect(compositionDepthIds(PM,a,0)).toEqual(new Set(['a']));
  });
  it('rotates grouped light and camera aim targets together with their positions',()=>{
    const PM=editor();
    editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'}}});
    editScene(PM,{operation:'add_light',light:{id:'sun',type:'sun',p:{x:960,y:140,z:-600}}});
    expect(editScene(PM,{operation:'add_camera',id:'camera',camera:{p:{x:960,y:540,z:-1200}}}).ok).toBe(true);
    const group=PM.groupLayers(['model','sun','camera']);
    PM.Edit.apply({type:'set_property',target:group.id,path:'rotation.y',value:90});
    const world=compositionRuntime(PM,0),parent=new THREE.Matrix4().fromArray(parent3D(PM,PM.L('camera'),0));
    const aim=(['X','Y','Z'] as const).map(axis=>PM.L('camera').d.data.camera.p[`target${axis}`].v) as [number,number,number];
    // Both aim at the composition centre, held in the group's space.
    expect(aim).toEqual((['X','Y','Z'] as const).map(axis=>PM.L('sun').d.data.light.p[`target${axis}`].v));
    const expected=new THREE.Vector3(...aim).applyMatrix4(parent).applyMatrix4(sceneFromComp(PM.proj)),sun=world.objects.get('sun') as THREE.DirectionalLight;
    expect(sun.target.position.distanceTo(expected)).toBeLessThan(1e-8);
    expect(world.camera.getWorldDirection(new THREE.Vector3()).distanceTo(expected.clone().sub(world.camera.position).normalize())).toBeLessThan(1e-8);
    const direction=world.camera.getWorldDirection(new THREE.Vector3());
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    const ungrouped=compositionRuntime(PM,0),ungroupedSun=ungrouped.objects.get('sun') as THREE.DirectionalLight;
    expect(ungroupedSun.target.position.distanceTo(expected)).toBeLessThan(1e-8);
    expect(ungrouped.camera.getWorldDirection(new THREE.Vector3()).distanceTo(direction)).toBeLessThan(1e-8);
    const next=PM.groupLayers(['model']);PM.Edit.apply({type:'set_property',target:next.id,path:'rotation.y',value:30});
    expect(PM.Edit.apply({type:'move_to_group',targets:['sun'],group:next.id}).ok).toBe(true);
    expect((compositionRuntime(PM,0).objects.get('sun') as THREE.DirectionalLight).target.position.distanceTo(expected)).toBeLessThan(1e-8);
    expect(PM.Edit.apply({type:'set_layer',target:'camera',patch:{parent:next.id}}).ok).toBe(true);
    expect(compositionRuntime(PM,0).camera.getWorldDirection(new THREE.Vector3()).distanceTo(direction)).toBeLessThan(1e-8);
  });
  it('preserves animated camera and light aim when a 3D group is ungrouped',()=>{
    const PM=editor();PM.proj.dur=1;PM.proj.fps=4;
    editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'}}});
    editScene(PM,{operation:'add_light',light:{id:'sun',type:'sun'}});
    editScene(PM,{operation:'add_camera',id:'camera'});
    const group=PM.groupLayers(['model','sun','camera']);
    expect(PM.Edit.apply([{type:'replace_keyframes',target:group.id,path:'rotation.y',keyframes:[{time:0,value:0},{time:1,value:90}]},
      {type:'replace_keyframes',target:'sun',path:'light.targetX',keyframes:[{time:0,value:0},{time:1,value:2}]}]).ok).toBe(true);
    const times=[0,.25,.5,.75,1],before=times.map(time=>{const world=compositionRuntime(PM,time);return {direction:world.camera.getWorldDirection(new THREE.Vector3()),target:(world.objects.get('sun') as THREE.DirectionalLight).target.position.clone()};});
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    times.forEach((time,i)=>{const world=compositionRuntime(PM,time);expect(world.camera.getWorldDirection(new THREE.Vector3()).distanceTo(before[i]!.direction)).toBeLessThan(1e-7);
      expect((world.objects.get('sun') as THREE.DirectionalLight).target.position.distanceTo(before[i]!.target)).toBeLessThan(1e-7);});
  });
  it('creates independent model/light/camera timeline rows and native channels',()=>{
    const PM=editor();
    const created=editScene(PM,{operation:'add_object',object:{id:'body',source:{lathe:[[0,-1],[.4,-.8],[.4,.7],[0,1]]},p:{x:2,sx:1.5},material:{maps:{color:'paint'}}}});
    expect(created,created.message).toMatchObject({ok:true});
    expect(editScene(PM,{operation:'add_light',light:{id:'fill',type:'point',p:{intensity:20}}}).ok).toBe(true);
    expect(editScene(PM,{operation:'add_camera'}).ok).toBe(true);
    expect(PM.proj.layers).toHaveLength(3);expect(PM.proj.layers.some((l:any)=>l.d.data.scene)).toBe(false);
    const body=PM.L('body');expect(body.p['position.x'].v).toBe(2);expect(body.p['scale.x'].v).toBe(150);
    expect(body.d.data.object.p).toBeUndefined();expect(sceneProperties(body).map((p:any)=>p.key)).toContain('m.roughness');
    expect(layer3DAssetIds(body)).toEqual(['paint']);
    expect(PM.Edit.apply({type:'replace_keyframes',target:'body',path:'rotation.y',keyframes:[{time:0,value:0},{time:2,value:180}]}).ok).toBe(true);
    expect(PM.ev(body,'rotation.y',1)).toBeCloseTo(90);
    expect(compositionScene(PM,1).objects[0]!.p.ry!.v).toBeCloseTo(90);
  });
  it('validates channels before mutations, honors locks and hand edits',()=>{
    const PM=editor();editScene(PM,{operation:'create'});const target=PM.proj.layers[0].id,before=JSON.stringify(PM.proj);
    expect(PM.Edit.apply({type:'set_property',target,path:'m.roughness',value:20}).ok).toBe(false);expect(JSON.stringify(PM.proj)).toBe(before);
    PM.L(target).lock=true;expect(editScene(PM,{operation:'update_object',target,patch:{p:{x:2}}}).ok).toBe(false);
    PM.L(target).lock=false;PM.L(target).locked_intent['position.x']={by:'human'};
    expect(editScene(PM,{operation:'update_object',target,patch:{p:{x:2}}},{origin:'agent'}).ok).toBe(false);
    expect(editScene(PM,{operation:'remove',target},{origin:'agent'}).ok).toBe(false);
    expect(editScene(PM,{operation:'update_object',target,patch:{p:{y:2}}},{origin:'agent'}).ok).toBe(true);
    const data=JSON.stringify(PM.L(target).d.data);PM.Edit.begin('Drag');
    expect(PM.Edit.apply({type:'replace_keyframes',target,path:'m.metalness',keyframes:[{time:0,value:2}]}).ok).toBe(false);
    expect(JSON.stringify(PM.L(target).d.data)).toBe(data);PM.Edit.cancel();
    expect(editScene(PM,{operation:'update_object',target,patch:{p:{typo:2}}}).ok).toBe(false);
  });
  it('keeps ordinary mask channels separate from 3D material channels',()=>{
    const PM=editor();editScene(PM,{operation:'create'});const model=PM.proj.layers[0];
    const mask={id:'cutout',on:true,p:{opacity:PM.P(100)}};model.masks=[mask];
    expect(PM.findProp(model,'m.cutout.opacity')).toBe(mask.p.opacity);
    expect(PM.Edit.apply({type:'set_property',target:model.id,path:'m.cutout.opacity',value:50}).ok).toBe(true);
    expect(mask.p.opacity.v).toBe(50);expect(PM.findProp(model,'m.opacity')).toBe(model.d.data.object.material.p.opacity);
    const shape=PM.mkLayer('shape');shape.masks=[mask];expect(PM.findProp(shape,'m.cutout.opacity')).toBe(mask.p.opacity);
  });
  it('keeps supplied camera IDs and static visibility, and resolves unique names',()=>{
    const PM=editor();
    expect(editScene(PM,{operation:'add_camera',id:'lens'}).ok).toBe(true);expect(PM.L('lens')).toBeTruthy();
    editScene(PM,{operation:'add_object',object:{id:'hidden',name:'Hidden model',source:{primitive:'box'},p:{visible:false}}});
    expect(PM.L('hidden').on).toBe(false);expect(compositionScene(PM).objects).toHaveLength(0);
    expect(editScene(PM,{operation:'update_object',target:'Hidden model',patch:{p:{visible:true}}}).ok).toBe(true);
    expect(PM.L('hidden').on).toBe(true);
    expect(editScene(PM,{operation:'set_camera',target:'lens',patch:{name:'Main camera'}}).ok).toBe(true);
    expect(PM.L('lens').name).toBe('Main camera');
  });
  it('commits a whole gesture in one Undo and cancels without changing channels',()=>{
    const PM=editor();editScene(PM,{operation:'create'});const target=PM.proj.layers[0].id,count=PM.hist.list().length;
    PM.Edit.begin('Move model',{origin:'canvas'});
    for(const x of [1,2,3])expect(PM.Edit.dispatch({type:'set_property',target,path:'position.x',value:x}).ok).toBe(true);
    PM.Edit.commit();expect(PM.hist.list().length).toBe(count+1);PM.hist.undo();expect(PM.L(target).p['position.x'].v).toBe(960);
    PM.Edit.begin('Cancelled move');PM.Edit.dispatch({type:'set_property',target,path:'position.x',value:5});PM.Edit.cancel();expect(PM.L(target).p['position.x'].v).toBe(960);
  });
  it('duplicates with independent transforms and keyframe identities',()=>{
    const PM=editor();editScene(PM,{operation:'create'});const target=PM.proj.layers[0].id;
    PM.Edit.apply({type:'replace_keyframes',target,path:'position.x',keyframes:[{time:0,value:0},{time:1,value:2}]});
    const result=editScene(PM,{operation:'duplicate',target});expect(result.ok).toBe(true);if(!result.ok)return;
    const copy=PM.L((result.data.result as any).id);expect(copy.p['position.x'].kf[0].i).not.toBe(PM.L(target).p['position.x'].kf[0].i);
    copy.p['position.y'].v=12;expect(PM.L(target).p['position.y'].v).toBe(540);PM.hist.undo();expect(PM.proj.layers).toHaveLength(1);
  });
  it('splits a legacy OBJ into layers and restores it exactly with one undo',()=>{
    const PM=editor(),defs=PM.layerDefinition;
    PM.layerDefinition=(id:string)=>id==='old-model'?{id,label:'OBJ',version:1,renderer:{kind:'mesh',assetField:'assetId'},params:[],defaults:{}}:defs(id);
    PM.Edit.apply({type:'add_layer',layerType:'extension',content:{definition:'old-model',data:{assetId:'model'}}});
    const layer=PM.proj.layers[0];layer.d.params={yaw:PM.P(0,{kf:[PM.KF(0,0),PM.KF(2,90)]}),pitch:PM.P(0),distance:PM.P(4),rotationY:PM.P(15),background:PM.P('#112233')};
    PM.evP(layer,layer.d.params.yaw,1,'x.yaw');
    const before=JSON.stringify(layer.d),count=PM.hist.list().length,result=convertLegacyScene(PM,layer.id);expect(result,result.message).toMatchObject({ok:true});
    expect(PM.L(layer.id).d.definition).toBe(LAYER3D_DEFINITIONS.object);expect(PM.L(layer.id).p['rotation.y'].v).toBe(-15);
    const camera=PM.proj.layers.find((l:any)=>l.d.definition===LAYER3D_DEFINITIONS.camera);expect(camera.p['position.x'].kf.length).toBeGreaterThan(30);
    expect(camera.d.data.environment.background).toBe('#112233');expect(PM.hist.list().length).toBe(count+1);PM.hist.undo();expect(JSON.stringify(PM.L(layer.id).d)).toBe(before);
  });
  it('rejects invalid meshes, maps, hierarchy and camera planes',()=>{
    expect(()=>parseScene({objects:[{id:'a',source:{mesh:{positions:[0,0,0,1,0,0,0,1,0],indices:[0,1,7]}}}]})).toThrow();
    expect(()=>parseScene({objects:[{id:'a',source:{primitive:'box'},parent:'a'}]})).toThrow(/cycle/);
    expect(()=>parseScene({objects:[{id:'a',source:{primitive:'box'},material:{maps:{color:null}}}]})).toThrow();
    expect(()=>parseLayer3DData('camera',{camera:{p:{near:10,far:2}}})).toThrow(/far plane/);
  });
  it('rejects invalid native camera edits before changing live channels',()=>{
    const PM=editor();editScene(PM,{operation:'add_camera'});const camera=PM.proj.layers[0],before=JSON.stringify(camera.d.data);
    PM.Edit.begin('Camera edit');
    expect(PM.Edit.apply({type:'set_property',target:camera.id,path:'camera.near',value:200000}).ok).toBe(false);
    expect(JSON.stringify(camera.d.data)).toBe(before);PM.Edit.cancel();
    editScene(PM,{operation:'add_object',object:{id:'mirror',source:{primitive:'box'},p:{sx:-1,sy:0}}});
    expect(compositionScene(PM).objects[0]!.p.sx!.v).toBe(-1);expect(compositionScene(PM).objects[0]!.p.sy!.v).toBe(0);
  });
  it('uses native parent matrices and solo/opacity in shared depth groups',()=>{
    const PM=editor();
    editScene(PM,{operation:'add_object',object:{id:'parent',source:{primitive:'box'},p:{x:400}}});
    editScene(PM,{operation:'add_object',object:{id:'child',source:{primitive:'box'},p:{x:200}}});
    PM.L('child').parent='parent';PM.touch();
    const runtime=compositionRuntime(PM,0);
    // 600 px across a 1920 px composition is 1.8 scene units left of centre.
    expect(runtime.objects.get('child')!.getWorldPosition(new THREE.Vector3()).x).toBeCloseTo(-1.8);
    PM.L('child').solo=true;PM.touch();expect([...compositionDepthIds(PM,PM.L('child'),0)]).toEqual(['child']);
    PM.L('child').solo=false;PM.L('parent').p.opacity.v=0;PM.touch();
    expect([...compositionDepthIds(PM,PM.L('child'),0)]).toEqual(['child']);runtime.dispose();
  });
  it('orders translucent models by camera depth while retaining 2D barriers',()=>{
    const PM=editor();editScene(PM,{operation:'add_camera',camera:{p:{x:2160,y:540,z:0}}});
    editScene(PM,{operation:'add_object',object:{id:'near',source:{primitive:'box'},p:{x:1360}}});
    editScene(PM,{operation:'add_object',object:{id:'far',source:{primitive:'box'},p:{x:560}}});
    const near=PM.L('near'),far=PM.L('far');expect(compositionOrderedLayers(PM,[far,near],0)).toEqual([near,far]);
    const divider=PM.mkLayer('shape');expect(compositionOrderedLayers(PM,[far,divider,near],0)).toEqual([far,divider,near]);
    compositionRuntime(PM,0).dispose();
  });
});

describe('shared render graph, camera projection and picking',()=>{
  it('uses depth-aware picking and object bounds instead of a full-screen rectangle',()=>{
    const runtime=new SceneRuntime(),scene=parseScene({...createScene(),camera:{p:{x:0,y:0,z:6}},objects:[createObject('cube')]});
    runtime.sync(scene);runtime.resizeCamera(1920,1080);
    expect(runtime.pick(0,0)).toBe('cube');expect(runtime.pick(.9,.9)).toBeNull();
    const bounds=runtime.bounds(1920,1080)!;expect(bounds.w).toBeLessThan(500);expect(bounds.h).toBeLessThan(500);
    expect(runtime.scene.background).toBeNull();runtime.dispose();
  });
  it('retains geometry while updating transforms, UV textures, lights and shadows',()=>{
    const runtime=new SceneRuntime(),image={kind:'image',el:{width:2,height:2}};
    const scene=parseScene({...createScene(),objects:[{id:'a',source:{mesh:{positions:[0,0,0,1,0,0,0,1,0],uvs:[0,0,1,0,0,1]}},material:{maps:{color:'tex',normal:'tex'}}}],lights:[{id:'sun',type:'sun'},{id:'point',type:'point',p:{intensity:12}}]});
    runtime.sync(scene,()=>image);const object=runtime.objects.get('a') as THREE.Mesh,geometry=object.geometry;
    scene.objects[0]!.p.x!.v=2;runtime.sync(scene,()=>image);
    expect(runtime.objects.get('a')).toBe(object);expect(object.geometry).toBe(geometry);expect(object.position.x).toBe(2);
    expect(geometry.getAttribute('uv').count).toBe(3);
    const material=object.material as THREE.MeshStandardMaterial;
    expect(material.map?.colorSpace).toBe(THREE.SRGBColorSpace);expect(material.normalMap?.colorSpace).toBe(THREE.NoColorSpace);
    expect((runtime.objects.get('point') as THREE.PointLight).intensity).toBe(12);expect(object.castShadow).toBe(true);runtime.dispose();
  });
  it('uses the same evaluated channels for hierarchy and animation',()=>{
    const runtime=new SceneRuntime(),scene=parseScene({objects:[{id:'child',parent:'root',source:{primitive:'sphere'},p:{x:1}},{id:'root',source:{primitive:'box'},p:{x:2}}]});
    runtime.sync(scene,()=>null,(p,path)=>path==='o.root.x'?4:p.v);
    const child=runtime.objects.get('child')!;expect(child.getWorldPosition(new THREE.Vector3()).x).toBe(5);runtime.dispose();
  });
  it('keeps descendants materials and picking IDs independent when parents redraw',()=>{
    const runtime=new SceneRuntime(),scene=parseScene({objects:[{id:'child',parent:'root',source:{primitive:'sphere'},p:{x:2},material:{p:{color:'#FF0000'}}},{id:'root',source:{primitive:'box'},material:{p:{color:'#0000FF'}}}]});
    runtime.sync(scene);runtime.sync(scene);
    const child=runtime.objects.get('child') as THREE.Mesh;
    expect(child.userData.sceneId).toBe('child');expect((child.material as THREE.MeshStandardMaterial).color.getHexString()).toBe('ff0000');
    expect(child.parent).toBe(runtime.objects.get('root'));runtime.dispose();
  });
  it('restores imported source materials after applying a scene material override',()=>{
    const source=new THREE.Group();source.add(new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial({color:'#FF0000'})));
    const asset={kind:'model',object3d:source},runtime=new SceneRuntime(),scene=parseScene({objects:[{id:'model',source:{assetId:'asset'},useSourceMaterials:true}]});
    runtime.sync(scene,()=>asset);scene.objects[0]!.useSourceMaterials=false;runtime.sync(scene,()=>asset);
    scene.objects[0]!.useSourceMaterials=true;runtime.sync(scene,()=>asset);
    expect(((runtime.objects.get('model')!.children[0] as THREE.Mesh).material as THREE.MeshStandardMaterial).color.getHexString()).toBe('ff0000');runtime.dispose();
  });
  it('generates normals and UVs for every procedural primitive',()=>{
    for(const primitive of ['box','sphere','plane','cylinder','cone','torus','capsule','icosahedron'] as const){const g=primitiveGeometry({primitive});expect(g.getAttribute('position').count).toBeGreaterThan(0);expect(g.getAttribute('normal').count).toBeGreaterThan(0);g.dispose();}
  });
  it('plays embedded model animations from their layer start and reuses geometry',()=>{
    const root=new THREE.Group(),part=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial());part.name='Part';root.add(part);
    const asset={object3d:root,animations:[new THREE.AnimationClip('Slide',2,[new THREE.VectorKeyframeTrack('Part.position',[0,2],[0,0,0,2,0,0])])]},runtime=new SceneRuntime();
    const scene=parseScene({objects:[{id:'model',source:{assetId:'animated'},useSourceMaterials:true}]});
    runtime.sync(scene,()=>asset,p=>p.v,3,false,()=>1);
    expect(runtime.objects.get('model')!.getObjectByName('Part')!.position.x).toBeCloseTo(1);
    const geometry=(runtime.objects.get('model')!.getObjectByName('Part') as THREE.Mesh).geometry;
    runtime.sync(scene,()=>asset,p=>p.v,3.5,false,()=>1.5);
    expect(runtime.objects.get('model')!.getObjectByName('Part')!.position.x).toBeCloseTo(1.5);
    expect((runtime.objects.get('model')!.getObjectByName('Part') as THREE.Mesh).geometry).toBe(geometry);runtime.dispose();
  });
});
