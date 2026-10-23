// @vitest-environment happy-dom
import {afterEach,describe,expect,it,vi} from 'vitest';

const overlayDraws=vi.hoisted(()=>vi.fn());
vi.mock('./overlay',async importOriginal=>{
  const original=await importOriginal<typeof import('./overlay')>();
  const {Scene}=await import('three');
  return {...original,ViewportOverlay:class {
    canvas=document.createElement('canvas');scene=new Scene();
    constructor(stage:HTMLElement){stage.append(this.canvas);}
    draw=overlayDraws;
    dispose(){this.canvas.remove();}
  }};
});
vi.mock('./render-camera',async importOriginal=>{
  const original=await importOriginal<typeof import('./render-camera')>();
  const {PerspectiveCamera}=await import('three');
  return {...original,renderCamera:()=>{
    const camera=new PerspectiveCamera(50,16/9,.1,100);
    camera.position.z=6;camera.updateMatrixWorld(true);
    return camera;
  }};
});
vi.mock('./gizmo',async importOriginal=>{
  const original=await importOriginal<typeof import('./gizmo')>();
  const {Object3D}=await import('three');
  return {...original,createTransformGizmo:()=>({helper:new Object3D(),sync(){},attached:()=>false,active:()=>false,dispose(){},cancel(){}})};
});
vi.mock('./navigation',async importOriginal=>{
  const original=await importOriginal<typeof import('./navigation')>();
  return {...original,createSceneNavigation:()=>({active:()=>false,dispose(){},cancel(){}})};
});

const {attachViewport}=await import('./controller');

describe('viewport chrome while native overlay planes are pending',()=>{
  afterEach(()=>{overlayDraws.mockClear();vi.unstubAllGlobals();document.body.replaceChildren();});

  it('shows navigation, view label and cursor before the depth/coverage draw',()=>{
    let drawFrame:FrameRequestCallback|undefined;
    vi.stubGlobal('requestAnimationFrame',vi.fn((callback:FrameRequestCallback)=>{drawFrame=callback;return 1;}));
    vi.stubGlobal('cancelAnimationFrame',vi.fn());
    const stage=document.createElement('div');document.body.append(stage);
    Object.defineProperty(stage,'clientWidth',{value:640});
    Object.defineProperty(stage,'clientHeight',{value:360});
    const project={id:'test',w:640,h:360,fps:30,layers:[{id:'cube',d:{definition:'powermove.3d.object'}}]};
    const presentation={
      presenting:true,latest:{viewSeq:1,revision:2,depthId:null},state:{viewSeq:1},
      onPresented:()=>()=>{},onChromeSurface:()=>()=>{},
    };
    const PM={proj:project,time:0,sel:{layers:[]},EngineDoc:{enabled:true,barrier:{revision:2}},
      Kernel:{services:{get:()=>({enginePresentation:presentation})}},bus:{on:()=>()=>{}},
      curComp:()=>project,L:()=>null};
    const controller=attachViewport(PM,stage,{panComposition(){},zoomComposition(){}});
    controller.update({x:0,y:0,width:640,height:360});
    expect(drawFrame).toBeDefined();
    drawFrame!(0);
    expect(overlayDraws).not.toHaveBeenCalled();
    const nav=stage.querySelector<HTMLElement>('.vp-nav')!;
    const label=stage.querySelector<HTMLElement>('.vp-label')!;
    const cursor=stage.querySelector<SVGElement>('[data-scene-cursor]')!;
    expect(nav.hidden).toBe(false);
    expect(label.hidden).toBe(false);
    expect(label.textContent).toContain('Camera Perspective');
    expect(cursor.style.display).not.toBe('none');
    // A Library card clones these live DOM states while the planes arrive.
    const clone=stage.cloneNode(true) as HTMLElement;
    expect(clone.querySelector<HTMLElement>('.vp-nav')?.hidden).toBe(false);
    expect(clone.querySelector<HTMLElement>('.vp-label')?.textContent).toContain('Camera Perspective');
    controller.dispose();
  });
});
