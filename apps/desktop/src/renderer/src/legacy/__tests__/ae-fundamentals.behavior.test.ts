import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, dur: 6, fps: 30 }); PM.time = 1;
  return PM;
}
describe('AE fundamentals', () => {
  it('reparents rotated, scaled animated layers without a pose jump and undoes once', () => {
    const PM = editor();
    const child = PM.mkLayer('shape', { p: { 'position.x': 150, 'position.y': 100, rotation: 20 } });
    const parent = PM.mkLayer('null', { p: { 'position.x': 200, 'position.y': 80, rotation: 45, 'scale.x': 150, 'scale.y': 70, opacity: 50 } });
    PM.proj.layers = [child, parent]; PM.ProjectIndex.invalidate();
    PM.setKeyOn(child.p['position.x'], 0, 100, 'linear', 30); PM.setKeyOn(child.p['position.x'], 2, 200, 'linear', 30);
    const before = PM.worldMatrix(child, PM.time);
    expect(PM.Edit.apply({ type: 'set_layer', target: child.id, patch: { parent: parent.id } }).ok).toBe(true);
    PM.worldMatrix(child, PM.time).forEach((v: number, i: number) => expect(v).toBeCloseTo(before[i], 7));
    expect(child.p['position.x'].kf).toHaveLength(2);
    expect(PM.worldOpacity(child, 1)).toBe(1);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(child.id).parent).toBeNull();
    expect(PM.L(child.id).p['position.x'].kf.map((k: any) => k.v)).toEqual([100,200]);
  });
  it('rejects singular parenting atomically', () => {
    const PM = editor(), child = PM.mkLayer('shape'), parent = PM.mkLayer('null', { p: { 'scale.x': 0 } });
    PM.proj.layers = [child,parent]; PM.ProjectIndex.invalidate();
    expect(PM.Edit.apply({ type: 'set_layer', target: child.id, patch: { parent: parent.id } }).ok).toBe(false);
    expect(PM.L(child.id).parent).toBeNull();
  });
  it('rejects invalid and unknown expressions before modifying the property', () => {
    const PM = editor(), l = PM.mkLayer('shape'); PM.proj.layers = [l]; PM.ProjectIndex.invalidate();
    for (const expression of ['value +', 'unknownName * 2']) {
      expect(PM.Edit.apply({ type: 'set_expression', target: l.id, path: 'position.x', expression }).ok).toBe(false);
      expect(PM.L(l.id).p['position.x'].expr).toBeNull();
    }
  });
});

import { inspectorPM } from '../../../../extensions/inspector/multi-edit';
import { animateSelection } from '../../../../extensions/inspector/selection-animation';
import { makeVectorPath,makeVertex,pathValues } from '../core/vector-paths';
import { enableTimeRemap,sourceTime } from '../core/retiming';
import { animatedGlyphs,animatorWeight } from '../core/text-animation';
import { temporalKeys } from '../core/temporal-bridge';

describe('editable professional workflows',()=>{
  it('preserves expression-driven parenting without applying compensation twice',()=>{const PM=editor(),l=PM.mkLayer('shape'),parent=PM.mkLayer('null',{p:{'position.x':100,'scale.x':200}});PM.proj.layers=[l,parent];PM.ProjectIndex.invalidate();l.p['position.x'].v=20;l.p['position.x'].expr='value * 2';PM.touch();const before=PM.worldMatrix(l,1);expect(PM.Edit.apply({type:'set_layer',target:l.id,patch:{parent:parent.id}}).ok).toBe(true);PM.worldMatrix(l,1).forEach((v:number,i:number)=>expect(v).toBeCloseTo(before[i],6));});
  it('multi-edits absolute values and scrub offsets while preserving locked layers and one Undo',()=>{const PM=editor(),a=PM.mkLayer('shape',{p:{'position.x':10}}),b=PM.mkLayer('shape',{p:{'position.x':40}}),locked=PM.mkLayer('shape',{p:{'position.x':90}});locked.lock=true;PM.proj.layers=[a,b,locked];PM.ProjectIndex.invalidate();PM.selectLayers([a.id,b.id,locked.id]);const I=inspectorPM(PM);I.Edit.begin('Scrub',{origin:'inspector'});I.Edit.dispatch({type:'set_property',target:a.id,path:'position.x',value:20,preserveHandEdits:false});I.Edit.dispatch({type:'set_property',target:a.id,path:'position.x',value:30,preserveHandEdits:false});I.Edit.commit();expect(PM.proj.layers.map((l:any)=>l.p['position.x'].v)).toEqual([30,60,90]);PM.hist.undo();expect(PM.proj.layers.map((l:any)=>l.p['position.x'].v)).toEqual([10,40,90]);I.Edit.apply({type:'set_property',target:a.id,path:'position.x',value:100,preserveHandEdits:false});expect(PM.proj.layers.map((l:any)=>l.p['position.x'].v)).toEqual([100,100,90]);});
  it('adds selected animation without collapsing distinct layer values',()=>{const PM=editor(),a=PM.mkLayer('shape',{p:{'position.x':10}}),b=PM.mkLayer('shape',{p:{'position.x':40}});PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();PM.selectLayers([a.id,b.id]);animateSelection(inspectorPM(PM),a,['position.x'],false,1,'Animate');expect(a.p['position.x'].kf[0].v).toBe(10);expect(b.p['position.x'].kf[0].v).toBe(40);PM.hist.undo();expect(PM.proj.layers.every((l:any)=>!l.p['position.x'].kf.length)).toBe(true);});
  it('keeps native temporal speed on flat-value segments through serialization',()=>{const PM=editor(),l=PM.mkLayer('shape');PM.proj.layers=[l];PM.ProjectIndex.invalidate();const p=l.p['position.x'];p.kf=[{i:'a',t:0,v:10,inInterp:'linear',outInterp:'bezier',inEase:{speed:0,influence:33},outEase:{speed:100,influence:40}},{i:'b',t:1,v:10,inInterp:'bezier',outInterp:'linear',outEase:{speed:0,influence:33},inEase:{speed:-100,influence:40}}];const value=PM.evP(l,p,.5,'position.x');expect(value).toBeGreaterThan(10);p.kf=JSON.parse(JSON.stringify(p.kf));PM.touch();expect(PM.evP(l,p,.5,'position.x')).toBeCloseTo(value,6);expect(JSON.stringify(p.kf)).not.toContain('"eo"');});
  it('shows mixed content and scrubs selected shape sizes relatively',()=>{const PM=editor(),a=PM.mkLayer('shape',{d:{w:60,color:'#ff0000'}}),b=PM.mkLayer('shape',{d:{w:120,color:'#0000ff'}});PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();PM.sel.layers=[a.id,b.id];const proxy=inspectorPM(PM),binding={mode:'command',command:(value:any)=>({type:'set_content',target:a.id,patch:{color:value}})};expect(proxy.inspectorMixed(binding,'#ff0000')).toBe(true);proxy.Edit.begin('Resize selection');proxy.Edit.dispatch({type:'set_content',target:a.id,patch:{w:70}});proxy.Edit.commit();expect(PM.resolveContent(a,1).w).toBe(70);expect(PM.resolveContent(b,1).w).toBe(130);PM.hist.undo();expect(PM.resolveContent(PM.L(b.id),1).w).toBe(120);});
  it('edits easing after a previously unvisited native track has played',()=>{const PM=editor(),l=PM.mkLayer('shape');l.p.opacity.kf=[PM.KF(0,0),PM.KF(1,100)];PM.proj.layers=[l];PM.ProjectIndex.invalidate();expect(PM.ev(l,'opacity',.5)).toBeCloseTo(50);const keys=l.p.opacity.kf;expect(PM.Edit.apply({type:'set_easing',keyframes:keys.map((k:any)=>k.i),curve:[.2,.8,.3,1]}).ok).toBe(true);expect(PM.ev(l,'opacity',.5)).toBeGreaterThan(80);PM.hist.undo();expect(PM.ev(PM.L(l.id),'opacity',.5)).toBeCloseTo(50);});
  it('animates path vertices and retains keyframes in editable serialized data',()=>{const PM=editor(),l=PM.mkLayer('shape'),path=makeVectorPath(PM),vertex=makeVertex(PM,10,20);path.vertices.push(vertex);l.d.paths=[path];PM.proj.layers=[l];PM.ProjectIndex.invalidate();const channel=`g.${path.id}.v.${vertex.id}.x`;expect(PM.Edit.apply({type:'set_property',target:l.id,path:channel,value:50,time:2,mode:'keyframe',preserveHandEdits:false}).ok).toBe(true);expect(PM.allProps(l).some((p:any)=>p.key===channel)).toBe(true);expect(pathValues(PM,l,path,2).vertices[0].x).toBe(50);const saved=JSON.parse(JSON.stringify(l));expect(saved.d.paths[0].vertices[0].p.x.kf).toHaveLength(1);PM.hist.undo();expect(PM.L(l.id).d.paths[0].vertices[0].p.x.v).toBe(10);});
  it('rejects matte cycles and exposes an animated matte mode',()=>{const PM=editor(),a=PM.mkLayer('shape'),b=PM.mkLayer('shape');PM.proj.layers=[a,b];PM.ProjectIndex.invalidate();expect(PM.Edit.apply({type:'set_layer',target:a.id,patch:{matteSource:b.id}}).ok).toBe(true);expect(PM.Edit.apply({type:'set_layer',target:b.id,patch:{matteSource:a.id}}).ok).toBe(false);expect(PM.findProp(PM.L(a.id),'l.matteMode').v).toBe('alpha');});
  it('integrates animated speed and supports editable reverse/freeze with Undo',()=>{const PM=editor(),l=PM.mkLayer('video');l.from=0;l.d.trim=PM.P(2);l.d.speed=PM.P(1);l.d.speed.kf=[PM.KF(0,1),PM.KF(2,3)];PM.proj.layers=[l];PM.ProjectIndex.invalidate();PM.touch();expect(sourceTime(PM,l,2)).toBeCloseTo(6,3);PM.Kernel.services.register('inspector',{refresh:()=>{}});enableTimeRemap(PM,l,'freeze');const frozen=sourceTime(PM,l,1);expect(sourceTime(PM,l,3)).toBe(frozen);expect(l.d.sourceTime.kf).toHaveLength(1);PM.hist.undo();expect(PM.L(l.id).d.timeRemap).toBeUndefined();});
  it('keeps live text animator selectors independent of source text',()=>{expect(animatorWeight({start:0,end:50,offset:0,smoothness:0},0,4)).toBe(1);expect(animatorWeight({start:0,end:50,offset:0,smoothness:0},3,4)).toBe(0);const PM=editor(),l=PM.mkLayer('text');l.d.animators=[{id:'a',p:{unit:PM.P('words'),start:PM.P(0),end:PM.P(50),x:PM.P(20),scale:PM.P(100),opacity:PM.P(100)}}];const layout={characters:[{text:'A',x:0,y:0,index:0,line:0,word:0,sourceStart:0},{text:'B',x:20,y:0,index:1,line:0,word:1,sourceStart:2}],words:[{},{}],lines:[{}]};const result=animatedGlyphs(PM,l,1,{size:20,color:'white'},layout);expect(result.map((g:any)=>g.x)).toEqual([20,20]);expect(l.d.animators).toHaveLength(1);});
});
