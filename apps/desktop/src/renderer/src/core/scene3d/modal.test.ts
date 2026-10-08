import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {ModalTransform,toScreen,SCENE_AXIS,type ModalHost,type ModalMode} from './modal';
import {resolveTargets} from './targets';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS} from './layers';
import {applyDefaultCamera,defaultLens} from './space';
import {makePM} from '../../legacy/__tests__/make-pm';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Drag',fps:30,dur:5,w:1600,h:900});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  return PM;
}
/** The default camera over a composition-sized rectangle: one screen pixel is one composition pixel at z=0. */
const rect={x:0,y:0,width:1600,height:900};
function host(PM:any):ModalHost&{results:string[]} {
  const camera=new THREE.PerspectiveCamera(defaultLens(PM.proj).fov,16/9,.01,1000);applyDefaultCamera(camera,PM.proj);camera.updateMatrixWorld(true);
  const results:string[]=[];
  return {PM,camera:()=>camera,rect:()=>rect,changed:()=>{},finished:result=>results.push(result),results};
}
const value=(PM:any,id:string,path:string)=>PM.L(id).p[path].v;
const plain={shiftKey:false,ctrlKey:false};
const start=(PM:any,mode:ModalMode,ids:string[],options:Parameters<typeof ModalTransform.begin>[4]={},h=host(PM),at={x:800,y:450})=>
  ModalTransform.begin(h,mode,resolveTargets(PM,ids,compositionRuntime(PM)),at,options)!;
const cube=(PM:any,id='cube',p:any={})=>expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{sx:1,sy:1,sz:1,...p}}}).ok).toBe(true);

describe('3D drags',()=>{
  it('adds models at the composition centre in composition pixels',()=>{
    const PM=editor();cube(PM);
    expect(value(PM,'cube','position.x')).toBe(800);expect(value(PM,'cube','position.y')).toBe(450);expect(value(PM,'cube','position.z')).toBe(0);
  });
  it('moves with the pointer pixel for pixel through the default camera, as one Undo',()=>{
    const PM=editor();cube(PM);
    const count=PM.hist.list().length,h=host(PM),modal=start(PM,'translate',['cube'],{},h);
    modal.move({x:900,y:500},plain);
    expect(value(PM,'cube','position.x')).toBeCloseTo(900,0);expect(value(PM,'cube','position.y')).toBeCloseTo(500,0);expect(value(PM,'cube','position.z')).toBeCloseTo(0);
    expect(modal.readout()).toBe('X 100  Y 50  Z 0');
    modal.confirm();expect(h.results).toEqual(['confirm']);expect(PM.hist.list()).toHaveLength(count+1);
  });
  it('snaps to 10 px with Ctrl and locks to the dominant axis with Shift',()=>{
    const PM=editor();cube(PM);
    const modal=start(PM,'translate',['cube']);
    modal.move({x:904,y:453},{shiftKey:false,ctrlKey:true});expect(value(PM,'cube','position.x')).toBeCloseTo(900);expect(value(PM,'cube','position.y')).toBeCloseTo(450);
    modal.move({x:904,y:470},{shiftKey:true,ctrlKey:false});expect(value(PM,'cube','position.y')).toBeCloseTo(450);expect(value(PM,'cube','position.x')).toBeCloseTo(904,0);
    modal.cancel();
  });
  it('moves along one axis from an axis handle and Escape restores the start',()=>{
    const PM=editor();cube(PM);
    const camera=host(PM).camera(),pivot=new THREE.Vector3(),tip=toScreen(camera,rect,pivot.clone().add(SCENE_AXIS.z)),base=toScreen(camera,rect,pivot);
    const modal=start(PM,'translate',['cube'],{constraint:{axis:'x'}});
    modal.move({x:950,y:450},plain);
    expect(value(PM,'cube','position.x')).toBeCloseTo(950,0);expect(value(PM,'cube','position.y')).toBe(450);expect(value(PM,'cube','position.z')).toBe(0);
    expect(modal.readout()).toBe('X 150');
    expect(modal.key({key:'Escape'})).toBe(true);expect(value(PM,'cube','position.x')).toBe(800);
    // Z points away from the viewer, straight into the screen.
    expect(Math.hypot(tip.x-base.x,tip.y-base.y)).toBeLessThan(1);
  });
  it('turns in the view like the 2D rotate handle: clockwise on screen is positive',()=>{
    const PM=editor();cube(PM);
    const modal=start(PM,'rotate',['cube'],{},host(PM),{x:900,y:450});
    modal.move({x:800+100*Math.SQRT1_2,y:450+100*Math.SQRT1_2},plain);modal.move({x:800,y:550},plain);
    expect(value(PM,'cube','rotation')).toBeCloseTo(90,0);expect(modal.readout()).toBe('90°');
    modal.confirm();
  });
  it('scales from the pivot by the pointer distance',()=>{
    const PM=editor();cube(PM);
    const modal=start(PM,'scale',['cube'],{},host(PM),{x:900,y:450});
    modal.move({x:1000,y:450},plain);
    expect(value(PM,'cube','scale.x')).toBeCloseTo(200);expect(value(PM,'cube','scale.z')).toBeCloseTo(200);expect(modal.readout()).toBe('200%');
    modal.confirm();
  });
  it('keys moves while auto keying is on',()=>{
    const PM=editor();cube(PM);PM.autokey=true;PM.time=1;
    const modal=start(PM,'translate',['cube']);modal.move({x:820,y:450},plain);modal.confirm();
    expect(PM.L('cube').p['position.x'].kf).toHaveLength(1);
  });
  it('moves a light and keeps it aimed at the same point',()=>{
    const PM=editor();
    expect(editScene(PM,{operation:'add_light',light:{id:'key',type:'spot',p:{x:800,y:0,z:-400,targetX:800,targetY:450,targetZ:0}}}).ok).toBe(true);
    const id=PM.proj.layers[0].id,modal=start(PM,'translate',[id],{constraint:{axis:'x'}});
    modal.move({x:900,y:450},plain);modal.confirm();
    expect(value(PM,id,'position.x')).toBeGreaterThan(800);expect(PM.L(id).d.data.light.p.targetX.v).toBeCloseTo(800);
  });
});
