// @vitest-environment happy-dom
import {afterEach,describe,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {getRetainedViewportOverlayBitmap} from './overlay-capture';

/* happy-dom has no WebGL: the overlay draws through a renderer that records
   its size changes the way WebGLRenderer applies them (canvas size = CSS size
   × pixel ratio, floored). */
const sizes:Array<[number,number]>=[];
const operations:string[]=[];
vi.mock('three',async importOriginal=>{
  const actual=await importOriginal<typeof import('three')>();
  class FakeRenderer {
    domElement=document.createElement('canvas');autoClear=true;private ratio=1;
    setPixelRatio(ratio:number){this.ratio=ratio;}
    getPixelRatio(){return this.ratio;}
    setSize(width:number,height:number){sizes.push([width,height]);this.domElement.width=Math.floor(width*this.ratio);this.domElement.height=Math.floor(height*this.ratio);}
    setScissorTest(){}setScissor(){}setClearColor(){}clear(){}clearDepth(){}render(){operations.push('render');}getRenderTarget(){return null;}setRenderTarget(){}
  }
  return {...actual,WebGLRenderer:FakeRenderer};
});
const {ViewportOverlay}=await import('./overlay');

function stage(width:number,height:number){
  const element=document.createElement('div');
  const size={width,height};
  Object.defineProperty(element,'clientWidth',{get:()=>size.width});
  Object.defineProperty(element,'clientHeight',{get:()=>size.height});
  return {element,size};
}
function frame(){
  return {camera:new THREE.PerspectiveCamera(50,16/9,0.1,100),rect:{x:0,y:0,width:640,height:360},runtime:null,userView:false,axis:null,
    distance:10,fov:50,overlays:true,selected:new Set<string>(),active:new Set<string>(),glyphs:[],guides:[],gizmo:null};
}

describe('ViewportOverlay sizing', () => {
  afterEach(()=>{sizes.length=0;operations.length=0;vi.restoreAllMocks();vi.unstubAllGlobals();});
  it('sizes its canvas once for an unchanged stage, not on every draw', () => {
    vi.stubGlobal('devicePixelRatio',2);
    const {element}=stage(1195,546),overlay=new ViewportOverlay(element);
    for(let i=0;i<5;i++)overlay.draw(frame());
    expect(sizes).toEqual([[1195,546]]);
    expect([overlay.canvas.width,overlay.canvas.height]).toEqual([2390,1092]);
  });
  it('sizes it again when the stage changes size', () => {
    vi.stubGlobal('devicePixelRatio',2);
    const {element,size}=stage(800,450),overlay=new ViewportOverlay(element);
    overlay.draw(frame());
    size.width=801;overlay.draw(frame());overlay.draw(frame());
    size.height=300;overlay.draw(frame());
    expect(sizes).toEqual([[800,450],[801,450],[801,300]]);
  });
  it('compares in device pixels at a fractional pixel ratio', () => {
    vi.stubGlobal('devicePixelRatio',1.5);
    const {element}=stage(333,201),overlay=new ViewportOverlay(element);
    overlay.draw(frame());overlay.draw(frame());
    expect(sizes).toEqual([[333,201]]);
    expect([overlay.canvas.width,overlay.canvas.height]).toEqual([499,301]);
  });
  it('retains the completed WebGL pass before browser compositing',()=>{
    const context={clearRect:vi.fn(),drawImage:vi.fn(()=>operations.push('copy'))};
    vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue(context as never);
    const {element}=stage(640,360),overlay=new ViewportOverlay(element);
    overlay.draw({...frame(),gizmo:new THREE.Object3D()});
    expect(operations.slice(-2)).toEqual(['render','copy']);
    const bitmap=getRetainedViewportOverlayBitmap(overlay.canvas);
    expect(bitmap).not.toBeNull();
    expect([bitmap?.width,bitmap?.height]).toEqual([overlay.canvas.width,overlay.canvas.height]);
    expect(context.drawImage).toHaveBeenCalledWith(overlay.canvas,0,0);
  });
});
