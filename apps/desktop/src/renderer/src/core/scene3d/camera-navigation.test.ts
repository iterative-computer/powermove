import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {makePM} from '../../legacy/__tests__/make-pm';
import {LAYER3D_DEFINITIONS} from './layers';
import {editScene} from './operations';
import {compositionRuntime} from './service';
import {viewportPose,getViewportMode} from './viewport';
import {orbitView,panView,dollyView} from './navigation';
import {activeCameraLayer,cameraNavigationEdit} from './camera-navigation';

function fixture(projection='perspective'){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({dur:10});PM.time=2;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  expect(editScene(PM,{operation:'add_camera',id:'shot',camera:{projection,p:{x:0,y:0,z:6}}}).ok).toBe(true);
  PM.hist.clear();return {PM,shot:PM.L('shot')};
}
const close=(actual:THREE.Vector3,expected:THREE.Vector3)=>expect(actual.distanceTo(expected)).toBeLessThan(1e-7);

describe('camera navigation edits',()=>{
  it.each(['orbit','pan','dolly'])('writes real camera channels with one undo for a %s drag',mode=>{
    const {PM}=fixture(),before=JSON.stringify(PM.proj.layers),start=viewportPose(PM),camera=compositionRuntime(PM).camera;
    const edit=cameraNavigationEdit(PM,PM.L('shot'),start,'Move camera');
    const next=(n:number)=>mode==='orbit'?orbitView(start,n,20):mode==='pan'?panView(start,n,20,camera,600):dollyView(start,n,camera);
    for(const n of [10,20,40])edit.update(next(n));edit.commit();
    close(compositionRuntime(PM).camera.position,next(40).position);close(viewportPose(PM).target,next(40).target);
    expect(getViewportMode(PM)).toBe('camera');expect(PM.hist.list()).toHaveLength(1);
    expect(PM.hist.undo()).toBe(true);expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.redo()).toBe(true);close(compositionRuntime(PM).camera.position,next(40).position);
  });
  it('writes animated channels at the playhead in trimmed camera local time and preserves other keys',()=>{
    const {PM,shot}=fixture();shot.from=-3;
    expect(PM.Edit.apply({type:'replace_keyframes',target:'shot',path:'position.x',keyframes:[{time:0,value:0},{time:8,value:3}]}).ok).toBe(true);
    const original=JSON.parse(JSON.stringify(shot.p['position.x'].kf));PM.hist.clear();PM.autokey=true;
    const start=viewportPose(PM),next=panView(start,50,0,compositionRuntime(PM).camera,600),edit=cameraNavigationEdit(PM,shot,start,'Pan camera');
    edit.update(next);edit.commit();
    const keys=PM.L('shot').p['position.x'].kf;
    expect(keys.map((k:any)=>k.t)).toEqual([0,5,8]);expect(keys[0]).toEqual(original[0]);expect(keys[2]).toEqual(original[1]);
    expect(PM.L('shot').d.data.camera.p.targetX.kf[0].t).toBe(5);
    PM.hist.undo();expect(PM.L('shot').p['position.x'].kf).toEqual(original);
  });
  it('converts world motion through a rotated, scaled parent and preserves the camera anchor',()=>{
    const {PM,shot}=fixture(),parent=PM.mkLayer('null',{threeD:true,d:{boundsNullSpace:'scene'},p:{'position.x':2,'rotation.y':35,'scale.x':150,'scale.y':150,'scale.z':150}});
    PM.proj.layers.push(parent);shot.parent=parent.id;shot.p['anchor.x'].v=.4;PM.touch();
    const before=JSON.stringify(PM.proj.layers),start=viewportPose(PM),next=panView(start,60,35,compositionRuntime(PM).camera,600),edit=cameraNavigationEdit(PM,shot,start,'Pan camera');
    edit.update(next);edit.commit();close(compositionRuntime(PM).camera.position,next.position);close(viewportPose(PM).target,next.target);
    expect(PM.L('shot').p['anchor.x'].v).toBe(.4);PM.hist.undo();expect(JSON.stringify(PM.proj.layers)).toBe(before);
  });
  it('animates orthographic zoom and cancels without leaving history or keys',()=>{
    const {PM,shot}=fixture('orthographic');PM.autokey=true;
    const before=JSON.stringify(PM.proj.layers),start=viewportPose(PM),edit=cameraNavigationEdit(PM,shot,start,'Dolly camera');
    edit.update(dollyView(start,-50,compositionRuntime(PM).camera));
    expect(PM.L('shot').d.data.camera.p.zoom.kf).toHaveLength(1);expect(compositionRuntime(PM).camera.zoom).toBeGreaterThan(1);
    edit.cancel();expect(JSON.stringify(PM.proj.layers)).toBe(before);expect(PM.hist.list()).toHaveLength(0);
    const click=cameraNavigationEdit(PM,PM.L('shot'),viewportPose(PM),'Dolly camera');click.commit();expect(PM.hist.list()).toHaveLength(0);
  });
  it('respects active camera precedence, lock, expressions and singular parent transforms',()=>{
    const {PM,shot}=fixture();expect(editScene(PM,{operation:'add_camera',id:'other'}).ok).toBe(true);
    const active=activeCameraLayer(PM);expect(active.id).toBe(PM.proj.layers.find((l:any)=>l.d?.definition===LAYER3D_DEFINITIONS.camera).id);
    active.lock=true;expect(()=>cameraNavigationEdit(PM,active,viewportPose(PM),'Pan camera')).toThrow(/Unlock/);active.lock=false;
    const before=JSON.stringify(PM.proj.layers),start=viewportPose(PM),edit=cameraNavigationEdit(PM,active,start,'Pan camera');
    active.p['position.x'].expr='0';expect(()=>edit.update({...start,position:start.position.clone().add(new THREE.Vector3(1,0,0))})).toThrow(/expression/);edit.cancel();active.p['position.x'].expr=null;
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    const parent=PM.mkLayer('null',{threeD:true,p:{'scale.x':0}});PM.proj.layers.push(parent);shot.parent=parent.id;
    expect(()=>cameraNavigationEdit(PM,shot,start,'Pan camera')).toThrow(/zero scale/);
  });
});
