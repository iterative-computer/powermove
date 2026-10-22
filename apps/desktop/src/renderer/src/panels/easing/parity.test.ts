import { describe, expect, it } from 'vitest';
import { makePM } from '../../legacy/__tests__/make-pm';
import { cleanupPath, positionController } from './position';
import { importPresetFiles } from './preset-files';
import { evaluate } from './curve';
import { applyUnitCurve } from '../../core/anim/temporal-ease';
import { validateLibrary } from './library';
import { applyCurve, tracks } from './operations';
import { cubic } from './curve';

let serial=0; const uid=()=>`vector-${++serial}`;
const ease=(speed=0,influence=40)=>({speed,influence});
function vectorFile(values:number[][]) {
  const data={name:'Vector motion',color:'#F08040',presetData:values.map((value,i)=>({time:i/(values.length-1),value,intTypeIn:6613,intTypeOut:6613,
    tempEaseIn:value.map((_,axis)=>ease(0,30+axis*10)),tempEaseOut:value.map((_,axis)=>ease(0,50+axis*10))}))};
  return {name:'Vector.kease',size:1000,webkitRelativePath:'',text:async()=>JSON.stringify(data)} as File;
}
function curvedApp() {
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({fps:30,dur:8});
  const layer=PM.mkLayer('solid',{from:1,dur:6});PM.addLayer(layer,0);
  PM.animate(layer,'position.x',[{t:0,v:0},{t:2,v:200},{t:4,v:100}]);
  PM.animate(layer,'position.y',[{t:0,v:0},{t:2,v:100},{t:4,v:300}]);
  for(const [axis,handles] of [['x',[.75,-.3,.2,1.15]],['y',[.2,.1,.85,1.4]]] as const){
    const keys=layer.p[`position.${axis}`].kf;
    PM.Edit.apply({type:'set_easing',keyframes:keys.map((k:any)=>k.i),curve:handles});
  }
  PM.sel.layers=[layer.id];PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);PM.hist.clear();
  return {PM,layer};
}

describe('vector preset imports',()=>{
  it('imports each 3D component with independent normalized handles',async()=>{
    const library=await importPresetFiles([vectorFile([[10,60,-30],[25,20,0],[30,-20,70]])],uid);
    const presets=library.groups[0]!.presets;
    expect(presets.map(p=>p.name)).toEqual(['Vector motion · X','Vector motion · Y','Vector motion · Z']);
    expect(presets.map(p=>p.curve.points[1]!.v)).toEqual([.75,.5,.3]);
    expect(presets.map(p=>p.curve.points[0]!.outEase.influence)).toEqual([50,60,70]);
    expect(evaluate(presets[1]!.curve,.5)).toBeCloseTo(.5);
  });
  it('imports constant components without discarding moving axes',async()=>{
    const library=await importPresetFiles([vectorFile([[10,5],[20,5],[30,5]])],uid);
    expect(library.groups[0]!.presets).toHaveLength(2);
    expect(evaluate(library.groups[0]!.presets[1]!.curve,.25)).toBeCloseTo(.25);
  });
  it('handles four-component color data, shared handles, and library round trips',async()=>{
    const file=vectorFile([[0,10,20,0],[30,30,30,.5],[100,50,40,1]]);
    const data=JSON.parse(await file.text());data.name='Color motion';
    data.presetData.forEach((key:any)=>{key.tempEaseIn=[ease(0,45)];key.tempEaseOut=[ease(0,55)];});
    const library=await importPresetFiles([{...file,text:async()=>JSON.stringify(data)} as File],uid);
    expect(library.groups[0]!.presets.map(p=>p.name)).toEqual(['Color motion · R','Color motion · G','Color motion · B','Color motion · A']);
    expect(validateLibrary(JSON.parse(JSON.stringify(library)))).toEqual(library);
    expect(library.groups[0]!.presets.every(p=>p.curve.points[0]!.outEase.influence===55)).toBe(true);
  });
  it('rejects malformed vectors atomically',async()=>{
    await expect(importPresetFiles([vectorFile([[0,0],[1],[2,3]])],uid)).rejects.toThrow();
    await expect(importPresetFiles([vectorFile([[0,0],[1,Infinity],[2,3]])],uid)).rejects.toThrow();
  });
});

describe('lossless path cleanup',()=>{
  it('preserves original curved keys and handles rather than baking a polyline',()=>{
    const {PM,layer}=curvedApp();
    const original=['x','y'].map(axis=>JSON.parse(JSON.stringify(layer.p[`position.${axis}`].kf)));
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    for(const [i,axis] of ['x','y'].entries()){
      const prop=PM.L(layer.id).p[`position.${axis}`];
      expect(JSON.parse(JSON.stringify(prop.kf))).toEqual(original[i]);
      expect(prop.expr).toContain('motionPath(');
    }
  });
  it('retimes a parabola without any geometric chord error, including playback after saving',()=>{
    const {PM,layer}=curvedApp();
    PM.animate(layer,'position.x',[{t:0,v:0},{t:1,v:1}]);
    PM.animate(layer,'position.y',[{t:0,v:0},{t:1,v:1}]);
    const y=layer.p['position.y'].kf;
    y.forEach((_:unknown,i:number)=>applyUnitCurve(y,i,[1/3,0,2/3,1/3]));
    PM.sel.keys=layer.p['position.x'].kf.map((k:any)=>k.i);PM.touch();
    const count=layer.p['position.y'].kf.length;
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    expect(PM.L(layer.id).p['position.y'].kf).toHaveLength(count);
    const length=(x:number)=>x*Math.sqrt(1+4*x*x)/2+Math.asinh(2*x)/4;
    for(let i=0;i<=100;i++){
      const x=PM.ev(PM.L(layer.id),'position.x',layer.from+i/100),y=PM.ev(PM.L(layer.id),'position.y',layer.from+i/100);
      expect(y).toBeCloseTo(x*x,9);expect(length(x)/length(1)).toBeCloseTo(i/100,5);
    }
    expect(PM.expressionErrors.get(PM.L(layer.id).p['position.x'])).toBeUndefined();
  });
  it('preserves animation outside a cleaned subrange and permits repeated cleanup',()=>{
    const {PM,layer}=curvedApp();
    PM.sel.keys=layer.p['position.x'].kf.slice(1).map((k:any)=>k.i);
    const before=['x','y'].map(axis=>PM.ev(layer,`position.${axis}`,layer.from+.75));
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    expect(['x','y'].map(axis=>PM.ev(PM.L(layer.id),`position.${axis}`,layer.from+.75))).toEqual(before);
    const once=['x','y'].map(axis=>PM.ev(PM.L(layer.id),`position.${axis}`,layer.from+3));
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    expect(['x','y'].map(axis=>PM.ev(PM.L(layer.id),`position.${axis}`,layer.from+3))).toEqual(once);
  });
  it('allows new easing on the progress keys without changing the route geometry',()=>{
    const {PM,layer}=curvedApp(), geometry=JSON.stringify(layer.p['position.y'].kf);
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    const controller=PM.proj.layers.find((l:any)=>l.d?.easingPath?.layerId===layer.id);
    PM.sel.keys=controller.p.opacity.kf.map((k:any)=>k.i);
    const before=PM.ev(PM.L(layer.id),'position.x',layer.from+2);
    expect(PM.Edit.apply(applyCurve(tracks(PM),cubic([.7,0,1,1]),uid)).ok).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).p['position.y'].kf)).toBe(geometry);
    expect(PM.ev(PM.L(layer.id),'position.x',layer.from+2)).not.toBeCloseTo(before,3);
    expect(PM.expressionErrors.get(PM.L(layer.id).p['position.x'])).toBeUndefined();
  });
  it('keeps curves after controller creation, serialization, undo and redo',()=>{
    const {PM,layer}=curvedApp(), before=JSON.stringify(PM.proj);
    const original=JSON.stringify(layer.p['position.x'].kf);
    expect(PM.Edit.apply(positionController(PM)).ok).toBe(true);
    expect(JSON.stringify(PM.L(layer.id).p['position.x'].kf)).toBe(original);
    const positions=Array.from({length:21},(_,i)=>['x','y'].map(axis=>PM.ev(PM.L(layer.id),`position.${axis}`,1+i/5)));
    PM.proj=JSON.parse(JSON.stringify(PM.proj));PM.ProjectIndex.invalidate();PM.touch();
    expect(Array.from({length:21},(_,i)=>['x','y'].map(axis=>PM.ev(PM.L(layer.id),`position.${axis}`,1+i/5)))).toEqual(positions);
    PM.hist.undo();expect(JSON.stringify(PM.proj)).toBe(before);PM.hist.redo();
    expect(JSON.stringify(PM.L(layer.id).p['position.x'].kf)).toBe(original);
  });
});
