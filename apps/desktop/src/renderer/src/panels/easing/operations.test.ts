import { describe, expect, it } from 'vitest';
import { makePM } from '../../legacy/__tests__/make-pm';
import { applyUnitCurve, linearKeyframeEase, segmentUnitCurve, valueAtTime } from '../../core/anim/temporal-ease';
import { bounds, clone, cubic, evaluate, insertPoint, mirror, normalize, point, validateCurve } from './curve';
import { applyCurve, command, copyKeys, duplicate, expressionCommands, interpolate, loadCurve, pasteKeys, tracks, type Track } from './operations';
import { defaults, mergeLibrary, movePreset, readLibrary, validateLibrary } from './library';
import { cleanupPath, positionController } from './position';
import { importExternalPreset } from './preset-files';
import { parseEditCommand, ValidationError } from '../../core/validate/commands';
let serial = 0; const uid = () => `test-${++serial}`;
function app() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ fps: 30, dur: 12 });
  const layer = PM.mkLayer('solid', { from: 1, dur: 10 }); PM.addLayer(layer, 0);
  PM.animate(layer, 'position.x', [{ t: 0, v: 0 }, { t: 2, v: 100 }, { t: 4, v: 200 }, { t: 6, v: 250 }]);
  PM.sel.keys = layer.p['position.x'].kf.slice(1, 3).map((k: any) => k.i); PM.sel.layers = [layer.id]; PM.hist.clear();
  return { PM, layer, get: () => tracks(PM)[0]! };
}
function samples(PM: any, id: string, path = 'position.x') { const prop = PM.L(id).p[path]; return Array.from({ length: 31 }, (_, i) => PM.evalKfs(prop.kf, i / 5)); }

describe('Easing curves', () => {
  it('splits a curve without changing its evaluated shape', () => {
    const original = cubic([.8, -.4, .2, 1.5]), split = insertPoint(original, .37);
    expect(split.points).toHaveLength(3);
    for (let t = 0; t < 1; t += .01) expect(evaluate(split, t)).toBeCloseTo(evaluate(original, t), 5);
  });
  it('mirrors values, times, and tangents, and can be mirrored twice', () => {
    const curve = cubic([.7, -.2, .9, 1.8]), reversed = mirror(curve);
    for (let t = 0; t < 1; t += .05) expect(evaluate(reversed, t)).toBeCloseTo(1 - evaluate(curve, 1 - t), 5);
    expect(mirror(reversed)).toEqual(curve);
  });
  it('normalizes negative value ranges and preserves speeds', () => {
    const c = cubic([.7, .2, .4, 1.3]);
    const keys = c.points.map(p => ({ ...clone(p), t: 3 + p.t * 5, v: 90 - p.v * 40,
      inEase: { ...p.inEase, speed: p.inEase.speed * -8 }, outEase: { ...p.outEase, speed: p.outEase.speed * -8 } }));
    const normalized = normalize(keys);
    for (let t = 0; t <= 1; t += .05) expect(evaluate(normalized, t)).toBeCloseTo(evaluate(c, t), 6);
  });
  it('rejects malformed imports instead of poisoning the graph', () => {
    const bad = cubic(); bad.points[1]!.t = 0;
    expect(() => validateCurve(bad)).toThrow();
    expect(() => validateCurve({ points: Array(10000).fill(point(0,0)) })).toThrow();
  });
});

describe('native animation edits', () => {
  it('applies only the requested incoming section and preserves values, times, ids and opposite sides', () => {
    const { PM, layer, get } = app(), original = clone(get());
    const commands = applyCurve([get()], cubic([.8, 0, .2, 1]), uid, { incoming: true, middle: false, outgoing: false });
    expect(PM.Edit.apply(commands).ok).toBe(true);
    const next = PM.L(layer.id).p['position.x'].kf;
    expect(next.map((k: any) => [k.i,k.t,k.v])).toEqual(original.keys.map(k => [k.i,k.t,k.v]));
    expect(next[1].inInterp).toBe('bezier'); expect(next[1].inEase.influence).toBeCloseTo(80);
    expect(next[1].outEase).toEqual(original.keys[1]!.outEase); expect(next[2].inEase).toEqual(original.keys[2]!.inEase);
    PM.hist.undo(); expect(PM.L(layer.id).p['position.x'].kf.map((k: any) => k.inInterp)).toEqual(original.keys.map(k => k.inInterp));
  });
  it('rebuilds multi-keyframe spans, preserves outside keys and endpoint sides, and undoes exactly', () => {
    const { PM, layer, get } = app(), original = JSON.stringify(PM.proj), before = get();
    const curve = defaults().groups[0]!.presets[12]!.curve;
    const result = PM.Edit.apply(applyCurve([before], curve, uid)); expect(result.ok).toBe(true);
    const keys = PM.L(layer.id).p['position.x'].kf;
    expect(keys).toHaveLength(curve.points.length + 2);
    expect(keys[0].i).toBe(before.keys[0]!.i); expect(keys.at(-1).i).toBe(before.keys.at(-1)!.i);
    expect(keys[1].inEase).toEqual(before.keys[1]!.inEase); expect(keys.at(-2).outEase).toEqual(before.keys[2]!.outEase);
    expect(keys.some((k: any) => k.v > 200)).toBe(true);
    for (let t = 0; t <= 1; t += .025) expect(PM.evalKfs(keys,2+t*2)).toBeCloseTo(100+evaluate(curve,t)*100,4);
    PM.hist.undo(); expect(JSON.stringify(PM.proj)).toBe(original);
    PM.hist.redo(); expect(PM.L(layer.id).p['position.x'].kf).toHaveLength(curve.points.length+2);
  });
  it('keeps subframe timing and per-side speed through command validation', () => {
    const { PM, layer, get } = app(), track = get(); track.keys[1]!.t = 2.0123;
    track.keys[1]!.outInterp = 'bezier'; track.keys[1]!.outEase = {speed:83.4,influence:88};
    expect(PM.Edit.apply(command(track)).ok).toBe(true);
    expect(PM.L(layer.id).p['position.x'].kf[1]).toMatchObject({t:2.0123,outEase:{speed:83.4,influence:88}});
  });
  it('copies values without modifying interpolation and copies easing without modifying values', () => {
    const { PM, get } = app(), src = get();
    const eased = applyCurve([src], cubic([.9, 0, .1, 1]), uid)[0]!; PM.Edit.apply(eased);
    const clip = copyKeys([get()]);
    const target = get(); target.keys.forEach(k => k.v = Number(k.v)+40);
    const values = pasteKeys([target],clip,true)[0]!;
    expect(values.keyframes.map(k=>k.temporal)).toEqual(command(target).keyframes.map(k=>k.temporal));
    const pasted = pasteKeys([target],clip,false)[0]!;
    expect(pasted.keyframes.map(k=>k.value)).toEqual(target.keys.map(k=>k.v));
  });
  it('preserves custom color interpolation during value-only paste', () => {
    const { PM, layer } = app();
    const prop=PM.P('#000000');
    prop.kf=[{i:uid(),t:0,v:'#000000',eo:[.8,0],ei:[1,1],hold:false},{i:uid(),t:2,v:'#FFFFFF',eo:[0,0],ei:[.2,1],hold:false}];
    layer.d.color=prop;
    const track: Track={layerId:layer.id,from:layer.from,path:'c.color',keys:clone(prop.kf),selected:prop.kf.map((k:any)=>k.i)};
    const clipboard=copyKeys([track]); clipboard[0]!.keys[0]!.v='#FF0000';
    expect(PM.Edit.apply(pasteKeys([track],clipboard,true)).ok).toBe(true);
    const keys=PM.findProp(PM.L(layer.id),'c.color').kf;
    expect(keys[0].v).toBe('#FF0000'); expect(keys[0].eo).toEqual([.8,0]); expect(keys[1].ei).toEqual([.2,1]); expect(keys[0].hold).toBe(false);
  });
  it('preserves springs outside the edit and removes springs replaced by a curve',()=>{
    const {PM,layer,get}=app(); const keys=layer.p['position.x'].kf;
    keys[0].spring={mass:1,stiffness:100,damping:12}; keys[1].spring={mass:1,stiffness:100,damping:12};
    expect(PM.Edit.apply(applyCurve([get()],cubic(),uid)).ok).toBe(true);
    const next=PM.L(layer.id).p['position.x'].kf;expect(next[0].spring).toEqual(keys[0].spring);expect(next[1].spring).toBeUndefined();
  });
  it('mirrors and duplicates several layers relative to the global playhead', () => {
    const { get } = app(), track=get();
    const result=duplicate([track],8,uid,true)[0]!.keyframes;
    const newKeys=result.filter(k=>!track.keys.some(old=>old.i===k.id));
    expect(newKeys.map(k=>[k.time,k.value])).toEqual([[7,200],[9,100]]);
    expect(new Set(result.map(k=>k.id)).size).toBe(result.length);
  });
  it('refuses sparse multi-keyframe selections and ignores locked layers', () => {
    const { PM, layer, get }=app(); const track=get(); track.selected=[track.keys[0]!.i,track.keys[2]!.i];
    expect(()=>applyCurve([track],defaults().groups[0]!.presets[12]!.curve,uid)).toThrow('consecutive');
    PM.L(layer.id).lock=true; expect(tracks(PM)).toEqual([]);
  });
  it('rejects invalid native ease metadata and duplicate IDs atomically',()=>{
    const { PM, get }=app(), cmd=command(get()); const before=JSON.stringify(PM.proj);
    cmd.keyframes[1]!.id=cmd.keyframes[0]!.id;
    expect(PM.Edit.apply(cmd).ok).toBe(false); expect(JSON.stringify(PM.proj)).toBe(before);
    const invalid=command(get()); invalid.keyframes[0]!.temporal!.outEase.speed=Infinity;
    expect(parseEditCommand(invalid)).toBeInstanceOf(ValidationError);
  });
  it('supports continuous interpolation and editable overshoot/bounce expressions',()=>{
    const { PM, layer, get }=app();
    expect(PM.Edit.apply(interpolate([get()],'continuous')).ok).toBe(true);
    for(const kind of ['overshoot','bounce'] as const){
      expect(PM.Edit.apply(expressionCommands([get()],kind)).ok).toBe(true);
      const atEnd=PM.ev(PM.L(layer.id),'position.x',5),after=PM.ev(PM.L(layer.id),'position.x',5.1);
      expect(Number.isFinite(after)).toBe(true); expect(after).not.toBe(atEnd);
    }
  });
});

describe('library and preset files',()=>{
  it('ships standard and multi-keyframe presets, validates and round-trips the entire library',()=>{
    const l=defaults(); expect(l.groups[0]!.presets).toHaveLength(24);
    expect(validateLibrary(JSON.parse(JSON.stringify(l)))).toEqual(l);
    expect(readLibrary({getItem:()=>'{invalid'})).toEqual(l);
  });
  it('moves presets and merges libraries with fresh IDs',()=>{
    const l=defaults();l.groups.push({id:'second',name:'2',presets:[]});
    const id=l.groups[0]!.presets[0]!.id;
    const moved=movePreset(l,id,'second'); expect(moved.groups[1]!.presets[0]!.id).toBe(id);expect(l.groups[0]!.presets).toHaveLength(24);
    const merged=mergeLibrary(moved,defaults(),uid);expect(merged.groups).toHaveLength(3);expect(()=>validateLibrary(merged)).not.toThrow();
  });
  it('imports public simple and extended .kease data as native curves',()=>{
    const data={name:'Imported',color:'#FFF',presetData:[{time:0,value:0},{time:.4,value:1.4},{time:1,value:1}].map(p=>({...p,intTypeIn:6613,intTypeOut:6613,tempEaseIn:[{speed:0,influence:33}],tempEaseOut:[{speed:0,influence:33}]}))};
    const p=importExternalPreset(data,uid);expect(p.curve.points).toHaveLength(3);expect(p.color).toBe('');expect(evaluate(p.curve,.4)).toBeCloseTo(1.4,5);
    data.presetData[1]!.value=NaN;expect(()=>importExternalPreset(data,uid)).toThrow();
  });
});

describe('motion path tools',()=>{
  it('cleans timing while retaining a straight path and its endpoints',()=>{
    const {PM,layer}=app(); PM.sel.keys=PM.L(layer.id).p['position.x'].kf.map((k:any)=>k.i);
    expect(PM.Edit.apply(cleanupPath(PM)).ok).toBe(true);
    const keys=PM.L(layer.id).p['position.x'].kf;expect(keys).toHaveLength(2);expect(keys.map((k:any)=>k.v)).toEqual([0,250]);
  });
  it('creates an editable progress controller and preserves output across save/load and undo',()=>{
    const {PM,layer}=app();PM.sel.keys=PM.L(layer.id).p['position.x'].kf.map((k:any)=>k.i);
    const before=JSON.stringify(PM.proj); const commands=positionController(PM); const result=PM.Edit.apply(commands);
    expect(result.ok).toBe(true);
    const controller=PM.proj.layers.find((l:any)=>l.type==='null');expect(controller).toBeTruthy();
    expect(PM.ev(PM.L(layer.id),'position.x',1)).toBeCloseTo(0);expect(PM.ev(PM.L(layer.id),'position.x',7)).toBeCloseTo(250);
    expect(PM.ev(PM.L(layer.id),'position.x',4)).toBeCloseTo(125);
    const snapshot=JSON.parse(JSON.stringify(PM.proj));PM.proj=snapshot;PM.ProjectIndex?.invalidate();PM.touch();
    expect(PM.ev(PM.L(layer.id),'position.x',4)).toBeCloseTo(125);
    PM.hist.undo();expect(JSON.stringify(PM.proj)).toBe(before);
  });
});
