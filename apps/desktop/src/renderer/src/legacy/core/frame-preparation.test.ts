// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { frameFontsReady, prepareFrame } from './frame-preparation';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function fontFixture() {
  let ready: Promise<unknown> = Promise.resolve();
  const getter = vi.fn(() => ready);
  const fonts = Object.assign(new EventTarget(), { status: 'loaded', size: 0 });
  Object.defineProperty(fonts, 'ready', { get: getter });
  vi.stubGlobal('document', { fonts });
  const handlers = new Map<string, Array<() => void>>();
  let version = 1;
  const on = (event: string, run: () => void) => handlers.set(event, [...handlers.get(event) || [], run]);
  const PM: any = { proj: { fps: 30, layers: [] }, scope: [], active: () => false,
    animVersion: () => version, bus: { on }, Kernel: { events: { on } } };
  return { PM, fonts, getter, edit: () => version++, emit: (event: string) => handlers.get(event)?.forEach(run => run()),
    loading() {
      fonts.status = 'loading'; fonts.dispatchEvent(new Event('loading'));
      let finish!: () => void;
      ready = new Promise<void>(resolve => { finish = () => {
        fonts.status = 'loaded'; fonts.dispatchEvent(new Event('loadingdone')); resolve();
      }; });
      return finish;
    } };
}

it('reuses completed font readiness across unchanged captures without rereading the layout getter', async () => {
  const { PM, getter } = fontFixture();
  await prepareFrame(PM, 0); await prepareFrame(PM, 1); await prepareFrame(PM, 2);
  expect(getter).toHaveBeenCalledOnce();
});

it('reports cached font readiness without reading the layout getter and rejects invalidated readiness', async () => {
  const { PM, fonts, getter, emit, edit, loading } = fontFixture();
  expect(frameFontsReady(PM)).toBe(false);
  expect(getter).not.toHaveBeenCalled();
  await prepareFrame(PM, 0);
  for (const event of ['fonts', 'layers', 'project', 'extensions:changed', 'theme:changed']) {
    expect(frameFontsReady(PM)).toBe(true);
    const reads = getter.mock.calls.length;
    emit(event);
    expect(frameFontsReady(PM)).toBe(false);
    expect(getter.mock.calls.length).toBe(reads);
    await prepareFrame(PM, 0);
  }
  edit(); expect(frameFontsReady(PM)).toBe(false); await prepareFrame(PM, 0);
  PM.proj.layers = []; expect(frameFontsReady(PM)).toBe(false); await prepareFrame(PM, 0);
  fonts.size++; expect(frameFontsReady(PM)).toBe(false); await prepareFrame(PM, 0);
  const finish = loading(); expect(frameFontsReady(PM)).toBe(false);
  finish(); expect(frameFontsReady(PM)).toBe(false); await prepareFrame(PM, 0);
  expect(frameFontsReady(PM)).toBe(true);
  const reads = getter.mock.calls.length;
  expect(frameFontsReady(PM, { ...PM.proj })).toBe(false);
  (document as any).fonts = Object.assign(new EventTarget(), { status: 'loaded', size: fonts.size });
  expect(frameFontsReady(PM)).toBe(false);
  expect(getter.mock.calls.length).toBe(reads);
});

it('does not evaluate activity for a large scene without video or nested compositions', async () => {
  const { PM } = fontFixture();
  PM.proj.layers = Array.from({ length: 20_000 }, (_, i) => ({ type: i % 2 ? 'shape' : 'text' }));
  let activityCalls = 0;
  PM.active = () => { activityCalls++; return true; };
  await prepareFrame(PM, 0);
  expect(activityCalls).toBe(0);
  expect(PM.preparedVideoFrames.size).toBe(0);
});

it('invalidates font readiness on font, project, edit and layer-structure changes', async () => {
  const { PM, fonts, getter, emit, edit } = fontFixture();
  await prepareFrame(PM, 0);
  for (const event of ['fonts', 'layers', 'project', 'extensions:changed', 'theme:changed']) { emit(event); await prepareFrame(PM, 0); }
  edit(); await prepareFrame(PM, 0);
  PM.proj.layers.push({ type: 'text' }); await prepareFrame(PM, 0);
  PM.proj = { ...PM.proj }; await prepareFrame(PM, 0);
  fonts.size++; await prepareFrame(PM, 0);
  expect(getter).toHaveBeenCalledTimes(10);
  await prepareFrame(PM, 1);
  expect(getter).toHaveBeenCalledTimes(10);
});

it('waits for a fresh font load and keeps the completed result reusable', async () => {
  const { PM, getter, loading } = fontFixture();
  await prepareFrame(PM, 0);
  const finish = loading();
  let completed = false;
  const preparing = prepareFrame(PM, 1).then(() => { completed = true; });
  await Promise.resolve(); expect(completed).toBe(false);
  finish(); await preparing;
  const reads = getter.mock.calls.length;
  await prepareFrame(PM, 2);
  expect(getter).toHaveBeenCalledTimes(reads);
});

it('does not reuse a stale pending promise when another font load starts', async () => {
  const { PM, getter, loading, fonts } = fontFixture();
  const finishFirst = loading();
  let completed = false;
  const preparing = prepareFrame(PM, 0).then(() => { completed = true; });
  const finishSecond = loading();
  finishFirst(); fonts.status = 'loading';
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  expect(completed).toBe(false);
  expect(getter.mock.calls.length).toBeGreaterThan(1);
  finishSecond(); await preparing;
  expect(completed).toBe(true);
});

it('checks a replacement FontFaceSet instead of using the previous set\'s readiness', async () => {
  const { PM } = fontFixture();
  await prepareFrame(PM, 0);
  const replacement = Object.assign(new EventTarget(), { status: 'loaded', size: 0 });
  const ready = vi.fn(() => Promise.resolve());
  Object.defineProperty(replacement, 'ready', { get: ready });
  (document as any).fonts = replacement;
  await prepareFrame(PM, 1); await prepareFrame(PM, 2);
  expect(ready).toHaveBeenCalledOnce();
});

it.each(['edit', 'project', 'fonts'])('rechecks a pending font wait after a %s change', async change => {
  const { PM, getter, loading, edit, emit } = fontFixture();
  const finish = loading();
  const preparing = prepareFrame(PM, 0);
  if (change === 'edit') edit();
  else if (change === 'project') PM.proj = { ...PM.proj };
  else emit('fonts');
  finish(); await preparing;
  expect(getter.mock.calls.length).toBeGreaterThan(1);
});

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

function videoFixture() {
  let active = 0, maximum = 0;
  const videos: any[] = [];
  const draw = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(function (this: HTMLCanvasElement) {
    return { drawImage: (video: any) => {
      (this as any).frame = `${video.name}@${video.currentTime}`;
      draw(video.name, video.currentTime);
    } } as any;
  });
  const assets = new Map<string, any>();
  const PM: any = { proj: { fps: 30, layers: [], comps: {} }, scope: [], active: () => true, assets };
  function video(name: string, auto = true) {
    let time = 0;
    const el: any = new EventTarget();
    Object.assign(el, { name, readyState: 2, seeking: false, pause: vi.fn(), seeks: [], videoWidth: 16, videoHeight: 16,
      finish(error = false) {
        if (!el.seeking) return;
        el.seeking = false; active--;
        el.dispatchEvent(new Event(error ? 'error' : 'seeked'));
      } });
    Object.defineProperty(el, 'currentTime', { get: () => time, set: (at: number) => {
      time = el.quantize ? Math.round(at * 1e6) / 1e6 : at; el.seeks.push(at); el.seeking = true;
      maximum = Math.max(maximum, ++active);
      if (auto) queueMicrotask(() => el.finish());
    } });
    const asset = { el, w: 16, h: 16, dur: 10 };
    assets.set(name, asset); videos.push(el);
    return asset;
  }
  function clip(id: string, asset: string, trim = 0) {
    return { id, type: 'video', from: 0, d: { asset, speed: 1, trim } };
  }
  return { PM, video, clip, draw, videos, maximum: () => maximum };
}

it('evaluates only video and composition activity while preserving nested timing and inactive skips', async () => {
  const h = videoFixture();
  h.video('source');
  const calls: string[] = [];
  h.PM.active = (layer: any, time: number) => {
    if (layer.type !== 'video' && layer.type !== 'precomp') throw new Error('Non-media activity must be left to rendering');
    calls.push(`${layer.id}@${time}`);
    return !layer.id.startsWith('inactive');
  };
  h.PM.proj.comps.child = { layers: [
    { id: 'text', type: 'text' }, h.clip('nested', 'source'), h.clip('inactive-video', 'source'),
  ] };
  h.PM.proj.comps.hidden = { layers: [h.clip('unvisited', 'missing')] };
  h.PM.proj.layers.push({ id: 'shape', type: 'shape' },
    { id: 'visible-comp', type: 'precomp', from: 0, d: { comp: 'child', speed: 1, trim: 1 } },
    { id: 'inactive-comp', type: 'precomp', from: 0, d: { comp: 'hidden' } }, h.clip('inactive-root', 'source'));
  await prepareFrame(h.PM, 1);
  expect(calls).toEqual(['visible-comp@1', 'nested@2', 'inactive-video@2', 'inactive-comp@1', 'inactive-root@1']);
  expect(h.PM.preparedVideoFrames.size).toBe(1);
  expect(h.PM.preparedVideoFrames.get('nested@2').frame).toBe('source@2');
  expect(h.PM.scope).toEqual([]);
});

it('decodes independent videos concurrently with a bounded number of active decoders', async () => {
  const h = videoFixture();
  for (let i = 0; i < 9; i++) {
    h.video(`source${i}`); h.PM.proj.layers.push(h.clip(`clip${i}`, `source${i}`));
  }
  await prepareFrame(h.PM, 1);
  expect(h.maximum()).toBe(4);
  expect(h.draw).toHaveBeenCalledTimes(9);
  expect(h.PM.preparedVideoFrames.size).toBe(9);
});

it('copies a shared source frame only once per source time and size', async () => {
  const h = videoFixture();
  h.video('source');
  h.PM.proj.layers.push(h.clip('first', 'source'), h.clip('second', 'source'), h.clip('third', 'source'));
  await prepareFrame(h.PM, 1);
  expect(h.draw).toHaveBeenCalledOnce();
  const frames = h.PM.preparedVideoFrames;
  expect(frames.get('first@1')).toBe(frames.get('second@1'));
  expect(frames.get('third@1')).toBe(frames.get('first@1'));
});

it('keeps shared decoder times separate across asset aliases and nested compositions', async () => {
  const h = videoFixture();
  const asset = h.video('source'); h.PM.assets.set('alias', { ...asset });
  h.PM.proj.comps.child = { layers: [h.clip('nested', 'alias')] };
  h.PM.proj.layers.push(h.clip('first', 'source'), h.clip('later', 'alias', 2),
    { id: 'precomp', type: 'precomp', from: 0, d: { comp: 'child', speed: 1, trim: 1 } }, h.clip('repeat', 'source'));
  await prepareFrame(h.PM, 1);
  expect(h.maximum()).toBe(1);
  expect(asset.el.seeks).toEqual([1, 3, 2, 1]);
  expect(asset.el.currentTime).toBe(1);
  const frames = h.PM.preparedVideoFrames;
  expect(frames.get('first@1').frame).toBe('source@1');
  expect(frames.get('later@1').frame).toBe('source@3');
  expect(frames.get('nested@2').frame).toBe('source@2');
  expect(frames.get('repeat@1')).toBe(frames.get('first@1'));
  expect(h.PM.scope).toEqual([]);
});

it('preserves requested canvas dimensions for aliases of the same decoder', async () => {
  const h = videoFixture();
  const asset = h.video('source'); h.PM.assets.set('small', { ...asset, w: 8, h: 4 });
  h.PM.proj.layers.push(h.clip('large', 'source'), h.clip('small', 'small'));
  await prepareFrame(h.PM, 1);
  expect(h.draw).toHaveBeenCalledTimes(2);
  expect(h.PM.preparedVideoFrames.get('large@1').width).toBe(16);
  expect(h.PM.preparedVideoFrames.get('small@1').width).toBe(8);
  expect(h.PM.preparedVideoFrames.get('small@1').height).toBe(4);
});

it('preserves tolerance-skipped pixels separately from a later exact seek to the same target', async () => {
  const h = videoFixture();
  const asset = h.video('source');
  asset.el.currentTime = .9996;
  await Promise.resolve();
  asset.el.seeks.length = 0;
  h.PM.proj.layers.push(h.clip('first', 'source'), h.clip('later', 'source', .1), h.clip('last', 'source'));
  await prepareFrame(h.PM, 1);
  expect(asset.el.seeks).toEqual([1.1, 1]);
  const frames = h.PM.preparedVideoFrames;
  expect(frames.get('first@1').frame).toBe('source@0.9996');
  expect(frames.get('last@1').frame).toBe('source@1');
  expect(frames.get('first@1')).not.toBe(frames.get('last@1'));
});

it('recaptures pixels when a decoder source changes during preparation', async () => {
  const h = videoFixture();
  const asset = h.video('source'); asset.el.currentSrc = 'first.mp4';
  h.draw.mockImplementationOnce(() => { asset.el.currentSrc = 'second.mp4'; });
  h.PM.proj.layers.push(h.clip('first', 'source'), h.clip('last', 'source'));
  await prepareFrame(h.PM, 1);
  expect(h.draw).toHaveBeenCalledTimes(2);
  expect(h.PM.preparedVideoFrames.get('first@1')).not.toBe(h.PM.preparedVideoFrames.get('last@1'));
});

it('settles active decoders before reporting failure without publishing a partial capture', async () => {
  const h = videoFixture();
  for (let i = 0; i < 6; i++) {
    h.video(`source${i}`, false); h.PM.proj.layers.push(h.clip(`clip${i}`, `source${i}`));
  }
  const previous = new Map(); h.PM.preparedVideoFrames = previous;
  let settled = false;
  const preparing = prepareFrame(h.PM, 1).finally(() => { settled = true; });
  const rejected = expect(preparing).rejects.toThrow('Could not decode video frame');
  h.videos[0].finish(true);
  await Promise.resolve(); await Promise.resolve();
  expect(settled).toBe(false);
  for (const el of h.videos) el.finish();
  await rejected;
  expect(h.videos[4].seeks).toEqual([]);
  expect(h.videos[5].seeks).toEqual([]);
  expect(h.PM.preparedVideoFrames).toBe(previous);
});


it('bounds input video canvases without publishing partial frames', async () => {
  const h = videoFixture(); h.video('map'); h.PM.proj.layers.push(h.clip('map-layer','map'));
  h.PM.proj.layers.push({id:'owner',type:'shape',fx:[{type:'mapped',on:true,inputs:{map:{layerId:'map-layer',stage:'source'}}}]});
  h.PM.effectDef=()=>({params:[{k:'map',type:'layer'}]}); h.PM.Memory={budget:()=>100};
  const previous=new Map();h.PM.preparedVideoFrames=previous;
  await expect(prepareFrame(h.PM,1)).rejects.toThrow('memory budget');
  expect(h.PM.preparedVideoFrames).toBe(previous);expect(h.draw).not.toHaveBeenCalled();
});

it('drains cancelled input-video seeks and keeps previous prepared frames', async () => {
  const h = videoFixture(); h.video('map',false); h.PM.proj.layers.push(h.clip('map-layer','map'));
  h.PM.proj.layers.push({id:'owner',type:'shape',fx:[{type:'mapped',on:true,inputs:{map:{layerId:'map-layer',stage:'source'}}}]});
  h.PM.effectDef=()=>({params:[{k:'map',type:'layer'}]});const controller=new AbortController(),previous=new Map();h.PM.preparedVideoFrames=previous;
  const pending=prepareFrame(h.PM,1,h.PM.proj,{signal:controller.signal});const rejected=expect(pending).rejects.toThrow('cancelled');controller.abort();h.videos[0].finish();await rejected;
  expect(h.PM.preparedVideoFrames).toBe(previous);expect(h.draw).not.toHaveBeenCalled();
});


it('exact input plans seek across nearby frame boundaries instead of reusing the legacy tolerance', async () => {
 const h=videoFixture(),asset=h.video('source'),layer=h.clip('clip','source');
 h.PM.proj.layers=[layer,{id:'owner',type:'shape',fx:[{inputs:{map:{layerId:'clip',stage:'source'}}}]}];
 asset.el.currentTime=.4999;await Promise.resolve();
 h.PM.GL={frameVideoRequests:()=>[{key:'clip@0.5',layer,time:.5,asset,at:.5001}]};
 await prepareFrame(h.PM,.5);
 expect(asset.el.seeks).toEqual([.4999,.5001]);expect(h.PM.preparedVideoFrames.get('clip@0.5').frame).toBe('source@0.5001');
});


it('accepts microsecond-quantized browser seek completion at fractional shutter centers', async () => {
 const h=videoFixture(),asset=h.video('source'),layer=h.clip('clip','source');asset.el.quantize=true;
 h.PM.proj.layers=[layer,{id:'owner',fx:[{inputs:{map:{layerId:'clip',stage:'source'}}}]}];
 h.PM.GL={frameVideoRequests:()=>[{key:'clip@0.0078125',layer,time:.0078125,asset,at:.0078125}]};
 await prepareFrame(h.PM,.0078125);expect(h.PM.preparedVideoFrames.get('clip@0.0078125').frame).toBe('source@0.007813');
});

it('prepares timestamped temporal-only effects without static input records and skips their disabled path',async()=>{
 const h=videoFixture(),asset=h.video('source'),layer=h.clip('clip','source'),effect={type:'echo',on:true,p:{}};
 h.PM.proj.layers=[layer,{id:'owner',type:'shape',fx:[effect]}];h.PM.effectDef=()=>({params:[],temporal:{step:'step',count:'count',maxSamples:32}});
 h.PM.GL={frameVideoRequests:vi.fn(()=>[.875,.75,1].map(t=>({key:`clip@${t}`,layer,time:t,asset,at:t})))};
 await prepareFrame(h.PM,1);expect(h.PM.GL.frameVideoRequests).toHaveBeenCalledOnce();expect([...h.PM.preparedVideoFrames.keys()]).toEqual(['clip@0.875','clip@0.75','clip@1']);expect(h.PM.preparedVideoExactInputs.frames).toBe(h.PM.preparedVideoFrames);
 effect.on=false;await prepareFrame(h.PM,1);expect(h.PM.GL.frameVideoRequests).toHaveBeenCalledOnce();expect(h.PM.preparedVideoExactInputs).toBeNull();
});
it('drains cancelled temporal-only preparation without publishing partial frames',async()=>{
 const h=videoFixture(),asset=h.video('source',false),layer=h.clip('clip','source');h.PM.proj.layers=[layer,{id:'owner',type:'shape',fx:[{type:'echo',on:true}]}];h.PM.effectDef=()=>({params:[],temporal:{step:'step',count:'count',maxSamples:32}});h.PM.GL={frameVideoRequests:()=>[.875,.75].map(t=>({key:`clip@${t}`,layer,time:t,asset,at:t}))};const previous=new Map(),controller=new AbortController();h.PM.preparedVideoFrames=previous;
 const pending=prepareFrame(h.PM,1,h.PM.proj,{signal:controller.signal}),rejected=expect(pending).rejects.toThrow('cancelled');controller.abort();h.videos[0].finish();await rejected;expect(h.PM.preparedVideoFrames).toBe(previous);expect(h.draw).not.toHaveBeenCalled();expect(h.videos[0].seeks).toEqual([.875]);
});

it('does not publish a cancelled temporal frame while font readiness is pending',async()=>{
 const {PM,loading}=fontFixture(),finish=loading(),previous=new Map(),controller=new AbortController();PM.preparedVideoFrames=previous;PM.proj.layers=[{fx:[{type:'echo',on:true}]}];PM.effectDef=()=>({params:[],temporal:{step:'step',count:'count',maxSamples:32}});PM.GL={frameVideoRequests:()=>[]};
 const pending=prepareFrame(PM,1,PM.proj,{signal:controller.signal}),rejected=expect(pending).rejects.toThrow('cancelled');await Promise.resolve();controller.abort();finish();await rejected;expect(PM.preparedVideoFrames).toBe(previous);
});

it.each(['project','context','loss'])('rejects a temporal preparation after its %s lifetime changes',async(kind)=>{
 const {PM,loading}=fontFixture(),finish=loading(),previous=new Map();PM.preparedVideoFrames=previous;PM.proj.layers=[{fx:[{type:'echo',on:true}]}];PM.effectDef=()=>({params:[],temporal:{step:'step',count:'count',maxSamples:32}});PM.GL={gl:{},frameVideoRequests:()=>[]};const pending=prepareFrame(PM,1),rejected=expect(pending).rejects.toThrow('cancelled');await Promise.resolve();if(kind==='project')PM.proj={...PM.proj};else if(kind==='context')PM.GL.gl={};else PM.GL.contextLost=true;finish();await rejected;expect(PM.preparedVideoFrames).toBe(previous);
});

it('waits for initial decoded data and does not publish frames after cancellation', async () => {
  const video = Object.assign(new EventTarget(), { paused: true, seeking: false, readyState: 1, currentTime: 0, pause() {} });
  const layer = { id: 'clip', type: 'video', from: 0, d: { asset: 'source', speed: 1, trim: 0 } };
  const project = { layers: [layer], fps: 30 };
  const PM: any = { proj: project, active: () => true, scope: [], assets: { get: () => ({ el: video, w: 16, h: 16, dur: 2 }) } };
  const controller = new AbortController();
  const pending = prepareFrame(PM, 0, project, { signal: controller.signal });
  controller.abort(); await expect(pending).rejects.toHaveProperty('name', 'AbortError');
  video.readyState = 2; video.dispatchEvent(new Event('loadeddata'));
  await Promise.resolve();
  expect(PM.preparedVideoFrames).toBeUndefined();
});

function stalledVideoFixture(retryReady: boolean) {
  const h = videoFixture(), original = h.video('source', false);
  original.el.src = 'blob:source';
  h.PM.proj.layers = [{ ...h.clip('clip', 'source'), name: 'Interview' }];
  const retry = Object.assign(new EventTarget(), { name: 'retry', readyState: retryReady ? 2 : 1,
    seeking: false, currentTime: 0, pause: vi.fn(), load: vi.fn(), removeAttribute: vi.fn() });
  const create = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation(((tag: string) => tag === 'video' ? retry : create(tag)) as any);
  return { ...h, original, retry };
}

it('recovers a stalled decoder once using a fresh resource and releases it after copying pixels', async () => {
  vi.useFakeTimers();
  const h = stalledVideoFixture(true);
  const pending = prepareFrame(h.PM, 1, h.PM.proj, { videoFrameTimeoutMs: 200 });
  await vi.advanceTimersByTimeAsync(200); await pending;
  expect(h.draw).toHaveBeenCalledOnce();
  expect(h.draw).toHaveBeenCalledWith('retry', 1);
  expect((h.retry as any).src).toBe('blob:source');
  expect(h.retry.removeAttribute).toHaveBeenCalledWith('src');
  expect(h.retry.load).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it('bounds a permanently stalled frame to two attempts and reports clip and times without publishing partial frames', async () => {
  vi.useFakeTimers();
  const h = stalledVideoFixture(false), previous = new Map(); h.PM.preparedVideoFrames = previous;
  const pending = prepareFrame(h.PM, 1, h.PM.proj, { videoFrameTimeoutMs: 200 });
  const rejected = expect(pending).rejects.toThrow(/Interview.*timeline 1.000s.*source 1.000000s.*fresh decoder retry.*H.264 MP4/);
  await vi.advanceTimersByTimeAsync(400); await rejected;
  expect(h.PM.preparedVideoFrames).toBe(previous);
  expect(h.draw).not.toHaveBeenCalled();
  expect(h.retry.load).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it('cancels an export stuck decoding even without layer input effects', async () => {
  vi.useFakeTimers();
  const h = stalledVideoFixture(false); h.PM.Export = { busy: true, cancel: false };
  const pending = prepareFrame(h.PM, 1);
  const rejected = expect(pending).rejects.toHaveProperty('name', 'AbortError');
  h.PM.Export.cancel = true;
  await vi.advanceTimersByTimeAsync(100); await rejected;
  expect(h.draw).not.toHaveBeenCalled(); expect(h.retry.load).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it('reuses the recovered decoder on later export frames without retrying the stalled original', async () => {
  vi.useFakeTimers();
  const h = stalledVideoFixture(true), recoveryDecoders = new Map<HTMLVideoElement, HTMLVideoElement>();
  const options = { videoFrameTimeoutMs: 200, recoveryDecoders };
  const first = prepareFrame(h.PM, 1, h.PM.proj, options);
  await vi.advanceTimersByTimeAsync(200); await first;
  await prepareFrame(h.PM, 2, h.PM.proj, options);
  expect(h.original.el.seeks).toEqual([1]);
  expect(h.draw).toHaveBeenLastCalledWith('retry', 2);
  expect(recoveryDecoders.get(h.original.el)).toBe(h.retry);
  expect(h.retry.load).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});
