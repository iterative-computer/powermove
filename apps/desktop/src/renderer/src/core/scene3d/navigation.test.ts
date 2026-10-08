import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {orbitView,panView,dollyView,frameView,frameComposition,alignCameraToView,lockedCamera,setActiveCamera} from './navigation';
import {viewportPose,viewportCamera,getViewportMode,setViewportMode,applyViewportPose,setViewAxis,toggleViewProjection,viewLabel,viewportState,orbitViewStep,viewPosition,VIEW_AXES,type ViewPose} from './viewport';
import {setViewportPreferences} from './editor-state';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {makePM} from '../../legacy/__tests__/make-pm';
import {LAYER3D_DEFINITIONS} from './layers';
import {toComp} from './space';

const pose=():ViewPose=>({target:new THREE.Vector3(),rotation:new THREE.Quaternion(),distance:6});
function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject();PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  return PM;
}
const cameraLayer=(PM:any)=>PM.proj.layers.find((l:any)=>l.d?.definition===LAYER3D_DEFINITIONS.camera);

describe('3D view navigation',()=>{
  it('orbits around the pivot at a constant distance, through the poles like a turntable',()=>{
    const start=pose(),next=orbitView(start,100,80);
    expect(viewPosition(next).distanceTo(next.target)).toBeCloseTo(6);expect(next.target.toArray()).toEqual([0,0,0]);
    expect(viewPosition(orbitView(start,-100,0)).x).toBeGreaterThan(0);
    const over=orbitView(start,0,400);expect(Number.isFinite(viewPosition(over).y)).toBe(true);
  });
  it('pans the pivot so the scene follows the pointer, and dollies reversibly',()=>{
    const next=panView(pose(),100,50,50,600);
    expect(next.target.x).toBeLessThan(0);expect(next.target.y).toBeGreaterThan(0);expect(next.distance).toBe(6);
    const away=dollyView(pose(),100);expect(away.distance).toBeGreaterThan(6);expect(dollyView(away,-100).distance).toBeCloseTo(6);
    expect(dollyView(pose(),-1e6).distance).toBeGreaterThanOrEqual(.01);
  });
  it('frames every corner in wide and tall views',()=>{
    const box=new THREE.Box3(new THREE.Vector3(-3,-1,-1),new THREE.Vector3(3,1,1));
    for(const aspect of [16/9,9/16]){
      const framed=frameView(pose(),box,50,aspect),c=new THREE.PerspectiveCamera(50,aspect,.01,1000);
      c.position.copy(viewPosition(framed));c.quaternion.copy(framed.rotation);c.updateMatrixWorld();
      for(const x of [-3,3])for(const y of [-1,1])for(const z of [-1,1]){
        const point=new THREE.Vector3(x,y,z).project(c);expect(Math.abs(point.x)).toBeLessThan(1);expect(Math.abs(point.y)).toBeLessThan(1);
      }
    }
  });
  it('numpad views switch to orthographic and back to perspective when orbited (Auto Perspective)',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    setViewAxis(PM,'top');
    expect(viewLabel(PM)).toBe('Top Orthographic');
    const camera=viewportCamera(PM,compositionRuntime(PM).camera);
    expect(camera).toBeInstanceOf(THREE.OrthographicCamera);
    expect(new THREE.Vector3(0,0,-1).applyQuaternion(camera.quaternion).y).toBeCloseTo(-1);
    expect(new THREE.Vector3(0,1,0).applyQuaternion(camera.quaternion).z).toBeCloseTo(-1);
    orbitViewStep(PM,15,0);expect(viewLabel(PM)).toBe('User Perspective');
    setViewAxis(PM,'front');toggleViewProjection(PM);expect(viewLabel(PM)).toBe('Front Perspective');
    toggleViewProjection(PM);applyViewportPose(PM,orbitView(viewportPose(PM),20,0));expect(viewLabel(PM)).toBe('User Orthographic');
    setViewAxis(PM,'right');orbitViewStep(PM,180,0);expect(viewportState(PM).axis).toBe('left');
    expect(new THREE.Vector3(0,0,-1).applyQuaternion(VIEW_AXES.right).x).toBeCloseTo(-1);
  });
  it('Numpad 0 toggles to the shot camera and back to the previous user view without touching the project',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_camera',camera:{p:{x:0,y:0,z:6}}}).ok).toBe(true);
    const snapshot=JSON.stringify(PM.proj.layers),count=PM.hist.list().length,world=compositionRuntime(PM);
    expect(getViewportMode(PM)).toBe('camera');expect(viewLabel(PM)).toBe('Camera Perspective');
    setViewAxis(PM,'right');const right=viewportPose(PM);
    setViewportMode(PM,'camera');expect(viewportCamera(PM,world.camera)).toBe(world.camera);
    setViewportMode(PM,'editor');expect(viewportPose(PM).rotation.angleTo(right.rotation)).toBeCloseTo(0);expect(viewLabel(PM)).toBe('Right Orthographic');
    expect(JSON.stringify(PM.proj.layers)).toBe(snapshot);expect(PM.hist.list()).toHaveLength(count);
  });
  it('keeps editor views per composition and independent of animated shot cameras',()=>{
    const PM=editor();PM.proj.compId='first';
    expect(editScene(PM,{operation:'add_camera',camera:{p:{x:960,y:540,z:-1200}}}).ok).toBe(true);
    const shot=cameraLayer(PM);
    // 800 px right of centre is 4 scene units.
    expect(PM.Edit.apply({type:'replace_keyframes',target:shot.id,path:'position.x',keyframes:[{time:0,value:960},{time:2,value:1760}]}).ok).toBe(true);
    applyViewportPose(PM,orbitView(viewportPose(PM),70,40));const editor1=viewportPose(PM);
    PM.time=2;const rendered=compositionRuntime(PM);
    expect(rendered.camera.position.x).toBeCloseTo(4);expect(viewportPose(PM).rotation.angleTo(editor1.rotation)).toBeCloseTo(0);
    PM.proj.compId='second';expect(getViewportMode(PM)).toBe('camera');
    PM.proj.compId='first';expect(getViewportMode(PM)).toBe('editor');
  });
  it('frames the selection without editing the project',()=>{
    const PM=editor();
    for(const [id,x] of [['a',2960],['b',3360]] as const)expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{x}}}).ok).toBe(true);
    const group=PM.groupLayers(['a','b']),before=JSON.stringify(PM.proj.layers),count=PM.hist.list().length;
    PM.selectLayers([group.id]);
    expect(frameComposition(PM,true)).toBe(true);
    expect(viewportPose(PM).target.x).toBeCloseTo(11);expect(getViewportMode(PM)).toBe('editor');
    expect(PM.hist.list()).toHaveLength(count);expect(JSON.stringify(PM.proj.layers)).toBe(before);
    PM.selectLayers([]);expect(frameComposition(PM,true)).toBe(false);
  });
  it('aligns the camera to the view and, with Lock Camera to View, frames by moving the camera (one undo each)',()=>{
    const PM=editor();
    expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'},p:{x:1560}}}).ok).toBe(true);
    expect(editScene(PM,{operation:'add_camera',camera:{p:{x:960,y:540,z:-1200}}}).ok).toBe(true);
    setViewAxis(PM,'right');const count=PM.hist.list().length,view=viewportPose(PM);
    alignCameraToView(PM);
    expect(PM.hist.list()).toHaveLength(count+1);expect(getViewportMode(PM)).toBe('camera');
    // The view is in scene units; the camera layer stores composition pixels.
    const camera=cameraLayer(PM),position=toComp(PM.proj,viewPosition(view)),target=toComp(PM.proj,view.target);
    expect(camera.p['position.x'].v).toBeCloseTo(position.x,1);expect(camera.d.data.camera.p.targetX.v).toBeCloseTo(target.x,1);
    expect(lockedCamera(PM)).toBeNull();setViewportPreferences(PM,{lockCamera:true});expect(lockedCamera(PM)).toBe(camera);
    PM.selectLayers(['cube']);frameComposition(PM,true);
    expect(PM.hist.list().slice(count)).toEqual(['Align camera to view','Selection','View camera']);expect(camera.d.data.camera.p.targetX.v).toBeCloseTo(1560,1);
    expect(getViewportMode(PM)).toBe('camera');
    setViewportPreferences(PM,{lockCamera:false});
  });
  it('makes a selected camera the composition camera by moving it above the others',()=>{
    const PM=editor();
    for(const z of [-1200,-1800])expect(editScene(PM,{operation:'add_camera',camera:{p:{x:960,y:540,z}}}).ok).toBe(true);
    const cameras=PM.proj.layers.filter((l:any)=>l.d?.definition===LAYER3D_DEFINITIONS.camera),second=cameras[1];
    expect(setActiveCamera(PM,second.id)).toBe(true);
    expect(PM.proj.layers.find((l:any)=>l.d?.definition===LAYER3D_DEFINITIONS.camera)).toBe(second);
    expect(()=>setActiveCamera(PM,'missing')).toThrow();
  });
});
