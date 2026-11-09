import { afterEach, describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { adoptTemporalEase } from '../anim/temporal-ease';
import { LAYER3D_DEFINITIONS, compositionScene, compositionSceneWarnings } from './layers';
import { editScene } from './operations';
import { compositionRuntime, disposeSceneRuntimes } from './service';

const editors: any[] = [];
function edit(PM: any, args: any) {
  const result = editScene(PM, args, { origin: 'agent' });
  expect(result.ok, result.message).toBe(true);
  return result;
}
function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: '3D history', dur: 5, fps: 30 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  PM.assets.get = (id: string) => PM.assets.map.get(id);
  const root = new THREE.Group(), mesh = new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshStandardMaterial());
  root.userData.powermovePartsRoot = true;
  mesh.name = 'Body'; mesh.userData.name = 'Body'; mesh.material.userData.sourceMaterialIndex = 0; root.add(mesh);
  PM.assets.map.set('model-asset', { kind: 'model', object3d: root, materials: [{ index: 0, name: 'Body' }] });
  edit(PM, { operation: 'add_object', object: { id: 'model', source: { assetId: 'model-asset' }, useSourceMaterials: true, p: { x: -1 } } });
  edit(PM, { operation: 'add_object', object: { id: 'other', source: { primitive: 'box' }, p: { x: 1 } } });
  edit(PM, { operation: 'add_camera', id: 'lens' });
  edit(PM, { operation: 'add_light', light: { id: 'softbox', type: 'area' } });
  edit(PM, { operation: 'set_world', id: 'world' });
  edit(PM, { operation: 'add_terrain', id: 'ground', terrain: { p: { resolution: 2 } } });
  edit(PM, { operation: 'add_scatter', id: 'copies', scatter: { model: 'model', surface: 'ground', p: { count: 3 } } });
  edit(PM, { operation: 'add_fog', id: 'mist' });
  PM.GL.bounds = () => ({ x0: 800, x1: 1100, y0: 400, y1: 700 });
  const group = PM.groupLayers(['model', 'other']);
  editors.push(PM);
  return { PM, group: group.id };
}
function persistent(PM: any) {
  const project = JSON.parse(PM.serialize()).proj;
  delete project.revision;
  return project;
}
afterEach(() => { for (const PM of editors.splice(0)) disposeSceneRuntimes(PM); });

const actions: [string, (group: string) => any][] = [
  ['World preset and atmosphere', () => ({ operation: 'set_world', patch: { preset: 'golden-hour', p: { fogDensity: .01, fogAnisotropy: .2, fogHeightFalloff: .4, fogBaseHeight: 2, fogDistance: 50, groundColor: '#223344', metersPerUnit: 1 } } })],
  ['sun drag', () => ({ operation: 'set_world', patch: { p: { elevation: 12, azimuth: 35 } } })],
  ['World image source and visibility', () => ({ operation: 'set_world', patch: { mode: 'hdri', source: 'sky-asset', p: { visible: false, rotation: 45, intensity: 2 } } })],
  ['terrain and ground cover', () => ({ operation: 'update_terrain', target: 'ground', patch: { p: { amplitude: 3, seed: 41, detail: .1, erosion: 2, adaptive: true, blend: .5, natural: true, waterLevel: .2, snowLevel: 2 }, regions: [{ p: { color: '#779933', minSlope: 20, maxSlope: 50 } }] } })],
  ['heightmap', () => ({ operation: 'update_terrain', target: 'ground', patch: { heights: Array(9).fill(.2) } })],
  ['scatter', () => ({ operation: 'update_scatter', target: 'copies', patch: { p: { count: 6, seed: 71, density: .8, scaleMin: .5, scaleMax: 1.5, rotationMin: -10, rotationMax: 120, maxSlope: 30 } } })],
  ['fog', () => ({ operation: 'update_fog', target: 'mist', patch: { p: { shape: 'sphere', width: 4, height: 2, depth: 3, color: '#AABBCC', density: .03, anisotropy: .4 } } })],
  ['Material Options', () => ({ operation: 'set_material_options', target: 'model', patch: { castsShadows: 'only', acceptsLights: false, acceptsShadows: false, appearsInReflections: false } })],
  ['iris and focus', () => ({ operation: 'set_camera', target: 'lens', patch: { p: { fStop: 2.8, focusDistance: 4, irisShape: 6, irisRotation: 30, irisRoundness: .4, irisAspect: 1.5 } } })],
  ['render sample policy', () => ({ operation: 'set_environment', target: 'lens', patch: { samples: 128 } })],
  ['part offsets', () => ({ operation: 'set_part', target: 'model', part: '0', patch: { x: .2, ry: 30, scale: 120, visible: false } })],
  ['finish', () => ({ operation: 'apply_material', target: 'model', preset: 'polished' })],
  ['turntable', group => ({ operation: 'shot', target: group, shot: 'turntable', start: .5, duration: 3, degrees: 270 })],
  ['orbit shot', () => ({ operation: 'shot', shot: 'orbit', start: 0, duration: 3, degrees: 70 })],
  ['rack focus', () => ({ operation: 'shot', target: 'model', shot: 'rack_focus', from: 2, to: 5, duration: 2 })],
  ['push-in shot', () => ({ operation: 'shot', shot: 'push_in', amount: .3, duration: 2 })],
  ['hero tilt shot', () => ({ operation: 'shot', shot: 'hero_tilt', degrees: 20, duration: 2 })],
  ['exploded view', group => ({ operation: 'explode', target: group, amount: .5, duration: 2 })],
  ['drop to floor', () => ({ operation: 'drop_to_floor', target: 'model' })],
  ['place on model', () => ({ operation: 'place_on', target: 'model', on_target: 'other' })],
  ['light by pointing', () => ({ operation: 'light_highlight', at: [0, 1, 0], normal: [0, 1, 0], distance: 4, size: 2 })],
  ['softbox size', () => ({ operation: 'soften', target: 'softbox', amount: 1.5 })],
  ['automatic studio', () => ({ operation: 'auto_studio' })],
  ['studio set', () => ({ operation: 'set_studio', on: true })],
  ['model source replacement', () => ({ operation: 'update_object', target: 'other', patch: { source: { assetId: 'model-asset' }, useSourceMaterials: true } })],
  ['duplicate', () => ({ operation: 'duplicate', target: 'model' })],
  ['remove', () => ({ operation: 'remove', target: 'model' })],
];

describe('3D action history and durable data', () => {
  it('applies a finish to an imported model and restores its source material with Undo', () => {
    const { PM }=editor();
    const material=()=>{
      let result:any;
      compositionRuntime(PM).objects.get('model')!.traverse((node:any)=>{if(node.isMesh)result=node.material;});
      return result;
    };
    expect(material().metalness).toBe(0);
    edit(PM,{operation:'apply_material',target:'model',preset:'polished'});
    expect(material().metalness).toBe(1);
    expect(PM.L('model').d.data.object.useSourceMaterials).toBe(false);
    PM.hist.undo();expect(material().metalness).toBe(0);
    PM.hist.redo();expect(material().metalness).toBe(1);
  });
  it.each(actions)('%s undoes/redoes exactly and survives save/reopen', (_name, action) => {
    const { PM, group } = editor(), before = persistent(PM);
    edit(PM, action(group));
    const after = persistent(PM), evaluated = compositionScene(PM, 1);
    expect(after).not.toEqual(before);
    expect(PM.hist.undo()).toBe(true); expect(persistent(PM)).toEqual(before);
    expect(PM.hist.redo()).toBe(true); expect(persistent(PM)).toEqual(after);
    const reopened = PM.deserialize(PM.serialize());
    disposeSceneRuntimes(PM); PM.proj = reopened; PM.touch();
    expect(persistent(PM)).toEqual(after);
    expect(compositionScene(PM, 1)).toEqual(evaluated);
  });

  it.each([
    { operation: 'set_world', patch: { p: { elevation: NaN } } },
    { operation: 'update_terrain', target: 'ground', patch: { p: { resolution: 1e9 } } },
    { operation: 'update_scatter', target: 'copies', patch: { p: { count: 1e9 } } },
    { operation: 'set_camera', target: 'lens', patch: { p: { irisAspect: 0 } } },
    { operation: 'set_world', patch: { p: { haze: { v: 2, kf: [{ t: 1, v: 99 }] } } } },
    { operation: 'apply_material', target: 'model', p: { ior: 0 } },
  ])('rejects invalid agent data atomically %#', args => {
    const { PM } = editor(), before = PM.serialize();
    const result = editScene(PM, args as any, { origin: 'agent' });
    expect(result.ok).toBe(false); expect(result.message).toBeTruthy();
    expect(PM.serialize()).toBe(before);
  });
  it('rejects aggregate object count and scatter budgets without making a valid project unrenderable',()=>{
    const {PM}=editor(),model=PM.L('other');
    PM.proj.layers=Array.from({length:512},(_,i)=>({...structuredClone(model),id:`model_${i}`}));PM.touch();
    let before=PM.serialize();
    expect(editScene(PM,{operation:'add_object',object:{source:{primitive:'box'}}},{origin:'agent'}).ok).toBe(false);
    expect(PM.serialize()).toBe(before);expect(compositionScene(PM).objects).toHaveLength(512);
    const setup=editor().PM,scatter=setup.L('copies');
    setup.proj.layers=setup.proj.layers.filter((l:any)=>l.id!=='copies');
    for(let i=0;i<11;i++){const copy=structuredClone(scatter);copy.id=`scatter_${i}`;copy.d.data.object.scatter.p.count.v=90_000;setup.proj.layers.push(copy);}
    setup.touch();before=setup.serialize();
    expect(editScene(setup,{operation:'update_scatter',target:'scatter_0',patch:{p:{count:100_000}}},{origin:'agent'}).ok).toBe(true);
    before=setup.serialize();
    expect(editScene(setup,{operation:'update_scatter',target:'scatter_1',patch:{p:{count:100_000}}},{origin:'agent'}).ok).toBe(false);
    expect(setup.serialize()).toBe(before);expect(()=>compositionScene(setup)).not.toThrow();
  });
  it('reduces animated scatter counts proportionally at evaluation without changing saved data',()=>{
    const {PM}=editor(),template=PM.L('copies');PM.proj.layers=PM.proj.layers.filter((l:any)=>l.id!=='copies');
    for(let i=0;i<11;i++){const copy=structuredClone(template);copy.id=`scatter_${i}`;copy.d.data.object.scatter.p.count={v:90_000,kf:adoptTemporalEase([{t:0,v:90_000},{t:1,v:100_000}]),expr:null};PM.proj.layers.push(copy);}
    PM.touch();const before=PM.serialize(),evaluated=compositionScene(PM,1);
    const counts=evaluated.objects.filter(o=>o.scatter).map(o=>o.scatter!.p.count!.v);
    expect(counts.reduce((n,v)=>n+v,0)).toBeLessThanOrEqual(1_000_000);
    expect(new Set(counts).size).toBe(1);expect(compositionSceneWarnings(evaluated)[0]).toContain('reduced animated scatter');
    expect(PM.serialize()).toBe(before);
  });
  it('keeps crossed animated scatter ranges and changing heightmap resolution renderable',()=>{
    const {PM}=editor();
    edit(PM,{operation:'update_scatter',target:'copies',patch:{p:{scaleMin:{v:.8,kf:[{t:0,v:.8},{t:1,v:2}]}}}});
    edit(PM,{operation:'update_terrain',target:'ground',patch:{heights:Array.from({length:9},(_,i)=>i/8),p:{resolution:{v:2,kf:[{t:0,v:2},{t:1,v:4}]}}}});
    const before=PM.serialize(),scene=compositionScene(PM,1);
    const scatter=scene.objects.find(o=>o.scatter)!.scatter!,terrain=scene.objects.find(o=>o.terrain)!.terrain!;
    expect(scatter.p.scaleMax!.v).toBeGreaterThanOrEqual(scatter.p.scaleMin!.v);
    expect(terrain.heights).toHaveLength(25);expect(terrain.heights[12]).toBeCloseTo(.5);
    expect(PM.serialize()).toBe(before);
  });
});
