import {describe,it,expect} from 'vitest';
import {makePM} from '../__tests__/make-pm';
import {world3D,planeMatrix,projectPoint,is3DLayer,depthOrderedLayers} from './space-3d';
import {sanitizeProject} from '../../core/validate/project';
function setup(){
 const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
 PM.proj=PM.mkProject({w:640,h:360,dur:2,fps:10});PM.time=0;
 const a=PM.mkLayer('shape',{name:'A',p:{'position.x':250}}),b=PM.mkLayer('shape',{name:'B',p:{'position.x':390}});
 PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();const g=PM.groupLayers([a.id,b.id]);
 g.threeD=true;g.p['rotation.y'].v=35;g.p['rotation.x'].v=15;g.p['position.z'].v=140;PM.touch();
 return {PM,a,b,g};
}
function close(actual:number[],expected:number[]){expect(actual.length).toBe(expected.length);actual.forEach((v,i)=>expect(v).toBeCloseTo(expected[i]!,5));}
describe('2.5D groups',()=>{
 it('animates shared perspective and preserves it through ungroup and save',()=>{
  const {PM,a,g}=setup();g.p.perspective.kf=[PM.KF(0,20),PM.KF(2,200)];PM.touch();
  const times=[0,1,2],before=times.map(t=>planeMatrix(PM,a,t));
  const result=PM.Edit.apply({type:'ungroup_layers',targets:[g.id]});expect(result.ok,result.message).toBe(true);
  times.forEach((t,i)=>close(planeMatrix(PM,PM.L(a.id),t),before[i]!));
  const saved=sanitizeProject(JSON.parse(PM.serialize()).proj),child=saved.layers.find(l=>l.id===a.id)!;
  if(child.type==='audio')throw new Error('Expected a visual layer');
  expect(child.p.perspective.kf.length).toBeGreaterThan(0);
 });

 it('projects 2D children through a shared group transform without changing their switches',()=>{
  const {PM,a,b,g}=setup();expect(is3DLayer(PM,a)).toBe(true);expect(a.threeD).toBeFalsy();
  const ca=projectPoint(planeMatrix(PM,a,0),{x:0,y:0}),cb=projectPoint(planeMatrix(PM,b,0),{x:0,y:0});expect(ca.x).not.toBe(250);expect(cb.x).not.toBe(390);
  g.threeD=false;PM.touch();expect(projectPoint(planeMatrix(PM,a,0),{x:0,y:0})).toEqual({x:250,y:180});
 });
 it('preserves group 3D channels, key identities and membership through sanitization',()=>{
  const {PM,a,g}=setup();g.p['rotation.x'].kf=[PM.KF(0,0),PM.KF(1,60)];
  const saved=sanitizeProject(JSON.parse(PM.serialize()).proj),group=saved.layers.find(l=>l.id===g.id)!;
  if(group.type !== 'group')throw new Error('Expected a group');
  expect(group.threeD).toBe(true);expect(group.p['rotation.x'].kf.map(k=>({i:k.i,t:k.t,v:k.v}))).toEqual(g.p['rotation.x'].kf.map((k:any)=>({i:k.i,t:k.t,v:k.v})));expect(saved.layers.find(l=>l.id===a.id)?.group).toBe(g.id);
 });
 it('keeps coplanar group artwork in stack order under either orientation axis, including reflections',()=>{
  const {PM,a,b,g}=setup();a.p['scale.x'].v=-100;b.p['position.y'].v=260;
  for(const axis of ['orientation.x','orientation.y'])for(const angle of [-70,-30,-10,10,30,70]){
   g.p[axis].v=angle;PM.touch();
   expect(depthOrderedLayers(PM,PM.proj.layers,0).map(l=>l.id)).toEqual(PM.proj.layers.map((l:any)=>l.id));
  }
 });
 it('moves a member out of a 3D group with its pose intact and one Undo',()=>{
  const {PM,a,g}=setup(),before=world3D(PM,a,0);
  const result=PM.Edit.apply({type:'move_to_group',targets:[a.id],group:null});expect(result.ok,result.message).toBe(true);
  close(world3D(PM,PM.L(a.id),0),before);expect(PM.L(a.id).threeD).toBe(true);
  PM.hist.undo();expect(PM.L(a.id).group).toBe(g.id);expect(PM.L(a.id).threeD).toBeFalsy();close(world3D(PM,PM.L(a.id),0),before);
 });
 it('moves a 2D member into a rotated 3D group without changing its pose',()=>{
  const {PM,g}=setup();const c=PM.mkLayer('shape',{name:'Outside',p:{'position.x':100}});PM.proj.layers.push(c);PM.touch();
  const before=world3D(PM,c,0);const result=PM.Edit.apply({type:'move_to_group',targets:[c.id],group:g.id});
  expect(result.ok,result.message).toBe(true);expect(PM.L(c.id).threeD).toBe(true);close(world3D(PM,PM.L(c.id),0),before);
 });
 it('ungroups into editable 3D channels while preserving the rendered world pose',()=>{
  const {PM,a,b,g}=setup(),before=[world3D(PM,a,0),world3D(PM,b,0)];
  const result=PM.Edit.apply({type:'ungroup_layers',targets:[g.id]});expect(result.ok,result.message).toBe(true);
  close(world3D(PM,PM.L(a.id),0),before[0]!);close(world3D(PM,PM.L(b.id),0),before[1]!);expect(PM.L(g.id)).toBeNull();
 });
 it('bakes animated nested 3D groups to editable channels when ungrouped',()=>{
  const {PM,a,b,g}=setup();const outer=PM.groupLayers([g.id],'Outer');outer.threeD=true;outer.p['rotation.x'].v=-20;
  g.p['rotation.y'].kf=[PM.KF(0,0),PM.KF(2,60)];PM.touch();
  const times=[0,.5,1,1.5,2],before=times.map(t=>world3D(PM,a,t));
  const result=PM.Edit.apply({type:'ungroup_layers',targets:[g.id]});expect(result.ok,result.message).toBe(true);
  times.forEach((t,i)=>close(world3D(PM,PM.L(a.id),t),before[i]!));expect(PM.L(a.id).p['rotation.y'].kf.length).toBeGreaterThan(1);expect(PM.L(b.id).group).toBe(outer.id);
 });
});
