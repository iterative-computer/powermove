// @vitest-environment happy-dom
import {expect,it} from 'vitest';
import {makePM} from '../../legacy/__tests__/make-pm';
import {createSceneNavigation,setNavigationMode,frameComposition} from './navigation';
import {editScene} from './operations';
import {viewportPose} from './viewport';

it('orbits and frames a standalone 3D plane through the shared viewer camera',()=>{
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({w:640,h:360,dur:3});PM.time=0;
  expect(PM.Edit.apply({type:'add_layer',id:'card',layerType:'shape',content:{shape:'rect',w:100,h:60},properties:{'position.x':440,'position.y':150}}).ok).toBe(true);
  expect(PM.Edit.apply({type:'set_layer',target:'card',patch:{threeD:true}}).ok).toBe(true);
  expect(editScene(PM,{operation:'set_material_options',target:'card',patch:{acceptsLights:true}}).ok).toBe(true);
  PM.GL={bounds:()=>({x0:-50,y0:-30,x1:50,y1:30,w:100,h:60}),renderMaterialComposition:()=>new Uint8Array(100*60*4).fill(255)};
  const stage=document.createElement('div'),navigation=createSceneNavigation(PM,stage),before=viewportPose(PM).position.clone();
  setNavigationMode(PM,'orbit');
  try {
    expect(navigation.pointerDown(new MouseEvent('pointerdown',{button:0,clientX:100,clientY:100}) as PointerEvent)).toBe(true);
    window.dispatchEvent(new MouseEvent('pointermove',{clientX:155,clientY:75}));
    window.dispatchEvent(new MouseEvent('pointerup',{button:0}));
    expect(viewportPose(PM).position).not.toEqual(before);
    PM.selectLayers(['card']);frameComposition(PM,true);
    expect(viewportPose(PM).target.x).toBeCloseTo(2);
  } finally {navigation.dispose();}
});
