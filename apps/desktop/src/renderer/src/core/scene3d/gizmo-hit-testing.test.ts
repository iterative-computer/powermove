// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createSceneGizmo, setGizmoMode, setGizmoSpace, type SceneGizmo } from './gizmo';
import { compositionRuntime } from './service';
import { editScene } from './operations';
import { LAYER3D_DEFINITIONS } from './layers';
import { viewportCamera } from './viewport';
import { Vector3, Matrix4 } from 'three';
import { readFileSync } from 'node:fs';
import { installExtrusionFont } from './extrusion-fonts';
import { nativeSceneBridge, nativeTransformValues } from './native-transform';
import { world3D } from '../../legacy/core/space-3d';
import { makePM } from '../../legacy/__tests__/make-pm';

// Keep Three's real raycasts and TransformControls; only GPU painting is fake.
vi.mock('three', async () => {
  const actual = await vi.importActual<typeof import('three')>('three');
  return { ...actual, WebGLRenderer: class {
    domElement: HTMLCanvasElement;
    constructor(options: { canvas?: HTMLCanvasElement }) { this.domElement = options.canvas ?? document.createElement('canvas'); }
    setPixelRatio() {} setSize() {} setClearColor() {} clear() {} dispose() {} forceContextLoss() {}
    render(scene: import('three').Scene, camera: import('three').Camera) { camera.updateMatrixWorld(true); scene.updateMatrixWorld(true); }
  } };
});

const controllers: SceneGizmo[] = [];
beforeEach(() => { setGizmoMode('translate'); setGizmoSpace('world'); });
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.dispose());
  document.body.replaceChildren(); vi.restoreAllMocks();
});

function fixture(x = 0, cameraPosition = { x: 0, y: 0, z: 6 }) {
  const frames = new Map<number, FrameRequestCallback>();
  let frameId = 0;
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { frames.delete(id); });
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 320, h: 180, dur: 2 }); PM.time = 0;
  PM.layerDefinition = (id: string) => Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? { id, label: '3D', version: 1, params: [], defaults: {}, renderer: { kind: 'layer3d' } } : null;
  expect(editScene(PM, { operation: 'add_object', object: { id: 'model', p: { x }, source: { primitive: 'box' } } }).ok).toBe(true);
  expect(editScene(PM, { operation: 'add_camera', camera: { p: cameraPosition } }).ok).toBe(true);
  const stage = document.createElement('div'); document.body.append(stage);
  Object.defineProperties(stage, { clientWidth: { value: 320 }, clientHeight: { value: 180 } });
  vi.spyOn(stage, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 320, 180));
  const controller = createSceneGizmo(PM, stage, () => compositionRuntime(PM)); controllers.push(controller);
  controller.update({ x: 0, y: 0, width: 320, height: 180 }, ['model']);
  const paint = () => { const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0)); };
  const handle = (axis: string) => {
    const camera = viewportCamera(PM, compositionRuntime(PM)!.camera).clone();
    if ('aspect' in camera) camera.aspect = 320 / 180;
    camera.updateProjectionMatrix(); camera.updateMatrixWorld(true);
    const center = new Vector3(x, 0, 0).project(camera);
    for (let y = (1 - center.y) * 90; y < 175; y += 2) for (let px = (center.x + 1) * 160 + 20; px < 315; px += 2) {
      if (controller.hover(px, y) && stage.querySelector<HTMLElement>('[data-scene-gizmo]')!.dataset.gizmoAxis === axis) return { x: px, y };
    }
    throw new Error(`No ${axis} handle`);
  };
  return { PM, controller, paint, handle };
}

it.each([
  { x: 0, painted: false },
  { x: 1.5, painted: false },
  { x: 1.5, painted: true },
  { x: 1.5, painted: true, camera: { x: 3, y: 2, z: 6 } }
])('scales with real TransformControls before the next paint ($x / painted: $painted / camera: $camera)', ({ x, painted, camera }) => {
  const f = fixture(x, camera);
  if (painted) { f.paint(); setGizmoMode('rotate'); f.paint(); }
  setGizmoMode('scale');
  const point = f.handle('X');
  expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y }))).toBe(true);
  window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y }));
  window.dispatchEvent(new PointerEvent('pointerup', { button: 0, clientX: point.x + 25, clientY: point.y }));
  expect(f.PM.L('model').p['scale.x'].v).toBeGreaterThan(100);
  expect(f.PM.L('model').p['scale.y'].v).toBe(100);
  expect(f.PM.L('model').p['scale.z'].v).toBe(100);
  expect(f.PM.L('model').p['position.x'].v).toBe(x);
  const scaled = f.PM.L('model').p['scale.x'].v;
  expect(f.PM.hist.undo()).toBe(true);
  expect(f.PM.L('model').p['scale.x'].v).toBe(100);
  expect(f.PM.hist.redo()).toBe(true);
  expect(f.PM.L('model').p['scale.x'].v).toBe(scaled);
});

it('uses the picked-axis drag plane without waiting for the hover highlight to paint', () => {
  const drag = (paintHover: boolean) => {
    const f = fixture(1.5, { x: 3, y: 2, z: 6 });
    f.paint(); setGizmoMode('scale');
    const point = f.handle('X');
    if (paintHover) f.paint();
    expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y }))).toBe(true);
    window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y }));
    window.dispatchEvent(new PointerEvent('pointerup', { button: 0, clientX: point.x + 25, clientY: point.y }));
    const value = f.PM.L('model').p['scale.x'].v;
    f.controller.dispose();
    return value;
  };
  expect(drag(false)).toBeCloseTo(drag(true), 6);
});

it('preserves snapping and Escape cancellation when a scale gesture starts before paint', () => {
  const f = fixture(1.5);
  const history = f.PM.hist.list().length;
  setGizmoMode('scale');
  const point = f.handle('X');
  expect(f.controller.pointerDown(new PointerEvent('pointerdown', { button: 0, clientX: point.x, clientY: point.y, ctrlKey: true }))).toBe(true);
  window.dispatchEvent(new PointerEvent('pointermove', { clientX: point.x + 25, clientY: point.y, ctrlKey: true }));
  const scaled = f.PM.L('model').p['scale.x'].v;
  expect(scaled).toBeGreaterThan(100);
  expect(scaled / 10).toBeCloseTo(Math.round(scaled / 10), 6);
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
  expect(f.controller.active()).toBe(false);
  expect(f.PM.L('model').p['scale.x'].v).toBe(100);
  expect(f.PM.hist.list().length).toBe(history);
});

function artworkFixture(kind: 'shape'|'text'|'solid' = 'shape', parented = false) {
  const f = fixture(0, { x: 3, y: 2, z: 8 }), PM = f.PM;
  expect(PM.Edit.apply({type:'add_layer',id:'art',layerType:kind,
    content:kind==='text'?{text:'3D',font:'Inter',size:40}:{w:80,h:55,color:'#FF6600'},
    properties:{'position.x':160,'position.y':90,'anchor.x':13,'anchor.y':9,'anchor.z':4,
      'rotation.x':12,'rotation.y':-18,'rotation':7,'orientation.x':9,'orientation.y':4,'orientation.z':-6}}).ok).toBe(true);
  if(kind==='text'){
    const bytes=readFileSync('assets/fonts/Inter-Variable.ttf');
    installExtrusionFont(PM,PM.L('art').d,bytes.buffer.slice(bytes.byteOffset,bytes.byteOffset+bytes.byteLength));
  }
  if(kind==='solid') {
    PM.Edit.apply({type:'add_layer',id:'support',layerType:'shape',content:{w:1,h:1}});
    expect(editScene(PM,{operation:'set_extrusion',target:'support',patch:{enabled:true,depth:1}}).ok).toBe(true);
    PM.Edit.apply([{type:'set_layer',target:'art',patch:{threeD:true}},{type:'set_property',target:'art',path:'position.x',value:160},{type:'set_property',target:'art',path:'position.y',value:90}]);
    PM.GL={bounds:()=>({x0:0,y0:0,x1:80,y1:55,w:80,h:55}),renderMaterialComposition:()=>new Uint8Array(80*55*4).fill(255)};
  } else expect(editScene(PM,{operation:'set_extrusion',target:'art',patch:{enabled:true,depth:20}}).ok).toBe(true);
  if(parented){
    expect(PM.Edit.apply({type:'add_layer',id:'parent',layerType:'null',properties:{'position.x':20,'position.y':10,'rotation':25,'scale.x':120,'scale.y':120,'scale.z':120}}).ok).toBe(true);
    PM.L('parent').threeD=true;PM.L('art').parent='parent';
  }
  PM.touch();f.controller.update({x:0,y:0,width:320,height:180},['art']);f.paint();
  const scenePosition=()=>new Vector3(...['x','y','z'].map(a=>PM.ev(PM.L('art'),`anchor.${a}`,PM.time)) as [number,number,number])
    .applyMatrix4(nativeSceneBridge(PM.proj).multiply(new Matrix4().fromArray(world3D(PM,PM.L('art'),PM.time))));
  const handle=(axis:string)=>{
    for(let y=4;y<178;y+=2)for(let x=4;x<318;x+=2){
      const canvases=document.querySelectorAll<HTMLElement>('[data-scene-gizmo]');
      if(f.controller.hover(x,y) && canvases[canvases.length-1]!.dataset.gizmoAxis===axis)return {x,y};
    }
    throw new Error(`No native ${axis} handle`);
  };
  const drag=(axis:string,options:PointerEventInit={})=>{
    const point=handle(axis);
    expect(f.controller.pointerDown(new PointerEvent('pointerdown',{button:0,clientX:point.x,clientY:point.y,...options}))).toBe(true);
    window.dispatchEvent(new PointerEvent('pointermove',{clientX:point.x+17,clientY:point.y-13,...options}));
    return ()=>window.dispatchEvent(new PointerEvent('pointerup',{button:0,clientX:point.x+17,clientY:point.y-13,...options}));
  };
  return {...f,scenePosition,drag};
}

it.each(['shape','text','solid'] as const)('moves native %s along each exact world axis with parents and one undo per drag',kind=>{
  const f=artworkFixture(kind,true);
  for(const axis of ['X','Y','Z']){
    const before=f.scenePosition(),history=f.PM.hist.mark().index;
    const finish=f.drag(axis),mid=f.scenePosition();
    expect(mid.distanceTo(before)).toBeGreaterThan(.01);
    for(const component of ['x','y','z'] as const)if(component!==axis.toLowerCase())expect(mid[component]).toBeCloseTo(before[component],7);
    finish();expect(f.PM.hist.mark().index).toBe(history+1);
    expect(f.PM.hist.undo()).toBe(true);expect(f.scenePosition().distanceTo(before)).toBeLessThan(1e-7);
    f.controller.update({x:0,y:0,width:320,height:180},['art']);f.paint();
  }
});

it.each((['rotate','scale'] as const).flatMap(mode=>(['X','Y','Z'] as const).map(axis=>({mode,axis}))))('native $mode $axis keeps its anchor fixed and orientation intact',({mode,axis})=>{
  const f=artworkFixture('shape',true),before=f.scenePosition(),orientation=f.PM.L('art').p['orientation.x'].v;
  setGizmoMode(mode);setGizmoSpace('local');f.paint();
  const snapshot=f.PM.serialize(),finish=f.drag(axis);
  expect(f.scenePosition().distanceTo(before)).toBeLessThan(1e-7);
  expect(f.PM.L('art').p['orientation.x'].v).toBe(orientation);
  expect(f.PM.serialize()).not.toBe(snapshot);finish();
  expect(f.PM.hist.undo()).toBe(true);expect(f.PM.serialize()).toBe(snapshot);
});

it('native animation writes at the current time and shares Ctrl snap, Shift precision and Escape behavior',()=>{
  const f=artworkFixture();
  f.PM.Edit.apply({type:'replace_keyframes',target:'art',path:'position.x',keyframes:[{time:0,value:160},{time:2,value:200}]});
  f.PM.time=1;f.controller.update({x:0,y:0,width:320,height:180},['art']);
  const before=f.scenePosition(),finish=f.drag('X',{ctrlKey:true}),mid=f.scenePosition();finish();
  expect(mid.x).toBeCloseTo(Math.round(mid.x),7);
  expect(f.PM.L('art').p['position.x'].kf.map((k:any)=>k.t)).toContain(1);
  expect(f.PM.hist.undo()).toBe(true);expect(f.scenePosition().distanceTo(before)).toBeLessThan(1e-7);
  f.controller.update({x:0,y:0,width:320,height:180},['art']);
  const normalFinish=f.drag('X'),normal=f.scenePosition();normalFinish();f.PM.hist.undo();
  f.controller.update({x:0,y:0,width:320,height:180},['art']);
  f.drag('X',{shiftKey:true});const fine=f.scenePosition();
  expect(fine.distanceTo(before)).toBeCloseTo(normal.distanceTo(before)*.1,6);
  window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}));
  expect(f.scenePosition().distanceTo(before)).toBeLessThan(1e-7);
});

it('keeps ordinary and mixed 2D selections available to the viewer',()=>{
  const f=artworkFixture();
  f.PM.Edit.apply({type:'add_layer',id:'flat',layerType:'shape',content:{w:50,h:50}});
  expect(f.controller.covers(['art'])).toBe(true);
  expect(f.controller.covers(['flat'])).toBe(false);
  expect(f.controller.covers(['art','flat'])).toBe(false);
  f.controller.update({x:0,y:0,width:320,height:180},['art','flat']);
  expect(f.controller.hover(160,90)).toBe(false);
});

it.each([0,18,-23])('round-trips native orientation, anchor, scale and %s degree skew without changing the pose',skew=>{
  const f=artworkFixture();
  f.PM.Edit.apply([{type:'set_property',target:'art',path:'skew',value:skew},
    {type:'set_property',target:'art',path:'scale.x',value:140},
    {type:'set_property',target:'art',path:'scale.y',value:75}]);
  const local=new Matrix4().fromArray(world3D(f.PM,f.PM.L('art'),0)),values=nativeTransformValues(f.PM,f.PM.L('art'),local,0);
  expect(Object.keys(values).some(key=>key.startsWith('orientation.'))).toBe(false);
  const result=f.PM.Edit.apply(Object.entries(values).map(([path,value])=>({type:'set_property',target:'art',path,value})));
  expect(result.ok).toBe(true);
  const after=world3D(f.PM,f.PM.L('art'),0);
  after.forEach((value:number,index:number)=>expect(value).toBeCloseTo(local.elements[index]!,7));
});
it('refuses unrepresentable depth shear instead of silently deforming artwork',()=>{
  const f=artworkFixture(),matrix=new Matrix4();matrix.elements[8]=.3;
  expect(()=>nativeTransformValues(f.PM,f.PM.L('art'),matrix,0)).toThrow(/depth shear/);
});

it('scales tilted native artwork from the default Global setting without introducing depth shear',()=>{
  const f=artworkFixture('shape',true),before=f.scenePosition();
  setGizmoMode('scale');setGizmoSpace('world');f.paint();
  const finish=f.drag('X');
  expect(f.PM.L('art').p['scale.x'].v).not.toBe(100);
  expect(f.PM.L('art').p['scale.y'].v).toBeCloseTo(100,7);
  expect(f.PM.L('art').p['scale.z'].v).toBeCloseTo(100,7);
  expect(f.scenePosition().distanceTo(before)).toBeLessThan(1e-7);finish();
});
