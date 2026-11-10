// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fitFrame, mountViewPanes, paneRects } from './view-panes';

afterEach(()=>{vi.restoreAllMocks();vi.unstubAllGlobals();document.body.replaceChildren();});

it('presents every ready side pane on each display tick and leaves unchanged pictures alone',()=>{
  const frames=new Map<number,FrameRequestCallback>();let next=0,version=1;
  vi.stubGlobal('requestAnimationFrame',(callback:FrameRequestCallback)=>{frames.set(++next,callback);return next;});
  vi.stubGlobal('cancelAnimationFrame',(id:number)=>frames.delete(id));
  vi.stubGlobal('ResizeObserver',class{observe(){}disconnect(){}});
  const drawImage=vi.fn();vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue({fillRect(){},drawImage} as any);
  const panel=document.createElement('div'),body=document.createElement('div'),stage=document.createElement('div');
  panel.className='panel';panel.append(body);body.append(stage);document.body.append(panel);
  Object.defineProperties(body,{clientWidth:{value:801},clientHeight:{value:601}});
  const picture=document.createElement('canvas');
  const render=vi.fn(()=>({canvas:picture,version:String(version)}));
  const subscription={dispose:vi.fn()},structure={dispose:vi.fn()};
  const api:any={scene3d:{views:{get:()=>({layout:4,active:3,panes:['top','front','right','camera'].map(view=>({view}))}),options:[],render,selection:()=>[],onChange:()=>subscription}},model:{curComp:()=>({layers:[{threeD:true}]})},project:{get:()=>({w:1920,h:1080})},ui:{icon:()=>''},events:{on:()=>structure}};
  const cleanup=mountViewPanes(api,body,{stage});
  const tick=(at:number)=>{const [id,callback]=frames.entries().next().value!;frames.delete(id);callback(at);};
  try{
    tick(0);expect(render.mock.calls.map((call:any[])=>call[0])).toEqual([0,1,2]);expect(drawImage).toHaveBeenCalledTimes(3);
    version++;tick(8.3);expect(drawImage).toHaveBeenCalledTimes(6);
    tick(16.7);expect(drawImage).toHaveBeenCalledTimes(6);
    version++;tick(25);expect(drawImage).toHaveBeenCalledTimes(9);
  }finally{cleanup();}
  expect(frames.size).toBe(0);expect(subscription.dispose).toHaveBeenCalledOnce();expect(structure.dispose).toHaveBeenCalledOnce();
});

describe('view pane geometry', () => {
  it('splits the viewer into 1, 2 or 4 panes with a hairline between them', () => {
    expect(paneRects(1, 800, 600)).toEqual([{ x: 0, y: 0, w: 800, h: 600 }]);
    expect(paneRects(2, 801, 600)).toEqual([{ x: 0, y: 0, w: 400, h: 600 }, { x: 401, y: 0, w: 400, h: 600 }]);
    const four = paneRects(4, 801, 601);
    expect(four.map(r => [r.x, r.y])).toEqual([[0, 0], [401, 0], [0, 301], [401, 301]]);
    expect(four.every(r => r.w === 400 && r.h === 300)).toBe(true);
  });
  it('fits the composition frame in a pane like the viewer does', () => {
    const frame = fitFrame({ w: 416, h: 300 }, { w: 1920, h: 1080 });
    expect(frame.w / frame.h).toBeCloseTo(1920 / 1080, 5);
    expect(frame.x).toBeCloseTo(8, 5);
    expect(frame.y).toBeGreaterThan(8);
  });
});
