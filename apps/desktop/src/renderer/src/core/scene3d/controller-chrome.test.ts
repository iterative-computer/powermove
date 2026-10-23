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
    workDone=()=>Promise.resolve();
    dispose(){this.canvas.remove();}
  }};
});
vi.mock('./overlay-capture',async importOriginal=>{
  const original=await importOriginal<typeof import('./overlay-capture')>();
  return {...original,getRetainedViewportOverlayBitmap:()=>document.createElement('canvas')};
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
  afterEach(()=>{overlayDraws.mockReset();vi.restoreAllMocks();vi.unstubAllGlobals();document.body.replaceChildren();});

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

  it('uses the existing bound presenter and tags only a completed overlay draw',()=>{
    let drawFrame:FrameRequestCallback|undefined;
    vi.stubGlobal('requestAnimationFrame',vi.fn((callback:FrameRequestCallback)=>{drawFrame=callback;return 1;}));
    vi.stubGlobal('cancelAnimationFrame',vi.fn());
    const stage=document.createElement('div'),inner=document.createElement('div'),bound=document.createElement('canvas');
    inner.append(bound);stage.append(inner);document.body.append(stage);
    Object.defineProperty(stage,'clientWidth',{value:640});
    Object.defineProperty(stage,'clientHeight',{value:360});
    stage.getBoundingClientRect=()=>({left:10,top:20,width:640,height:360} as DOMRect);
    inner.getBoundingClientRect=()=>({left:110,top:70,width:400,height:225} as DOMRect);
    const plane=()=>({displayWidth:800,displayHeight:450,close:vi.fn()}) as unknown as VideoFrame;
    const presentation={presenting:true,state:{viewSeq:7},
      latest:{seq:11,viewSeq:7,revision:2,depthId:{encoding:'chromePlanesV1',selection:{selected:[],active:[]},glyphs:[]}},
      onPresented:()=>()=>{},onChromeSurface:()=>()=>{},retainChromeFrame:()=>()=>{},
      depthFrame:plane,coverageFrame:plane};
    const project={id:'bound-test',w:640,h:360,fps:30,layers:[{id:'cube',d:{definition:'powermove.3d.object'}}]};
    const PM={proj:project,time:0,sel:{layers:[]},EngineDoc:{enabled:true,barrier:{revision:2}},
      EngineCanvas:{bound,viewerPresentation:presentation},
      Kernel:{services:{get:()=>({enginePresentation:{presenting:false}})}},bus:{on:()=>()=>{}},
      curComp:()=>project,L:()=>null};
    const controller=attachViewport(PM,stage,{panComposition(){},zoomComposition(){}});
    controller.update({x:100,y:50,width:400,height:225});
    drawFrame!(0);
    expect(overlayDraws).toHaveBeenCalledTimes(1);
    expect(overlayDraws.mock.calls[0]![0].native.viewport).toEqual({x:100,y:50,width:400,height:225});
    const canvas=stage.querySelector('canvas:not(:first-child)') as HTMLCanvasElement;
    expect(canvas.dataset.sceneOverlayFrame).toBe('2:7:11');
    presentation.latest.seq=12;
    controller.update({x:100,y:50,width:400,height:225});
    drawFrame!(0);
    expect(canvas.dataset.sceneOverlayFrame).toBe('2:7:12');
    overlayDraws.mockImplementationOnce(()=>{throw new Error('draw failed');});
    vi.spyOn(console,'error').mockImplementation(()=>{});
    controller.update({x:100,y:50,width:400,height:225});
    drawFrame!(0);
    expect(canvas.dataset.sceneOverlayFrame).toBeUndefined();
    controller.dispose();
  });
});
