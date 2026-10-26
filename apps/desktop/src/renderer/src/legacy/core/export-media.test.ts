import { afterEach, expect, it, vi } from 'vitest';
import { trackExportMedia } from './export-media';

class Video {
  videoWidth = 128; videoHeight = 64;
  src = 'blob:source'; currentSrc = 'blob:source'; srcObject = null; error = null;
  currentTime = 2; seeking = false; readyState = 2; paused = true;
}
class Bitmap {
  width = 128; height = 64;
  close() { this.width = this.height = 0; }
}
class Canvas { width = 128; height = 64; }
afterEach(() => vi.unstubAllGlobals());

function fixture(image = false) {
  vi.stubGlobal('HTMLVideoElement', Video);
  vi.stubGlobal('ImageBitmap', Bitmap);
  vi.stubGlobal('HTMLCanvasElement', Canvas);
  const layer = { id: 'first', type: image ? 'image' : 'video', d: { asset: 'source' } };
  const element = image ? new Bitmap() : new Video();
  const asset: any = { id: 'source', kind: layer.type, el: element, w: 128, h: 64, dur: 5 };
  const assets = new Map([['source', asset]]), listeners = new Set<() => void>();
  const unsubscribe = vi.fn(() => listeners.clear());
  const PM: any = { proj: { layers: [layer] }, assets: { get: (id: string) => assets.get(id) },
    scope: { push: vi.fn(), pop: vi.fn() }, active: () => true,
    preparedVideoVersion: 1, preparedVideoFrames: new Map(image ? [] : [['first@1', new Canvas()]]),
    bus: { on: (name: string, run: () => void) => { expect(name).toBe('assets'); listeners.add(run); return unsubscribe; } } };
  const tracker = trackExportMedia(PM), layers = [layer];
  const sources = () => tracker.sources(layers)!;
  const prepared = () => tracker.prepared(layers, 1, sources())!;
  return { PM, layer, layers, asset, element: element as any, assets, tracker, sources, prepared,
    emit: () => listeners.forEach(run => run()), unsubscribe };
}

it('allows deliberate decoder changes during preparation, then freezes the ready state', () => {
  const h = fixture();
  h.element.seeking = true; h.element.paused = false; h.element.readyState = 1;
  const source = h.sources();
  h.element.currentTime = 3; h.element.seeking = false; h.element.paused = true; h.element.readyState = 2;
  expect(h.tracker.sameSources(source)).toBe(true);
  const stamp = h.tracker.prepared(h.layers, 1, source)!;
  expect(h.tracker.valid(stamp)).toBe(true);
  h.element.currentTime = 4;
  expect(h.tracker.valid(stamp)).toBe(false);
  expect(h.PM.scope.push.mock.calls.length).toBe(h.PM.scope.pop.mock.calls.length);
});

it.each(['runtime', 'element', 'src', 'currentSrc', 'dimensions', 'metadata', 'duration', 'scale', 'url', 'project', 'layers'])(
  'detects %s replacement while preparation is awaiting', async change => {
    const h = fixture(), source = h.sources(), ready = h.prepared();
    let finish!: () => void;
    const preparing = new Promise<void>(resolve => { finish = resolve; });
    const checked = (async () => { await preparing; return h.tracker.sameSources(source); })();
    if (change === 'runtime') h.assets.set('source', { ...h.asset });
    if (change === 'element') h.asset.el = new Video();
    if (change === 'src') h.element.src = 'blob:replacement';
    if (change === 'currentSrc') h.element.currentSrc = 'blob:replacement';
    if (change === 'dimensions') h.element.videoWidth++;
    if (change === 'metadata') h.asset.w++;
    if (change === 'duration') h.asset.dur++;
    if (change === 'scale') h.asset.videoTimestampScale = .001;
    if (change === 'url') h.asset.url = 'blob:replacement';
    if (change === 'project') h.PM.proj = { ...h.PM.proj };
    if (change === 'layers') h.PM.proj.layers = [...h.layers];
    finish();
    expect(await checked).toBe(false);
    expect(h.tracker.prepared(h.layers, 1, source)).toBeNull();
    expect(h.tracker.valid(ready)).toBe(false);
  },
);

it.each(['map', 'version', 'entry', 'dimensions', 'extra', 'missing'])(
  'rejects prepared %s changes even when the other identities remain unchanged', change => {
    const h = fixture(), stamp = h.prepared();
    if (change === 'map') h.PM.preparedVideoFrames = new Map(h.PM.preparedVideoFrames);
    if (change === 'version') h.PM.preparedVideoVersion++;
    if (change === 'entry') h.PM.preparedVideoFrames.set('first@1', new Canvas());
    if (change === 'dimensions') h.PM.preparedVideoFrames.get('first@1').width++;
    if (change === 'extra') h.PM.preparedVideoFrames.set('unexpected@1', new Canvas());
    if (change === 'missing') h.PM.preparedVideoFrames.delete('first@1');
    expect(h.tracker.valid(stamp)).toBe(false);
  },
);

it('requires every active clip snapshot while allowing aliases at different source times', () => {
  const h = fixture(), second = { id: 'second', type: 'video', d: { asset: 'alias' } };
  h.assets.set('alias', { ...h.asset, id: 'alias' });
  h.layers.push(second); h.PM.proj.layers = h.layers;
  h.PM.preparedVideoFrames.set('second@1', new Canvas());
  // One shared decoder can be at its last requested source time while both
  // earlier and later immutable canvases remain valid for the timeline time.
  h.element.currentTime = 4;
  const stamp = h.prepared();
  expect(h.tracker.valid(stamp)).toBe(true);
  h.PM.preparedVideoFrames.delete('first@1');
  expect(h.tracker.prepared(h.layers, 1, h.sources())).toBeNull();
});

it('does not require a decoder frame for an inactive video layer', () => {
  const h = fixture();
  h.PM.active = () => false; h.PM.preparedVideoFrames.clear();
  h.element.seeking = true;
  expect(h.tracker.valid(h.prepared())).toBe(true);
});

it('allows buffering to improve readiness without changing the prepared frame', () => {
  const h = fixture(), stamp = h.prepared();
  h.element.readyState = 4;
  expect(h.tracker.valid(stamp)).toBe(true);
  h.element.readyState = 1;
  expect(h.tracker.valid(stamp)).toBe(false);
});

it.each(['seeking', 'readyState', 'paused', 'error'])('rejects a decoder that becomes invalid: %s', field => {
  const h = fixture(), stamp = h.prepared();
  h.element[field] = ({ seeking: true, readyState: 1, paused: false, error: {} } as Record<string, unknown>)[field];
  expect(h.tracker.valid(stamp)).toBe(false);
});

it('accepts immutable bitmap images and detects replacement or close', () => {
  const h = fixture(true), stamp = h.prepared();
  expect(h.tracker.valid(stamp)).toBe(true);
  h.asset.el = new Bitmap();
  expect(h.tracker.valid(stamp)).toBe(false);
  const replaced = h.prepared();
  h.asset.el.close();
  expect(h.tracker.valid(replaced)).toBe(false);
  expect(h.tracker.sources(h.layers)).toBeNull();
});

it.each(['svg', 'canvas', 'html-image', 'sequence', 'stream', 'property-id'])(
  'keeps unsupported %s sources on the serial path', kind => {
    const h = fixture(kind === 'svg' || kind === 'canvas' || kind === 'html-image');
    if (kind === 'svg') h.asset.format = 'svg';
    if (kind === 'canvas') h.asset.el = new Canvas();
    if (kind === 'html-image') h.asset.el = { width: 128, height: 64, src: 'blob:image' };
    if (kind === 'sequence') h.asset.imageSequence = { fps: 30, frames: 10 };
    if (kind === 'stream') h.element.srcObject = {};
    if (kind === 'property-id') h.layer.d.asset = { v: 'source', kf: [] } as any;
    expect(h.tracker.sources(h.layers)).toBeNull();
  },
);

it('invalidates stamps on asset events and releases its subscription once', () => {
  const h = fixture(), source = h.sources(), stamp = h.prepared();
  h.emit();
  expect(h.tracker.sameSources(source)).toBe(false);
  expect(h.tracker.valid(stamp)).toBe(false);
  const renewed = h.prepared();
  expect(h.tracker.valid(renewed)).toBe(true);
  h.tracker.dispose(); h.tracker.dispose();
  expect(h.unsubscribe).toHaveBeenCalledOnce();
  expect(h.tracker.valid(renewed)).toBe(false);
  expect(h.tracker.sources(h.layers)).toBeNull();
});

it('releases an object subscription without retaining a live tracker', () => {
  const h = fixture(), dispose = vi.fn();
  h.tracker.dispose();
  h.PM.bus.on = () => ({ dispose });
  const tracker = trackExportMedia(h.PM);
  tracker.dispose(); tracker.dispose();
  expect(dispose).toHaveBeenCalledOnce();
});
