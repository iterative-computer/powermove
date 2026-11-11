import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {makeScene3DAPI} from './api';
import {LAYER3D_DEFINITIONS} from './layers';
import {PARTS_ROOT_FLAG,partNode,partsRoot} from './model-parts';
import {compositionRuntime} from './service';

function fixture(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({dur:3});PM.time=.5;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  const source=new THREE.Group(),file=new THREE.Group(),body=new THREE.Group();file.userData[PARTS_ROOT_FLAG]=true;body.name='Body';body.userData.name='Body';body.rotation.y=.3;body.scale.set(1.5,2,1);
  for(let i=0;i<5;i++){const mesh=new THREE.Mesh(new THREE.BoxGeometry(.2,.2,.2));mesh.name=`Part ${i}`;mesh.userData.name=mesh.name;mesh.position.set(i*.3,.1*i,0);body.add(mesh);}
  file.add(body);source.add(file);PM.assets.map.set('asset',{object3d:source});PM.assets.get=(id:string)=>PM.assets.map.get(id);
  const api=makeScene3DAPI(PM,()=>{});expect(api.edit({operation:'add_object',object:{id:'model',source:{assetId:'asset'}}}).ok).toBe(true);
  const group=(keys:string[])=>{const result=api.groupParts('model',keys);expect(result.ok,result.message).toBe(true);return api.getActivePart('model')!;};
  const matrices=()=>{const model=compositionRuntime(PM).objects.get('model')!;model.updateWorldMatrix(true,true);const root=partsRoot(model);return Array.from({length:5},(_,i)=>partNode(root,`0_${i}`)!.matrixWorld.toArray());};
  const same=(before:number[][])=>matrices().forEach((m,i)=>m.forEach((v,j)=>expect(v).toBeCloseTo(before[i]![j]!,7)));
  return {PM,api,group,matrices,same};
}

describe('moving model parts between existing groups',()=>{
  it('keeps placement and individual animation when adding, moving between transformed groups, and moving out',()=>{
    const {PM,api,group,matrices,same}=fixture(),a=group(['0_0','0_1']),b=group(['0_2','0_3']);
    api.setPart('model',a,{x:.3,rz:35,scale:130});api.setPart('model',b,{x:-.4,ry:25,scale:80});
    api.setPart('model','0_4',{y:{v:0,kf:[{t:0,v:0},{t:1,v:.3}],expr:null}});
    const channels=JSON.stringify(PM.L('model').d.data.object.parts['0_4'].p);
    for(const destination of [a,b,null]){
      const before=matrices(),count=PM.hist.list().length;
      const result=api.moveParts('model',['0_4'],destination);expect(result.ok,result.message).toBe(true);same(before);
      expect(api.listParts('model').find(p=>p.key==='0_4')!.parent).toBe(destination || '0');
      expect(JSON.stringify(PM.L('model').d.data.object.parts['0_4'].p)).toBe(channels);expect(PM.hist.list()).toHaveLength(count+1);
      PM.hist.undo();same(before);PM.hist.redo();same(before);
    }
    const before=matrices();PM.proj=PM.deserialize(PM.serialize());PM.touch();same(before);
  });
  it('adds to an animated group at the playhead and follows its later movement',()=>{
    const {PM,api,group,matrices,same}=fixture(),a=group(['0_0','0_1']);
    api.setPart('model',a,{x:{v:0,kf:[{t:0,v:0},{t:1,v:1}],expr:null}});
    const before=matrices();expect(api.moveParts('model',['0_4'],a).ok).toBe(true);same(before);
    PM.time=1;const after=matrices();expect(after[4]![12]! - before[4]![12]!).toBeCloseTo(.5,7);
    expect(api.getPart('model',a).channels!.x.kf).toHaveLength(2);
  });
  it('moves nested groups and several members, and removes an emptied source group',()=>{
    const {PM,api,group,matrices,same}=fixture(),a=group(['0_0','0_1']),b=group(['0_2','0_3']),outer=group([a,b,'0_4']);
    api.setPart('model',outer,{ry:20,x:.3});api.setPart('model',a,{z:.2,rz:30});api.setPart('model',b,{scale:150});
    let before=matrices();expect(api.moveParts('model',[a],b).ok).toBe(true);same(before);
    expect(api.listParts('model').find(p=>p.key===a)!.parent).toBe(b);
    before=matrices();expect(api.moveParts('model',['0_0','0_1'],b).ok).toBe(true);same(before);
    expect(api.listParts('model').some(p=>p.key===a)).toBe(false);expect(api.listParts('model').some(p=>p.key===outer)).toBe(true);
    PM.hist.undo();expect(api.listParts('model').some(p=>p.key===a)).toBe(true);same(before);
  });
  it('refuses cycles, unknown parts, locked layers, incompatible file levels and zero-scale targets atomically',()=>{
    const {PM,api,group}=fixture(),a=group(['0_0','0_1']),b=group(['0_2','0_3']),outer=group([a,b]);
    const refused=(keys:string[],destination:string)=>{const before=PM.serialize();expect(api.moveParts('model',keys,destination).ok).toBe(false);expect(PM.serialize()).toBe(before);};
    refused([outer],a);refused([a],a);refused(['0_4'],'0_0');refused(['missing'],a);refused(['0'],a);
    PM.L('model').lock=true;refused(['0_4'],a);PM.L('model').lock=false;
    api.setPart('model',a,{scale:0});refused(['0_4'],a);
  });
});
