import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {orbitAround,dollyAt,cursorPivot,getNavigationVariant,setNavigationVariant,orbitView,panView,dollyView,frameView,frameComposition} from './navigation';
import {viewportPose,viewportCamera,getViewportMode,setViewportMode,applyViewportPose,resetViewport} from './viewport';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {makePM} from '../../legacy/__tests__/make-pm';
import {LAYER3D_DEFINITIONS} from './layers';

const pose=()=>({position:new THREE.Vector3(0,0,6),target:new THREE.Vector3(),zoom:1});
const camera=()=>{const c=new THREE.PerspectiveCamera(60,16/9,.01,1000);c.position.set(0,0,6);c.lookAt(0,0,0);c.updateMatrixWorld();return c;};
describe('3D viewport navigation',()=>{
  it('orbits around the target at a constant distance and stays clear of the poles',()=>{
    const start=pose(),next=orbitView(start,100,80);
    expect(next.position.distanceTo(next.target)).toBeCloseTo(6);expect(next.target).toEqual(start.target);
    expect(start.position.toArray()).toEqual([0,0,6]);
    for(const dy of [-100000,100000])expect(Math.abs(orbitView(start,0,dy).position.y)).toBeLessThan(6);
  });
  it('pans position and target together in the camera plane',()=>{
    const start=pose(),next=panView(start,100,50,camera(),600);
    expect(next.position.clone().sub(next.target).toArray()).toEqual([0,0,6]);
    expect(next.position.x).toBeLessThan(0);expect(next.position.y).toBeGreaterThan(0);
    expect(next.position.distanceTo(start.position)).toBeGreaterThan(0);
  });
  it('dollies reversibly and uses zoom for orthographic views',()=>{
    const start=pose(),c=camera(),away=dollyView(start,100,c);
    expect(away.position.z).toBeGreaterThan(6);expect(dollyView(away,-100,c).position.z).toBeCloseTo(6);
    const ortho=new THREE.OrthographicCamera(-3,3,3,-3,.01,1000),next=dollyView(start,-100,ortho);
    expect(next.zoom).toBeGreaterThan(1);expect(next.position).toEqual(start.position);
    expect(dollyView(start,-100000,c).position.z).toBeGreaterThanOrEqual(.05);
  });
  it('orbits about an off-centre hit without changing the view on a click',()=>{
    const start=pose(),pivot=new THREE.Vector3(1,0,0),idle=orbitAround(start,pivot,0,0),next=orbitAround(start,pivot,90,30);
    expect(idle.position.distanceTo(start.position)).toBeLessThan(1e-10);expect(idle.target.distanceTo(start.target)).toBeLessThan(1e-10);
    expect(next.position.distanceTo(pivot)).toBeCloseTo(start.position.distanceTo(pivot));
    expect(next.position.distanceTo(next.target)).toBeCloseTo(6);
    expect(next.target.distanceTo(pivot)).toBeCloseTo(start.target.distanceTo(pivot));
  });
  it('uses a mesh hit or the POI plane for an empty cursor, and pans at its depth',()=>{
    const c=camera(),start=pose(),hit=new THREE.Vector3(1,2,3);
    expect(cursorPivot(c,start.target,.5,.5,hit)).toEqual(hit);
    const fallback=cursorPivot(c,start.target,.5,.5);expect(fallback.z).toBeCloseTo(0);expect(fallback.x).toBeGreaterThan(0);
    const near=panView(start,50,0,c,600,new THREE.Vector3(0,0,3)),far=panView(start,50,0,c,600);
    expect(near.position.x).toBeCloseTo(far.position.x/2);
  });
  it('keeps the clicked pixel fixed during perspective and orthographic dollies',()=>{
    const pivot=new THREE.Vector3(1,1,0),start=pose();
    for(const c of [camera(),new THREE.OrthographicCamera(-3,3,3,-3,.01,1000)]){
      c.position.copy(start.position);c.lookAt(start.target);c.updateMatrixWorld();c.updateProjectionMatrix();
      const before=pivot.clone().project(c),next=dollyAt(start,pivot,-80,c);
      c.position.copy(next.position);c.zoom=next.zoom;c.lookAt(next.target);c.updateProjectionMatrix();c.updateMatrixWorld();
      const after=pivot.clone().project(c);expect(after.x).toBeCloseTo(before.x);expect(after.y).toBeCloseTo(before.y);
      expect(next.position.clone().sub(next.target).toArray()).toEqual([0,0,6]);
    }
    const c=camera(),out=dollyAt(start,pivot,80,c,true);
    expect(out.position.x).toBe(0);expect(dollyAt(start,pivot,80,c).position.x).toBeLessThan(0);
  });
  it('remembers variants separately and rejects unsupported tool combinations',()=>{
    const PM={};expect(getNavigationVariant(PM,'orbit')).toBe('cursor');expect(getNavigationVariant(PM,'dolly')).toBe('towards-cursor');
    setNavigationVariant(PM,'orbit','scene');expect(getNavigationVariant(PM,'orbit')).toBe('scene');expect(getNavigationVariant(PM,'pan')).toBe('cursor');
    expect(()=>setNavigationVariant(PM,'pan','scene')).toThrow();
  });
  it('frames every corner in wide and tall perspective views',()=>{
    const box=new THREE.Box3(new THREE.Vector3(-3,-1,-1),new THREE.Vector3(3,1,1));
    for(const aspect of [16/9,9/16]){
      const c=camera();c.aspect=aspect;const framed=frameView(pose(),box,c);c.position.copy(framed.position);c.lookAt(framed.target);c.updateProjectionMatrix();c.updateMatrixWorld();
      for(const x of [-3,3])for(const y of [-1,1])for(const z of [-1,1]){
        const point=new THREE.Vector3(x,y,z).project(c);expect(Math.abs(point.x)).toBeLessThan(1);expect(Math.abs(point.y)).toBeLessThan(1);
      }
    }
  });
  it('frames a selected group without adding a camera or changing project history',()=>{
    const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
    PM.proj=PM.mkProject();PM.time=0;
    PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
    for(const [id,x] of [['a',10],['b',12]] as const)expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{x}}}).ok).toBe(true);
    const group=PM.groupLayers(['a','b']),before=JSON.stringify(PM.proj.layers),count=PM.hist.list().length;
    frameComposition(PM,true);
    expect(viewportPose(PM).target.x).toBeCloseTo(11);expect(PM.sel.layers).toEqual([group.id]);
    expect(getViewportMode(PM)).toBe('editor');
    expect(PM.hist.list()).toHaveLength(count);expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(editScene(PM,{operation:'add_camera'}).ok).toBe(true);
    const locked=PM.proj.layers.find((l:any)=>l.d?.definition===LAYER3D_DEFINITIONS.camera);locked.lock=true;PM.selectLayers(group.id);
    const snapshot=JSON.stringify(PM.proj.layers),steps=PM.hist.list().length,world=compositionRuntime(PM),real=world.camera.position.clone();
    setViewportMode(PM,'camera');const start=viewportPose(PM);
    applyViewportPose(PM,orbitView(start,100,60));frameComposition(PM,true);
    expect(world.camera.position).toEqual(real);expect(viewportCamera(PM,world.camera)).not.toBe(world.camera);
    expect(JSON.stringify(PM.proj.layers)).toBe(snapshot);expect(PM.hist.list()).toHaveLength(steps);
    setViewportMode(PM,'camera');expect(viewportCamera(PM,world.camera)).toBe(world.camera);
    PM.proj=PM.mkProject();expect(getViewportMode(PM)).toBe('camera');
  });
  it('resets Free View to the current shot or default camera without changing the document',()=>{
    for(const withCamera of [false,true]){
      const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
      PM.proj=PM.mkProject();PM.time=0;
      PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
      if(withCamera){
        expect(editScene(PM,{operation:'add_camera',id:'shot',camera:{p:{x:0,y:1,z:6,targetX:2}}}).ok).toBe(true);
        expect(PM.Edit.apply({type:'replace_keyframes',target:'shot',path:'position.x',keyframes:[{time:0,value:0},{time:2,value:4}]}).ok).toBe(true);
        PM.L('shot').lock=true;
      }
      const start=viewportPose(PM);
      applyViewportPose(PM,{...orbitView(start,70,40),zoom:2});
      PM.time=2;
      const world=compositionRuntime(PM),snapshot=JSON.stringify(PM.proj),history=PM.hist.list().length;
      resetViewport(PM);
      const reset=viewportCamera(PM,world.camera);
      expect(getViewportMode(PM)).toBe('editor');expect(reset).not.toBe(world.camera);
      expect(reset.position.toArray()).toEqual(world.camera.position.toArray());
      expect(reset.quaternion.toArray()).toEqual(world.camera.quaternion.toArray());
      expect(reset.zoom).toBe(world.camera.zoom);
      if(withCamera)expect(viewportPose(PM).target.x).toBe(2);
      expect(JSON.stringify(PM.proj)).toBe(snapshot);expect(PM.hist.list()).toHaveLength(history);
    }
  });
  it('keeps editor orbit independent of animated shot cameras and resumes it per composition',()=>{
    const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
    PM.proj=PM.mkProject();PM.proj.compId='first';PM.time=0;
    PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
    expect(editScene(PM,{operation:'add_camera',camera:{p:{x:0,y:0,z:6}}}).ok).toBe(true);
    const shot=PM.proj.layers.find((l:any)=>l.d.definition===LAYER3D_DEFINITIONS.camera);
    expect(PM.Edit.apply({type:'replace_keyframes',target:shot.id,path:'position.x',keyframes:[{time:0,value:0},{time:2,value:4}]}).ok).toBe(true);
    const start=viewportPose(PM),orbit=orbitView(start,70,40),snapshot=JSON.stringify(PM.proj),count=PM.hist.list().length;
    applyViewportPose(PM,orbit);const editor=viewportCamera(PM,compositionRuntime(PM).camera);
    PM.time=2;const rendered=compositionRuntime(PM);
    expect(rendered.camera.position.x).toBeCloseTo(4);expect(editor.position.toArray()).toEqual(orbit.position.toArray());
    expect(viewportCamera(PM,rendered.camera)).toBe(editor);
    expect(JSON.stringify(PM.proj)).toBe(snapshot);expect(PM.hist.list()).toHaveLength(count);
    setViewportMode(PM,'camera');expect(viewportCamera(PM,rendered.camera).position.x).toBeCloseTo(4);
    setViewportMode(PM,'editor');expect(viewportCamera(PM,rendered.camera)).toBe(editor);
    PM.proj.compId='second';expect(getViewportMode(PM)).toBe('camera');
    PM.proj.compId='first';expect(viewportCamera(PM,rendered.camera)).toBe(editor);
  });

});
