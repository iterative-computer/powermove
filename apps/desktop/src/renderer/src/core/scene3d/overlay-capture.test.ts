import {describe,expect,it,vi} from 'vitest';
import {getRetainedViewportOverlayBitmap,releaseViewportOverlayBitmap,retainViewportOverlayBitmap} from './overlay-capture';

function canvases() {
  const calls:unknown[][]=[];
  let fail=false;
  const context={
    clearRect:(...args:unknown[])=>calls.push(['clear',...args]),
    drawImage:(...args:unknown[])=>{calls.push(['draw',...args]);if(fail)throw new Error('readback failed');},
  };
  const bitmap={width:0,height:0,getContext:vi.fn(()=>context)};
  const source={width:640,height:360,ownerDocument:{createElement:vi.fn(()=>bitmap)}} as unknown as HTMLCanvasElement;
  return {source,bitmap,calls,failReadback:(value:boolean)=>{fail=value;}};
}

describe('retained viewport overlay bitmap',()=>{
  it('copies the final draw at source backing size and clears previous pixels',()=>{
    const {source,bitmap,calls}=canvases();
    expect(getRetainedViewportOverlayBitmap(source)).toBeNull();
    retainViewportOverlayBitmap(source);
    expect(getRetainedViewportOverlayBitmap(source)).toBe(bitmap);
    expect(calls).toEqual([['clear',0,0,640,360],['draw',source,0,0]]);
    calls.length=0;
    retainViewportOverlayBitmap(source);
    expect(source.ownerDocument.createElement).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([['clear',0,0,640,360],['draw',source,0,0]]);
  });

  it('never returns a stale size, cleared frame, or failed readback',()=>{
    const {source,bitmap,failReadback}=canvases();
    retainViewportOverlayBitmap(source);
    source.width=800;
    expect(getRetainedViewportOverlayBitmap(source)).toBeNull();
    retainViewportOverlayBitmap(source);
    expect(bitmap.width).toBe(800);
    expect(getRetainedViewportOverlayBitmap(source)).toBe(bitmap);
    failReadback(true);
    retainViewportOverlayBitmap(source);
    expect(getRetainedViewportOverlayBitmap(source)).toBeNull();
    failReadback(false);
    retainViewportOverlayBitmap(source);
    expect(getRetainedViewportOverlayBitmap(source)).toBe(bitmap);
    source.height=0;
    retainViewportOverlayBitmap(source);
    expect(getRetainedViewportOverlayBitmap(source)).toBeNull();
  });

  it('releases the retained bitmap when its viewport is disposed',()=>{
    const {source}=canvases();
    retainViewportOverlayBitmap(source);
    releaseViewportOverlayBitmap(source);
    expect(getRetainedViewportOverlayBitmap(source)).toBeNull();
  });
});
