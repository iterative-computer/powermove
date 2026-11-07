// @vitest-environment happy-dom
import {afterEach,describe,expect,it,vi} from 'vitest';
import {makePM} from '../../legacy/__tests__/make-pm';
import {createMaterialThumbnails} from './material-thumbnails';

function editor(){
  vi.useFakeTimers();
  vi.stubGlobal('requestIdleCallback',(callback:()=>void)=>setTimeout(callback,1));
  vi.stubGlobal('ImageData',class {constructor(public data:Uint8ClampedArray,public width:number,public height:number){}});
  const drawImage=vi.fn(),putImageData=vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype,'getContext').mockReturnValue({drawImage,putImageData} as any);
  vi.spyOn(HTMLCanvasElement.prototype,'toDataURL').mockImplementation(function(this:HTMLCanvasElement){return `data:image/png;base64,${this.width}x${this.height}`;});
  const PM=makePM();let version=0;PM.time=1;PM.animVersion=()=>version;
  const receiver={id:'model',d:{data:{object:{source:{primitive:'box'}}}}};
  const source={id:'paint',type:'solid',d:{w:800,h:400}};
  PM.proj={id:'project',compId:'root',w:800,h:400,fps:30,dur:5,layers:[receiver,source],comps:{sub:{id:'sub',w:400,h:800,fps:30,dur:5,layers:[]}}};
  PM.L=(id:string)=>PM.proj.layers.find((l:any)=>l.id===id);
  PM.GL={renderMaterialComposition:vi.fn((_c:any,_t:number,w:number,h:number)=>new Uint8Array(w*h*4))};
  PM.assets.get=(id:string)=>PM.assets.map.get(id);
  const service=createMaterialThumbnails(PM);
  return {PM,service,drawImage,putImageData,touch:()=>version++};
}
afterEach(()=>{vi.restoreAllMocks();vi.unstubAllGlobals();vi.useRealTimers();});

describe('material picker thumbnails',()=>{
  it('defers capture, deduplicates concurrent requests and reuses small retina previews',async()=>{
    const {PM,service}=editor(),map={layerId:'paint'};
    const first=service.get('model',map,{pixelRatio:2}),second=service.get('model',map,{pixelRatio:2});
    expect(second).toBe(first);expect(PM.GL.renderMaterialComposition).not.toHaveBeenCalled();
    await vi.runAllTimersAsync();const result=await first;
    expect(result).toMatchObject({width:128,height:64,url:expect.stringContaining('data:image/png')});
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledWith(PM.proj,1,128,64,1,'paint');
    expect(await service.get('model',map,{pixelRatio:2})).toBe(result);
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledOnce();service.dispose();
  });
  it('handles compositions and images independently of the main material texture cache',async()=>{
    const {PM,service,drawImage}=editor();
    const el={naturalWidth:640,naturalHeight:320};PM.assets.map.set('image',{kind:'image',el});
    const comp=service.get('model',{compositionId:'sub'},{size:48}),image=service.get('model','image',{size:48});
    await vi.runAllTimersAsync();
    expect(await comp).toMatchObject({width:24,height:48});expect(await image).toMatchObject({width:48,height:24});
    expect(drawImage).toHaveBeenCalledWith(el,0,0,48,24);service.dispose();
  });
  it('invalidates after animation/source edits, history and project replacement, discarding stale jobs',async()=>{
    const {PM,service,touch}=editor(),map={layerId:'paint'};
    const old=service.get('model',map);PM.bus.emit('layers');expect(await old).toBeNull();
    const fresh=service.get('model',map);await vi.runAllTimersAsync();expect(await fresh).not.toBeNull();
    touch();const animated=service.get('model',map);await vi.runAllTimersAsync();await animated;
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(2);
    PM.bus.emit('history:project-patch');const undone=service.get('model',map);await vi.runAllTimersAsync();await undone;
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(3);
    const stale=service.get('model',map,{time:2});touch();await vi.runAllTimersAsync();expect(await stale).toBeNull();service.dispose();
  });
  it('leaves placeholders for missing sources, rendering errors and active interactions',async()=>{
    const {PM,service}=editor();
    expect(await service.get('model',{layerId:'missing'})).toBeNull();
    PM.playing=true;const playing=service.get('model',{layerId:'paint'});await vi.runAllTimersAsync();expect(await playing).toBeNull();
    expect(PM.GL.renderMaterialComposition).not.toHaveBeenCalled();PM.playing=false;
    PM.GL.renderMaterialComposition.mockImplementation(()=>{throw Error('context lost');});
    const failed=service.get('model',{layerId:'paint'});await vi.runAllTimersAsync();expect(await failed).toBeNull();service.dispose();
  });
  it('bounds pending work and cached memory and cancels cleanly on disposal',async()=>{
    const {PM,service}=editor(),map={layerId:'paint'};
    const jobs=Array.from({length:70},(_,time)=>service.get('model',map,{time}));
    await vi.runAllTimersAsync();expect((await Promise.all(jobs)).filter(Boolean)).toHaveLength(64);
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(64);
    const evicted=service.get('model',map,{time:0});await vi.runAllTimersAsync();await evicted;
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(65);
    const oldest=service.get('model',map,{time:6});await vi.runAllTimersAsync();await oldest;
    expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(66);
    const pending=service.get('model',map,{time:100});service.dispose();expect(await pending).toBeNull();
    await vi.runAllTimersAsync();expect(PM.GL.renderMaterialComposition).toHaveBeenCalledTimes(66);
    expect(await service.get('model',map)).toBeNull();
  });
});
