// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { prepareThumbnailVideoFrames } from './thumbnail-video-frames';
import { install as installCompositor } from '../legacy/gl/compositor';
import { makePM } from '../legacy/__tests__/make-pm';

let decoders: PrivateVideo[] = [], canvases: HTMLCanvasElement[] = [];
let deferSeek = false;
const copies: Array<{ time: number; source: string; mode: string }> = [];

class PrivateVideo extends EventTarget {
  videoWidth = 16; videoHeight = 8; readyState = 0; seeking = false;
  muted = false; playsInline = false; preload = ''; currentSrc = '';
  private position = 0;
  get src() { return this.currentSrc; }
  set src(value: string) {
    this.currentSrc = value; this.readyState = 0; this.position = 0;
    Promise.resolve().then(() => { this.readyState = 2; this.dispatchEvent(new Event('loadeddata')); });
  }
  get currentTime() { return this.position; }
  set currentTime(value: number) {
    this.position = value; this.seeking = true; this.readyState = 1;
    if (!deferSeek) Promise.resolve().then(() => this.finish());
  }
  finish() { this.seeking = false; this.readyState = 2; this.dispatchEvent(new Event('seeked')); }
  pause = vi.fn(); load = vi.fn();
  removeAttribute = vi.fn(() => { this.currentSrc = ''; });
}

beforeEach(() => {
  vi.useFakeTimers(); decoders = []; canvases = []; copies.length = 0; deferSeek = false;
  const native = document.createElement.bind(document);
  vi.spyOn(document, 'createElement').mockImplementation((tag: string) => {
    if (tag === 'video') { const decoder = new PrivateVideo(); decoders.push(decoder); return decoder as any; }
    const element = native(tag); if (tag === 'canvas') canvases.push(element as HTMLCanvasElement); return element;
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => {
    const context = { globalCompositeOperation: 'source-over', drawImage(video: PrivateVideo) {
      copies.push({ time: video.currentTime, source: video.currentSrc, mode: context.globalCompositeOperation });
    } };
    return context as any;
  });
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const layer = (id: string, asset = 'source', from = 0, trim = 0, speed = 1) => ({
  id, type: 'video', from, dur: 10, d: { asset, trim, speed },
});
function fixture(layers = [layer('clip')]) {
  const original: any = { src: 'blob:original', currentSrc: 'blob:original', currentTime: .8, videoWidth: 16, videoHeight: 8, pause: vi.fn() };
  const asset = { el: original, preview: { el: { src: 'blob:proxy' } }, dur: 20, w: 16, h: 8 };
  const PM: any = { proj: { fps: 30, layers, comps: {} }, scope: [], assets: { get: () => asset },
    quality: .25, perf: { auto: true }, TYPE_META: {}, FX: {},
    clamp: (value: number, low: number, high: number) => Math.max(low, Math.min(high, value)),
    ev: (item: any, key: string) => item.p?.[key]?.v ?? 100,
    active: (item: any, time: number) => item.on !== false && time >= item.from && time < item.from + item.dur };
  PM.curComp = () => PM.scope.at(-1) || PM.proj;
  PM.compOf = (item: any) => PM.curComp().comps?.[item.d.comp] || PM.proj.comps?.[item.d.comp];
  PM.groupAncestors = (item: any, items = PM.curComp().layers) => {
    const ancestors: any[] = [], seen = new Set();
    while (item.group && !seen.has(item.group)) {
      seen.add(item.group); item = items.find((entry: any) => entry.id === item.group);
      if (!item) break; ancestors.push(item);
    }
    return ancestors;
  };
  installCompositor(PM);
  return { PM, original, asset };
}
const flush = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };

describe('private thumbnail video frames', () => {
  it('captures the original poster frame without seeking, pausing or replacing the viewer decoder', async () => {
    const { PM, original } = fixture();
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 1.5, new AbortController().signal)!;
    expect(copies).toEqual([{ time: 1.5, source: 'blob:original', mode: 'copy' }]);
    expect([...result.frames.keys()]).toEqual(['clip@1.5']);
    expect(original.currentTime).toBe(.8); expect(original.pause).not.toHaveBeenCalled();
    expect(PM.scope).toEqual([]); expect(PM).not.toHaveProperty('preparedVideoFrames');
    expect(decoders).toHaveLength(1); expect(decoders[0]?.removeAttribute).toHaveBeenCalledWith('src');
    expect(decoders[0]?.load).toHaveBeenCalledOnce();
    result.dispose(); expect(canvases[0]?.width).toBe(0); expect(canvases[0]?.height).toBe(0);
    expect(result.frames.size).toBe(0); expect(vi.getTimerCount()).toBe(0);
  });

  it('maps trimmed, accelerated and nested source times, skips inactive clips and deduplicates equal requests', async () => {
    const { PM } = fixture([layer('first', 'source', 1, 2, 2), layer('same', 'source', 1, 2, 2), layer('late', 'source', 8)]);
    const nested = { id: 'nested', layers: [layer('inside', 'source', 0, 1, .5)] };
    PM.proj.comps.nested = nested;
    PM.proj.layers.push({ id: 'parent', type: 'precomp', from: 0, dur: 10, d: { comp: 'nested', trim: 1, speed: 2 } });
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect(copies.map(copy => copy.time).sort()).toEqual([3.5, 4]);
    expect([...result.frames.keys()].sort()).toEqual(['first@2', 'parent/inside@5', 'same@2']);
    expect(result.frames.get('first@2')).toBe(result.frames.get('same@2'));
    expect(result.frames.get('parent/inside@5')).not.toBe(result.frames.get('first@2'));
    expect(decoders).toHaveLength(1); expect(PM.scope).toEqual([]);
    result.dispose(); expect(canvases.every(canvas => canvas.width === 0)).toBe(true);
  });

  it('keeps different times of the same nested video in separate frame keys', async () => {
    const { PM } = fixture([]);
    PM.proj.comps.inner = { layers: [layer('video')] };
    PM.proj.layers = [0, 1].map((trim, index) => ({ id: 'parent-' + index, type: 'precomp', from: 0, dur: 10, d: { comp: 'inner', trim, speed: 1 } }));
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect([...result.frames.keys()].sort()).toEqual(['parent-0/video@2', 'parent-1/video@3']);
    expect(copies.map(copy => copy.time).sort()).toEqual([2, 3]); result.dispose();
  });

  it('captures all requested pixels even when their footprint exceeds the soft texture cache budget', async () => {
    const { PM } = fixture([layer('a'), layer('b', 'source', 0, 1)]);
    PM.Memory = { budget: () => 1 };
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect(result.frames.size).toBe(2); expect(copies.map(copy => copy.time).sort()).toEqual([2, 3]);
    result.dispose(); expect(canvases.every(canvas => canvas.width === 0)).toBe(true);
    expect(decoders[0]?.load).toHaveBeenCalledOnce(); expect(vi.getTimerCount()).toBe(0);
  });

  it('aborts a pending seek, unloads the private decoder and allows the next queued capture', async () => {
    deferSeek = true;
    const { PM } = fixture(), abort = new AbortController();
    const first = prepareThumbnailVideoFrames(PM, PM.proj, 2, abort.signal)!;
    const rejected = expect(first).rejects.toThrow('cancelled');
    const next = prepareThumbnailVideoFrames(PM, PM.proj, 3, new AbortController().signal)!;
    await flush(); expect(decoders).toHaveLength(1);
    deferSeek = false; abort.abort(); await rejected;
    await flush();
    const result = await next;
    expect(decoders).toHaveLength(2); expect(copies.map(copy => copy.time)).toEqual([3]);
    expect(decoders.every(decoder => decoder.load.mock.calls.length === 1)).toBe(true);
    expect(vi.getTimerCount()).toBe(0); result.dispose();
  });

  it('cleans up a failed pixel copy and does not poison the queue', async () => {
    const { PM } = fixture();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValueOnce({ drawImage() { throw new Error('copy failed'); } } as any);
    await expect(prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)).rejects.toThrow('copy failed');
    expect(canvases[0]?.width).toBe(0); expect(decoders[0]?.load).toHaveBeenCalledOnce();
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 3, new AbortController().signal)!;
    result.dispose(); expect(vi.getTimerCount()).toBe(0);
  });

  it('keeps static composition work synchronous and allocates no cancellation or media resources', () => {
    const { PM } = fixture([]), signal = vi.fn(() => new AbortController().signal);
    expect(prepareThumbnailVideoFrames(PM, PM.proj, 2, signal)).toBeNull();
    expect(signal).not.toHaveBeenCalled(); expect(decoders).toHaveLength(0); expect(canvases).toHaveLength(0);
  });

  it('preserves native decoded frames without RGBA copies and budgets their actual format', async () => {
    const native: any[] = [];
    vi.stubGlobal('VideoFrame', class {
      displayWidth: number; displayHeight: number; time: number; close = vi.fn();
      constructor(video: PrivateVideo) {
        this.displayWidth = video.videoWidth; this.displayHeight = video.videoHeight; this.time = video.currentTime; native.push(this);
      }
      allocationSize() { return this.displayWidth * this.displayHeight * 1.5; }
    });
    const { PM } = fixture([layer('first'), layer('second', 'source', 0, 1)]);
    PM.Memory = { budget: () => 512 };
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect(native.map(frame => frame.time).sort()).toEqual([2, 3]);
    expect((result.frames.get('first@2') as any)?.time).toBe(2); expect((result.frames.get('second@2') as any)?.time).toBe(3);
    expect(canvases).toHaveLength(0); expect(copies).toEqual([]);
    result.dispose(); result.dispose(); expect(native.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
  });

  it('captures and releases native frames above the configured soft budget', async () => {
    const native: any[] = [];
    vi.stubGlobal('VideoFrame', class {
      close = vi.fn(); constructor() { native.push(this); } allocationSize() { return 512; }
    });
    const { PM } = fixture([layer('first'), layer('second', 'source', 0, 1)]);
    PM.Memory = { budget: () => 1 };
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect(result.frames.size).toBe(2); result.dispose();
    expect(native).toHaveLength(2); expect(native.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
    expect(decoders[0]?.load).toHaveBeenCalledOnce(); expect(vi.getTimerCount()).toBe(0);
  });

  it('fits ten distinct native 4K frames in the existing128MiB texture budget without RGBA copies', async () => {
    const native: any[] = [];
    vi.stubGlobal('VideoFrame', class {
      displayWidth = 3840; displayHeight = 2160; close = vi.fn();
      constructor() { native.push(this); }
      allocationSize() { return this.displayWidth * this.displayHeight * 1.5; }
    });
    const { PM, asset } = fixture(Array.from({ length: 10 }, (_, index) => layer('clip-' + index, 'source', 0, index)));
    asset.w = 3840; asset.h = 2160;
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 1, new AbortController().signal)!;
    expect(result.frames.size).toBe(10); expect(decoders).toHaveLength(1); expect(canvases).toHaveLength(0);
    expect(native.reduce((bytes, frame) => bytes + frame.allocationSize(), 0)).toBeLessThan(128 * 1024 * 1024);
    result.dispose(); expect(native.every(frame => frame.close.mock.calls.length === 1)).toBe(true);
  });

  it('captures hidden video matte sources in the compositor’s forced-on scope', async () => {
    const { PM } = fixture([layer('matte'), layer('visible')]);
    PM.proj.layers[0].on = false; PM.proj.layers[1].matteSource = 'matte';
    PM.active = (item: any) => item.on !== false;
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect([...result.frames.keys()].sort()).toEqual(['matte@2', 'visible@2']);
    expect(PM.proj.layers[0].on).toBe(false); expect(PM.scope).toEqual([]);
    result.dispose();
  });

  it('isolates imported nested compositions with equal child identifiers and local times', async () => {
    const { PM, asset } = fixture([]);
    const other = { ...asset, el: { ...asset.el, currentSrc: 'blob:other', src: 'blob:other' } };
    PM.assets.get = (id: string) => id === 'other' ? other : asset;
    PM.proj.comps.first = { layers: [layer('shared')] };
    PM.proj.comps.second = { layers: [layer('shared', 'other')] };
    PM.proj.layers = ['first', 'second'].map(comp => ({ id: comp, type: 'precomp', from: 0, dur: 10, d: { comp } }));
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect([...result.frames.keys()].sort()).toEqual(['first/shared@2', 'second/shared@2']);
    expect(result.frames.get('first/shared@2')).not.toBe(result.frames.get('second/shared@2'));
    expect(copies.map(copy => copy.source).sort()).toEqual(['blob:original', 'blob:other']);
    result.dispose();
  });

  it('falls back to a full-resolution private canvas when native snapshots reject the source', async () => {
    vi.stubGlobal('VideoFrame', class { constructor() { throw new Error('unsupported snapshot'); } });
    const { PM, original } = fixture();
    const result = await prepareThumbnailVideoFrames(PM, PM.proj, 2, new AbortController().signal)!;
    expect(copies).toEqual([{ time: 2, source: 'blob:original', mode: 'copy' }]);
    expect(canvases[0]?.width).toBe(16); expect(canvases[0]?.height).toBe(8);
    expect(original.currentTime).toBe(.8); result.dispose(); expect(canvases[0]?.width).toBe(0);
  });

  it('plans precomp retiming in the nested scope used by the real expression evaluator', () => {
    const PM: any = makePM('core/easing', 'core/model', 'core/anim', 'gl/compositor');
    PM.proj = PM.mkProject({ dur: 10 }); PM.proj.params.gain = { value: 2 };
    const sub = PM.mkProject({ dur: 10 }); sub.params.gain = { value: 4 };
    const video = PM.mkLayer('video', { d: { asset: 'source' }, from: 0, dur: 10 }, sub); sub.layers = [video];
    PM.proj.comps[sub.id] = sub;
    const parent = PM.mkLayer('precomp', { d: { comp: sub.id, timeRemap: PM.P(true), sourceTime: PM.P(0) } });
    parent.d.sourceTime.expr = 'param("gain")'; PM.proj.layers = [parent];
    const asset = { el: {}, dur: 20 }; PM.assets.get = () => asset;
    expect(PM.GL.thumbnailVideoRequests(PM.proj, 2)).toEqual([{ key: parent.id + '/' + video.id + '@4', asset, at: 4 }]);
    expect(PM.scope).toEqual([]); expect(PM.expressionErrors.get(parent.d.sourceTime)).toBeUndefined();
  });

  it('plans matte retiming in the full composition scope, including an otherwise hidden source', () => {
    const PM: any = makePM('core/easing', 'core/model', 'core/anim', 'gl/compositor');
    PM.proj = PM.mkProject({ dur: 10 });
    const matte = PM.mkLayer('video', { d: { asset: 'source', timeRemap: PM.P(true), sourceTime: PM.P(0) } });
    matte.on = false; matte.d.sourceTime.expr = 'comp.layers.length';
    const plate = PM.mkLayer('solid'), other = PM.mkLayer('solid'); plate.matteSource = matte.id;
    PM.proj.layers = [matte, plate, other];
    const asset = { el: {}, dur: 20 }; PM.assets.get = () => asset;
    expect(PM.GL.thumbnailVideoRequests(PM.proj, 2)).toEqual([{ key: matte.id + '@2', asset, at: 3 }]);
    expect(PM.scope).toEqual([]); expect(matte.on).toBe(false); expect(PM.expressionErrors.get(matte.d.sourceTime)).toBeUndefined();
  });

  it('excludes solo-hidden, zero-opacity and zero-opacity grouped videos from capture ownership', () => {
    const PM: any = makePM('core/easing', 'core/model', 'core/anim', 'gl/compositor');
    PM.proj = PM.mkProject({ dur: 10 });
    const visible = PM.mkLayer('video', { d: { asset: 'source' } }); visible.solo = true;
    const hidden = Array.from({ length: 12 }, () => PM.mkLayer('video', { d: { asset: 'large' } }));
    PM.proj.layers = [visible, ...hidden]; PM.assets.get = (id: string) => ({ el: {}, id, dur: 20 });
    expect(PM.GL.thumbnailVideoRequests(PM.proj, 2).map((request: any) => request.key)).toEqual([visible.id + '@2']);
    visible.solo = false;
    const zero = PM.mkLayer('video', { d: { asset: 'large' }, p: { opacity: 0 } });
    const group = PM.mkLayer('group', { p: { opacity: 0 } });
    const child = PM.mkLayer('video', { d: { asset: 'large' } }); child.group = group.id;
    PM.proj.layers = [visible, zero, group, child]; PM.ProjectIndex.invalidate(); PM.touch();
    expect(PM.GL.thumbnailVideoRequests(PM.proj, 2).map((request: any) => request.key)).toEqual([visible.id + '@2']);
    expect(PM.scope).toEqual([]);
  });
});
