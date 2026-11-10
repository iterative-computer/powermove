import { afterEach, expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { viewsState,setViewLayout,setActivePane,setPaneView,paneCamera,renderPane } from './views';
import { getViewportMode } from './viewport';
import { disposeSceneRuntimes } from './service';
import * as service from './service';
import * as studio from '../studio';
const editors:any[]=[];
function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Views'});PM.time=0;editors.push(PM);return PM;
}
afterEach(()=>{for(const PM of editors.splice(0))disposeSceneRuntimes(PM);vi.restoreAllMocks();});
it('keeps a side view stationary while objects move, instead of refitting the scene every tick',()=>{
  const mesh=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshBasicMaterial());mesh.updateMatrixWorld(true);
  const world:any={camera:new THREE.PerspectiveCamera(45,16/9,.01,10000),objects:new Map([['model',mesh]])};
  const PM={L:()=>({threeD:true})},pane:any={view:'front',pose:null};
  const before=paneCamera(PM,pane,400,225,world);
  mesh.position.x=100;mesh.updateMatrixWorld(true);
  const after=paneCamera(PM,pane,400,225,world);
  expect(after.matrixWorld.elements).toEqual(before.matrixWorld.elements);
  expect(paneCamera(PM,{view:'front',pose:null},400,225,world).position.x).toBeCloseTo(100);
  mesh.geometry.dispose();(mesh.material as THREE.Material).dispose();
});
it('evaluates every playback pane at the same project-frame time and preserves exact paused time',()=>{
  const PM=editor();PM.L=()=>({threeD:true});setViewLayout(PM,4);PM.proj.fps=60;PM.time=1.019;PM.playing=true;
  const mesh=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshBasicMaterial());mesh.updateMatrixWorld(true);
  const world:any={camera:new THREE.PerspectiveCamera(45,16/9,.01,10000),objects:new Map([['model',mesh]])};
  const evaluate=vi.spyOn(service,'compositionRuntime').mockReturnValue(world),draw=vi.spyOn(studio,'studioLayer').mockReturnValue(null);
  for(let pane=0;pane<3;pane++)renderPane(PM,pane,400,225);
  expect(evaluate.mock.calls.map(call=>call[1])).toEqual([61/60,61/60,61/60]);
  expect(draw.mock.calls.map(call=>call[9])).toEqual([61/60,61/60,61/60]);
  PM.playing=false;renderPane(PM,0,400,225);expect(draw.mock.calls.at(-1)![9]).toBe(1.019);
  mesh.geometry.dispose();(mesh.material as THREE.Material).dispose();
});
const snapshot=(PM:any)=>JSON.stringify(viewsState(PM));
it('undoes/redoes layout, pane selection and view choices and reopens the same views',()=>{
  const PM=editor();
  for(const action of [()=>setViewLayout(PM,4),()=>setPaneView(PM,0,'left'),()=>setActivePane(PM,0),()=>setPaneView(PM,0,'back'),()=>setViewLayout(PM,2)]){
    const before=snapshot(PM);action();const after=snapshot(PM);
    expect(after).not.toBe(before);
    expect(PM.hist.undo()).toBe(true);expect(snapshot(PM)).toBe(before);
    expect(PM.hist.redo()).toBe(true);expect(snapshot(PM)).toBe(after);
    PM.proj=PM.deserialize(PM.serialize());PM.touch();expect(snapshot(PM)).toBe(after);
  }
});
it('keeps saved layouts per composition and defaults older projects to one camera view',()=>{
  const PM=editor(),first=PM.proj.compId;
  setViewLayout(PM,4);const saved=snapshot(PM);
  const next=PM.Comps.create({name:'Second'});expect(viewsState(PM).layout).toBe(1);
  setViewLayout(PM,2);PM.Comps.open(first);expect(snapshot(PM)).toBe(saved);
  PM.Comps.open(next);expect(viewsState(PM).layout).toBe(2);
  PM.proj=PM.mkProject();expect(viewsState(PM)).toEqual({layout:1,active:0,panes:[{view:'camera',pose:null}]});
  expect(getViewportMode(PM)).toBe('camera');
});
it('rejects invalid persisted view edits atomically',()=>{
  const PM=editor(),before=PM.serialize();
  for(const views3d of [{layout:4,active:7,panes:[]},{layout:1,active:0,panes:[{view:'custom',pose:{position:[0,0,0],target:[0,0,0],zoom:NaN}}]}]){
    expect(PM.Edit.apply({type:'set_composition',patch:{views3d}}).ok).toBe(false);
    expect(PM.serialize()).toBe(before);
  }
});
