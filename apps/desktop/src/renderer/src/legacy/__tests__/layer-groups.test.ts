import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { sanitizeProject } from '../../core/validate/project';
import { parseAgentEditCommand, ValidationError } from '../../core/validate/commands';
import { pasteLayers } from '../ui/shortcuts';
function editor() {
  const PM = makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj = PM.mkProject({dur:10}); PM.time = 2;
  const a = PM.mkLayer('shape', {name:'A',from:1,dur:4}), b = PM.mkLayer('text',{name:'B',from:2,dur:5}), c = PM.mkLayer('null',{name:'Rig'});
  a.parent=c.id; PM.proj.layers=[a,b,c]; PM.ProjectIndex.invalidate();
  return {PM,a,b,c};
}
describe('editable timeline groups',()=>{
  it('groups through the agent vocabulary, preserves source and world pose, and undoes once',()=>{
    const {PM,a,b,c}=editor(); const source=JSON.stringify([a,b,c]), pose=PM.worldMatrix(a,2);
    const cmd=parseAgentEditCommand({type:'group_layers',targets:[a.id,b.id],name:'Titles'});
    expect(cmd).not.toBeInstanceOf(ValidationError);
    expect(PM.Edit.apply(cmd,{origin:'agent'}).ok).toBe(true);
    const g=PM.firstSel(); expect(g.type).toBe('group'); expect(PM.proj.comps).toEqual({});
    expect(PM.proj.layers.filter((l:any)=>l.type!=='group').map((l:any)=>l.id)).toEqual([a.id,b.id,c.id]);
    expect(PM.worldMatrix(a,2)).toEqual(pose); expect(a.parent).toBe(c.id); expect(a.from).toBe(1);
    expect(sanitizeProject(JSON.parse(PM.serialize()).proj).layers.find(l=>l.id===a.id)?.group).toBe(g.id);
    expect(PM.hist.undo()).toBe(true); expect(JSON.stringify(PM.proj.layers)).toBe(source);
    expect(PM.hist.redo()).toBe(true); expect(PM.L(a.id).group).toBe(g.id);
  });
  it('nests, rejects cycles atomically, moves out and ungroups without rebasing clocks',()=>{
    const {PM,a,b}=editor(); const inner=PM.groupLayers([a.id],'Inner'), outer=PM.groupLayers([inner.id,b.id],'Outer');
    const before=JSON.stringify(PM.proj);
    expect(PM.Edit.apply({type:'move_to_group',targets:[outer.id],group:inner.id}).ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.Edit.apply({type:'move_to_group',targets:[a.id],group:null}).ok).toBe(true);
    expect(PM.L(a.id).group).toBeNull(); expect(PM.L(a.id).from).toBe(1);
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[outer.id]}).ok).toBe(true);
    expect(PM.L(outer.id)).toBeNull(); expect(PM.L(b.id).group).toBeNull();
  });
  it('inherits visibility and locks without overwriting individual switches',()=>{
    const {PM,a,b}=editor(); b.on=false; const g=PM.groupLayers([a.id,b.id]);
    g.on=false; expect(PM.active(a,2)).toBe(false);
    g.on=true; expect(PM.active(a,2)).toBe(true); expect(PM.active(b,2)).toBe(false);
    g.lock=true;
    expect(PM.Edit.apply({type:'set_layer',target:a.id,patch:{name:'Changed'}},{origin:'agent'}).ok).toBe(false);
    expect(PM.L(a.id).name).toBe('A');
  });
  it('duplicates the entire hierarchy with remapped membership and internal parent links',()=>{
    const {PM,a,b}=editor(); b.parent=a.id; const g=PM.groupLayers([a.id,b.id]);
    const sources=PM.proj.layers.filter((l:any)=>PM.expandGroups([g.id]).includes(l.id));
    pasteLayers(PM,()=>sources);
    const copies=PM.selLayers(), gc=copies.find((l:any)=>l.type==='group'), ac=copies.find((l:any)=>l.type==='shape'), bc=copies.find((l:any)=>l.type==='text');
    expect(ac.group).toBe(gc.id); expect(bc.group).toBe(gc.id); expect(bc.parent).toBe(ac.id); expect(ac.id).not.toBe(a.id);
  });
  it('deletes descendants together and restores the hierarchy on Undo',()=>{
    const {PM,a,b,c}=editor(); const g=PM.groupLayers([a.id,b.id]);
    expect(PM.Edit.apply({type:'delete_layers',targets:[g.id]}).ok).toBe(true);
    expect(PM.proj.layers.map((l:any)=>l.id)).toEqual([c.id]);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(a.id).group).toBe(g.id);
  });
  it('keeps nested timeline order identical to the render stack when gathering non-adjacent layers',()=>{
    const {PM,a,b,c}=editor();
    const group=PM.groupLayers([a.id,c.id]);
    expect(PM.proj.layers.map((layer:any)=>layer.id)).toEqual([group.id,a.id,c.id,b.id]);
    expect(a.parent).toBe(c.id);
    PM.moveToGroup([b.id],group.id);
    expect(PM.proj.layers.map((layer:any)=>layer.id)).toEqual([group.id,a.id,c.id,b.id]);
  });
  it('cannot delete a locked child by deleting its unlocked group',()=>{
    const {PM,a,b}=editor(),group=PM.groupLayers([a.id,b.id]); a.lock=true;
    expect(PM.Edit.apply({type:'delete_layers',targets:[group.id]},{origin:'agent'}).ok).toBe(false);
    expect(PM.L(a.id)).toBeTruthy(); expect(PM.L(group.id)).toBeTruthy();
  });

  it('edits and animates the group through agent commands without changing member channels', () => {
    const {PM,a,b}=editor(); const group=PM.groupLayers([a.id,b.id]);
    const source=JSON.stringify([a.p,b.p]), before=PM.worldMatrix(a,2);
    const x=PM.ev(group,'position.x',2);
    expect(PM.Edit.apply({type:'set_property',target:group.id,path:'position.x',value:x+80},{origin:'agent'}).ok).toBe(true);
    expect(PM.worldMatrix(a,2)[4]).toBeCloseTo(before[4]+80);
    expect(JSON.stringify([a.p,b.p])).toBe(source);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.worldMatrix(PM.L(a.id),2)[4]).toBeCloseTo(before[4]);
    expect(PM.Edit.apply([
      {type:'set_property',target:group.id,path:'position.x',value:x,time:0,mode:'keyframe'},
      {type:'set_property',target:group.id,path:'position.x',value:x+100,time:4,mode:'keyframe'},
      {type:'set_property',target:group.id,path:'opacity',value:50},
    ],{origin:'agent'}).ok).toBe(true);
    expect(PM.worldMatrix(PM.L(a.id),2)[4]).toBeCloseTo(before[4]+50);
    expect(PM.worldOpacity(PM.L(a.id),2)).toBe(.5);
    const saved=sanitizeProject(JSON.parse(PM.serialize()).proj);
    const restored=saved.layers.find(l=>l.id===group.id)!;
    expect(restored.type !== 'audio' && restored.p['position.x'].kf).toHaveLength(2);
  });
  it('applies nested scale/rotation exactly once to an internal parent rig', () => {
    const {PM,a,b,c}=editor(); a.parent=null; b.parent=a.id;
    const originalA=PM.worldMatrix(a,2), originalB=PM.worldMatrix(b,2);
    const inner=PM.groupLayers([a.id,b.id]), outer=PM.groupLayers([inner.id]);
    inner.p.rotation.v=30; inner.p['scale.x'].v=150; inner.p['scale.y'].v=75;
    outer.p.rotation.v=-15; outer.p['scale.x'].v=90; outer.p['scale.y'].v=120;
    PM.touch(); PM.beginEval(2);
    const matrix=PM.mul(PM.localMatrix(outer,2),PM.localMatrix(inner,2));
    for (const [layer, original] of [[a,originalA],[b,originalB]]) {
      const expected=PM.mul(matrix,original);
      PM.worldMatrix(layer,2).forEach((value:number,index:number)=>expect(value).toBeCloseTo(expected[index]));
    }
    // An ungrouped child still follows the visible pose of its parent.
    c.parent=b.id; PM.touch();
    const expected=PM.mul(PM.worldMatrix(b,2),PM.localMatrix(c,2));
    PM.worldMatrix(c,2).forEach((value:number,index:number)=>expect(value).toBeCloseTo(expected[index]));
  });
  it('keeps the current pose when changing membership, reparenting, or ungrouping a transformed group', () => {
    const {PM,a,b,c}=editor(); a.parent=null; b.parent=a.id;
    const group=PM.groupLayers([a.id,b.id]);
    group.p.rotation.v=35; group.p['scale.x'].v=150; group.p['scale.y'].v=80; group.p.opacity.v=60; PM.touch();
    const poseA=PM.worldMatrix(a,2), poseB=PM.worldMatrix(b,2);
    expect(PM.Edit.apply({type:'set_layer',target:a.id,patch:{parent:c.id}}).ok).toBe(true);
    PM.worldMatrix(a,2).forEach((value:number,index:number)=>expect(value).toBeCloseTo(poseA[index]));
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    for(const [layer,expected] of [[a,poseA],[b,poseB]]) {
      PM.worldMatrix(layer,2).forEach((value:number,index:number)=>expect(value).toBeCloseTo(expected[index]));
      expect(PM.worldOpacity(layer,2)).toBeCloseTo(.6);
    }
    const next=PM.groupLayers([a.id]); next.p.rotation.v=20; PM.touch();
    const pose=PM.worldMatrix(b,2);
    expect(PM.Edit.apply({type:'move_to_group',targets:[b.id],group:next.id}).ok).toBe(true);
    PM.worldMatrix(b,2).forEach((value:number,index:number)=>expect(value).toBeCloseTo(pose[index]));
  });
  it('persists a group animation clock before composition zero', () => {
    const {PM,a,b}=editor(), group=PM.groupLayers([a.id,b.id]);
    expect(PM.Edit.apply({type:'set_layer',target:group.id,patch:{from:-.5}}).ok).toBe(true);
    expect(sanitizeProject(JSON.parse(PM.serialize()).proj).layers.find(l=>l.id===group.id)?.from).toBe(-.5);
  });
  it('ungroups animation into editable frame keys without losing motion', () => {
    const {PM,a,b}=editor(), group=PM.groupLayers([a.id,b.id]);
    PM.Edit.apply([
      {type:'set_property',target:group.id,path:'rotation',value:0,time:0,mode:'keyframe'},
      {type:'set_property',target:group.id,path:'rotation',value:90,time:2,mode:'keyframe'},
      {type:'set_property',target:a.id,path:'position.x',value:300,time:1,mode:'keyframe'},
      {type:'set_property',target:a.id,path:'position.x',value:500,time:3,mode:'keyframe'},
    ]);
    const times=[0,.5,1,2,3,5];
    const poses=times.map(time=>[PM.worldMatrix(a,time),PM.worldMatrix(b,time)]);
    const before=PM.serialize();
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    times.forEach((time,i)=>[a,b].forEach((layer,j)=>PM.worldMatrix(layer,time).forEach((v:number,k:number)=>expect(v).toBeCloseTo(poses[i]![j]![k]))));
    expect(a.p.rotation.kf.length).toBeGreaterThan(2);
    expect(PM.hist.undo()).toBe(true);expect(PM.serialize()).toBe(before);
  });

  it('leaves existing animation keys intact when removing an identity group',()=>{
    const {PM,a,b}=editor();PM.Edit.apply({type:'set_property',target:a.id,path:'rotation',value:30,time:2,mode:'keyframe'});
    const group=PM.groupLayers([a.id,b.id]), source=JSON.stringify([a.p,b.p]);
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    expect(JSON.stringify([a.p,b.p])).toBe(source);
  });
  it('bakes a group scale animation that starts at zero',()=>{
    const {PM,a,b}=editor(),group=PM.groupLayers([a.id,b.id]);
    PM.Edit.apply(['scale.x','scale.y'].flatMap(path=>[
      {type:'set_property',target:group.id,path,value:0,time:0,mode:'keyframe'},
      {type:'set_property',target:group.id,path,value:100,time:2,mode:'keyframe'},
    ]));
    const times=[0,.5,1,2], poses=times.map(t=>PM.worldMatrix(a,t));
    expect(PM.Edit.apply({type:'ungroup_layers',targets:[group.id]}).ok).toBe(true);
    times.forEach((t,i)=>PM.worldMatrix(a,t).forEach((v:number,k:number)=>expect(v).toBeCloseTo(poses[i]![k])));
  });

});
