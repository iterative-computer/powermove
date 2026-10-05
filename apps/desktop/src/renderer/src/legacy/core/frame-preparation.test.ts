// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { prepareFrame,blenderFrameRequests } from './frame-preparation';

it('ignores a queued seek completion until the requested frame has decoded', async () => {
  const video = new EventTarget() as any;
  Object.assign(video, { paused: true, seeking: true, readyState: 2, currentTime: 0, pause() {} });
  const draw = vi.fn();
  const context = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: draw } as any);
  const layer = { id: 'clip', type: 'video', from: 0, d: { asset: 'source', speed: 1, trim: 0 } };
  const asset = { el: video, w: 16, h: 16, dur: 2 };
  const PM: any = { proj: { layers: [layer], fps: 30 }, active: () => true,
    scope: { push() {}, pop() {} }, assets: { get: () => asset } };
  try {
    const pending = prepareFrame(PM, 1.5);
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.seeking = false; video.currentTime = 0;
    video.dispatchEvent(new Event('seeked'));
    await Promise.resolve();
    expect(draw).not.toHaveBeenCalled();
    video.currentTime = 1.5;
    video.dispatchEvent(new Event('seeked'));
    await pending;
    expect(draw).toHaveBeenCalledOnce();
    expect(PM.preparedVideoFrames.has('clip@1.5')).toBe(true);
  } finally { context.mockRestore(); }
});

it('prepares nested Blender output sizes and every group shutter sample',()=>{
  const model={id:'model',type:'extension',d:{data:{object:{}}}},sub={id:'nested',w:200,h:100,fps:30,render3d:{enabled:true},layers:[model]};
  const parent={id:'root',w:400,h:200,fps:30,layers:[{id:'group',type:'group',mblur:true,d:{}},{id:'precomp',type:'precomp',parent:'group',from:0,d:{comp:'nested',w:200,h:100,trim:0,speed:1}}],comps:{nested:sub}};
  const PM:any={proj:parent,scope:{push(){},pop(){}},active:()=>true};
  const frames=blenderFrameRequests(PM,1,parent,{width:800,height:400,mbSamples:2});
  expect(frames).toHaveLength(3);
  expect(frames.map(frame=>[frame.width,frame.height])).toEqual([[400,200],[400,200],[400,200]]);
  expect(frames.map(frame=>frame.time)).toEqual([1,1-.25*.5/30,1+.25*.5/30]);
  expect(blenderFrameRequests(PM,1,parent,{mblur:false})).toHaveLength(1);
});
