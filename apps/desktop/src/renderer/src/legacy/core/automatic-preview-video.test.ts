// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { createAutomaticPreviewVideos } from './automatic-preview-video';

type VideoOptions = { width?: number; height?: number; load?: boolean; present?: boolean; stale?: boolean; callbacks?: boolean };
class PrivateVideo extends EventTarget {
  muted = false; playsInline = false; preload = ''; crossOrigin: string | null = null;
  srcObject = null; paused = true; seeking = false; readyState = 0;
  videoWidth = 0; videoHeight = 0; pixelTime = 0; fps = 24;
  callbacks = new Map<number, VideoFrameRequestCallback>();
  private next = 0; private url = ''; private time = 0;
  constructor(readonly options: VideoOptions = {}) { super(); }
  get src() { return this.url; }
  get currentSrc() { return this.url; }
  set src(url: string) {
    this.url = url;
    queueMicrotask(() => {
      if (this.url !== url || this.options.load === false) return;
      this.readyState = 2; this.videoWidth = this.options.width || 64; this.videoHeight = this.options.height || 32;
      this.dispatchEvent(new Event('loadeddata'));
    });
  }
  get currentTime() { return this.time; }
  set currentTime(at: number) {
    this.seek(at); this.time = at; this.seeking = true;
    queueMicrotask(() => {
      if (!this.url) return;
      this.seeking = false;
      if (!this.options.stale) this.pixelTime = Math.floor(at * this.fps + 1e-7) / this.fps;
      this.dispatchEvent(new Event('seeked')); this.present();
    });
  }
  seek = vi.fn();
  pause = vi.fn(() => { this.paused = true; });
  play = vi.fn(() => { this.paused = false; queueMicrotask(() => this.present()); return Promise.resolve(); });
  removeAttribute = vi.fn((name: string) => { if (name === 'src') this.url = ''; });
  load = vi.fn();
  requestVideoFrameCallback(callback: VideoFrameRequestCallback) { this.callbacks.set(++this.next, callback); return this.next; }
  cancelVideoFrameCallback(id: number) { this.callbacks.delete(id); }
  present() {
    if (this.options.present === false || !this.url) return;
    const first = this.callbacks.entries().next().value;
    if (!first) return;
    const [id, callback] = first; this.callbacks.delete(id);
    callback(0, { mediaTime: this.pixelTime } as VideoFrameCallbackMetadata);
  }
}

function fixture() {
  const privateVideos: PrivateVideo[] = [];
  const nextOptions: VideoOptions[] = [];
  const snapshots: any[] = [];
  const bitmaps: any[] = [];
  const create = vi.fn((tag: string) => {
    if (tag !== 'video') throw new Error('Automatic capture must not use CPU canvases');
    const video = new PrivateVideo(nextOptions.shift());
    if (video.options.callbacks === false) (video as any).requestVideoFrameCallback = undefined;
    privateVideos.push(video); return video;
  });
  vi.stubGlobal('document', { createElement: create });
  vi.stubGlobal('VideoFrame', class {
    timestamp: number; duration: number; displayWidth: number; displayHeight: number;
    close = vi.fn(); src: string;
    constructor(video: PrivateVideo) {
      this.timestamp = video.pixelTime * 1e6; this.duration = 1e6 / video.fps;
      this.displayWidth = video.videoWidth; this.displayHeight = video.videoHeight; this.src = video.src;
      snapshots.push(this);
    }
  });
  vi.stubGlobal('createImageBitmap', vi.fn(async (video: PrivateVideo) => {
    expect(video).toBeInstanceOf(PrivateVideo); expect(video.paused).toBe(true); expect(video.seeking).toBe(false);
    const bitmap = { pixelTime: video.pixelTime, src: video.src,
      width: video.videoWidth, height: video.videoHeight, close: vi.fn() };
    bitmaps.push(bitmap); return bitmap;
  }));
  const source = new PrivateVideo(); source.src = 'blob:original';
  const layer: any = { id: 'clip', type: 'video', from: 0, dur: 10, d: { asset: 'asset', trim: 0, speed: 1 } };
  const asset: any = { el: source, fps: 24, dur: 10 };
  const PM: any = { proj: { w: 64, h: 32, fps: 24, layers: [layer], comps: {} }, scope: [],
    quality: 1, perf: { auto: false }, assets: new Map([['asset', asset]]),
    active: (item: any, time: number) => item.on !== false && time >= item.from && time < item.from + item.dur,
    clamp: (n: number, min: number, max: number) => Math.max(min, Math.min(max, n)),
    evP: (_: any, value: any) => value.v, animVersion: () => 1,
  };
  const cache = createAutomaticPreviewVideos(PM);
  return { PM, layer, asset, source, cache, privateVideos, nextOptions, snapshots, bitmaps, create };
}
const microtasks = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('captures the requested middle frame without seeking, playing or pausing the editing decoder', async () => {
  const { cache, source, privateVideos, snapshots, bitmaps, create } = fixture();
  const frames = await cache.prepare(2.5, () => true);
  expect((frames?.get('clip@2.5') as any).pixelTime).toBe(2.5);
  expect(source.seek).not.toHaveBeenCalled(); expect(source.play).not.toHaveBeenCalled(); expect(source.pause).not.toHaveBeenCalled();
  expect(privateVideos[0]?.muted).toBe(true); expect(privateVideos[0]?.playsInline).toBe(true);
  expect(privateVideos[0]?.paused).toBe(true);
  expect(createImageBitmap).toHaveBeenCalledWith(privateVideos[0], { premultiplyAlpha: 'premultiply' });
  expect(create).toHaveBeenCalledWith('video'); expect(snapshots.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
  expect(cache.bytes).toBe(64 * 32 * 4);
  cache.release(frames!); cache.release(frames!);
  expect(cache.bytes).toBe(0); expect(bitmaps[0].close).toHaveBeenCalledOnce();
  cache.dispose(); expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
});

it('freezes repeated nested instances independently and addresses them by complete path and nested time', async () => {
  const { PM, cache, layer, privateVideos } = fixture();
  PM.proj.comps.sub = { layers: [layer] };
  PM.proj.layers = [0, 1].map(i => ({ id: `instance${i}`, type: 'precomp', from: 0, dur: 5,
    d: { comp: 'sub', trim: i, speed: 2 } }));
  const frames = await cache.prepare(.25, () => true);
  expect([...frames!.keys()]).toEqual(['instance0/clip@0.5', 'instance1/clip@1.5']);
  expect([...frames!.values()].map(frame => (frame as any).pixelTime)).toEqual([.5, 1.5]);
  expect(frames!.get('instance0/clip@0.5')).not.toBe(frames!.get('instance1/clip@1.5'));
  expect(privateVideos).toHaveLength(1); expect(PM.scope).toEqual([]);
  cache.dispose();
});

it('uses the selected editing proxy without changing deferred-proxy bookkeeping or touching either live element', async () => {
  const { PM, asset, cache, source, privateVideos } = fixture();
  const proxy = new PrivateVideo(); proxy.src = 'blob:proxy';
  asset.preview = { el: proxy, deferred: true }; PM.quality = .5;
  const frames = await cache.prepare(1, () => true);
  expect((frames!.get('clip@1') as any).src).toBe('blob:proxy');
  expect(asset.preview.deferred).toBe(true);
  for (const live of [proxy, source]) {
    expect(live.seek).not.toHaveBeenCalled(); expect(live.play).not.toHaveBeenCalled(); expect(live.pause).not.toHaveBeenCalled();
  }
  cache.release(frames!); PM.quality = 1;
  const original = await cache.prepare(1, () => true);
  expect((original!.get('clip@1') as any).src).toBe('blob:original');
  expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
  cache.dispose();
});

it('reuses a paused decoded frame without waiting for a presentation that will never occur', async () => {
  const { cache, privateVideos } = fixture();
  const first = await cache.prepare(1, () => true); cache.release(first!);
  const video = privateVideos[0]!; const seeks = video.seek.mock.calls.length;
  video.options.present = false;
  const second = await cache.prepare(1, () => true);
  expect((second!.get('clip@1') as any).pixelTime).toBe(1);
  expect(video.seek).toHaveBeenCalledTimes(seeks); cache.dispose();
});

it('rejects stale decoded pixels even when readyState and seeked claim the requested time is ready', async () => {
  vi.useFakeTimers();
  const { cache, nextOptions, snapshots, bitmaps, privateVideos } = fixture();
  nextOptions.push({ stale: true });
  const pending = cache.prepare(2, () => true);
  await microtasks(); await vi.advanceTimersByTimeAsync(2000);
  expect(await pending).toBeNull(); expect(bitmaps).toEqual([]);
  expect(snapshots.length).toBeGreaterThan(0); expect(snapshots.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
  expect(privateVideos[0]?.callbacks.size).toBe(0); expect(cache.bytes).toBe(0);
  cache.dispose();
});

it('bounds loading waits and releases their private media on cancellation', async () => {
  const { cache, nextOptions, privateVideos, source } = fixture();
  nextOptions.push({ load: false });
  const pending = cache.prepare(1, () => true); await microtasks();
  cache.cancel(); expect(await pending).toBeNull();
  expect(privateVideos[0]?.src).toBe(''); expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
  expect(source.src).toBe('blob:original'); expect(source.pause).not.toHaveBeenCalled(); expect(cache.bytes).toBe(0);
  cache.dispose();
});

it('times out a private decoder that never presents instead of caching an unconfirmed frame', async () => {
  vi.useFakeTimers();
  const { cache, nextOptions, privateVideos, bitmaps } = fixture(); nextOptions.push({ present: false });
  const pending = cache.prepare(1, () => true); await microtasks(); await vi.advanceTimersByTimeAsync(2000);
  expect(await pending).toBeNull(); expect(bitmaps).toEqual([]); expect(privateVideos[0]?.callbacks.size).toBe(0);
  expect(privateVideos[0]?.load).toHaveBeenCalledOnce(); cache.dispose();
});

it('closes a late asynchronous bitmap after cancellation and releases its reserved memory immediately', async () => {
  const { cache, snapshots, privateVideos } = fixture();
  let finish!: (bitmap: ImageBitmap) => void;
  vi.mocked(createImageBitmap).mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const pending = cache.prepare(1, () => true); await microtasks();
  expect(createImageBitmap).toHaveBeenCalledOnce(); expect(cache.bytes).toBe(64 * 32 * 4);
  cache.cancel(); expect(cache.bytes).toBe(0); expect(await pending).toBeNull();
  const bitmap = { close: vi.fn() }; finish(bitmap as any); await microtasks();
  expect(bitmap.close).toHaveBeenCalledOnce(); expect(snapshots[0].close).toHaveBeenCalledOnce();
  expect(privateVideos[0]?.callbacks.size).toBe(0); cache.dispose();
});

it('discards frozen output when an edit invalidates the preparation before publication', async () => {
  const { cache, bitmaps } = fixture(); let valid = true;
  const capture = vi.mocked(createImageBitmap).getMockImplementation()!;
  vi.mocked(createImageBitmap).mockImplementation(async (...args: any[]) => {
    const bitmap = await (capture as any)(...args); valid = false; return bitmap;
  });
  expect(await cache.prepare(1, () => valid)).toBeNull();
  expect(bitmaps[0].close).toHaveBeenCalledOnce(); expect(cache.bytes).toBe(0); cache.dispose();
});

it('keeps at most four private sources and prunes sources that leave the active clip list', async () => {
  const { PM, cache, privateVideos } = fixture();
  PM.proj.layers = Array.from({ length: 6 }, (_, i) => {
    const source = new PrivateVideo(); source.src = `blob:source-${i}`;
    PM.assets.set(`asset-${i}`, { el: source, fps: 24, dur: 10 });
    return { id: `clip-${i}`, type: 'video', from: 0, dur: 10, d: { asset: `asset-${i}`, speed: 1, trim: 0 } };
  });
  const first = await cache.prepare(1, () => true); expect(first!.size).toBe(6);
  expect(privateVideos.filter(video => video.src)).toHaveLength(4); cache.release(first!);
  PM.proj.layers = PM.proj.layers.slice(-1);
  const second = await cache.prepare(2, () => true);
  expect(second!.size).toBe(1); expect(privateVideos.filter(video => video.src)).toHaveLength(1);
  cache.dispose(); expect(privateVideos.every(video => !video.src)).toBe(true);
});

it('reserves temporary source images and final output within the shared preview budget', async () => {
  const { PM, layer, cache, nextOptions, snapshots, bitmaps } = fixture();
  PM.proj.layers.push({ ...layer, id: 'copy', d: { ...layer.d, trim: 1 } });
  PM.Memory = { budget: () => 30_000 }; PM.GL = { canvas: { width: 64, height: 32 }, previewFrames: { bytes: 8000 } };
  PM.Preview = { bytes: 1000 }; nextOptions.push({ width: 64, height: 32 });
  // 30,000 - 8,000 cached - 1,000 prepared - 8,192 output leaves 12,808.
  // One source snapshot fits; a second copy of its pixels must be rejected.
  expect(await cache.prepare(1, () => true)).toBeNull();
  expect(snapshots).toHaveLength(1); expect(bitmaps[0].close).toHaveBeenCalledOnce(); expect(cache.bytes).toBe(0);
  cache.dispose();
});

it('rejects a single oversize source before creating a snapshot, even with a large user memory limit', async () => {
  const { PM, cache, nextOptions, snapshots } = fixture();
  PM.Memory = { budget: () => 2 * 1024 * 1024 * 1024 }; nextOptions.push({ width: 8192, height: 8192 });
  expect(await cache.prepare(1, () => true)).toBeNull(); expect(snapshots).toEqual([]); expect(cache.bytes).toBe(0);
  cache.dispose();
});

it('returns no prepared video frames for unsupported sources or missing presentation APIs', async () => {
  const { cache, nextOptions, source, privateVideos } = fixture();
  nextOptions.push({ callbacks: false });
  expect(await cache.prepare(1, () => true)).toBeNull(); expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
  (source as any).srcObject = {};
  expect(await cache.prepare(1, () => true)).toBeNull(); expect(privateVideos).toHaveLength(1); cache.dispose();
});

it('supersedes unfinished work safely and never starts new media after disposal', async () => {
  const { cache, nextOptions, privateVideos } = fixture(); nextOptions.push({ load: false });
  const first = cache.prepare(1, () => true); await microtasks();
  const second = cache.prepare(2, () => true);
  expect(await first).toBeNull(); expect((await second)!.get('clip@2')).toBeDefined();
  expect(privateVideos[0]?.src).toBe(''); expect(privateVideos[1]?.src).toBe('blob:original');
  cache.dispose(); cache.dispose(); expect(cache.bytes).toBe(0);
  expect(await cache.prepare(3, () => true)).toBeNull(); expect(privateVideos).toHaveLength(2);
});

it('returns an empty frame map without allocating private media when no video is active', async () => {
  const { cache, layer, create } = fixture(); layer.on = false;
  const frames = await cache.prepare(1, () => true);
  expect(frames!.size).toBe(0); expect(create).not.toHaveBeenCalled(); expect(cache.bytes).toBe(0); cache.dispose();
});

it('uses compositor requests for hidden mattes and freezes repeated identical matte requests once', async () => {
  const { PM, cache, layer, asset, source, bitmaps } = fixture(); layer.on = false;
  PM.time = .125; PM.beginEval = vi.fn();
  const request = { key: 'nested/hidden-matte@2.5', asset, at: .5 };
  const collect = vi.fn(() => { PM.beginEval(2.5); return [request, { ...request }]; });
  PM.GL = { thumbnailVideoRequests: collect };
  const frames = await cache.prepare(2.5, () => true);
  expect(collect).toHaveBeenCalledWith(PM.proj, 2.5);
  expect([...frames!.keys()]).toEqual(['nested/hidden-matte@2.5']);
  expect((frames!.get(request.key) as any).pixelTime).toBe(.5);
  expect(PM.beginEval.mock.calls).toEqual([[2.5], [.125]]);
  expect(bitmaps).toHaveLength(1); expect(cache.bytes).toBe(64 * 32 * 4);
  expect(source.seek).not.toHaveBeenCalled(); expect(source.play).not.toHaveBeenCalled(); expect(source.pause).not.toHaveBeenCalled();
  cache.dispose();
});

it('restores the current evaluation time immediately after collecting a future frame, before loading yields', async () => {
  const { PM, cache, asset, nextOptions } = fixture(); nextOptions.push({ load: false });
  PM.time = .125; PM.beginEval = vi.fn();
  PM.GL = { thumbnailVideoRequests: () => {
    PM.beginEval(2); return [{ key: 'clip@2', asset, at: 2 }];
  } };
  const pending = cache.prepare(2, () => true);
  expect(PM.beginEval.mock.calls).toEqual([[2], [.125]]);
  cache.cancel(); expect(await pending).toBeNull(); cache.dispose();
});

it.each(['asset', 'time'])('declines conflicting duplicate compositor requests with a different %s', async change => {
  const { PM, cache, asset, create } = fixture();
  const request = { key: 'shared-matte@1', asset, at: 1 };
  PM.GL = { thumbnailVideoRequests: () => [request, {
    ...request, ...(change === 'asset' ? { asset: { ...asset } } : { at: 2 }),
  }] };
  expect(await cache.prepare(1, () => true)).toBeNull(); expect(create).not.toHaveBeenCalled();
  expect(cache.bytes).toBe(0); cache.dispose();
});

it('restores evaluation time even when the compositor source collector throws', async () => {
  const { PM, cache, create } = fixture(); PM.time = .25; PM.beginEval = vi.fn();
  PM.GL = { thumbnailVideoRequests: () => { PM.beginEval(2); throw new Error('invalid composition'); } };
  expect(await cache.prepare(2, () => true)).toBeNull();
  expect(PM.beginEval.mock.calls).toEqual([[2], [.25]]); expect(create).not.toHaveBeenCalled(); cache.dispose();
});

it('unloads decoder codec buffers after idle warming ends while preserving caller-owned frozen frames', async () => {
  vi.useFakeTimers();
  const { cache, privateVideos, bitmaps } = fixture();
  const frames = await cache.prepare(1, () => true); const video = privateVideos[0]!;
  await vi.advanceTimersByTimeAsync(1999); expect(video.src).toBe('blob:original');
  await vi.advanceTimersByTimeAsync(1); expect(video.src).toBe(''); expect(video.load).toHaveBeenCalledOnce();
  expect((frames!.get('clip@1') as any).pixelTime).toBe(1); expect(bitmaps[0].close).not.toHaveBeenCalled();
  expect(cache.bytes).toBe(64 * 32 * 4);
  cache.release(frames!); expect(cache.bytes).toBe(0); expect(bitmaps[0].close).toHaveBeenCalledOnce(); cache.dispose();
});

it('extends decoder reuse across nearby preparation jobs and cancels the inactivity release cleanly', async () => {
  vi.useFakeTimers();
  const { cache, privateVideos } = fixture();
  const first = await cache.prepare(1, () => true); cache.release(first!);
  await vi.advanceTimersByTimeAsync(1500);
  const second = await cache.prepare(1.5, () => true); cache.release(second!);
  expect(privateVideos).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1000); expect(privateVideos[0]?.src).toBe('blob:original');
  cache.cancel(); expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(3000); expect(privateVideos[0]?.load).toHaveBeenCalledOnce();
  cache.dispose();
});

it('uses the exact shutter planner and freezes timestamp-validated paused seeks before playback skips them', async () => {
 const {PM,cache,asset,source,privateVideos,bitmaps}=fixture();
 const options={mblur:true,mbSamples:4,shutter:1};
 PM.GL={frameVideoRequests:vi.fn(()=>[.5-.0001,.5,.625].map(at=>({key:`group/clip@${at}`,asset,at})))};
 const frames=await cache.prepare(.5,()=>true,options);
 expect(PM.GL.frameVideoRequests).toHaveBeenCalledWith(PM.proj,.5,options);
 expect([...frames!.keys()]).toEqual(['group/clip@0.4999','group/clip@0.5','group/clip@0.625']);
 expect(bitmaps.map(bitmap=>bitmap.pixelTime)).toEqual([11/24,.5,.625]);
 expect(privateVideos[0]!.play).not.toHaveBeenCalled();
 expect(source.seek).not.toHaveBeenCalled();expect(cache.error).toBeNull();
 cache.release(frames!);expect(cache.bytes).toBe(0);cache.dispose();
});

it('diagnoses and releases an exact private frame that never covers the requested timestamp', async () => {
 vi.useFakeTimers();const {PM,cache,asset,privateVideos}=fixture();
 PM.GL={frameVideoRequests:()=>[{key:'clip@2',asset,at:2}]};
 // A seek can finish while the decoded frame is still stale.
 const create=document.createElement as any;create.mockImplementationOnce(()=>{const video=new PrivateVideo({stale:true});privateVideos.push(video);return video;});
 const pending=cache.prepare(2,()=>true,{mblur:true});await microtasks();await vi.advanceTimersByTimeAsync(2000);
 expect(await pending).toBeNull();expect(cache.error).toContain('did not decode 2');expect(cache.bytes).toBe(0);
 expect(privateVideos[0]!.load).toHaveBeenCalledOnce();cache.dispose();
});


it('allows full-HD-sized exact shutter sets above the idle ceiling within shared and texture budgets', async () => {
 const {PM,cache,asset,nextOptions}=fixture();nextOptions.push({width:1500,height:1000});
 PM.Memory={budget:(name:string)=>name==='textures'?128*1024*1024:256*1024*1024};
 PM.GL={frameVideoRequests:()=>Array.from({length:17},(_,i)=>({key:`clip@${i/24}`,asset,at:i/24}))};
 const frames=await cache.prepare(.5,()=>true,{mblur:true,mbSamples:16});
 expect(frames?.size).toBe(17);expect(cache.bytes).toBe(102000000);cache.release(frames!);expect(cache.bytes).toBe(0);cache.dispose();
});
