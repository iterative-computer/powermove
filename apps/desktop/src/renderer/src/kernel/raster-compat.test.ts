import { describe, expect, it, vi } from 'vitest';
import { createServicesRegistry } from './services';
import { createRasterCompat, findLayer, TICKS_PER_SECOND } from './raster-compat';
import { EngineRenderTransport, type TransportSession } from '../engine-connection/render-transport';
import type { CompatNeed } from '../../../shared/engine-protocol';

const text = { id: 'title', type: 'text', d: { text: 'Hello' }, fx: [] as any[] };
const shape = { id: 'box', type: 'shape', d: { shape: 'rect', w: 40, h: 20 }, fx: [] as any[] };
const captions = { id: 'subs', type: 'captions', d: {}, fx: [] as any[] };
const nested = { id: 'inner', type: 'text', d: { text: 'Nested' }, fx: [] as any[] };

function setup(withAsync = false, rasterKey?: (layer: any, scale: number, time: number, crop?: any) => string | null) {
  const PM: any = {
    time: 0,
    proj: { id: 'P', compId: 'P-root', layers: [text, shape, captions], comps: { inner: { id: 'inner', layers: [nested] } } },
    bus: { on: vi.fn(() => () => {}) }
  };
  const services = createServicesRegistry();
  const surface = (layer: any, scale: number, time: number) => ({
    cv: { width: Math.round(48 * scale), height: Math.round(24 * scale) }, w: 48, h: 24, anchorX: 4, anchorY: 18, layer, scale, time, key: `engine:${layer.id}`
  });
  const engineRaster = vi.fn(surface);
  const engineRasterAsync = vi.fn(async (layer: any, scale: number, time: number) => surface(layer, scale, time));
  const readPixels = vi.fn((source: { width: number; height: number }) => ({ width: source.width, height: source.height, format: 'rgba8Premultiplied' as const, data: new Uint8Array(source.width * source.height * 4).fill(7) }));
  let base: any;
  const compat = createRasterCompat(PM, {
    isBase: (fn) => fn === base,
    chain: () => services.get('raster'),
    onChainChange: (fn) => services.onChange('raster', fn),
    engineRaster,
    engineRasterAsync: withAsync ? engineRasterAsync : undefined,
    rasterKey,
    readPixels,
    log: () => {}
  });
  base = (...args: any[]) => compat.producing ? compat.base(args as any) : null;
  services.register('raster', base);
  const need = (layer: string, time = 0.5, scale = 2, composition = 'P-root'): CompatNeed =>
    ({ composition, layer, time: Math.round(time * TICKS_PER_SECOND), source: 'rasterOverride', scale });
  return { PM, services, compat, engineRaster, engineRasterAsync, readPixels, base, need };
}

describe('the raster override producer', () => {
  it('reuses a native-evaluated unchanged base across playback times while wrappers still run at each time', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true, (layer, scale, _time, crop) => JSON.stringify([layer.d, scale, crop]));
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, scale: number, time: number, ...rest: any[]) => ({ ...original(layer, scale, time, ...rest), key: `animated-wrapper:${time}` }));
    expect(compat.answer(need('box', 0), 4)).toMatchObject({ key: 'animated-wrapper:0' });
    await compat.prepare(need('box', 1), 4);
    expect(compat.answer(need('box', 1), 4, { prepared: true })).toMatchObject({ key: 'animated-wrapper:1' });
    expect(engineRaster).toHaveBeenCalledTimes(1);
    expect(engineRasterAsync).not.toHaveBeenCalled();
    // A new document revision invalidates content-identical cached pixels.
    await compat.prepare(need('box', 1.5), 5);
    expect(engineRasterAsync).toHaveBeenCalledTimes(1);
  });

  it('reuses a detached copy\'s content-identical base across frames instead of a synchronous engine raster each frame', async () => {
    // A typewriter: the source layer (prepared) and a detached typed prefix
    // rasterized through the service. Today's cache keys both by content.
    const key = (layer: any, scale: number, _time: number, crop?: any) => JSON.stringify([layer.id === 'title' ? 'own' : 'detached', layer.d, scale, crop ?? null]);
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true, key);
    const original = services.get<any>('raster');
    const prefix = (time: number) => (time < 1 ? 'He' : 'Hel');
    services.register('raster', (layer: any, scale: number, time: number, ...rest: any[]) => {
      if (layer.id !== 'title') return original(layer, scale, time, ...rest);
      const full = original(layer, scale, time, ...rest);
      const typed = original({ ...layer, id: `title:typed`, d: { ...layer.d, text: prefix(time) } }, scale, time, ...rest);
      return { ...full, cv: { width: full.cv.width + typed.cv.width, height: full.cv.height }, key: `typed:${time}` };
    });
    for (const time of [0.2, 0.4, 0.6]) {
      await compat.prepare(need('title', time), 4);
      expect(compat.answer(need('title', time), 4, { prepared: true })).toMatchObject({ kind: 'raster', key: `typed:${time}` });
    }
    // Both source and detached prefix are asynchronous. The prefix is
    // reused only while its content is identical, then fetched anew.
    expect(engineRasterAsync).toHaveBeenCalledTimes(2);
    expect(engineRaster).not.toHaveBeenCalled();
    expect(engineRasterAsync.mock.calls[1]![0]).toMatchObject({ d: { text: 'He' } });
    await compat.prepare(need('title', 1.2), 4);
    compat.answer(need('title', 1.2), 4, { prepared: true });
    expect(engineRasterAsync).toHaveBeenCalledTimes(3);
    expect(engineRasterAsync.mock.calls[2]![0]).toMatchObject({ d: { text: 'Hel' } });
    expect(compat.stats).toMatchObject({ syncLoads: 0, asyncLoads: 3 });
    await compat.prepare(need('title', 1.4), 5);
    compat.answer(need('title', 1.4), 5, { prepared: true });
    expect(engineRasterAsync).toHaveBeenCalledTimes(5);
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('discovers a newly read base asynchronously even after the chain passed it through', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true);
    const original = services.get<any>('raster');
    let tint = false;
    services.register('raster', (layer: any, ...rest: any[]) => {
      const base = original(layer, ...rest);
      return tint ? { ...base, cv: base.cv, key: 'tinted' } : base;
    });
    for (const time of [0.2, 0.4]) {
      await compat.prepare(need('box', time), 4);
      expect(compat.answer(need('box', time), 4, { prepared: true })).toEqual({ kind: 'native' });
    }
    expect(engineRasterAsync).not.toHaveBeenCalled();
    tint = true;
    await compat.prepare(need('box', 0.6), 4);
    expect(compat.answer(need('box', 0.6), 4, { prepared: true })).toMatchObject({ kind: 'raster', key: 'tinted' });
    await compat.prepare(need('box', 0.8), 4);
    expect(compat.answer(need('box', 0.8), 4, { prepared: true })).toMatchObject({ kind: 'raster', key: 'tinted' });
    expect(engineRasterAsync).toHaveBeenCalledTimes(2);
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('never publishes a result from a wrapper that caught a suspended base read', async () => {
    const { services, compat, engineRaster, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => {
      try { return { ...original(layer, ...rest), key: 'real-pixels' }; }
      catch { return { cv: { width: 1, height: 1 }, key: 'caught-missing' }; }
    });
    await compat.prepare(need('title'), 4);
    expect(compat.answer(need('title'), 4, { prepared: true })).toMatchObject({ key: 'real-pixels', width: 96 });
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('rejects an incomplete result a wrapper cached while catching a suspended read', async () => {
    const { services, compat, need } = setup(true);
    const original = services.get<any>('raster');
    let cached: any;
    services.register('raster', (layer: any, ...rest: any[]) => {
      if (cached) return cached;
      try { return { ...original(layer, ...rest), key: 'real' }; }
      catch { return cached = { cv: { width: 1, height: 1 }, key: 'incomplete' }; }
    });
    await expect(compat.prepare(need('title'), 4)).rejects.toThrow('cached a result before its base pixels');
    expect(compat.answer(need('title'), 4, { prepared: true })).toBeNull();
  });

  it('lets a ready sibling finish while another detached base is slow, and runs a completed wrapper only once', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true);
    const original = services.get<any>('raster');
    const finished = vi.fn();
    services.register('raster', (layer: any, scale: number, time: number) => {
      const detached = { ...layer, id: `${layer.id}:copy`, d: { ...layer.d, time } };
      const result = { ...original(detached, scale, time), key: `done:${layer.id}` };
      finished(layer.id);
      return result;
    });
    let resolve!: (surface: any) => void;
    const render = engineRasterAsync.getMockImplementation()!;
    engineRasterAsync.mockImplementation((layer, scale, time) => layer.id === 'title:copy' ? new Promise(done => { resolve = done; }) : render(layer, scale, time));
    const slow = compat.prepare(need('title'), 4);
    await Promise.resolve();
    await compat.prepare(need('box'), 4);
    expect(compat.answer(need('box'), 4, { prepared: true })).toMatchObject({ key: 'done:box' });
    expect(finished.mock.calls).toEqual([['box']]);
    resolve(await render(text, 2, .5));
    await slow;
    expect(compat.answer(need('title'), 4, { prepared: true })).toMatchObject({ key: 'done:title' });
    expect(finished.mock.calls).toEqual([['box'], ['title']]);
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('prepares fresh pixels when evaluated content changes with time', async () => {
    const { services, compat, engineRasterAsync, need } = setup(true, (_layer, scale, time) => `${scale}:${time >= 1 ? 'blue' : 'red'}`);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...args: any[]) => ({ ...original(layer, ...args), key: 'animated-content' }));
    compat.answer(need('box', 0), 4);
    await compat.prepare(need('box', .5), 4);
    expect(engineRasterAsync).not.toHaveBeenCalled();
    await compat.prepare(need('box', 1), 4);
    expect(engineRasterAsync).toHaveBeenCalledTimes(1);
    expect(engineRasterAsync).toHaveBeenCalledWith(shape, 2, 1, undefined);
  });

  it('propagates a preparation failure instead of drawing a blank or blocking in a sync retry', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...args: any[]) => ({ ...original(layer, ...args), key: 'prepared' }));
    engineRasterAsync.mockRejectedValue(new Error('native preparation failed'));
    await expect(compat.prepare(need('title'), 4)).rejects.toThrow('native preparation failed');
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('uses a prepared live base for the exact need without a synchronous engine raster', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), key: 'prepared' }));
    const request = need('title', 0.5, 2);
    await compat.prepare(request, 4);
    expect(engineRasterAsync).toHaveBeenCalledWith(text, 2, 0.5, undefined);
    expect(compat.answer(request, 4, { prepared: true })).toMatchObject({ kind: 'raster', key: 'prepared' });
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('keeps a synchronous caller on the current base after preparation', async () => {
    const { services, compat, engineRaster, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), key: 'current' }));
    const request = need('title');
    await compat.prepare(request, 4);
    expect(compat.answer(request, 4)).toMatchObject({ kind: 'raster', key: 'current' });
    expect(engineRaster).toHaveBeenCalledTimes(1);
  });

  it('does not use a prepared base for another layer, time, crop, or detached layer', async () => {
    const { services, compat, engineRaster, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), key: `copy:${layer.id}` }));
    await compat.prepare(need('title', 0.5, 2), 4);
    expect(compat.answer(need('box', 0.5, 2), 4, { prepared: true })).toBeNull();
    expect(compat.answer(need('title', 1, 2), 4, { prepared: true })).toBeNull();
    expect(compat.answer({ ...need('title', 0.5, 2), crop: { x: 1, y: 2, width: 3, height: 4 } }, 4, { prepared: true })).toBeNull();
    expect(engineRaster).not.toHaveBeenCalled();
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original({ ...layer }, ...rest), key: 'detached' }));
    await compat.prepare(need('title', 0.5, 2), 4);
    compat.answer(need('title', 0.5, 2), 4, { prepared: true });
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('drops prepared samples after reset and when the document revision changes', async () => {
    const { services, compat, engineRaster, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), key: 'changed' }));
    const request = need('title');
    await compat.prepare(request, 4);
    compat.reset();
    expect(compat.answer(request, 4, { prepared: true })).toBeNull();
    await compat.prepare(request, 4);
    expect(compat.answer(request, 5, { prepared: true })).toBeNull();
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('ignores an asynchronous surface that finishes after reset', async () => {
    const { services, compat, engineRaster, engineRasterAsync, need } = setup(true);
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), key: 'late' }));
    let resolve!: (surface: any) => void;
    engineRasterAsync.mockImplementation(() => new Promise((done) => { resolve = done; }));
    const pending = compat.prepare(need('title'), 4);
    await Promise.resolve();
    compat.reset();
    resolve({ cv: { width: 96, height: 48 }, w: 48, h: 24, anchorX: 4, anchorY: 18 });
    await pending;
    expect(compat.answer(need('title'), 4, { prepared: true })).toBeNull();
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('answers native for a wrapper that passes a layer through, without drawing the base', () => {
    const { services, compat, engineRaster, need } = setup();
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => original(layer, scale, time, uploaded, crop));
    expect(compat.answer(need('title'), 4)).toEqual({ kind: 'native' });
    expect(engineRaster).not.toHaveBeenCalled();
  });

  it('runs the chain at the layer\'s own time and density and posts the wrapper\'s pixels on today\'s quad', () => {
    const { services, compat, engineRaster, readPixels, need } = setup();
    const original = services.get<any>('raster');
    const seen: unknown[] = [];
    services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      seen.push([layer.id, scale, time, uploaded, crop]);
      if (layer.type !== 'text') return original(layer, scale, time, uploaded, crop);
      const full = original(layer, scale, time, uploaded, crop);
      return { ...full, cv: { width: 100, height: 50 }, key: `typed:${layer.id}:${time}`, w: 50, h: 25, anchorX: 6, anchorY: 20 };
    });
    const answer = compat.answer(need('title', 1 / 30, 2), 9);
    expect(seen).toEqual([['title', 2, 23_520_000 / TICKS_PER_SECOND, expect.any(Function), undefined]]);
    expect(engineRaster).toHaveBeenCalledTimes(1);
    expect(answer).toMatchObject({ kind: 'raster', key: 'typed:title:0.03333333333333333', width: 100, height: 50, format: 'rgba8Premultiplied', anchor: [6, 20], size: [50, 25] });
    expect(readPixels).toHaveBeenCalledTimes(1);
    // The same cache key at the same layer, time and density: reused, not sent again.
    expect(compat.answer(need('title', 1 / 30, 2), 10)).toEqual({ kind: 'reuse', key: 'typed:title:0.03333333333333333' });
    expect(readPixels).toHaveBeenCalledTimes(1);
    // Unaffected layers are the engine's.
    expect(compat.answer(need('box'), 10)).toEqual({ kind: 'native' });
  });

  it('anchors a shape without paths at its center, as the compositor does', () => {
    const { services, compat, need } = setup();
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => ({ ...original(layer, ...rest), cv: { width: 80, height: 40 }, key: 'k' }));
    expect(compat.answer(need('box', 0, 2), 1)).toMatchObject({ kind: 'raster', anchor: [24, 12], size: [48, 24] });
  });

  it('finds layers of nested compositions, and answers native for a layer the window does not have', () => {
    const { PM, services, compat, need } = setup();
    expect(findLayer(PM, 'inner', 'inner')).toBe(nested);
    expect(findLayer(PM, 'P-root', 'title')).toBe(text);
    const seen: string[] = [];
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => { seen.push(layer.id); return original(layer, ...rest); });
    expect(compat.answer(need('inner', 0, 1, 'inner'), 1)).toEqual({ kind: 'native' });
    expect(compat.answer(need('gone'), 1)).toEqual({ kind: 'native' });
    expect(seen).toEqual(['inner']);
  });

  it('decides a captions layer between cues now, and leaves hidden captions to the engine', () => {
    const { PM, services, compat, engineRaster, need } = setup();
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => original(layer, ...rest));
    // A captions base is not lazy: a wrapper can test it for null.
    expect(compat.answer(need('subs'), 1)).toMatchObject({ kind: 'raster' });
    expect(engineRaster).toHaveBeenCalledTimes(1);
    PM.captionsHidden = true;
    expect(compat.answer(need('subs'), 2)).toEqual({ kind: 'native' });
  });

  it('passes wrappers today\'s uploaded lookup (a miss), and no crop for a need without a window', () => {
    const { services, compat, need } = setup();
    let args: any[] = [];
    services.register('raster', (...a: any[]) => { args = a; return { cv: { width: 2, height: 2 }, w: 2, h: 2, anchorX: 0, anchorY: 0 }; });
    compat.answer(need('title'), 1);
    expect(typeof args[3]).toBe('function');
    expect(args[3]('any-key')).toBeUndefined();
    expect(args[4]).toBeUndefined();
  });

  it('passes a viewer need\'s crop window to the chain and the engine\'s base, as today\'s compositor does', () => {
    const { services, compat, engineRaster, need } = setup();
    const original = services.get<any>('raster');
    const crops: unknown[] = [];
    services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      crops.push(crop);
      return original(layer, scale, time, uploaded, crop);
    });
    const window = { x: 256, y: 0, width: 512, height: 256 };
    // Passed through untouched, for this window: the engine draws it.
    expect(compat.answer({ ...need('title', 0.5, 8), crop: window }, 3)).toEqual({ kind: 'native' });
    expect(crops).toEqual([window]);
    // A wrapper that reads the base gets the engine's raster of that window.
    services.register('raster', (layer: any, scale: number, time: number, uploaded: unknown, crop: unknown) => {
      const base = original(layer, scale, time, uploaded, crop);
      return { ...base, cv: { width: 512, height: 256 }, key: 'typed' };
    });
    compat.answer({ ...need('title', 0.5, 8), crop: window }, 3);
    expect(engineRaster).toHaveBeenLastCalledWith(text, 8, 0.5, window);
  });

  it('maps a cropped text result\'s sourceWindow onto the uploaded raster as today\'s draw-window program samples it', () => {
    const { services, compat, need } = setup();
    // Today: `uv = (v_uv * [gw, gh] - [x, y]) / textureSize`, the quad the whole layer box.
    services.register('raster', () => ({ cv: { width: 512, height: 256 }, w: 400, h: 100, anchorX: 200, anchorY: 50, sourceWindow: [256, 0, 3200, 800], key: 'k' }));
    expect(compat.answer({ ...need('title', 0.5, 8), crop: { x: 256, y: 0, width: 512, height: 256 } }, 1)).toMatchObject({
      kind: 'raster', width: 512, height: 256, size: [400, 100], anchor: [200, 50], uv: [-0.5, 0, 6.25, 3.125]
    });
    // A shape's `uv` goes through unchanged.
    services.register('raster', () => ({ cv: { width: 512, height: 256 }, w: 48, h: 24, uv: [-0.5, 0, 3.25, 3.375], key: 'k2' }));
    expect(compat.answer({ ...need('box', 0.5, 8), crop: { x: 256, y: 0, width: 512, height: 256 } }, 1)).toMatchObject({ uv: [-0.5, 0, 3.25, 3.375] });
  });

  it('draws a base the wrapper changed in place: a written field or a canvas it took', () => {
    const { services, compat, readPixels, need } = setup();
    const original = services.get<any>('raster');
    services.register('raster', (layer: any, ...rest: any[]) => {
      const base = original(layer, ...rest);
      if (layer.id === 'title') base.anchorX -= 10;
      if (layer.id === 'box') base.cv.getContext = undefined;
      return base;
    });
    expect(compat.answer(need('title'), 2)).toMatchObject({ kind: 'raster', anchor: [-6, 18] });
    expect(compat.answer(need('box'), 2)).toMatchObject({ kind: 'raster' });
    expect(readPixels).toHaveBeenCalledTimes(2);
    // Reading a field that is not the canvas leaves it the engine's.
    services.register('raster', (layer: any, ...rest: any[]) => {
      const base = original(layer, ...rest);
      void base.selection;
      return base;
    });
    expect(compat.answer(need('title', 1), 3)).toEqual({ kind: 'native' });
  });

  it('an export frame draws the snapshot\'s layer: the live one when equal, else the snapshot\'s copy', () => {
    const { services, compat, need } = setup();
    const layers: any[] = [];
    services.register('raster', (layer: any) => { layers.push(layer); return { cv: { width: 2, height: 2 }, w: 2, h: 2, anchorX: 0, anchorY: 0, key: `w:${layer.d.text}` }; });
    compat.answer(need('title'), 1, { fresh: true, layer: JSON.stringify(text) });
    expect(layers[0]).toBe(text);
    const queued = { ...text, d: { text: 'As queued' } };
    const result = compat.answer(need('title'), 1, { fresh: true, layer: JSON.stringify(queued) });
    expect(layers[1]).not.toBe(text);
    expect(layers[1].d.text).toBe('As queued');
    expect(result).toMatchObject({ kind: 'raster', key: 'w:As queued' });
  });

  it('draws nothing for a null result, and fails (never draws the plain layer) when the wrapper throws', () => {
    const { services, compat, need } = setup();
    services.register('raster', () => null);
    expect(compat.answer(need('title'), 1)).toMatchObject({ kind: 'raster', width: 1, height: 1, data: new Uint8Array(4) });
    services.register('raster', () => { throw new Error('broken'); });
    // Today the exception leaves the compositor's draw and fails the frame.
    expect(() => compat.answer(need('title'), 1)).toThrow('broken');
  });

  it('is the session\'s producer only while a wrapper is registered, and drops its answers when the chain changes', () => {
    const { services, compat, need } = setup();
    const posted: any[] = [];
    const session: TransportSession = { id: 1, closed: false, generation: 1, resyncPending: false, applied: 1, uploadFallback: () => {}, post: (event) => { posted.push(event); return true; } };
    const transport = new EngineRenderTransport({
      native: {}, session: () => session, revision: () => 12, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {}
    } as any);
    compat.attach(transport);
    expect(posted).toEqual([]);
    const original = services.get<any>('raster');
    const first = services.register('raster', (layer: any, ...rest: any[]) => original(layer, ...rest));
    expect(posted.map((e) => e.sources)).toEqual([['rasterOverride']]);
    const second = services.register('raster', (layer: any, ...rest: any[]) => original(layer, ...rest));
    // The chain changed: the list goes empty (the engine drops answers) and back.
    expect(posted.map((e) => e.sources)).toEqual([['rasterOverride'], [], ['rasterOverride']]);
    // The engine's ask is answered on the same session.
    transport.receiveCompat({ type: 'compat', compat: 'needs', revision: 12, needs: [need('title')] }, session);
    expect(posted.at(-1)).toMatchObject({ type: 'compat', compat: 'native', layer: 'title', time: need('title').time, scale: 2, revision: 12 });
    second.dispose();
    first.dispose();
    expect(posted.filter((e) => e.compat === 'producers').at(-1).sources).toEqual([]);
  });
});

describe('the render transport\'s compatibility answers', () => {
  function transport(posted: any[], snapshotSync?: any) {
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: (e) => { posted.push(e); return true; } };
    const t = new EngineRenderTransport({
      native: { snapshotSync }, session: () => session, revision: () => 5, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {}
    } as any);
    return { t, session };
  }
  const need: CompatNeed = { composition: 'P-root', layer: 'title', time: 352_800_000, source: 'rasterOverride', scale: 1.5 };

  it('posts rasters in bands with the need\'s own time and density, and the quad', () => {
    const posted: any[] = [];
    const { t, session } = transport(posted);
    t.answerCompat('rasterOverride', () => ({ kind: 'raster', key: 'k1', width: 2, height: 3, format: 'rgba8Premultiplied', data: new Uint8Array(24).fill(9), anchor: [1, 2], size: [4, 6], uv: [0, 0, 1, 1] }));
    t.receiveCompat({ compat: 'needs', needs: [need] }, session);
    const band = posted.find((e) => e.compat === 'pixels');
    expect(band).toMatchObject({ id: { composition: 'P-root', layer: 'title', time: 352_800_000, source: 'rasterOverride', key: 'k1' }, revision: 5, width: 2, height: 3, origin: [1, 2], scale: 1.5, row: 0, rows: 3, size: [4, 6], uv: [0, 0, 1, 1] });
    expect(band.data.byteLength).toBe(24);
  });

  it('answers a need asked again after a reuse with fresh pixels (the engine evicted that raster)', () => {
    const posted: any[] = [];
    const { t, session } = transport(posted);
    const calls: Array<{ fresh?: boolean } | undefined> = [];
    t.answerCompat('rasterOverride', (_need, _revision, options) => {
      calls.push(options);
      return options?.fresh
        ? { kind: 'raster', key: 'k', width: 1, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(4), anchor: [0, 0], size: [1, 1] }
        : { kind: 'reuse', key: 'k' };
    });
    t.receiveCompat({ compat: 'needs', needs: [need] }, session);
    t.receiveCompat({ compat: 'needs', needs: [need] }, session);
    expect(posted.filter((e) => e.compat !== 'producers').map((e) => e.compat)).toEqual(['reuse', 'pixels']);
    expect(calls).toEqual([undefined, { fresh: true }]);
    expect(t.compatStats.unanswered).toBe(0);
  });

  it('reports a producer that throws as a failed frame and leaves the need unanswered', () => {
    const posted: any[] = [];
    const failures: unknown[] = [];
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: (e) => { posted.push(e); return true; } };
    const t = new EngineRenderTransport({
      native: {}, session: () => session, revision: () => 5, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {},
      onProducerError: (error: unknown) => failures.push(error)
    } as any);
    t.answerCompat('rasterOverride', () => { throw new Error('wrapper broke'); });
    t.receiveCompat({ compat: 'needs', needs: [need] }, session);
    expect(failures.map((e) => (e as Error).message)).toEqual(['wrapper broke']);
    expect(posted.filter((e) => e.compat !== 'producers')).toEqual([]);
    expect(t.compatStats.unanswered).toBe(1);
  });

  it('answers a synchronous snapshot\'s needs on the calling thread and asks again', () => {
    const posted: any[] = [];
    const replies = [
      { status: 'needsRenderer', revision: 5, fallback: [], compat: [need] },
      { status: 'rendered', revision: 5, viewSeq: null, width: 1, height: 1, pixels: 'opaque', data: new Uint8Array(4), origin: [0, 0], layerBox: null, raster: null, quality: 1, waitedUs: 0, renderUs: 0, mainMs: 0 }
    ];
    const snapshotSync = vi.fn(() => replies.shift());
    const { t } = transport(posted, snapshotSync);
    t.answerCompat('rasterOverride', () => ({ kind: 'native' }));
    expect(t.snapshotSync({ time: 0.5, width: 1, height: 1 }).width).toBe(1);
    expect(snapshotSync).toHaveBeenCalledTimes(2);
    expect(posted.map((e) => e.compat)).toEqual(['producers', 'native']);
  });

  it('fails a snapshot whose needs no producer answers, by name', () => {
    const snapshotSync = vi.fn(() => ({ status: 'needsRenderer', revision: 5, fallback: [], compat: [need] }));
    const { t } = transport([], snapshotSync);
    expect(() => t.snapshotSync({ time: 0, width: 1, height: 1 })).toThrow(/No raster override in this window answered for layer title/);
  });
});

describe('a raster override that asks for a frame from inside its answer', () => {
  it('gets a named failure instead of recursing', () => {
    const need: CompatNeed = { composition: 'P-root', layer: 'title', time: 0, source: 'rasterOverride', scale: 1 };
    const posted: any[] = [];
    const session: TransportSession = { id: 3, closed: false, generation: 2, resyncPending: false, applied: 5, uploadFallback: () => {}, post: (e) => { posted.push(e); return true; } };
    const snapshotSync = vi.fn(() => ({ status: 'needsRenderer', revision: 5, fallback: [], compat: [need] }));
    const t = new EngineRenderTransport({
      native: { snapshotSync }, session: () => session, revision: () => 5, composition: () => 'P-root', document: () => 'P', viewSeq: () => null, fallbackRunner: () => null, log: () => {}
    } as any);
    let inner = 'unset';
    t.answerCompat('rasterOverride', () => {
      inner = t.snapshotDataUrlSync(0, 160, { w: 320, h: 180 });
      return { kind: 'native' };
    });
    t.receiveCompat({ compat: 'needs', needs: [need] }, session);
    expect(inner).toBe('');
    expect(posted.at(-1)).toMatchObject({ compat: 'native' });
    expect(snapshotSync).toHaveBeenCalledTimes(1);
  });
});
