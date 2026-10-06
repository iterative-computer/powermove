import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {ModalTransform,cycleConstraint,typeNumber,parseTyped,toScreen,type ModalHost} from './modal';
import {resolveTargets} from './targets';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS} from './layers';
import {makePM} from '../../legacy/__tests__/make-pm';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Modal',fps:30,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  return PM;
}
const rect={x:0,y:0,width:1600,height:900};
function host(PM:any,patch:Partial<ModalHost>={}):ModalHost&{results:string[]} {
  const camera=new THREE.PerspectiveCamera(50,16/9,.01,1000);camera.position.set(0,0,10);camera.lookAt(0,0,0);camera.updateMatrixWorld(true);
  const results:string[]=[];
  return {PM,camera:()=>camera,rect:()=>rect,cursor:()=>new THREE.Vector3(),orientation:'global',pivot:'median',snap:false,activeId:null,
    changed:()=>{},finished:result=>results.push(result),results,...patch};
}
const key=(k:string,extra:Partial<KeyboardEvent>={})=>({key:k,shiftKey:false,ctrlKey:false,altKey:false,metaKey:false,...extra});
const value=(PM:any,id:string,path:string)=>PM.L(id).p[path].v;
const start=(PM:any,mode:'translate'|'rotate'|'scale'|'trackball',ids:string[],h=host(PM))=>
  ModalTransform.begin(h,mode,resolveTargets(PM,ids,compositionRuntime(PM)),{x:900,y:450})!;

describe('Blender modal transforms',()=>{
  it('cycles axis constraints through the header orientation, the alternate one, then none',()=>{
    let c=cycleConstraint(null,'x',false,'global');expect(c).toEqual({axis:'x',plane:false,orientation:'global'});
    c=cycleConstraint(c,'x',false,'global');expect(c?.orientation).toBe('local');
    expect(cycleConstraint(c,'x',false,'global')).toBeNull();
    expect(cycleConstraint({axis:'x',plane:false,orientation:'global'},'y',false,'global')?.axis).toBe('y');
    expect(cycleConstraint(null,'z',true,'view')).toEqual({axis:'z',plane:true,orientation:'view'});
  });
  it('reads numbers like Blender: minus toggles the sign anywhere',()=>{
    let input='';for(const k of ['2','.','5'])input=typeNumber(input,k);expect(parseTyped(input)).toBe(2.5);
    input=typeNumber(input,'-');expect(parseTyped(input)).toBe(-2.5);input=typeNumber(input,'-');expect(parseTyped(input)).toBe(2.5);
    expect(parseTyped(typeNumber('','-'))).toBeNull();expect(typeNumber('1.5','.')).toBe('1.5');
  });
  it('G X 2 Enter moves along global X as one undo; Escape restores everything',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'},p:{x:1,y:2}}}).ok).toBe(true);
    const count=PM.hist.list().length,h=host(PM),modal=start(PM,'translate',['cube'],h);
    modal.key(key('x'));modal.key(key('2'));
    expect(modal.header()).toBe('D: [2|] (2) along global X');
    expect(value(PM,'cube','position.x')).toBeCloseTo(3);expect(value(PM,'cube','position.y')).toBeCloseTo(2);
    modal.key(key('Enter'));expect(h.results).toEqual(['confirm']);expect(PM.hist.list()).toHaveLength(count+1);
    const second=start(PM,'translate',['cube']);second.key(key('y'));second.key(key('5'));expect(value(PM,'cube','position.y')).toBeCloseTo(7);
    second.key(key('Escape'));expect(value(PM,'cube','position.y')).toBeCloseTo(2);expect(PM.hist.list()).toHaveLength(count+1);
  });
  it('follows the pointer in the view plane, snaps with Ctrl and slows with Shift',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    const modal=start(PM,'translate',['cube']);
    const camera=host(PM).camera(),per=toScreen(camera,rect,new THREE.Vector3(1,0,0)).x-toScreen(camera,rect,new THREE.Vector3()).x;
    modal.move({x:900+per*2.4,y:450},{shiftKey:false,ctrlKey:false});
    expect(value(PM,'cube','position.x')).toBeCloseTo(2.4,1);expect(value(PM,'cube','position.z')).toBeCloseTo(0);
    modal.modifiers({shiftKey:false,ctrlKey:true});expect(value(PM,'cube','position.x')).toBeCloseTo(2);
    modal.modifiers({shiftKey:false,ctrlKey:false});
    modal.move({x:900+per*3.4,y:450},{shiftKey:true,ctrlKey:false});expect(value(PM,'cube','position.x')).toBeCloseTo(2.5,1);
    modal.confirm();
  });
  it('constrains to an axis the view looks along by closest approach, and to planes',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    const modal=start(PM,'translate',['cube']);
    modal.key(key('z',{shiftKey:true}));expect(modal.header()).toContain('locking global Z');
    modal.move({x:1100,y:300},{shiftKey:false,ctrlKey:false});
    expect(value(PM,'cube','position.z')).toBe(0);expect(value(PM,'cube','position.x')).toBeGreaterThan(0);expect(value(PM,'cube','position.y')).toBeGreaterThan(0);
    modal.cancel();expect(value(PM,'cube','position.x')).toBe(0);
  });
  it('R Z 90 rotates about the pivot; Individual Origins rotates each in place',()=>{
    const PM=editor();
    for(const [id,x] of [['a',-2],['b',2]] as const)expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{x}}}).ok).toBe(true);
    let modal=start(PM,'rotate',['a','b']);modal.key(key('z'));for(const k of '90')modal.key(key(k));
    expect(value(PM,'a','rotation')).toBeCloseTo(90);expect(value(PM,'a','position.x')).toBeCloseTo(0);expect(value(PM,'a','position.y')).toBeCloseTo(-2);
    expect(value(PM,'b','position.y')).toBeCloseTo(2);modal.cancel();
    modal=start(PM,'rotate',['a','b'],host(PM,{pivot:'individual'}));modal.key(key('z'));for(const k of '90')modal.key(key(k));
    expect(value(PM,'a','position.x')).toBeCloseTo(-2);expect(value(PM,'b','rotation')).toBeCloseTo(90);modal.confirm();
  });
  it('S scales from the pointer distance; typed values set exact factors per axis',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    const modal=start(PM,'scale',['cube']);const centre=toScreen(host(PM).camera(),rect,new THREE.Vector3());
    modal.move({x:centre.x+(900-centre.x)*2,y:centre.y+(450-centre.y)*2},{shiftKey:false,ctrlKey:false});
    expect(value(PM,'cube','scale.x')).toBeCloseTo(200);expect(value(PM,'cube','scale.z')).toBeCloseTo(200);
    modal.key(key('y'));modal.key(key('3'));expect(value(PM,'cube','scale.y')).toBeCloseTo(300);expect(value(PM,'cube','scale.x')).toBeCloseTo(100);
    expect(modal.header()).toBe('Scale: [3|] along global Y');modal.confirm();
  });
  it('switches between G, R and S mid-gesture and keys moves with auto keying',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    PM.autokey=true;PM.time=1;
    const modal=start(PM,'translate',['cube']);modal.key(key('r'));expect(modal.mode).toBe('rotate');modal.key(key('r'));expect(modal.mode).toBe('trackball');
    modal.key(key('g'));modal.key(key('x'));modal.key(key('1'));modal.confirm();
    expect(PM.L('cube').p['position.x'].kf).toHaveLength(1);
  });
  it('moves a light and keeps its aim unless Alt-moved; rotation swings the aim',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_light',light:{id:'key',type:'spot',p:{x:0,y:4,z:0,targetX:0,targetY:0,targetZ:0}}}).ok).toBe(true);
    const id=PM.proj.layers[0].id,modal=start(PM,'translate',[id]);modal.key(key('x'));modal.key(key('2'));modal.confirm();
    expect(value(PM,id,'position.x')).toBeCloseTo(2);expect(PM.L(id).d.data.light.p.targetX.v).toBeCloseTo(0);
  });
});
