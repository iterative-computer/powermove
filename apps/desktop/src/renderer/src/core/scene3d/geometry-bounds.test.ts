import { describe,it,expect } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { createBoundsNull,geometryBounds,boundsScope } from './geometry-bounds';
import { editScene } from './operations';
import { world3D } from '../../legacy/core/space-3d';
import {nativeSceneBridge} from './native-transform';
import { isModelGroup,isModelNull,LAYER3D_DEFINITIONS } from './layers';
import { nullViewerPosition,nullViewerState } from './light-view';
import {applyViewportPose} from './viewport';
import { SceneRuntime } from './runtime';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Bounds',w:600,h:600,fps:30,dur:3});PM.time=.47;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  PM.GL.bounds=()=>({x0:0,y0:0,x1:100,y1:50,w:100,h:50});return PM;
}
function ok(result:any){expect(result.ok,result.message).toBe(true);return result.data;}
function nullIds(result:any):string[]{return ok(result).result.nullIds;}
const times=[0,.133,.47,.713,1.17,2.83];
function poses(PM:any,ids:string[]){return times.map(time=>ids.map(id=>world3D(PM,PM.L(id),time)));}
function samePoses(a:number[][][],b:number[][][]){a.forEach((sample,t)=>sample.forEach((m,l)=>m.forEach((v,i)=>expect(b[t]![l]![i]).toBeCloseTo(v,9))));}

describe('evaluated bounds and native bounds null insertion',()=>{
  it('matches a trimmed source layer without shifting its animation or replacement timing',()=>{
    const PM=editor(),source=PM.mkLayer('shape',{from:.75,dur:1.5});PM.proj.layers=[source];PM.time=1.2;
    source.p.rotation.kf=[{id:'start',t:0,v:10},{id:'end',t:1.5,v:65}];PM.touch();
    const before=poses(PM,[source.id]),[id]=nullIds(createBoundsNull(PM,[source.id],4));
    expect(PM.L(id)).toMatchObject({from:.75,dur:1.5});samePoses(before,poses(PM,[source.id]));
    PM.L(id).from=.5;PM.L(id).dur=2;
    expect(nullIds(createBoundsNull(PM,[source.id],8,{replace:true}))).toEqual([id]);
    expect(PM.L(id)).toMatchObject({from:.5,dur:2});samePoses(before,poses(PM,[source.id]));
  });
  it('spans the earliest selected start and latest selected end across parent spaces',()=>{
    const PM=editor(),a=PM.mkLayer('shape',{from:.25,dur:1}),b=PM.mkLayer('shape',{from:1,dur:1.75}),parent=PM.mkLayer('null');
    b.parent=parent.id;PM.proj.layers=[a,b,parent];PM.time=1.1;PM.touch();
    const ids=nullIds(createBoundsNull(PM,[a.id,b.id],4)),[first]=ids;expect(ids).toHaveLength(1);
    expect(PM.L(first)).toMatchObject({from:.25,dur:2.5});expect(parent.parent).toBe(first);expect(b.parent).toBe(parent.id);
    PM.hist.undo();expect(PM.L(first)).toBeNull();
    PM.hist.redo();expect(PM.L(first)).toMatchObject({from:.25,dur:2.5});
  });
  it('measures rotated mesh vertices tightly rather than rotating its source bounding box',()=>{
    const PM=editor();ok(editScene(PM,{operation:'add_object',object:{id:'triangle',source:{mesh:{positions:[0,0,0,2,0,0,0,1,0]}},p:{rz:45}}}));
    const bounds=geometryBounds(PM,['triangle'])!;
    expect(bounds.x0).toBeCloseTo(-Math.SQRT1_2);expect(bounds.x1).toBeCloseTo(Math.SQRT2);
    expect(bounds.y1).toBeCloseTo(Math.SQRT2);expect(bounds.z0).toBe(0);
  });
  it('supports grouped models under an animated master without changing a single existing channel',()=>{
    const PM=editor();for(const id of ['a','b'])ok(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{x:id==='a'?-2:2}}}));
    const group=PM.groupLayers(['a','b']);
    ok(PM.Edit.apply({type:'replace_keyframes',target:group.id,path:'rotation.y',keyframes:[{time:0,value:-30},{time:3,value:150}]}));
    PM.L('a').p['position.y'].expr='value + sin(time * 2)';PM.touch();
    const channels=JSON.stringify(PM.proj.layers.map((l:any)=>l.p)),before=poses(PM,['a','b']),count=PM.hist.list().length;
    const [id]=nullIds(createBoundsNull(PM,['a','b'],0));
    expect(isModelGroup(PM,group)).toBe(true);expect(PM.L(id).group).toBe(group.id);
    expect(JSON.stringify(PM.proj.layers.filter((l:any)=>l.id!==id).map((l:any)=>l.p))).toBe(channels);
    samePoses(before,poses(PM,['a','b']));expect(PM.hist.list()).toHaveLength(count+1);
    PM.hist.undo();expect(PM.L(id)).toBeNull();samePoses(before,poses(PM,['a','b']));
    PM.hist.redo();expect(PM.L(id)).toBeTruthy();samePoses(before,poses(PM,['a','b']));
  });
  it('includes descendants of selected groups and parents automatically, but does not move siblings',()=>{
    const PM=editor();const a=PM.mkLayer('shape'),b=PM.mkLayer('shape'),outside=PM.mkLayer('shape');
    PM.proj.layers=[a,b,outside];PM.touch();const group=PM.groupLayers([a.id,b.id]);
    const before=poses(PM,[a.id,b.id,outside.id]);const [id]=nullIds(createBoundsNull(PM,[group.id],8));
    expect(group.parent).toBe(id);expect(outside.parent).toBeNull();samePoses(before,poses(PM,[a.id,b.id,outside.id]));
    expect(boundsScope(PM,[id!]).affected.map(l=>l.id)).toEqual(expect.arrayContaining([a.id,b.id,group.id,id]));
  });
  it('wraps existing animated nulls with one shared controller in one transaction',()=>{
    const PM=editor(),a=PM.mkLayer('shape'),b=PM.mkLayer('shape'),pa=PM.mkLayer('null'),pb=PM.mkLayer('null');
    PM.proj.layers=[a,b,pa,pb];a.parent=pa.id;b.parent=pb.id;
    pa.p.rotation.expr='time * 25';pb.p['scale.x'].expr='100 + time * 20';PM.touch();
    const before=poses(PM,[a.id,b.id,pa.id,pb.id]),count=PM.hist.list().length;
    const ids=nullIds(createBoundsNull(PM,[a.id,b.id],4));expect(ids).toHaveLength(1);
    expect(pa.parent).toBe(ids[0]);expect(pb.parent).toBe(ids[0]);expect(a.parent).toBe(pa.id);expect(b.parent).toBe(pb.id);
    samePoses(before,poses(PM,[a.id,b.id,pa.id,pb.id]));expect(PM.hist.list()).toHaveLength(count+1);
  });
  it('uses the nearest shared non-null ancestor and wraps the outermost existing null',()=>{
    const PM=editor(),outer=PM.mkLayer('group'),a=PM.mkLayer('shape'),b=PM.mkLayer('shape'),pa=PM.mkLayer('null'),nested=PM.mkLayer('null'),pb=PM.mkLayer('null');
    pa.parent=outer.id;pb.parent=outer.id;nested.parent=pa.id;a.parent=nested.id;b.parent=pb.id;
    PM.proj.layers=[outer,pa,nested,a,pb,b];outer.p.rotation.expr='time * 10';nested.p['position.x'].expr='20 * time';PM.touch();
    const before=poses(PM,[a.id,b.id]),[id]=nullIds(createBoundsNull(PM,[a.id,b.id],7));
    expect(PM.L(id).parent).toBe(outer.id);expect(pa.parent).toBe(id);expect(pb.parent).toBe(id);expect(nested.parent).toBe(pa.id);expect(a.parent).toBe(nested.id);
    samePoses(before,poses(PM,[a.id,b.id]));
  });
  it('moves mixed native artwork and models by the same scene motion without rebasing animation',()=>{
    const PM=editor(),shape=PM.mkLayer('shape',{p:{'position.x':400,'position.y':250}});PM.proj.layers=[shape];
    ok(editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:1,y:.5}}}));
    const group=PM.groupLayers(['model']);group.p.rotation.expr='time * 15';shape.p['position.x'].expr='value + time * 20';PM.touch();
    const before=poses(PM,[shape.id,'model']),channels=JSON.stringify([shape.p,group.p,PM.L('model').p]);
    const ids=nullIds(createBoundsNull(PM,[shape.id,group.id],4));expect(ids).toHaveLength(1);const node=PM.L(ids[0]);
    expect(shape.parent).toBe(node.id);expect(group.parent).toBe(node.id);expect(node.d.boundsNullSpace).toBe('scene');
    samePoses(before,poses(PM,[shape.id,'model']));expect(JSON.stringify([shape.p,group.p,PM.L('model').p])).toBe(channels);
    const flatBefore=PM.worldMatrix(shape,PM.time).slice();node.p['position.x'].v+=1;node.p['position.y'].v+=.5;PM.touch();
    const after=poses(PM,[shape.id,'model']);
    for(let t=0;t<times.length;t++){
      expect(after[t]![0]![12]!-before[t]![0]![12]!).toBeCloseTo(100,8);expect(after[t]![0]![13]!-before[t]![0]![13]!).toBeCloseTo(-50,8);
      expect(after[t]![1]![12]!-before[t]![1]![12]!).toBeCloseTo(1,8);expect(after[t]![1]![13]!-before[t]![1]![13]!).toBeCloseTo(.5,8);
    }
    expect(PM.worldMatrix(shape,PM.time)[4]-flatBefore[4]).toBeCloseTo(100,8);expect(PM.worldMatrix(shape,PM.time)[5]-flatBefore[5]).toBeCloseTo(-50,8);
    const [next]=nullIds(createBoundsNull(PM,[shape.id,group.id],4));expect(node.parent).toBe(next);expect(shape.parent).toBe(node.id);expect(group.parent).toBe(node.id);
    samePoses(after,poses(PM,[shape.id,'model']));
  });
  it('applies a shared controller only once across crossed group and null-parent branches',()=>{
    const PM=editor(),a=PM.mkLayer('shape'),b=PM.mkLayer('shape'),parent=PM.mkLayer('null');PM.proj.layers=[a,b,parent];PM.touch();
    const group=PM.groupLayers([a.id,b.id]);a.parent=parent.id;group.p.rotation.expr='time * 20';parent.p['scale.x'].expr='100 + time * 15';PM.touch();
    const before=poses(PM,[a.id,b.id]),[id]=nullIds(createBoundsNull(PM,[a.id,b.id],4));
    expect(parent.parent).toBe(id);expect(group.parent).toBe(id);samePoses(before,poses(PM,[a.id,b.id]));
    PM.L(id).p['position.x'].v+=75;PM.touch();const after=poses(PM,[a.id,b.id]);
    for(let t=0;t<times.length;t++)for(let l=0;l<2;l++)expect(after[t]![l]![12]!-before[t]![l]![12]!).toBeCloseTo(75,8);
  });
  it.each([false,true])('rotates and scales native artwork (%s 3D) and models in one scene space',threeD=>{
    const PM=editor(),art=PM.mkLayer('shape',{p:{'position.x':420,'position.y':250}});art.threeD=threeD;PM.proj.layers=[art];
    ok(editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:-1,y:.5}}}));
    PM.touch();const bridge=nativeSceneBridge(PM.proj),nativeBefore=bridge.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,art,PM.time))),modelBefore=new THREE.Matrix4().fromArray(world3D(PM,PM.L('model'),PM.time));
    const [id]=nullIds(createBoundsNull(PM,[art.id,'model'],7)),node=PM.L(id);
    node.p.rotation.v=37;node.p['rotation.x'].v=threeD?23:0;node.p['scale.x'].v=135;node.p['scale.y'].v=80;PM.touch();
    const delta=new THREE.Matrix4().fromArray(world3D(PM,node,PM.time));
    const expectedNative=delta.clone().multiply(nativeBefore),expectedModel=delta.clone().multiply(modelBefore);
    const actualNative=bridge.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,art,PM.time))),actualModel=new THREE.Matrix4().fromArray(world3D(PM,PM.L('model'),PM.time));
    expectedNative.elements.forEach((v,i)=>expect(actualNative.elements[i]).toBeCloseTo(v,9));expectedModel.elements.forEach((v,i)=>expect(actualModel.elements[i]).toBeCloseTo(v,9));
  });
  it.each(Array.from({length:9},(_,i)=>i))('places button %i at the evaluated box edge and middle depth',index=>{
    const PM=editor();ok(editScene(PM,{operation:'add_object',object:{id:'box',source:{primitive:'box',parameters:{width:4,height:2,depth:6}},p:{x:8,y:4,z:-3}}}));
    const before=geometryBounds(PM,['box'])!,[id]=nullIds(createBoundsNull(PM,['box'],index)),node=PM.L(id);
    expect(node.p['anchor.x'].v).toBeCloseTo(before.x0+(before.x1-before.x0)*(index%3)/2);
    expect(node.p['anchor.y'].v).toBeCloseTo(before.y1-(before.y1-before.y0)*Math.floor(index/3)/2);
    expect(node.p['anchor.z'].v).toBeCloseTo(-3);expect(geometryBounds(PM,['box'])).toEqual(before);
  });
  it('repositions a static bounds null, retains all children and undoes once',()=>{
    const PM=editor(),a=PM.mkLayer('shape',{p:{'position.x':0,'position.y':0}});PM.proj.layers=[a];PM.touch();
    const [id]=nullIds(createBoundsNull(PM,[a.id],0)),before=poses(PM,[a.id]);
    expect(nullIds(createBoundsNull(PM,[a.id],8,{replace:true}))).toEqual([id]);
    expect(PM.L(id).p['anchor.x'].v).toBeCloseTo(100);samePoses(before,poses(PM,[a.id]));
    PM.hist.undo();expect(PM.L(id).p['anchor.x'].v).toBeCloseTo(0);
  });
  it('retains scene units when wrapping an existing model null',()=>{
    const PM=editor();ok(editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:2,y:1,z:3}}}));
    const [first]=nullIds(createBoundsNull(PM,['model'],4));
    const before=poses(PM,['model']),[second]=nullIds(createBoundsNull(PM,[first!],4));
    expect(['x','y','z'].map(axis=>PM.L(second).p[`anchor.${axis}`].v)).toEqual([2,1,3]);
    samePoses(before,poses(PM,['model']));
  });
  it('recognizes new and previously saved model controls and projects their anchors through the scene camera',()=>{
    const PM=editor();ok(editScene(PM,{operation:'add_camera',camera:{projection:'orthographic',p:{x:0,y:0,z:6,zoom:1}}}));
    ok(editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'},p:{x:1,y:.5}}}));
    const [id]=nullIds(createBoundsNull(PM,['model'],4)),node=PM.L(id);
    expect(PM.sel.layers).toEqual([id]);expect(node.d.boundsNullSpace).toBe('scene');expect(isModelNull(PM,node)).toBe(true);
    const rect={x:10,y:20,width:600,height:600};expect(nullViewerPosition(PM,id!,PM.time,rect)).toEqual({x:410,y:270});
    const visible=nullViewerState(PM,id!,PM.time,rect)!;expect(visible.corners).toHaveLength(4);
    expect(Math.max(...visible.corners.map(p=>p.x))-Math.min(...visible.corners.map(p=>p.x))).toBeCloseTo(100,8);
    PM.selectLayers([]);expect(nullViewerState(PM,id!,PM.time,rect)).toEqual(visible);
    applyViewportPose(PM,{position:new THREE.Vector3(4,2,6),target:new THREE.Vector3(),zoom:1});
    const orbited=nullViewerState(PM,id!,PM.time,rect)!;expect(orbited.corners).toHaveLength(4);expect(orbited.corners).not.toEqual(visible.corners);
    applyViewportPose(PM,{position:new THREE.Vector3(0,0,6),target:new THREE.Vector3(),zoom:1});
    delete node.d.boundsNullSpace;expect(isModelNull(PM,node)).toBe(true);expect(nullViewerPosition(PM,id!,PM.time,rect)).toEqual({x:410,y:270});
    expect(nullIds(createBoundsNull(PM,[id!],8,{replace:true}))).toEqual([id]);
    expect(PM.L(id).p['anchor.x'].v).toBeCloseTo(1.5);expect(PM.L(id).p['anchor.y'].v).toBeCloseTo(0);
  });
  it('keeps crossed group and parent animation and unselected branches unchanged',()=>{
    const PM=editor(),a=PM.mkLayer('shape'),b=PM.mkLayer('shape'),parent=PM.mkLayer('null');
    PM.proj.layers=[a,b,parent];PM.touch();const group=PM.groupLayers([a.id]);
    const other=PM.groupLayers([b.id,parent.id]);a.parent=parent.id;
    group.p.rotation.expr='time * 37';other.p['scale.x'].expr='100 + time * 10';parent.p.rotation.expr='time * 19';PM.touch();
    const before=poses(PM,[a.id,b.id,parent.id,group.id,other.id]);
    nullIds(createBoundsNull(PM,[a.id],4));samePoses(before,poses(PM,[a.id,b.id,parent.id,group.id,other.id]));
  });
  it.each(['locked descendant','singular parent','index expression','stale revision','missing model'])('rejects %s atomically',reason=>{
    const PM=editor(),a=PM.mkLayer('shape'),p=PM.mkLayer('null');PM.proj.layers=[a,p];a.parent=p.id;
    if(reason==='locked descendant')a.lock=true;
    if(reason==='singular parent')p.p['scale.x'].v=0;
    if(reason==='index expression')a.p['position.x'].expr='idx * 10';
    if(reason==='missing model'){ok(editScene(PM,{operation:'add_object',object:{id:'model',source:{assetId:'missing'}}}));}
    PM.touch();const before=PM.serialize(),count=PM.hist.list().length;
    const result=createBoundsNull(PM,[reason==='missing model'?'model':reason==='singular parent'?a.id:p.id],4,{},reason==='stale revision'?{baseRevision:999}:{});
    expect(result.ok).toBe(false);expect(PM.serialize()).toBe(before);expect(PM.hist.list()).toHaveLength(count);
  });
  it('measures evaluated morphs and respects hidden parts',()=>{
    const runtime=new SceneRuntime(),geometry=new THREE.BoxGeometry(2,2,2);
    geometry.morphAttributes.position=[geometry.getAttribute('position').clone()];
    const morph=geometry.morphAttributes.position[0]!;for(let i=0;i<morph.count;i++)morph.setX(i,morph.getX(i)+4);
    const mesh=new THREE.Mesh(geometry,new THREE.MeshBasicMaterial());mesh.morphTargetInfluences![0]=.5;
    const hidden=new THREE.Mesh(new THREE.BoxGeometry(100,100,100));hidden.visible=false;mesh.add(hidden);
    runtime.objects.set('model',mesh);runtime.scene.add(mesh);
    const box=runtime.geometryBounds(['model']);expect(box.min.x).toBeCloseTo(1);expect(box.max.x).toBeCloseTo(3);
    runtime.dispose();
  });
  it('measures skinned vertices after the evaluated bone pose',()=>{
    const runtime=new SceneRuntime(),geometry=new THREE.BufferGeometry();
    geometry.setAttribute('position',new THREE.Float32BufferAttribute([0,0,0,1,0,0,0,1,0],3));
    geometry.setAttribute('skinIndex',new THREE.Uint16BufferAttribute(new Array(12).fill(0),4));
    geometry.setAttribute('skinWeight',new THREE.Float32BufferAttribute([1,0,0,0,1,0,0,0,1,0,0,0],4));
    const mesh=new THREE.SkinnedMesh(geometry,new THREE.MeshBasicMaterial()),bone=new THREE.Bone();mesh.add(bone);mesh.bind(new THREE.Skeleton([bone]));
    bone.position.x=5;runtime.objects.set('skin',mesh);runtime.scene.add(mesh);
    const box=runtime.geometryBounds(['skin']);expect(box.min.x).toBeCloseTo(5);expect(box.max.x).toBeCloseTo(6);runtime.dispose();
  });
});
