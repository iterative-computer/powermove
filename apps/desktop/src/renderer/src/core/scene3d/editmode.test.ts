import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {toggleEditMode,editModeLayer,editSummary,selectPoints,selectAllPoints,pointsTarget,planExtrude,deleteSelectedPoints,mergeSelectedPoints,editState,canEdit} from './editmode';
import {ModalTransform,type ModalHost} from './modal';
import {runViewportOperator} from './commands';
import {pointPosition} from './editmesh';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS} from './layers';
import {makePM} from '../../legacy/__tests__/make-pm';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject();PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'},p:{x:2}}}).ok).toBe(true);
  PM.selectLayers(['cube']);return PM;
}
const camera=()=>{const c=new THREE.PerspectiveCamera(50,16/9,.01,1000);c.position.set(0,0,10);c.lookAt(0,0,0);c.updateMatrixWorld(true);return c;};
const host=(PM:any):ModalHost=>({PM,camera,rect:()=>({x:0,y:0,width:1600,height:900}),cursor:()=>new THREE.Vector3(),orientation:'global',pivot:'median',snap:false,activeId:null,changed:()=>{},finished:()=>{}});
const key=(k:string)=>({key:k,shiftKey:false,ctrlKey:false,altKey:false,metaKey:false});
const topPoints=(PM:any)=>{const s=editState(PM)!;return s.weld.points.map((_,p)=>p).filter(p=>pointPosition(s.mesh,s.weld,p).y>.4);};

describe('Edit Mode',()=>{
  it('enters for the active model without changing it, and leaves with Tab',()=>{
    const PM=editor(),before=JSON.stringify(PM.proj.layers);
    expect(toggleEditMode(PM)).toBe(true);expect(editModeLayer(PM)).toBe('cube');
    expect(editSummary(PM)).toEqual({layer:'cube',selected:0,total:8});expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(toggleEditMode(PM)).toBe(true);expect(editModeLayer(PM)).toBeNull();
  });
  it('G Z 1 moves the selected points in world space as one undo, converting the primitive to a mesh',()=>{
    const PM=editor();toggleEditMode(PM);selectPoints(PM,topPoints(PM),'set');
    const count=PM.hist.list().length,modal=ModalTransform.begin(host(PM),'translate',[pointsTarget(PM)!],{x:800,y:450})!;
    modal.key(key('y'));modal.key(key('1'));modal.key(key('Enter'));
    expect(PM.hist.list()).toHaveLength(count+1);
    const source=PM.L('cube').d.data.object.source;expect(source.mesh).toBeDefined();
    expect(Math.max(...source.mesh.positions.filter((_:number,i:number)=>i%3===1))).toBeCloseTo(1.5);
    expect(PM.L('cube').p['position.x'].v).toBe(2);
    expect(editSummary(PM)?.selected).toBe(4);
    PM.hist.undo();expect(PM.L('cube').d.data.object.source.primitive).toBe('box');
  });
  it('extrudes and moves along the normal in one undo; cancelling removes the extrusion',()=>{
    const PM=editor();toggleEditMode(PM);selectPoints(PM,topPoints(PM),'set');
    const count=PM.hist.list().length;let plan=planExtrude(PM)!;
    let modal=ModalTransform.begin(host(PM),'translate',[plan.target],{x:800,y:450},{prefix:[plan.command],constraint:{axis:'z',plane:false,orientation:'local'}})!;plan.adopt();
    modal.key(key('2'));modal.key(key('Enter'));
    expect(PM.hist.list()).toHaveLength(count+1);
    const mesh=PM.L('cube').d.data.object.source.mesh;expect(mesh.indices.length).toBe(36+24);
    expect(Math.max(...mesh.positions.filter((_:number,i:number)=>i%3===1))).toBeCloseTo(2.5);
    expect(editSummary(PM)).toMatchObject({total:12,selected:4});
    plan=planExtrude(PM)!;modal=ModalTransform.begin(host(PM),'translate',[plan.target],{x:800,y:450},{prefix:[plan.command]})!;plan.adopt();
    modal.cancel();expect(PM.L('cube').d.data.object.source.mesh.indices.length).toBe(60);expect(editSummary(PM)?.total).toBe(12);
  });
  it('deletes and merges points, and routes A, X and object operators in Edit Mode',()=>{
    const PM=editor();runViewportOperator(PM,'edit.toggle');
    expect(runViewportOperator(PM,'select.all')).toBe(true);expect(editSummary(PM)?.selected).toBe(8);
    expect(runViewportOperator(PM,'object.duplicate')).toBe(false);
    selectAllPoints(PM,'deselect');selectPoints(PM,topPoints(PM),'set');
    expect(mergeSelectedPoints(PM)).toBe(true);expect(editSummary(PM)).toMatchObject({total:5,selected:1});
    expect(deleteSelectedPoints(PM)).toBe(true);expect(editSummary(PM)?.total).toBe(4);
    expect(PM.L('cube')).toBeDefined();
  });
  it('refuses imported models and leaves Edit Mode when the model is deselected',()=>{
    const PM=editor();
    expect(canEdit(PM,{...PM.L('cube'),d:{...PM.L('cube').d,data:{object:{...PM.L('cube').d.data.object,source:{assetId:'x'}}}}})).toMatch(/Imported/);
    toggleEditMode(PM);PM.selectLayers([]);expect(editModeLayer(PM)).toBe('cube');expect(editState(PM)).toBeNull();expect(editModeLayer(PM)).toBeNull();
  });
});
