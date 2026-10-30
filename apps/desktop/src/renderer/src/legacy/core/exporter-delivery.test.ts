// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { install } from './exporter';
import * as framePreparation from './frame-preparation';
import { exportEffects } from './export-effects';
import { AlphaReadbackRetryError } from '../gl/half-readback';
import { Registry } from '../../kernel/registry';
import { toLegacyFx } from '../../kernel/glsl';
import { EFFECTS } from '../../../../extensions/effects-basic/effects';

vi.mock('./export-preview', () => ({ animateExportPreview: () => () => {} }));
vi.mock('../../player/export-web', () => ({
  buildWebExport: async () => ({ bytes: new Uint8Array([1]), scene: { warnings: [] } }),
  inspectWebExport: () => ({ errors: [], fonts: new Set() }),
}));

function element(tagAndClasses: string, ...args: any[]): HTMLElement {
  const node = document.createElement(tagAndClasses.split('.')[0]!);
  for (const arg of args) {
    if (arg instanceof Node) node.append(arg);
    else if (typeof arg === 'string') node.append(arg);
    else if (arg?.style) Object.assign(node.style, arg.style);
  }
  return node;
}

function setup(download: any = vi.fn(async (_blob: Blob, _name: string): Promise<any> => ({ ok: true, path: '/tmp/export' }))) {
  const toast = vi.fn();
  const frame = {
    toBlob(callback: (blob: Blob) => void) { callback(new Blob(['png'], { type: 'image/png' })); },
    getContext() { return { getImageData: () => ({ data: new Uint8ClampedArray(16) }) }; },
  };
  const PM: any = {
    version: 'test', h: element, proj: { name: 'Test', w: 2, h: 2, fps: 2, dur: 1, layers: [] },
    store: { get: () => ({}), set() {} }, toast, download,
    modal: () => ({ el: document.createElement('div'), close() {} }),
    scope: [], pause() {}, setTime() {}, time: 0, playing: false, quality: 1,
    active: (layer: any, time: number) => time >= layer.from && time < layer.from + layer.dur,
    round: (number: number) => number, tc: () => '0:00',
    Audio: { hasAudibleLayers: () => false },
    GL: { canvas: frame, resize() {}, render() {}, renderToPixels: vi.fn(() => new Uint8Array(16)) },
    renderFrameTo: () => frame,
  };
  install(PM);
  return { X: PM.Export, PM, toast, download, frame };
}

const options = { format: 'png', fps: 2, scale: 1, audio: false, mblur: false };
let originalMediaRecorder: any;

beforeEach(() => {
  vi.stubGlobal('ImageData', class {
    data: Uint8ClampedArray;
    width: number;
    height: number;
    constructor(data: Uint8ClampedArray, width: number, height: number) {
      this.data = data; this.width = width; this.height = height;
    }
  });
  originalMediaRecorder = (window as any).MediaRecorder;
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({ drawImage() {}, putImageData() {} }) as any);
  delete (window as any).showDirectoryPicker;
  delete (window as any).powermove;
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  delete (window as any).showDirectoryPicker;
  delete (window as any).powermove;
  if (originalMediaRecorder === undefined) delete (window as any).MediaRecorder;
  else (window as any).MediaRecorder = originalMediaRecorder;
});

describe('export delivery', () => {
  function webCodecs({ controlled = false, payloadBytes = 1 } = {}) {
    const frames: Frame[] = [], encoders: Encoder[] = [], encoded: Array<{ timestamp: number; duration: number }> = [];
    let failure: 'clone' | 'encode' | undefined;
    let holding = controlled;
    class Frame {
      closed = false;
      constructor(readonly source: unknown, readonly init: { timestamp: number; duration: number }) {
        if (source instanceof Frame) this.source = source.source;
        frames.push(this);
      }
      clone() { if (failure === 'clone') throw new Error('Clone failed'); return new Frame(this.source, this.init); }
      close() { this.closed = true; }
    }
    class Encoder {
      static async isConfigSupported() { return { supported: true }; }
      state = 'unconfigured'; encodeQueueSize = 0;
      pending: Array<{ timestamp: number; duration: number }> = [];
      constructor(readonly callbacks: any) { encoders.push(this); }
      configure() { this.state = 'configured'; }
      encode(frame: Frame) {
        if (failure === 'encode') throw new Error('Encode failed');
        encoded.push(frame.init);
        this.pending.push(frame.init); this.encodeQueueSize++;
        if (!holding) this.drain();
      }
      drain(count = this.pending.length) {
        for (let i = 0; i < count && this.pending.length; i++) {
          const init = this.pending.shift()!; this.encodeQueueSize--;
          this.callbacks.output({ timestamp: init.timestamp, byteLength: payloadBytes, type: 'key', copyTo: (data: Uint8Array) => data.fill(1) });
        }
      }
      flush = vi.fn(async () => { this.drain(); });
      close() { this.state = 'closed'; this.pending = []; this.encodeQueueSize = 0; }
    }
    vi.stubGlobal('VideoEncoder', Encoder); vi.stubGlobal('VideoFrame', Frame);
    return { frames, encoders, encoded, fail: (value: typeof failure) => { failure = value; }, hold: (value: boolean) => { holding = value; } };
  }

  function staticImage() {
    const h = setup();
    class Bitmap {
      width = 2; height = 2;
      close = vi.fn(() => { this.width = this.height = 0; });
    }
    vi.stubGlobal('ImageBitmap', Bitmap);
    const makeAsset = (paint = 1) => ({ id: 'photo', kind: 'image', el: new Bitmap(), w: 2, h: 2, paint });
    const assets = new Map([['photo', makeAsset()]]);
    h.PM.assets = { get: vi.fn((id: string) => assets.get(id)) };
    h.PM.proj.layers = [{ id: 'photo-layer', type: 'image', from: 0, dur: 1, d: { asset: 'photo' } }];
    const listeners = new Map<string, Set<() => void>>();
    h.PM.bus = {
      on(event: string, callback: () => void) {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event)!.add(callback); return () => listeners.get(event)!.delete(callback);
      },
      emit(event: string) { for (const callback of listeners.get(event) || []) callback(); },
    };
    const paint = () => { const asset = assets.get('photo')!; return asset.el.width ? asset.paint : 0; };
    h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(16).fill(paint()));
    h.PM.GL.render = vi.fn(() => { h.PM.GL.canvas = { paint: paint() }; });
    h.PM.renderFrameTo = vi.fn((_time: number, width: number, height: number, _options: any, target: HTMLCanvasElement) => {
      const captured = paint(); target.width = width; target.height = height;
      target.toBlob = callback => callback(new Blob([String(captured)])); return target;
    });
    return { ...h, assets, makeAsset, listeners };
  }

  it.each(['success', 'chunk', 'cancel'] as const)('streams large WebM delivery in bounded sequential chunks: %s', async outcome => {
    const { X } = setup();
    webCodecs({ payloadBytes: 3 * 1024 * 1024 });
    let acknowledge!: () => void;
    const held = new Promise<void>(resolve => { acknowledge = resolve; });
    const received: Uint8Array[] = [];
    const chunk = vi.fn(async (_id: string, bytes: Uint8Array) => {
      received.push(bytes);
      if (received.length === 1) await held;
      if (outcome === 'chunk') throw new Error('Upload failed');
    });
    const begin = vi.fn(async (_size: number) => 'upload');
    const finish = vi.fn(async () => outcome === 'cancel' ? { ok: false, cancelled: true } : { ok: true, path: '/tmp/movie.webm' });
    const abort = vi.fn(async () => {}), release = vi.fn(async () => {}), saveFile = vi.fn();
    (window as any).powermove = { exportDestination: { choose: async () => 'destination', release }, fileUpload: { begin, chunk, finish, abort }, saveFile };
    const exporting = X.run({ ...options, format: 'webm' });
    try {
      await vi.waitFor(() => expect(chunk).toHaveBeenCalledOnce());
      expect(finish).not.toHaveBeenCalled();
      expect(abort).not.toHaveBeenCalled();
    } finally { acknowledge(); }
    expect(await exporting).toEqual(outcome === 'chunk' ? { error: 'Upload failed' } : { cancelled: outcome === 'cancel' });
    expect(saveFile).not.toHaveBeenCalled();
    expect(received.every(bytes => bytes.length > 0 && bytes.length <= 1024 * 1024)).toBe(true);
    if (outcome === 'chunk') {
      expect(chunk).toHaveBeenCalledOnce(); expect(finish).not.toHaveBeenCalled();
    } else {
      const size = begin.mock.calls[0]![0];
      expect(received.reduce((sum, bytes) => sum + bytes.length, 0)).toBe(size);
      expect(finish).toHaveBeenCalledWith('upload', { name: 'Test.webm', destinationToken: 'destination' });
    }
    expect(abort).toHaveBeenCalledExactlyOnceWith('upload');
    expect(release).toHaveBeenCalledExactlyOnceWith('destination');
    expect(X.busy).toBe(false);
  });

  it('waits for encoder capacity before submitting a fourteenth WebM frame', async () => {
    vi.useFakeTimers();
    const { X } = setup(), codecs = webCodecs({ controlled: true });
    const running = X.run({ ...options, format: 'webm', fps: 30 });
    try {
      await vi.advanceTimersByTimeAsync(100);
      expect(codecs.encoded.length).toBe(13);
      expect(codecs.encoders[0]!.encodeQueueSize).toBe(13);
      codecs.encoders[0]!.drain(1);
      await vi.advanceTimersByTimeAsync(6);
      expect(codecs.encoded.length).toBe(14);
      expect(codecs.encoders[0]!.encodeQueueSize).toBe(13);
    } finally {
      X.cancel = true;
      await vi.runAllTimersAsync();
      await running;
    }
  });

  it('aborts immediately after an encoder error while waiting for WebM capacity', async () => {
    vi.useFakeTimers();
    const { X, download } = setup(), codecs = webCodecs({ controlled: true });
    const running = X.run({ ...options, format: 'webm', fps: 30 });
    try {
      await vi.advanceTimersByTimeAsync(100);
      codecs.encoders[0]!.callbacks.error(new Error('Delayed encode failure'));
      await vi.runAllTimersAsync();
      expect(await running).toEqual({ error: 'Delayed encode failure' });
      expect(codecs.encoded.length).toBe(13);
      expect(codecs.encoders[0]!.flush).not.toHaveBeenCalled();
      expect(download).not.toHaveBeenCalled();
      expect(codecs.frames.every(frame => frame.closed)).toBe(true);
      expect(codecs.encoders[0]!.state).toBe('closed');
    } finally {
      X.cancel = true;
      await vi.runAllTimersAsync();
      await running;
    }
  });

  it('cancels a saturated WebM export without flushing or delivering and permits retry', async () => {
    vi.useFakeTimers();
    const { X, download } = setup(), codecs = webCodecs({ controlled: true });
    const running = X.run({ ...options, format: 'webm', fps: 30 });
    await vi.advanceTimersByTimeAsync(100);
    X.cancel = true;
    await vi.runAllTimersAsync();
    expect(await running).toEqual({ cancelled: true });
    expect(codecs.encoded.length).toBe(13);
    expect(codecs.encoders[0]!.flush).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(codecs.encoders[0]!.state).toBe('closed');
    codecs.hold(false);
    const retry = X.run({ ...options, format: 'webm', fps: 30 });
    await vi.runAllTimersAsync();
    expect(await retry).toEqual({ cancelled: false });
    expect(download).toHaveBeenCalledOnce();
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(codecs.encoders.every(encoder => encoder.state === 'closed')).toBe(true);
  });

  it('reuses a static WebM picture with distinct frame timestamps and closes every frame', async () => {
    const { X, PM } = setup(), codecs = webCodecs();
    PM.GL.render = vi.fn();
    expect(await X.run({ ...options, format: 'webm' })).toEqual({ cancelled: false });
    expect(PM.GL.render).toHaveBeenCalledOnce();
    expect(codecs.encoded).toEqual([{ timestamp: 0, duration: 500000 }, { timestamp: 500000, duration: 500000 }]);
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(codecs.encoders[0]!.state).toBe('closed');
  });

  it.each(['replace', 'close', 'assets-event'] as const)('refreshes and closes a retained WebM bitmap after %s inside encode', async change => {
    const h = staticImage(), codecs = webCodecs();
    const original = h.assets.get('photo')!, paints: number[] = [];
    const Encoder = window.VideoEncoder as any, encode = Encoder.prototype.encode;
    vi.spyOn(Encoder.prototype, 'encode').mockImplementation(function (this: any, frame: any) {
      paints.push(frame.source.paint); encode.call(this, frame);
      if (paints.length === 1) {
        if (change === 'replace') h.assets.set('photo', h.makeAsset(2));
        else if (change === 'close') original.el.close();
        else h.PM.bus.emit('assets');
      }
    });
    expect(await h.X.run({ ...options, format: 'webm' })).toEqual({ cancelled: false });
    expect(paints).toEqual([1, change === 'replace' ? 2 : change === 'close' ? 0 : 1]);
    expect(h.PM.GL.render.mock.calls.map((call: any[]) => call[0])).toEqual([0, .5]);
    expect(codecs.encoded).toEqual([{ timestamp: 0, duration: 500000 }, { timestamp: 500000, duration: 500000 }]);
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(codecs.encoders.every(encoder => encoder.state === 'closed')).toBe(true);
    expect(original.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
    expect(h.assets.get('photo')!.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
    expect(h.listeners.get('assets')!.size).toBe(0);
  });

  it.each(['saved', 'cancelled', 'error'] as const)('drains WebM bitmap queue changes with correct frame ownership after %s', async outcome => {
    vi.useFakeTimers();
    const h = staticImage(), codecs = webCodecs({ controlled: true }), old = h.assets.get('photo')!;
    const Encoder = window.VideoEncoder as any, encode = Encoder.prototype.encode;
    const paints: number[] = [];
    vi.spyOn(Encoder.prototype, 'encode').mockImplementation(function (this: any, frame: any) {
      paints.push(frame.source.paint); encode.call(this, frame);
    });
    const running = h.X.run({ ...options, format: 'webm', fps: 30 });
    try {
      await vi.advanceTimersByTimeAsync(100);
      expect(codecs.encoded).toHaveLength(13); expect(h.PM.GL.render).toHaveBeenCalledOnce();
      const retained = codecs.frames.find(frame => frame !== codecs.frames[0] && frame.init.timestamp === 0)!;
      expect(retained.closed).toBe(false);
      h.assets.set('photo', h.makeAsset(2));
      if (outcome === 'cancelled') h.X.cancel = true;
      else if (outcome === 'error') codecs.encoders[0]!.callbacks.error(new Error('Queued bitmap encode failed'));
      else {
        codecs.encoders[0]!.drain(1); await vi.advanceTimersByTimeAsync(6);
        expect(paints).toHaveLength(14); expect(paints[13]).toBe(2);
        expect(codecs.encoded[13]).toEqual({ timestamp: 433333, duration: 33333 });
        expect(h.PM.GL.render.mock.calls.map((call: any[]) => call[0])).toEqual([0, 13 / 30]);
        expect(retained.closed).toBe(true); codecs.hold(false); codecs.encoders[0]!.drain();
      }
      await vi.runAllTimersAsync();
      expect(await running).toEqual(outcome === 'error' ? { error: 'Queued bitmap encode failed' } : { cancelled: outcome === 'cancelled' });
      expect(h.download).toHaveBeenCalledTimes(outcome === 'saved' ? 1 : 0);
      expect(codecs.frames.every(frame => frame.closed)).toBe(true);
      expect(codecs.encoders.every(encoder => encoder.state === 'closed')).toBe(true);
      expect(old.el.close).not.toHaveBeenCalled(); expect(h.assets.get('photo')!.el.close).not.toHaveBeenCalled();
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.X.cancel = true; await vi.runAllTimersAsync(); await running; }
  });

  it.each(['edit', 'draw', 'animated'])('refreshes retained WebM pictures after %s changes', async change => {
    const { X, PM } = setup(), codecs = webCodecs();
    let version = 0;
    const handlers = new Map<string, () => void>();
    PM.animVersion = () => version;
    PM.bus = { on: (event: string, handler: () => void) => { handlers.set(event, handler); return () => handlers.delete(event); } };
    if (change === 'animated') PM.proj.layers = [{ type: 'solid', from: 0, dur: 1, p: { rotation: { expr: 'time' } } }];
    PM.GL.render = vi.fn(() => {
      if (change === 'edit') version++;
      if (change === 'draw') handlers.get('draw')!();
    });
    expect(await X.run({ ...options, format: 'webm' })).toEqual({ cancelled: false });
    expect(PM.GL.render).toHaveBeenCalledTimes(2);
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(handlers.size).toBe(0);
  });

  it.each(['clone', 'encode'] as const)('releases WebM frames and encoder after %s failure, then allows retry', async failure => {
    const { X } = setup(), codecs = webCodecs();
    codecs.fail(failure);
    expect(await X.run({ ...options, format: 'webm' })).toEqual({ error: failure === 'clone' ? 'Clone failed' : 'Encode failed' });
    expect(codecs.frames.every(frame => frame.closed)).toBe(true);
    expect(codecs.encoders.every(encoder => encoder.state === 'closed')).toBe(true);
    codecs.fail(undefined);
    expect(await X.run({ ...options, format: 'webm' })).toEqual({ cancelled: false });
  });

  it('waits for the native destination before showing progress or rendering', async () => {
    const { X, PM } = setup();
    PM.modal = vi.fn(PM.modal);
    PM.renderFrameTo = vi.fn(PM.renderFrameTo);
    let choose!: (token: string | null) => void;
    const start = vi.fn(() => new Promise(resolve => { choose = resolve; }));
    const write = vi.fn();
    (window as any).powermove = { render: { start, write, finish: vi.fn(), cancel: vi.fn() } };
    const result = X.run({ ...options, format: 'mp4' });
    await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
    const openedProgress = PM.modal.mock.calls.length;
    choose(null);
    expect(await result).toEqual({ cancelled: true });
    expect(openedProgress).toBe(0);
    expect(PM.renderFrameTo).not.toHaveBeenCalled();
    expect(PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(write).not.toHaveBeenCalled();
    expect(X.busy).toBe(false);
  });

  it.each([false, true])('only anticipates audio in the selected native export range when enabled=%s', async audio => {
    const { X, PM } = setup();
    PM.proj.work = [.2, .7];
    PM.Audio.expectsOfflineMix = vi.fn(() => true);
    const start = vi.fn(async () => null);
    (window as any).powermove = { render: { start, cancel: vi.fn() } };
    expect(await X.run({ ...options, format: 'mp4', range: 'work', audio })).toEqual({ cancelled: true });
    expect(start).toHaveBeenCalledWith(expect.objectContaining({ expectsAudio: audio }));
    if (audio) expect(PM.Audio.expectsOfflineMix).toHaveBeenCalledWith(.2, .7);
    else expect(PM.Audio.expectsOfflineMix).not.toHaveBeenCalled();
  });

  it.each(['still', 'web', 'json', 'png', 'webm', 'rec'])('cancels %s before export work begins', async format => {
    const { X, PM, download } = setup();
    PM.modal = vi.fn(PM.modal);
    PM.pause = vi.fn();
    PM.renderFrameTo = vi.fn(PM.renderFrameTo);
    const choose = vi.fn(async (_name: string, _directory: boolean) => null);
    const saveFile = vi.fn();
    (window as any).powermove = { exportDestination: { choose, release: vi.fn() }, saveFile };
    expect(await X.run({ ...options, format })).toEqual({ cancelled: true });
    expect(choose).toHaveBeenCalledOnce();
    expect(choose.mock.calls[0]).toEqual([expect.any(String), format === 'png']);
    expect(PM.modal).not.toHaveBeenCalled();
    expect(PM.pause).not.toHaveBeenCalled();
    expect(PM.renderFrameTo).not.toHaveBeenCalled();
    expect(saveFile).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
    expect(X.busy).toBe(false);
  });

  it.each(['still', 'web', 'png'])('delivers %s to the chosen destination and releases it', async format => {
    const { X, download } = setup();
    const choose = vi.fn(async () => 'destination');
    const release = vi.fn(async () => {});
    const saveFile = vi.fn(async (_request: any) => ({ ok: true, path: '/tmp/export' }));
    (window as any).powermove = { exportDestination: { choose, release }, saveFile };
    expect(await X.run({ ...options, format })).toEqual({ cancelled: false });
    expect(choose).toHaveBeenCalledOnce();
    expect(saveFile).toHaveBeenCalledTimes(format === 'png' ? 2 : 1);
    for (const call of saveFile.mock.calls) expect(call[0]).toMatchObject({ destinationToken: 'destination' });
    expect(download).not.toHaveBeenCalled();
    expect(release).toHaveBeenCalledWith('destination');
  });

  it('releases the destination after rendering fails and allows another export', async () => {
    const { X, PM } = setup();
    const release = vi.fn(async () => {});
    (window as any).powermove = { exportDestination: { choose: async () => 'destination', release } };
    PM.renderFrameTo = () => { throw new Error('Render failed'); };
    expect(await X.run({ ...options, format: 'still' })).toEqual({ error: 'Render failed' });
    expect(release).toHaveBeenCalledWith('destination');
    expect(X.busy).toBe(false);
  });

  it('encodes static PNG pixels once while writing every frame', async () => {
    const { X, PM, frame, download } = setup();
    const encode = vi.spyOn(frame, 'toBlob');
    PM.renderFrameTo = vi.fn(() => frame);
    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(PM.renderFrameTo).toHaveBeenCalledOnce();
    expect(encode).toHaveBeenCalledOnce();
    expect(download).toHaveBeenCalledTimes(2);
    expect(download.mock.calls[0][0]).toBe(download.mock.calls[1][0]);
  });

  it.each(['replace', 'close', 'assets-event'] as const)('recaptures a reused PNG bitmap at the same timestamp after %s during a held save', async change => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let release!: () => void;
    h.download.mockImplementationOnce(async () => { await new Promise<void>(yes => { release = yes; }); return { ok: true }; });
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.download).toHaveBeenCalledOnce());
      await new Promise(resolve => setTimeout(resolve, 15));
      expect(h.PM.renderFrameTo).toHaveBeenCalledOnce();
      if (change === 'replace') h.assets.set('photo', h.makeAsset(2));
      else if (change === 'close') old.el.close();
      else h.PM.bus.emit('assets');
      release(); expect(await result).toEqual({ cancelled: false });
      expect(h.PM.renderFrameTo.mock.calls.map((call: any[]) => call[0])).toEqual([0, .5]);
      expect(await Promise.all(h.download.mock.calls.map(([blob]: any[]) => blob.text()))).toEqual(['1', change === 'replace' ? '2' : change === 'close' ? '0' : '1']);
      expect(h.PM.renderFrameTo.mock.calls[0]![4]).toMatchObject({ width: 0, height: 0 });
      expect(old.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { release?.(); await result; }
  });

  it.each(['replace', 'close', 'gpu-loss'] as const)('revalidates a cached PNG bitmap after an empty join permits a %s microtask', async change => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let queued = false;
    h.PM.assets.get.mockImplementation((id: string) => {
      const asset = h.assets.get(id);
      if (!queued && h.download.mock.calls.length === 1) {
        queued = true; queueMicrotask(() => {
          if (change === 'replace') h.assets.set('photo', h.makeAsset(2));
          else if (change === 'close') old.el.close();
          else h.PM.GL.contextLost = true;
        });
      }
      return asset;
    });
    expect(await h.X.run(options)).toEqual(change === 'gpu-loss' ? { error: 'The GPU could not capture the export frame' } : { cancelled: false });
    expect(queued).toBe(true);
    expect(h.download).toHaveBeenCalledTimes(change === 'gpu-loss' ? 1 : 2);
    if (change !== 'gpu-loss') {
      expect(h.PM.renderFrameTo.mock.calls.map((call: any[]) => call[0])).toEqual([0, .5]);
      expect(await (h.download.mock.calls[1]![0] as Blob).text()).toBe(change === 'replace' ? '2' : '0');
    }
    expect(h.PM.renderFrameTo.mock.calls[0]![4]).toMatchObject({ width: 0, height: 0 });
    expect(old.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
    expect(h.listeners.get('assets')!.size).toBe(0);
  });

  it.each(['cancelled', 'error', 'undefined'] as const)('preserves a held PNG save outcome %s over a changed cached bitmap', async outcome => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let release!: (result: any) => void, reject!: (error: unknown) => void;
    h.download.mockImplementationOnce(() => new Promise((yes, no) => { release = yes; reject = no; }));
    const running = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.download).toHaveBeenCalledOnce());
      await new Promise(resolve => setTimeout(resolve, 15));
      h.assets.set('photo', h.makeAsset(2));
      if (outcome === 'cancelled') release({ cancelled: true });
      else reject(outcome === 'error' ? new Error('Earlier bitmap save failed') : undefined);
      expect(await running).toEqual(outcome === 'cancelled' ? { cancelled: true } : { error: outcome === 'error' ? 'Earlier bitmap save failed' : 'undefined' });
      expect(h.download).toHaveBeenCalledOnce(); expect(h.PM.renderFrameTo).toHaveBeenCalledOnce();
      expect(h.PM.renderFrameTo.mock.calls[0]![4]).toMatchObject({ width: 0, height: 0 });
      expect(old.el.close).not.toHaveBeenCalled(); expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { release?.({ ok: true }); await running; }
  });

  it('keeps a completed PNG bitmap capture when its source changes during the first encode', async () => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let encode!: () => void;
    h.PM.renderFrameTo.mockImplementationOnce((_time: number, width: number, height: number, _options: any, target: HTMLCanvasElement) => {
      target.width = width; target.height = height;
      target.toBlob = callback => { encode = () => callback(new Blob(['1'])); }; return target;
    });
    const running = h.X.run(options);
    try {
      await vi.waitFor(() => expect(encode).toBeTypeOf('function'));
      h.assets.set('photo', h.makeAsset(2)); encode();
      expect(await running).toEqual({ cancelled: false });
      expect(await Promise.all(h.download.mock.calls.map(([blob]: any[]) => blob.text()))).toEqual(['1', '2']);
      expect(h.PM.renderFrameTo.mock.calls.map((call: any[]) => call[0])).toEqual([0, .5]);
      expect(old.el.close).not.toHaveBeenCalled(); expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { encode?.(); await running; }
  });

  it.each([false, true])('reuses and releases the opaque PNG surface after delivery failure=%s', async fail => {
    const { X, PM, frame, download } = setup();
    PM.proj.layers = [{ type: 'shape', from: 0, dur: 1, p: { rotation: { expr: 'time' } } }];
    const surfaces: HTMLCanvasElement[] = [];
    PM.renderFrameTo = vi.fn((_time, width, height, _options, target: HTMLCanvasElement) => {
      target.width = width; target.height = height;
      target.toBlob = frame.toBlob as any;
      surfaces.push(target);
      return target;
    });
    if (fail) download.mockImplementation(async () => { throw new Error('Save failed'); });
    expect(await X.run(options)).toEqual(fail ? { error: 'Save failed' } : { cancelled: false });
    expect(surfaces).toHaveLength(fail ? 1 : 2);
    expect(surfaces.every(surface => surface === surfaces[0])).toBe(true);
    expect(surfaces[0]!.width).toBe(0);
    expect(surfaces[0]!.height).toBe(0);
  });

  it.each(['edit', 'project', 'boundary', 'animated'])('refreshes PNG pixels after a %s change', async change => {
    const { X, PM, frame, download } = setup();
    let version = 0;
    PM.animVersion = () => version;
    PM.renderFrameTo = vi.fn(() => frame);
    if (change === 'boundary') PM.proj.layers = [{ type: 'solid', from: .5, dur: .5 }];
    if (change === 'animated') PM.proj.layers = [{ type: 'solid', from: 0, dur: 1, p: { rotation: { expr: 'time' } } }];
    download.mockImplementation(async () => {
      if (change === 'edit') version++;
      if (change === 'project') PM.proj = { ...PM.proj };
      return { ok: true };
    });
    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(PM.renderFrameTo).toHaveBeenCalledTimes(2);
  });

  it('rejects GPU loss before writing a retained PNG', async () => {
    const { X, PM, download } = setup();
    download.mockImplementation(async () => { PM.GL.contextLost = true; return { ok: true }; });
    expect(await X.run(options)).toEqual({ error: 'The GPU could not capture the export frame' });
    expect(download).toHaveBeenCalledOnce();
  });

  function alphaReadback(depth: 16 | 32 = 16, width = 1920, height = 1080) {
    const h = setup();
    const raw = new Uint8Array(width * height * 4).fill(128);
    const source = (height - 1) * width * 4;
    raw.set([64, 32, 16, 128], source);
    h.PM.proj.w = width; h.PM.proj.h = height; h.PM.proj.colorDepth = depth;
    h.PM.proj.layers = [{ type: 'shape', from: 0, dur: 1,
      p: { rotation: { v: 0, expr: null, kf: [{ t: 0, v: 0 }, { t: 1, v: 90 }] } } }];
    let displayed: Uint8ClampedArray;
    const createImage = vi.fn((w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }));
    const put = vi.fn((image: ImageData) => { displayed = image.data; });
    const get = vi.fn(() => ({ data: new Uint8ClampedArray(displayed) }));
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({ drawImage() {}, putImageData: put,
      createImageData: createImage, getImageData: get }) as any);
    vi.spyOn(HTMLCanvasElement.prototype, 'toBlob').mockImplementation(function (_callback: BlobCallback) {
      _callback(new Blob([displayed.slice(0, 4)]));
    });
    const captures: Uint8Array[] = [];
    const capture = vi.fn((_time: number, _options: any) => { captures.push(raw); return raw; });
    const dispose = vi.fn(), create = vi.fn((_w: number, _h: number): any => ({ capture, dispose }));
    const otherCapture = vi.fn((_time: number, _options: any) => raw), otherDispose = vi.fn();
    const otherCreate = vi.fn((_w: number, _h: number): any => ({ capture: otherCapture, dispose: otherDispose }));
    h.PM.GL[depth === 32 ? 'createFloatReadback' : 'createHalfReadback'] = create;
    h.PM.GL[depth === 32 ? 'createHalfReadback' : 'createFloatReadback'] = otherCreate;
    h.PM.GL.renderToPixels.mockImplementation((_time: number, w: number, h: number) => raw.length === w * h * 4 ? raw : new Uint8Array(w * h * 4));
    const sent: number[][] = [], write = vi.fn(async (_token: string, bytes: Uint8Array) => { sent.push([...bytes.subarray(0, 4)]); });
    const finish = vi.fn(async () => ({ path: '/tmp/alpha.mov' })), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, finish, cancel } };
    return { ...h, raw, captures, capture, dispose, create, otherCapture, otherDispose, otherCreate, createImage, put, get, sent, write, finish, cancel,
      run: (extra = {}) => h.X.run({ ...options, format: 'prores', alpha: true, ...extra }) };
  }

  it.each(([16, 32] as const).flatMap(depth => ['prores', 'png'].map(format => ({ depth, format }))))('owns one reusable $depth-bit alpha capture for an animated $format sequence', async ({ depth, format }) => {
    const h = alphaReadback(depth); h.PM.quality = .5; h.PM.proj.shutter = -.25;
    const run = () => format === 'png' ? h.X.run({ ...options, alpha: true }) : h.run();
    expect(await run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledExactlyOnceWith(1920, 1080);
    expect(h.capture).toHaveBeenCalledTimes(2);
    expect(new Set(h.captures).size).toBe(1);
    expect(h.capture.mock.calls[0]).toEqual([0, { transparent: true, mblur: false, mbSamples: 20, shutter: -.25 }]);
    expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(h.createImage).toHaveBeenCalledOnce(); expect(h.put).toHaveBeenCalledTimes(2);
    if (format === 'prores') { expect(h.sent).toEqual([[127, 63, 31, 128], [255, 255, 255, 128], [127, 63, 31, 128], [255, 255, 255, 128]]); expect(h.get).toHaveBeenCalledTimes(2); }
    else {
      expect(h.get).not.toHaveBeenCalled();
      for (const [blob] of h.download.mock.calls) expect([...new Uint8Array(await blob.arrayBuffer())]).toEqual([127, 63, 31, 128]);
    }
    expect(h.dispose).toHaveBeenCalledOnce(); expect(h.PM.quality).toBe(.5);
    expect(h.otherCreate).not.toHaveBeenCalled();
  });

  it.each(([16, 32] as const).flatMap(depth => (['capture', 'canvas', 'write', 'finish', 'cancel'] as const).map(failure => ({ depth, failure }))))('releases a $depth-bit alpha readback after $failure and permits a fresh export', async ({ depth, failure }) => {
    const h = alphaReadback(depth);
    if (failure === 'capture') h.capture.mockImplementationOnce(() => { throw new Error('Alpha failed'); });
    if (failure === 'canvas') h.put.mockImplementationOnce(() => { throw new Error('Alpha failed'); });
    if (failure === 'write') h.write.mockRejectedValueOnce(new Error('Alpha failed'));
    if (failure === 'finish') h.finish.mockRejectedValueOnce(new Error('Alpha failed'));
    if (failure === 'cancel') h.write.mockImplementationOnce(async () => { h.X.cancel = true; });
    expect(await h.run()).toEqual(failure === 'cancel' ? { cancelled: true } : { error: 'Alpha failed' });
    expect(h.dispose).toHaveBeenCalledOnce();
    if (failure !== 'finish') expect(h.finish).not.toHaveBeenCalled();
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
  });

  it.each(([16, 32] as const).flatMap(depth => ['static', 'still', 'single'].map(kind => ({ depth, kind }))))('uses and releases one validated $depth-bit readback for a large alpha $kind capture', async ({ depth, kind }) => {
    const h = alphaReadback(depth);
    if (kind === 'static') h.PM.proj.layers = [];
    if (kind === 'single') h.PM.proj.dur = .5;
    expect(await h.run(kind === 'still' ? { format: 'still' } : {})).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledExactlyOnceWith(1920, 1080);
    expect(h.capture).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
    expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(h.createImage).toHaveBeenCalledOnce();
    expect(h.otherCreate).not.toHaveBeenCalled();
  });

  it.each(['small', '8-bit'] as const)('leaves alpha %s captures on their original path', async safeguard => {
    const h = alphaReadback();
    if (safeguard === 'small') { h.PM.proj.w = 2; h.PM.proj.h = 2; }
    if (safeguard === '8-bit') h.PM.proj.colorDepth = 8;
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).not.toHaveBeenCalled(); expect(h.capture).not.toHaveBeenCalled(); expect(h.dispose).not.toHaveBeenCalled();
    expect(h.PM.GL.renderToPixels).toHaveBeenCalled();
  });

  it.each(([16, 32] as const).flatMap(depth => ['png', 'prores'].map(format => ({ depth, format }))))('amortizes one $depth-bit readback across a small animated $format sequence', async ({ depth, format }) => {
    const h = alphaReadback(depth, 640, 360);
    expect(await h.run({ format, fps: 12 })).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledExactlyOnceWith(640, 360);
    expect(h.capture).toHaveBeenCalledTimes(12);
    expect(new Set(h.captures).size).toBe(1);
    expect(h.dispose).toHaveBeenCalledOnce();
    expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
  });

  it.each(([16, 32] as const).flatMap(depth => ['png', 'prores'].flatMap(format =>
    ['short', 'static', 'still', 'smaller', '8-bit'].map(safeguard => ({ depth, format, safeguard })))))(
    'keeps small $depth-bit $format $safeguard captures on the original path', async ({ depth, format, safeguard }) => {
      const h = alphaReadback(depth, safeguard === 'smaller' ? 638 : 640, 360);
      if (safeguard === 'static') h.PM.proj.layers = [];
      if (safeguard === '8-bit') h.PM.proj.colorDepth = 8;
      expect(await h.run({ format: safeguard === 'still' ? 'still' : format, fps: safeguard === 'short' ? 11 : 12 })).toEqual({ cancelled: false });
      expect(h.create).not.toHaveBeenCalled(); expect(h.capture).not.toHaveBeenCalled(); expect(h.dispose).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(safeguard === 'static' || safeguard === 'still' ? 1 : safeguard === 'short' ? 11 : 12);
    });

  it.each([16, 32] as const)('keeps a nullable %s-bit factory on the original capture path without repeated setup', async depth => {
    const h = alphaReadback(depth); h.create.mockReturnValue(null);
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledOnce(); expect(h.capture).not.toHaveBeenCalled();
    expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(2); expect(h.dispose).not.toHaveBeenCalled();
  });

  it.each([16, 32] as const)('recreates a %s-bit alpha session when the same project changes depth', async depth => {
    const h = alphaReadback(depth), project = h.PM.proj;
    h.write.mockImplementationOnce(async () => { project.colorDepth = depth === 16 ? 32 : 16; });
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.PM.proj).toBe(project);
    for (const spy of [h.create, h.capture, h.dispose, h.otherCreate, h.otherCapture, h.otherDispose]) expect(spy).toHaveBeenCalledOnce();
    expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
  });

  it.each([16, 32] as const)('invalidates a cached null %s-bit factory when the same project changes depth', async depth => {
    const h = alphaReadback(depth); h.create.mockReturnValue(null);
    h.write.mockImplementationOnce(async () => { h.PM.proj.colorDepth = depth === 16 ? 32 : 16; });
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledOnce(); expect(h.capture).not.toHaveBeenCalled(); expect(h.dispose).not.toHaveBeenCalled();
    for (const spy of [h.otherCreate, h.otherCapture, h.otherDispose]) expect(spy).toHaveBeenCalledOnce();
    expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
  });

  it('releases an alpha capture session when the project changes before the next frame', async () => {
    const h = alphaReadback();
    h.write.mockImplementationOnce(async () => { h.PM.proj = { ...h.PM.proj }; });
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
    expect(h.capture).toHaveBeenCalledTimes(2);
  });

  it('recreates the alpha half-readback when a new project makes later pictures static', async () => {
    const h = alphaReadback();
    h.write.mockImplementationOnce(async () => { h.PM.proj = { ...h.PM.proj, layers: [] }; });
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
    expect(h.capture).toHaveBeenCalledTimes(2); expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
  });

  it('drains a prior PNG write before releasing a failed speculative alpha capture', async () => {
    const h = alphaReadback();
    let reject!: (error: Error) => void;
    h.download.mockImplementationOnce(() => new Promise((_resolve, no) => { reject = no; }));
    h.capture.mockReturnValueOnce(h.raw).mockImplementationOnce(() => { throw new Error('Later alpha capture failed'); });
    const result = h.X.run({ ...options, alpha: true });
    await vi.waitFor(() => expect(h.capture).toHaveBeenCalledTimes(2));
    expect(h.dispose).not.toHaveBeenCalled();
    reject(new Error('Earlier PNG save failed'));
    expect(await result).toEqual({ error: 'Earlier PNG save failed' });
    expect(h.dispose).toHaveBeenCalledOnce(); expect(h.download).toHaveBeenCalledOnce();
  });

  it.each(([16, 32] as const).flatMap(depth => (['saved', 'cancelled', 'failed'] as const).map(outcome => ({ depth, outcome }))))('retains $depth-bit still alpha resources until the save ACK is $outcome, restores playback, and allows retry', async ({ depth, outcome }) => {
    const h = alphaReadback(depth); h.PM.time = .25; h.PM.quality = .5; h.PM.playing = true;
    h.PM.pause = vi.fn(() => { h.PM.playing = false; });
    h.PM.play = vi.fn(() => { h.PM.playing = true; });
    h.PM.setTime = vi.fn();
    let resolve!: (value: any) => void, reject!: (error: Error) => void;
    h.download.mockImplementationOnce(() => new Promise((yes, no) => { resolve = yes; reject = no; }));
    let settled = false;
    const result = h.run({ format: 'still' }).finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.download).toHaveBeenCalledOnce());
      const canvas = vi.mocked(HTMLCanvasElement.prototype.toBlob).mock.contexts[0] as HTMLCanvasElement;
      expect(canvas.width).toBe(1920); expect(canvas.height).toBe(1080);
      expect(h.capture).toHaveBeenCalledOnce(); expect(h.dispose).not.toHaveBeenCalled();
      expect(h.PM.setTime).not.toHaveBeenCalled(); expect(h.PM.play).not.toHaveBeenCalled();
      expect(settled).toBe(false); expect(h.X.busy).toBe(true);
      if (outcome === 'failed') reject(new Error('Still save failed'));
      else resolve(outcome === 'cancelled' ? { ok: false, cancelled: true } : { ok: true });
      expect(await result).toEqual(outcome === 'failed' ? { error: 'Still save failed' } : { cancelled: outcome === 'cancelled' });
      expect(h.dispose).toHaveBeenCalledOnce(); expect(canvas).toMatchObject({ width: 0, height: 0 });
      expect(h.PM.preparedVideoFrames).toBeNull(); expect(h.PM.setTime).toHaveBeenCalledWith(.25, { force: true });
      expect(h.PM.play).toHaveBeenCalledOnce(); expect(h.PM.playing).toBe(true); expect(h.PM.quality).toBe(.5);
      expect(await h.run({ format: 'still' })).toEqual({ cancelled: false });
      expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
    } finally { resolve?.({ ok: true }); await result; }
  });

  it.each(['preparation', 'capture', 'canvas', 'encoding'] as const)('cleans up a still alpha %s failure and restores its source state before retry', async failure => {
    const h = alphaReadback(); h.PM.time = .25; h.PM.playing = true;
    h.PM.pause = vi.fn(() => { h.PM.playing = false; }); h.PM.play = vi.fn(() => { h.PM.playing = true; }); h.PM.setTime = vi.fn();
    if (failure === 'preparation') vi.spyOn(framePreparation, 'prepareFrame').mockRejectedValueOnce(new Error('Still failed'));
    if (failure === 'capture') h.capture.mockImplementationOnce(() => { throw new Error('Still failed'); });
    if (failure === 'canvas') h.put.mockImplementationOnce(() => { throw new Error('Still failed'); });
    if (failure === 'encoding') vi.mocked(HTMLCanvasElement.prototype.toBlob).mockImplementationOnce(callback => { callback(null); });
    expect(await h.run({ format: 'still' })).toEqual({ error: failure === 'encoding' ? 'Could not encode frame' : 'Still failed' });
    expect(h.download).not.toHaveBeenCalled(); expect(h.dispose).toHaveBeenCalledTimes(failure === 'preparation' ? 0 : 1);
    expect(h.PM.preparedVideoFrames).toBeNull(); expect(h.PM.setTime).toHaveBeenCalledWith(.25, { force: true });
    expect(h.PM.play).toHaveBeenCalledOnce(); expect(h.PM.playing).toBe(true); expect(h.X.busy).toBe(false);
    expect(await h.run({ format: 'still' })).toEqual({ cancelled: false });
    expect(h.dispose).toHaveBeenCalledTimes(failure === 'preparation' ? 1 : 2);
  });

  it.each(([16, 32] as const).flatMap(depth => ['prores', 'png'].map(format => ({ depth, format }))))('releases $depth-bit static alpha raw/GPU storage before its cached $format delivery ACK', async ({ depth, format }) => {
    const h = alphaReadback(depth); h.PM.proj.layers = [];
    let resolve!: () => void;
    const held = new Promise<void>(yes => { resolve = yes; });
    if (format === 'png') h.download.mockImplementationOnce(async () => { await held; return { ok: true }; });
    else h.write.mockImplementationOnce(async (_token: string, bytes: Uint8Array) => {
      expect([...bytes.subarray(0, 4)]).toEqual([127, 63, 31, 128]); await held;
      expect([...bytes.subarray(0, 4)]).toEqual([127, 63, 31, 128]);
    });
    const result = format === 'png' ? h.X.run({ ...options, alpha: true }) : h.run();
    try {
      // A 1080p alpha frame is two IPC chunks, both in flight together.
      await vi.waitFor(() => format === 'png' ? expect(h.download).toHaveBeenCalledOnce() : expect(h.write).toHaveBeenCalledTimes(2));
      expect(h.capture).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
      expect(h.create).toHaveBeenCalledOnce(); expect(h.finish).not.toHaveBeenCalled();
      if (format === 'png') expect([...new Uint8Array(await h.download.mock.calls[0]![0].arrayBuffer())]).toEqual([127, 63, 31, 128]);
      resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.create).toHaveBeenCalledOnce(); expect(h.capture).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
      if (format === 'png') {
        expect(h.download).toHaveBeenCalledTimes(2);
        expect(h.download.mock.calls[1]![0]).toBe(h.download.mock.calls[0]![0]);
      } else expect(h.write).toHaveBeenCalledTimes(4);
    } finally { resolve(); await result; }
  });

  it('keeps one alpha session across unique motion-blur edge pictures instead of reallocating it per frame', async () => {
    const h = alphaReadback();
    h.PM.proj.layers[0].p = {}; h.PM.proj.layers[0].mblur = true; h.PM.proj.shutter = 4;
    let resolve!: () => void;
    h.write.mockImplementationOnce(async () => { await new Promise<void>(yes => { resolve = yes; }); });
    const result = h.run({ mblur: true });
    try {
      // A 1080p alpha frame is two IPC chunks, both in flight together.
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledTimes(2));
      expect(h.capture).toHaveBeenCalledOnce(); expect(h.dispose).not.toHaveBeenCalled();
      resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.create).toHaveBeenCalledOnce(); expect(h.capture).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledOnce();
    } finally { resolve?.(); await result; }
  });

  function nativeReadback() {
    const h = setup();
    h.PM.proj.fps = 3;
    h.PM.proj.layers = [{ type: 'solid', from: 0, dur: 1, p: { rotation: { v: 0, expr: null, kf: [{ t: 0, v: 0 }, { t: 1, v: 90 }] } } }];
    const prepare = vi.spyOn(framePreparation, 'prepareFrame').mockResolvedValue();
    const fonts = vi.spyOn(framePreparation, 'frameFontsReady').mockReturnValue(true);
    let resolve!: () => void, reject!: (error: unknown) => void;
    const held = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
    let activeWrites = 0, maxWrites = 0, activeTickets = 0, maxTickets = 0;
    const targets: Uint8Array[] = [], received: number[][] = [];
    const tickets: Array<{ paint: number; readInto: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn> }> = [];
    const controls = { color: 0, beginFailure: false, read: null as null | ((target: Uint8Array, cancelled: () => boolean) => Promise<boolean>) };
    const begin = vi.fn((_time: number, _options: any) => {
      if (controls.beginFailure) throw new Error('GPU submission failed');
      activeTickets++; maxTickets = Math.max(maxTickets, activeTickets);
      let disposed = false;
      const entry = { paint: controls.color || tickets.length + 1,
        readInto: vi.fn(async (target: Uint8Array, cancelled: () => boolean) => {
          targets.push(target);
          if (controls.read) return controls.read(target, cancelled);
          target.fill(entry.paint); return true;
        }), dispose: vi.fn(() => { if (!disposed) { disposed = true; activeTickets--; } }) };
      tickets.push(entry); return entry;
    });
    const dispose = vi.fn(), create = vi.fn(() => ({ begin, dispose }));
    h.PM.GL.createOpaqueReadback = create;
    const write = vi.fn(async (_token: string, bytes: Uint8Array) => {
      activeWrites++; maxWrites = Math.max(maxWrites, activeWrites);
      received.push([...bytes]);
      try { if (received.length === 1) await held; } finally { activeWrites--; }
    });
    const finish = vi.fn(async () => ({ path: '/tmp/native.mp4' })), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, finish, cancel } };
    return { ...h, prepare, fonts, resolve, reject, targets, received, tickets, controls, begin, create, dispose, write, finish, cancel,
      maxWrites: () => maxWrites, maxTickets: () => maxTickets, activeTickets: () => activeTickets,
      run: (extra = {}) => h.X.run({ ...options, format: 'mp4', fps: 3, ...extra }) };
  }

  function nativeAlphaReadback(depth: 16 | 32 = 16) {
    const h = alphaReadback(depth), width = 1920, height = 1080, visible = width * height * 4;
    h.PM.proj.fps = 12; h.PM.quality = .5;
    const prepare = vi.spyOn(framePreparation, 'prepareFrame').mockResolvedValue();
    const fonts = vi.spyOn(framePreparation, 'frameFontsReady').mockReturnValue(true);
    let resolve!: () => void, reject!: (error: unknown) => void;
    const held = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
    let activeTickets = 0, maxTickets = 0, activeWrites = 0, maxWrites = 0, activeSessions = 0, maxSessions = 0;
    const targets: Uint8Array[] = [], sent: number[][] = [], lengths: number[] = [], qualities: number[] = [];
    const tickets: Array<{ paint: number; readInto: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn> }> = [];
    const controls = { paint: 32, read: null as null | ((target: Uint8Array, cancelled: () => boolean) => Promise<boolean>) };
    const begin = vi.fn((_time: number, _options: any): any => {
      qualities.push(h.PM.quality); activeTickets++; maxTickets = Math.max(maxTickets, activeTickets);
      let disposed = false;
      const ticket = { paint: controls.paint,
        readInto: vi.fn(async (target: Uint8Array, cancelled: () => boolean) => {
          targets.push(target);
          if (controls.read) return controls.read(target, cancelled);
          target.fill(128); target.set([ticket.paint, 16, 8, 128], (height - 1) * width * 4); target.fill(211, visible);
          return !cancelled();
        }), dispose: vi.fn(() => { if (!disposed) { disposed = true; activeTickets--; } }) };
      tickets.push(ticket); return ticket;
    });
    const dispose = vi.fn();
    const create = vi.fn((): any => {
      activeSessions++; maxSessions = Math.max(maxSessions, activeSessions); let disposed = false;
      return { byteLength: width * (height + 1) * 4, begin, dispose() {
        if (!disposed) { disposed = true; activeSessions--; dispose(); }
      } };
    });
    h.PM.GL.createAlphaReadback = create;
    h.write.mockImplementation(async (_token: string, bytes: Uint8Array) => {
      sent.push([...bytes.subarray(0, 4)]); lengths.push(bytes.length);
      activeWrites++; maxWrites = Math.max(maxWrites, activeWrites);
      // Hold the final chunk, proving all chunks acknowledge before raw/Canvas reuse.
      try { if (sent.length === 2) await held; } finally { activeWrites--; }
    });
    return { ...h, prepare, fonts, asyncCreate: create, asyncDispose: dispose, begin, tickets, controls, targets, sent, lengths, qualities, resolve, reject,
      activeTickets: () => activeTickets, maxTickets: () => maxTickets, activeSessions: () => activeSessions,
      maxSessions: () => maxSessions, maxWrites: () => maxWrites,
      run: (extra = {}) => h.X.run({ ...options, format: 'prores', alpha: true, fps: 12, ...extra }) };
  }

  it.each([16, 32] as const)('overlaps %s-bit native alpha without raw, Canvas, or next-write reuse before the final chunk ACK', async depth => {
    const h = nativeAlphaReadback(depth), running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledTimes(2));
      expect(h.targets).toHaveLength(1); expect(h.targets[0]!.length).toBe(1920 * 1081 * 4);
      expect(h.put).toHaveBeenCalledOnce(); expect(h.get).toHaveBeenCalledOnce(); expect(h.PM.quality).toBe(.5);
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.asyncCreate).toHaveBeenCalledExactlyOnceWith(1920, 1080); expect(h.begin).toHaveBeenCalledTimes(12);
      expect(h.qualities).toEqual(Array(12).fill(.5));
      expect(h.begin.mock.calls[0]).toEqual([0, { transparent: true, mblur: false, mbSamples: 20, shutter: .5 }]);
      expect(h.create).not.toHaveBeenCalled(); expect(h.otherCreate).not.toHaveBeenCalled(); expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
      expect(h.createImage).toHaveBeenCalledOnce(); expect(h.put).toHaveBeenCalledTimes(12); expect(h.get).toHaveBeenCalledTimes(12);
      expect(h.sent.filter((_value, index) => index % 2 === 0)).toEqual(Array.from({ length: 12 }, () => [63, 31, 15, 128]));
      expect(h.lengths).toEqual(Array.from({ length: 12 }, () => [4 * 1024 * 1024, 1920 * 1080 * 4 - 4 * 1024 * 1024]).flat());
      expect(new Set(h.targets).size).toBe(1); expect(h.maxTickets()).toBe(1); expect(h.maxSessions()).toBe(1); expect(h.maxWrites()).toBe(1);
      expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0); expect(h.asyncDispose).toHaveBeenCalledOnce();
    } finally { h.resolve(); await running; }
  });

  it.each(['short', 'small', 'guard-limit', '8-bit', 'static', 'image', 'video', 'other-format', 'expression'] as const)
    ('preserves original serial alpha for a %s export', async reason => {
      const h = nativeAlphaReadback();
      if (reason === 'short') h.PM.proj.dur = .5;
      if (reason === 'small' || reason === 'guard-limit') {
        h.PM.proj.w = reason === 'small' ? 1280 : 4096; h.PM.proj.h = reason === 'small' ? 720 : 2048;
        h.create.mockReturnValue(null);
      }
      if (reason === '8-bit') h.PM.proj.colorDepth = 8;
      if (reason === 'static') h.PM.proj.layers = [];
      if (reason === 'image' || reason === 'video') h.PM.proj.layers[0].type = reason;
      if (reason === 'expression') h.PM.proj.layers[0].p.rotation.expr = 'time';
      h.write.mockImplementation(async () => { h.X.cancel = true; }); h.resolve();
      expect(await h.run(reason === 'other-format' ? { format: 'mp4' } : {})).toEqual({ cancelled: true });
      expect(h.asyncCreate).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.put).toHaveBeenCalledOnce(); expect(h.get).toHaveBeenCalledOnce();
    });

  it('keeps a null asynchronous alpha factory cached without repeated allocation attempts', async () => {
    const h = nativeAlphaReadback(); h.asyncCreate.mockReturnValue(null); h.resolve();
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.asyncCreate).toHaveBeenCalledOnce(); expect(h.begin).not.toHaveBeenCalled();
    expect(h.create).toHaveBeenCalledOnce(); expect(h.capture).toHaveBeenCalledTimes(12); expect(h.dispose).toHaveBeenCalledOnce();
  });

  it('releases an existing synchronous alpha target before retrying the async factory for a new project', async () => {
    const h = nativeAlphaReadback(), original = h.asyncCreate.getMockImplementation()!;
    h.asyncCreate.mockReturnValueOnce(null).mockImplementation(() => {
      expect(h.dispose).toHaveBeenCalledOnce(); return original();
    });
    h.write.mockImplementation(async () => {
      if (h.write.mock.calls.length === 2) h.PM.proj = { ...h.PM.proj };
    }); h.resolve();
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.asyncCreate).toHaveBeenCalledTimes(2); expect(h.capture).toHaveBeenCalledOnce();
    expect(h.begin).toHaveBeenCalledTimes(11); expect(h.dispose).toHaveBeenCalledOnce(); expect(h.asyncDispose).toHaveBeenCalledOnce();
    expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0);
  });

  it.each(['null', 'guard'] as const)('serially recaptures the same alpha timestamp after a healthy %s result and disables that descriptor', async kind => {
    const h = nativeAlphaReadback();
    if (kind === 'null') h.begin.mockReturnValueOnce(null);
    else h.controls.read = async () => { throw new AlphaReadbackRetryError(); };
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.asyncCreate).toHaveBeenCalledOnce(); expect(h.begin).toHaveBeenCalledOnce(); expect(h.asyncDispose).toHaveBeenCalledOnce();
    expect(h.capture.mock.calls.map(call => call[0])).toEqual(Array.from({ length: 12 }, (_value, i) => i / 12));
    expect(h.prepare.mock.calls.slice(0, 2).map(call => call[1])).toEqual([0, 0]);
    expect(h.sent[0]).toEqual([127, 63, 31, 128]); expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0);
  });

  it.each(['quality', 'depth', 'project', 'fonts', 'provider-default'] as const)
    ('discards stale alpha after %s changes during the preceding ACK and recaptures the same timestamp', async change => {
      const h = nativeAlphaReadback(); let parameter: any;
      if (change === 'provider-default') {
        const definition = structuredClone(EFFECTS.find(effect => effect.id === 'blur')!.legacy![0]!), registry = new Registry<any>();
        registry.register('effects-basic', definition); h.PM.Kernel = { effects: registry }; parameter = definition.params[0];
        h.PM.proj.layers[0].fx = [{ type: 'blur', p: {} }];
      }
      const running = h.run();
      try {
        await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
        if (change === 'quality') h.PM.quality = .25;
        if (change === 'depth') h.PM.proj.colorDepth = 32;
        if (change === 'project') h.PM.proj = { ...h.PM.proj };
        if (change === 'fonts') h.fonts.mockReturnValue(false);
        if (change === 'provider-default') parameter.def = 89;
        h.resolve(); expect(await running).toEqual({ cancelled: false });
        expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
        const serial = change === 'depth' ? h.otherCapture : h.capture;
        expect(serial.mock.calls[0]![0]).toBe(1 / 12);
        expect(h.sent[0]).toEqual([63, 31, 15, 128]); expect(h.sent[2]).toEqual([127, 63, 31, 128]);
        expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0); expect(h.maxWrites()).toBe(1); expect(h.maxTickets()).toBe(1);
      } finally { h.resolve(); await running; }
    });

  it.each(['cancelled', 'error', 'undefined'] as const)('keeps the earlier alpha write %s outcome ahead of a queued frame failure', async outcome => {
    const h = nativeAlphaReadback(), running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      h.X.cancel = true; h.PM.proj.colorDepth = 32;
      h.controls.read = async () => { throw new AlphaReadbackRetryError(); };
      if (outcome === 'cancelled') h.resolve(); else h.reject(outcome === 'error' ? new Error('Earlier alpha write failed') : undefined);
      expect(await running).toEqual(outcome === 'cancelled' ? { cancelled: true } : { error: outcome === 'error' ? 'Earlier alpha write failed' : 'undefined' });
      expect(h.write).toHaveBeenCalledTimes(2); expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      expect(h.finish).not.toHaveBeenCalled(); expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each(['guard', 'cancel', 'loss'] as const)('handles alpha %s while waiting for its GPU fence without confusing retry with cancellation', async outcome => {
    const h = nativeAlphaReadback(); let complete!: () => void;
    h.controls.read = async () => {
      await new Promise<void>(yes => { complete = yes; });
      if (outcome === 'guard') { h.controls.read = null; throw new AlphaReadbackRetryError(); }
      if (outcome === 'loss') { h.PM.GL.contextLost = true; throw new Error('The GPU could not capture the export frame'); }
      h.X.cancel = true; return false;
    };
    const running = h.run();
    try {
      await vi.waitFor(() => expect(complete).toBeTypeOf('function'));
      expect(h.PM.quality).toBe(.5); expect(h.write).not.toHaveBeenCalled();
      complete(); h.resolve();
      expect(await running).toEqual(outcome === 'guard' ? { cancelled: false } : outcome === 'cancel' ? { cancelled: true }
        : { error: 'The GPU could not capture the export frame' });
      expect(h.write).toHaveBeenCalledTimes(outcome === 'guard' ? 24 : 0);
      expect(h.capture).toHaveBeenCalledTimes(outcome === 'guard' ? 12 : 0);
      expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0); expect(h.asyncDispose).toHaveBeenCalledOnce();
    } finally { complete?.(); h.resolve(); await running; }
  });

  it('retains grouped effects/path eligibility and exact alpha motion-blur options', async () => {
    const h = nativeAlphaReadback(), registry = new Registry<any>();
    registry.register('effects-basic', EFFECTS.find(effect => effect.id === 'glow')!); h.PM.Kernel = { effects: registry };
    const layer = h.PM.proj.layers[0]; layer.id = 'group'; layer.type = 'group'; layer.mblur = true;
    layer.fx = [{ type: 'glow', p: {} }]; layer.masks = [nativePathMask()]; h.PM.proj.shutter = -.75;
    h.PM.proj.layers.push({ id: 'child', type: 'shape', group: layer.id, from: 0, dur: 1, p: {} }); h.resolve();
    expect(await h.run({ mblur: true })).toEqual({ cancelled: false });
    expect(h.begin).toHaveBeenCalledTimes(12);
    expect(h.begin.mock.calls[0]).toEqual([0, { transparent: true, mblur: true, mbSamples: 20, shutter: -.75 }]);
    expect(h.qualities).toEqual(Array(12).fill(.5)); expect(h.create).not.toHaveBeenCalled();
    expect(h.asyncDispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
  });

  it.each([16, 32] as const)('invalidates a null %s-bit async factory when the same project changes requested depth', async depth => {
    const h = nativeAlphaReadback(depth); h.asyncCreate.mockReturnValueOnce(null);
    h.write.mockImplementation(async () => { if (h.write.mock.calls.length === 2) h.PM.proj.colorDepth = depth === 16 ? 32 : 16; });
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.asyncCreate).toHaveBeenCalledTimes(2); expect(h.capture).toHaveBeenCalledOnce();
    expect(h.begin).toHaveBeenCalledTimes(11); expect(h.dispose).toHaveBeenCalledOnce();
    expect(h.otherCreate).not.toHaveBeenCalled(); expect(h.asyncDispose).toHaveBeenCalledOnce();
    expect(h.maxSessions()).toBe(1); expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0);
  });

  it.each(['quality', 'depth'] as const)('recaptures the same alpha timestamp when %s changes during its GPU fence', async change => {
    const h = nativeAlphaReadback(); let complete!: () => void;
    h.controls.read = async target => {
      target.fill(128); target.set([32, 16, 8, 128], (1080 - 1) * 1920 * 4);
      await new Promise<void>(yes => { complete = yes; }); return true;
    };
    const running = h.run();
    try {
      await vi.waitFor(() => expect(complete).toBeTypeOf('function'));
      if (change === 'quality') h.PM.quality = .25; else h.PM.proj.colorDepth = 32;
      h.controls.read = null; complete();
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledTimes(2)); h.resolve();
      expect(await running).toEqual({ cancelled: false });
      const serial = change === 'depth' ? h.otherCapture : h.capture;
      expect(serial.mock.calls[0]![0]).toBe(0); expect(h.sent[0]).toEqual([127, 63, 31, 128]);
      expect(h.prepare.mock.calls.slice(0, 2).map(call => call[1])).toEqual([0, 0]);
      expect(h.asyncCreate).toHaveBeenCalledTimes(2); expect(h.asyncDispose).toHaveBeenCalledTimes(2);
      expect(h.maxTickets()).toBe(1); expect(h.maxSessions()).toBe(1); expect(h.activeTickets()).toBe(0);
    } finally { complete?.(); h.resolve(); await running; }
  });

  it('joins a null alpha begin before any serial recapture and keeps an earlier undefined write rejection', async () => {
    const h = nativeAlphaReadback(), original = h.begin.getMockImplementation()!;
    h.begin.mockImplementationOnce(original).mockReturnValueOnce(null);
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.capture).not.toHaveBeenCalled(); expect(h.targets).toHaveLength(1);
      h.reject(undefined); expect(await running).toEqual({ error: 'undefined' });
      expect(h.capture).not.toHaveBeenCalled(); expect(h.write).toHaveBeenCalledTimes(2); expect(h.finish).not.toHaveBeenCalled();
      expect(h.asyncDispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0); expect(h.activeSessions()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each([false, true])('keeps cold structurally static alpha effects serial when the linked program becomes ready=%s', async ready => {
    const h = nativeAlphaReadback(), definition = EFFECTS.find(effect => effect.id === 'blur')!.legacy![0]!, registry = new Registry<any>();
    registry.register('effects-basic', definition); h.PM.Kernel = { effects: registry }; h.PM.FX = { blur: toLegacyFx(definition, '') };
    h.PM.proj.layers[0].p.rotation.kf = [];
    h.PM.proj.layers[0].fx = [{ type: 'blur', on: true, p: { amount: { v: 13, kf: [] } } }];
    h.PM.GL.gl = { isContextLost: () => false, getUniformLocation: vi.fn(() => null) };
    h.PM.GL.progs = new Map(); h.PM.GL.errors = new Map();
    const original = h.write.getMockImplementation()!;
    h.write.mockImplementation(async (token: string, bytes: Uint8Array) => {
      const publish = ready && h.write.mock.calls.length === 2;
      await original(token, bytes);
      if (publish) h.PM.GL.progs.set('fx:blur', { pr: {}, u: new Map() });
    });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledTimes(2));
      expect(h.asyncCreate).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.capture).toHaveBeenCalledOnce();
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.asyncCreate).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled(); expect(h.targets).toHaveLength(0);
      expect(h.capture.mock.calls.map(call => call[0])).toEqual(ready ? [0, 1 / 12] : Array.from({ length: 12 }, (_value, i) => i / 12));
      expect(h.create).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
    } finally { h.resolve(); await running; }
  });

  function nativeEffects(ids = ['blur']) {
    const h = nativeReadback(), registry = new Registry<any>();
    for (const id of ids) registry.register('effects-basic', EFFECTS.find(effect => effect.id === id)!);
    h.PM.Kernel = { effects: registry };
    h.PM.proj.layers[0].fx = ids.map((type, index) => ({ id: `effect-${index}`, type, version: registry.get(type)?.version ?? 1, on: true,
      p: Object.fromEntries(EFFECTS.find(effect => effect.id === type)!.params.map(param => [param.k, { v: param.def, kf: [] }])) }));
    return { ...h, registry, definition: EFFECTS.find(effect => effect.id === ids[0])! };
  }

  it.each([['blur'], ['glow'], ['shadow'], ['blur', 'shadow'], ['grain'], ['displace', 'chroma'], ['gradient'], ['duotone']].map(ids => ({ ids })))('overlaps native capture with trusted built-in effects $ids', async ({ ids }) => {
    const h = nativeEffects(ids), running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.write).toHaveBeenCalledOnce(); expect(h.targets).toHaveLength(1);
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(h.begin).toHaveBeenCalledTimes(3);
      expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1); expect(new Set(h.targets).size).toBe(1);
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each(['ACK', 'GPU read'] as const)('recaptures after a shared effect default changes during a pending %s', async stage => {
    const h = nativeEffects(), definition = structuredClone(h.definition);
    (definition.params[0] as any).def = 24;
    h.registry.register('effects-basic', definition);
    delete h.PM.proj.layers[0].fx[0].p.amount;
    const legacy = toLegacyFx(definition, ''); h.PM.FX = { blur: legacy };
    const currentDefault = () => Number(legacy.params[0]!.def);
    const original = h.begin.getMockImplementation()!;
    h.begin.mockImplementation((...args: any[]) => { h.controls.color = currentDefault(); return original(args[0], args[1]); });
    h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(16).fill(currentDefault()));
    let complete: (() => void) | undefined, reads = 0;
    if (stage === 'GPU read') h.controls.read = async target => {
      target.fill(currentDefault());
      if (++reads === 1) await new Promise<void>(yes => { complete = yes; });
      return true;
    };
    const running = h.run();
    try {
      if (stage === 'ACK') await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      else await vi.waitFor(() => expect(complete).toBeTypeOf('function'));
      (definition.params[0] as any).def = 89;
      if (stage === 'GPU read') {
        complete!(); await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      }
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual([stage === 'ACK' ? 1 / 3 : 0]);
      expect(h.received.map(bytes => bytes[0])).toEqual(stage === 'ACK' ? [24, 89, 89] : [89, 89, 89]);
      if (stage === 'ACK') expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
      expect(h.activeTickets()).toBe(0); expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1);
    } finally { complete?.(); h.resolve(); await running; }
  });

  it('bypasses effect provider tracking entirely for native artwork without effects', async () => {
    const h = nativeReadback(), allow = vi.spyOn(exportEffects, 'allow'), capture = vi.spyOn(exportEffects, 'capture'), valid = vi.spyOn(exportEffects, 'valid');
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.begin).toHaveBeenCalledTimes(3);
    expect(allow).not.toHaveBeenCalled(); expect(capture).not.toHaveBeenCalled(); expect(valid).not.toHaveBeenCalled();
  });

  it.each(['override', 'registry', 'same-item-batch'] as const)('discards the same native timestamp after a provider %s while the prior ACK is held', async change => {
    const h = nativeEffects(); h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    let restore: undefined | (() => void);
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      if (change === 'override') {
        const override = h.registry.register('custom-effects', { ...h.definition, frag: 'custom shader' });
        restore = () => override.dispose();
        h.PM.GL.renderToPixels.mockImplementationOnce(() => { restore!(); return new Uint8Array(16).fill(9); });
      } else if (change === 'registry') {
        const replacement = new Registry<any>(); replacement.register('effects-basic', h.definition);
        h.PM.Kernel.effects = replacement;
      } else {
        const batch = h.registry.batchChanges(); h.registry.register('effects-basic', h.definition); batch.dispose();
      }
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual([1 / 3]);
      expect(h.received.map(bytes => bytes[0])).toEqual([1, 9, 3]);
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled(); expect(h.activeTickets()).toBe(0);
      expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
      expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1);
    } finally { restore?.(); h.resolve(); await running; }
  });

  it('rejects a same-item provider registration during begin even when a registry batch suppresses its notification', async () => {
    const h = nativeEffects(), original = h.begin.getMockImplementation()!, notification = vi.fn();
    h.registry.onChange(notification);
    h.begin.mockImplementationOnce((...args: any[]) => {
      const ticket = original(args[0], args[1]), batch = h.registry.batchChanges();
      h.registry.register('effects-basic', h.definition); batch.dispose(); return ticket;
    });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(notification).not.toHaveBeenCalled(); expect(h.tickets[0]!.readInto).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(0);
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([0, 1 / 3, 2 / 3]);
      expect(h.create).toHaveBeenCalledTimes(2); expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it('rechecks the provider after a synchronous GPU factory override before calling begin', async () => {
    const h = nativeEffects(), original = h.create.getMockImplementation()!;
    const overrides: Array<{ dispose(): void }> = [];
    h.create.mockImplementationOnce(() => {
      const session = original(); overrides.push(h.registry.register('custom-effects', { ...h.definition, frag: 'custom shader' }));
      return session;
    });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.begin).not.toHaveBeenCalled(); expect(h.dispose).toHaveBeenCalledOnce();
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3); expect(h.create).toHaveBeenCalledOnce();
      expect(h.activeTickets()).toBe(0);
    } finally { for (const override of overrides) override.dispose(); h.resolve(); await running; }
  });

  it('recaptures after a provider changes during a pending GPU read and resumes after built-in restoration', async () => {
    const h = nativeEffects(); let complete!: () => void, reads = 0;
    h.controls.read = async target => { target.fill(++reads); if (reads === 1) await new Promise<void>(yes => { complete = yes; }); return true; };
    let override: { dispose(): void } | undefined;
    h.PM.GL.renderToPixels.mockImplementationOnce(() => { override!.dispose(); return new Uint8Array(16).fill(9); });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(complete).toBeTypeOf('function'));
      override = h.registry.register('custom-effects', { ...h.definition, frag: 'custom shader' });
      complete(); await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(0); expect(h.received[0]).toEqual(Array(16).fill(9));
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.received.map(bytes => bytes[0])).toEqual([9, 2, 3]);
      expect(h.begin).toHaveBeenCalledTimes(3); expect(h.create).toHaveBeenCalledTimes(2);
      expect(h.dispose).toHaveBeenCalledTimes(2); expect(h.activeTickets()).toBe(0);
    } finally { override?.dispose(); complete?.(); h.resolve(); await running; }
  });

  it.each(['cancelled', 'error', 'undefined'] as const)('keeps the earlier native write %s outcome ahead of a changed provider', async outcome => {
    const h = nativeEffects(), running = h.run(); let override: { dispose(): void } | undefined;
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      override = h.registry.register('custom-effects', { ...h.definition, frag: 'custom shader' }); h.X.cancel = true;
      if (outcome === 'cancelled') h.resolve(); else h.reject(outcome === 'error' ? new Error('Earlier effect write failed') : undefined);
      expect(await running).toEqual(outcome === 'cancelled' ? { cancelled: true } : { error: outcome === 'error' ? 'Earlier effect write failed' : 'undefined' });
      expect(h.write).toHaveBeenCalledOnce(); expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled(); expect(h.finish).not.toHaveBeenCalled();
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
    } finally { override?.dispose(); h.resolve(); await running; }
  });

  it.each(['blur', 'grain', 'gradient'])('keeps a custom %s provider serial without allocating a GPU target and can recover without a project edit', async id => {
    const h = nativeEffects([id]), override = h.registry.register('custom-effects', { ...h.definition, frag: 'custom shader' });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.create).not.toHaveBeenCalled(); expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
      override.dispose(); h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([1 / 3, 2 / 3]);
      expect(h.create).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
    } finally { override.dispose(); h.resolve(); await running; }
  });

  it.each(['rect', 'ellipse'])('captures an animated analytic %s mask through the native PBO path', async shape => {
    const h = nativeReadback();
    h.PM.proj.layers[0].masks = [{ id: 'mask', shape, mode: 'subtract', on: true, p: {
      x: { v: 0, kf: [{ t: 0, v: 0 }, { t: 1, v: 1 }] }, y: { v: 0, kf: [] }, w: { v: 2, kf: [] },
      h: { v: 2, kf: [] }, rotation: { v: 0, kf: [] }, feather: { v: 1, kf: [] },
    } }];
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.begin).toHaveBeenCalledTimes(3); expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
    expect(h.dispose).toHaveBeenCalledOnce(); expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1);
  });

  function nativePathMask() {
    return { id: 'path-mask', shape: 'rect', mode: 'subtract', on: true,
      p: { x: { v: 0, kf: [] }, feather: { v: 1, kf: [] } },
      path: { p: { closed: { v: true, kf: [] }, rotation: { v: 0, kf: [] } }, vertices: [
        { id: 'v0', p: { x: { v: 4, kf: [{ t: 0, v: 4 }, { t: 1, v: 9 }] }, y: { v: 0, kf: [] } } },
        { id: 'v1', p: { x: { v: 2, kf: [] }, y: { v: 2, kf: [] } } },
      ] } };
  }

  it.each(['layer', 'mixed', 'group-effects'] as const)('overlaps built-in %s path-mask capture while preserving one CPU target and one ticket', async variant => {
    const h = variant === 'group-effects' ? nativeEffects(['glow']) : nativeReadback(), layer = h.PM.proj.layers[0];
    layer.masks = [nativePathMask()];
    if (variant === 'mixed') layer.masks.push({ id: 'ellipse', shape: 'ellipse', mode: 'add', p: { w: { v: 2, kf: [] } } });
    if (variant === 'group-effects') {
      layer.id = 'group'; layer.type = 'group';
      h.PM.proj.layers.push({ id: 'child', type: 'shape', group: layer.id, from: 0, dur: 1, p: {} });
    }
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.write).toHaveBeenCalledOnce(); expect(h.targets).toHaveLength(1);
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([0, 1 / 3, 2 / 3]);
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(h.create).toHaveBeenCalledOnce();
      expect(new Set(h.targets).size).toBe(1); expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1);
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each(['string', 'missing-path-properties', 'missing-vertices', 'object-vertices', 'null-vertex', 'missing-vertex-properties', 'array-path', 'array-path-properties', 'array-vertex', 'array-vertex-properties', 'path-expression', 'vertex-expression'] as const)
    ('keeps an unsupported %s path mask on the original serial capture path', async invalid => {
      const h = nativeReadback(), mask: any = nativePathMask();
      if (invalid === 'string') mask.path = 'not-a-built-in-path';
      else if (invalid === 'missing-path-properties') delete mask.path.p;
      else if (invalid === 'missing-vertices') delete mask.path.vertices;
      else if (invalid === 'object-vertices') mask.path.vertices = {};
      else if (invalid === 'null-vertex') mask.path.vertices[0] = null;
      else if (invalid === 'missing-vertex-properties') delete mask.path.vertices[0].p;
      else if (invalid === 'array-path') mask.path = Object.assign([], mask.path);
      else if (invalid === 'array-path-properties') mask.path.p = [];
      else if (invalid === 'array-vertex') mask.path.vertices[0] = Object.assign([], mask.path.vertices[0]);
      else if (invalid === 'array-vertex-properties') mask.path.vertices[0].p = [];
      else if (invalid === 'path-expression') mask.path.p.rotation.expr = 'time';
      else mask.path.vertices[0].p.x.expr = 'time';
      h.PM.proj.layers[0].masks = [mask]; h.resolve();
      expect(await h.run()).toEqual({ cancelled: false });
      expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual([0, 1 / 3, 2 / 3]);
    });

  it('discards a queued path-mask frame after a vertex edit during the preceding ACK and recaptures that timestamp', async () => {
    const h = nativeReadback(), mask = nativePathMask(); h.PM.proj.layers[0].masks = [mask];
    let version = 0; h.PM.animVersion = () => version; h.PM.touch = () => { version++; };
    const original = h.begin.getMockImplementation()!;
    h.begin.mockImplementation((...args: any[]) => { h.controls.color = mask.path.vertices[0]!.p.x.v; return original(args[0], args[1]); });
    h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(16).fill(mask.path.vertices[0]!.p.x.v));
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      mask.path.vertices[0]!.p.x.v = 9; h.PM.touch(); h.resolve();
      expect(await running).toEqual({ cancelled: false });
      expect(h.received.map(bytes => bytes[0])).toEqual([4, 9, 9]);
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual([1 / 3]);
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([0, 1 / 3, 2 / 3]);
      expect(h.create).toHaveBeenCalledTimes(2); expect(h.dispose).toHaveBeenCalledTimes(2);
      expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1); expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each(['cancelled', 'error', 'undefined'] as const)('keeps the earlier write %s outcome ahead of a queued path-mask edit', async outcome => {
    const h = nativeReadback(), mask = nativePathMask(); h.PM.proj.layers[0].masks = [mask];
    let version = 0; h.PM.animVersion = () => version;
    const running = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      mask.path.vertices[0]!.p.x.v = 9; version++; h.X.cancel = true;
      if (outcome === 'cancelled') h.resolve(); else h.reject(outcome === 'error' ? new Error('Earlier path write failed') : undefined);
      expect(await running).toEqual(outcome === 'cancelled' ? { cancelled: true } : { error: outcome === 'error' ? 'Earlier path write failed' : 'undefined' });
      expect(h.write).toHaveBeenCalledOnce(); expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(h.finish).not.toHaveBeenCalled();
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await running; }
  });

  it.each(['path', 'unknown', 'property-shape', 'expression', 'missing-properties', 'custom-effect'] as const)('keeps the unsupported native %s case serial', async type => {
    const h = nativeEffects();
    if (type === 'custom-effect') h.PM.proj.layers[0].fx = [{ type: 'custom-warp', on: true, p: {} }];
    else h.PM.proj.layers[0].masks = [{ id: 'mask', shape: type === 'unknown' ? 'triangle' : type === 'property-shape' ? { v: 'rect', kf: [] } : 'rect',
      mode: 'add', on: true, ...(type === 'path' ? { path: {} } : {}),
      ...(type === 'missing-properties' ? {} : { p: { x: type === 'expression' ? { expr: 'time' } : { v: 0, kf: [] } } }) }];
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled(); expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
  });

  function nativeMedia(kind: 'image' | 'video') {
    const h = nativeReadback();
    class Bitmap {
      width = 2; height = 2;
      close() { this.width = this.height = 0; }
    }
    vi.stubGlobal('ImageBitmap', Bitmap);
    const makeAsset = () => {
      const el: any = kind === 'image' ? new Bitmap() : document.createElement('video');
      if (kind === 'video') Object.defineProperties(el, {
        videoWidth: { value: 2, configurable: true }, videoHeight: { value: 2, configurable: true },
        readyState: { value: 4, configurable: true }, paused: { value: true, configurable: true },
        seeking: { value: false, configurable: true }, currentTime: { value: 0, writable: true, configurable: true },
        currentSrc: { value: 'blob:media', writable: true, configurable: true }, srcObject: { value: null, writable: true, configurable: true },
      });
      if (kind === 'video') el.src = 'blob:media';
      return { id: 'media', kind, el, w: 2, h: 2, dur: 1, storageKey: 'source' };
    };
    const assets = new Map([['media', makeAsset()]]);
    h.PM.assets = { get: (id: string) => assets.get(id), map: assets };
    h.PM.proj.layers = [{ id: 'clip', type: kind, from: 0, dur: 1, d: { asset: 'media' },
      ...(kind === 'image' ? { p: { rotation: { v: 0, kf: [{ t: 0, v: 0 }, { t: 1, v: 90 }] } } } : {}) }];
    const listeners = new Map<string, Set<() => void>>();
    h.PM.bus = {
      on(event: string, callback: () => void) {
        if (!listeners.has(event)) listeners.set(event, new Set());
        listeners.get(event)!.add(callback); return () => listeners.get(event)!.delete(callback);
      },
      emit(event: string) { for (const callback of listeners.get(event) || []) callback(); },
    };
    const publish = async (_PM: any, time: number) => {
      const frames = new Map();
      if (kind === 'video') {
        const canvas = document.createElement('canvas'); canvas.width = canvas.height = 2;
        frames.set(`clip@${time}`, canvas); assets.get('media')!.el.currentTime = time;
      }
      h.PM.preparedVideoFrames = frames;
      h.PM.preparedVideoVersion = (h.PM.preparedVideoVersion || 0) + 1;
    };
    h.prepare.mockImplementation(publish);
    return { ...h, assets, makeAsset, publish, listeners };
  }

  it.each(['image', 'video'] as const)('keeps custom-provider %s preparation on the original serial path while sources change', async kind => {
    const h = nativeMedia(kind), registry = new Registry<any>();
    registry.register('custom-effects', { ...EFFECTS.find(effect => effect.id === 'blur')!.legacy![0]!, frag: 'custom shader' });
    h.PM.Kernel = { effects: registry };
    h.PM.proj.layers[0].fx = [{ id: 'blur', type: 'blur', on: true, p: { amount: { v: 2, kf: [] } } }];
    const assets = vi.spyOn(h.PM.assets, 'get');
    h.prepare.mockImplementation(async (_PM: any, time: number) => {
      h.assets.set('media', h.makeAsset()); await h.publish(h.PM, time);
    });
    h.resolve(); expect(await h.run()).toEqual({ cancelled: false });
    expect(h.prepare.mock.calls.map(call => call[1])).toEqual([0, 1 / 3, 2 / 3]);
    expect(assets).not.toHaveBeenCalled(); expect(h.create).not.toHaveBeenCalled();
    expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3); expect(h.finish).toHaveBeenCalledOnce();
    expect(h.listeners.get('assets')!.size).toBe(0);
  });

  it.each(['image', 'video'] as const)('keeps an unstamped %s frame serial when its built-in provider restores during preparation', async kind => {
    const h = nativeMedia(kind), registry = new Registry<any>(), definition = EFFECTS.find(effect => effect.id === 'blur')!.legacy![0]!;
    registry.register('effects-basic', definition);
    const override = registry.register('custom-effects', { ...definition, frag: 'custom shader' });
    h.PM.Kernel = { effects: registry };
    h.PM.proj.layers[0].fx = [{ id: 'blur', type: 'blur', on: true, p: { amount: { v: 2, kf: [] } } }];
    h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    let release!: () => void;
    h.prepare.mockImplementationOnce(async (_PM: any, time: number) => {
      await new Promise<void>(yes => { release = yes; }); await h.publish(h.PM, time);
    });
    const running = h.run();
    try {
      await vi.waitFor(() => expect(release).toBeTypeOf('function'));
      override.dispose(); release();
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(0); expect(h.received[0]).toEqual(Array(16).fill(9));
      h.resolve(); expect(await running).toEqual({ cancelled: false });
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([1 / 3, 2 / 3]);
      expect(h.create).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { override.dispose(); release?.(); h.resolve(); await running; }
  });

  it('overlaps immutable-image GPU capture while retaining one CPU frame until its write ACK', async () => {
    const h = nativeMedia('image'), result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.prepare).toHaveBeenCalledTimes(2); expect(h.targets).toHaveLength(1);
      expect(h.write).toHaveBeenCalledOnce(); expect([...h.targets[0]!]).toEqual(Array(16).fill(1));
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.create).toHaveBeenCalledOnce(); expect(new Set(h.targets).size).toBe(1);
      expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1); expect(h.dispose).toHaveBeenCalledOnce();
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it.each(['saved', 'cancelled', 'error', 'undefined'] as const)('joins a video write before starting the next seek when its ACK is %s', async outcome => {
    const h = nativeMedia('video'), result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.prepare).toHaveBeenCalledOnce(); expect(h.begin).toHaveBeenCalledOnce();
      if (outcome === 'error' || outcome === 'undefined') h.reject(outcome === 'error' ? new Error('Earlier video write failed') : undefined);
      else { if (outcome === 'cancelled') h.X.cancel = true; h.resolve(); }
      expect(await result).toEqual(outcome === 'saved' ? { cancelled: false } : outcome === 'cancelled' ? { cancelled: true }
        : { error: outcome === 'error' ? 'Earlier video write failed' : 'undefined' });
      expect(h.prepare).toHaveBeenCalledTimes(outcome === 'saved' ? 3 : 1);
      expect(h.begin).toHaveBeenCalledTimes(outcome === 'saved' ? 3 : 1);
      expect(h.finish).toHaveBeenCalledTimes(outcome === 'saved' ? 1 : 0);
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it('drains a video seek already in progress before restoring preview after cancellation', async () => {
    const h = nativeMedia('video'); h.resolve(); h.PM.setTime = vi.fn();
    let release!: () => void, entered = false;
    h.prepare.mockImplementation(async (...args: any[]) => {
      if (h.prepare.mock.calls.length === 2) { entered = true; await new Promise<void>(yes => { release = yes; }); }
      await h.publish(args[0], args[1]);
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(entered).toBe(true));
      h.X.cancel = true; expect(h.PM.setTime).not.toHaveBeenCalled(); expect(h.dispose).not.toHaveBeenCalled();
      expect(h.begin).toHaveBeenCalledOnce(); expect(h.write).toHaveBeenCalledOnce();
      release(); expect(await result).toEqual({ cancelled: true });
      expect(h.begin).toHaveBeenCalledOnce(); expect(h.finish).not.toHaveBeenCalled();
      expect(h.PM.setTime).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
    } finally { release?.(); await result; }
  });

  it.each(['replace', 'close', 'assets-event'] as const)('recaptures the same image timestamp after %s while the preceding write is pending', async change => {
    const h = nativeMedia('image'), original = h.begin.getMockImplementation()!;
    h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    h.begin.mockImplementation((...args: any[]) => {
      const ticket = original(args[0], args[1]);
      if (h.begin.mock.calls.length === 2) {
        if (change === 'replace') h.assets.set('media', h.makeAsset());
        else if (change === 'close') h.assets.get('media')!.el.close();
        else h.PM.bus.emit('assets');
      }
      return ticket;
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.prepare).toHaveBeenCalledTimes(2); expect(h.targets).toHaveLength(1);
      expect(h.write).toHaveBeenCalledOnce(); h.resolve();
      expect(await result).toEqual({ cancelled: false });
      expect(h.prepare.mock.calls.slice(0, 3).map(call => call[1])).toEqual([0, 1 / 3, 1 / 3]);
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(1 / 3);
      expect(h.received[1]).toEqual(Array(16).fill(9));
      expect(h.maxTickets()).toBe(1); expect(h.activeTickets()).toBe(0);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it.each(['clear', 'replace-map', 'replace-frame', 'src', 'runtime'] as const)('discards a video capture after a synchronous %s mutation during GPU submission', async change => {
    const h = nativeMedia('video'), original = h.begin.getMockImplementation()!;
    h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    h.begin.mockImplementationOnce((...args: any[]) => {
      const ticket = original(args[0], args[1]);
      if (change === 'clear') h.PM.preparedVideoFrames.clear();
      else if (change === 'replace-map') h.PM.preparedVideoFrames = new Map(h.PM.preparedVideoFrames);
      else if (change === 'replace-frame') { const canvas = document.createElement('canvas'); canvas.width = canvas.height = 2; h.PM.preparedVideoFrames.set('clip@0', canvas); }
      else if (change === 'src') h.assets.get('media')!.el.src = 'blob:replacement';
      else h.assets.set('media', h.makeAsset());
      return ticket;
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.prepare.mock.calls.slice(0, 2).map(call => call[1])).toEqual([0, 0]);
      expect(h.received[0]).toEqual(Array(16).fill(9));
      expect(h.tickets[0]!.readInto).not.toHaveBeenCalled();
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.activeTickets()).toBe(0); expect(h.maxTickets()).toBe(1);
    } finally { h.resolve(); await result; }
  });

  it.each(['image', 'video'] as const)('reprepares the same %s timestamp after its source changes during async preparation', async kind => {
    const h = nativeMedia(kind); let preparations = 0;
    h.prepare.mockImplementation(async (...args: any[]) => {
      await h.publish(args[0], args[1]);
      if (++preparations === 1) { await Promise.resolve(); h.assets.set('media', h.makeAsset()); }
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.prepare.mock.calls.slice(0, 2).map(call => call[1])).toEqual([0, 0]);
      expect(h.tickets[0]!.readInto).toHaveBeenCalledOnce();
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.activeTickets()).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it.each(['image', 'video'] as const)('rejects a stale %s source after asynchronous GPU completion and recaptures its timestamp', async kind => {
    const h = nativeMedia(kind); let reads = 0;
    h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    h.controls.read = async target => {
      target.fill(7);
      if (++reads === 1) { await Promise.resolve(); h.assets.set('media', h.makeAsset()); }
      return true;
    };
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.prepare.mock.calls.slice(0, 2).map(call => call[1])).toEqual([0, 0]);
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(0);
      expect(h.received[0]).toEqual(Array(16).fill(9));
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.activeTickets()).toBe(0); expect(h.maxTickets()).toBe(1);
    } finally { h.resolve(); await result; }
  });

  it('keeps a ready video capture valid as buffering advances from readyState2 to4', async () => {
    const h = nativeMedia('video'); h.resolve();
    h.prepare.mockImplementation(async (...args: any[]) => {
      await h.publish(args[0], args[1]);
      Object.defineProperty(h.assets.get('media')!.el, 'readyState', { value: 2, configurable: true });
    });
    h.controls.read = async target => {
      Object.defineProperty(h.assets.get('media')!.el, 'readyState', { value: 4, configurable: true });
      target.fill(7); return true;
    };
    expect(await h.run()).toEqual({ cancelled: false });
    expect(h.prepare).toHaveBeenCalledTimes(3); expect(h.begin).toHaveBeenCalledTimes(3);
    expect(h.create).toHaveBeenCalledOnce(); expect(h.dispose).toHaveBeenCalledOnce();
    expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
  });

  it.each([new Error('Earlier media write failed'), undefined])('preserves a prior write failure over a late immutable-image replacement: %s', async failure => {
    const h = nativeMedia('image'), result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      h.assets.set('media', h.makeAsset()); h.reject(failure);
      expect(await result).toEqual({ error: failure instanceof Error ? failure.message : 'undefined' });
      expect(h.prepare).toHaveBeenCalledTimes(2); expect(h.write).toHaveBeenCalledOnce();
      expect(h.finish).not.toHaveBeenCalled(); expect(h.activeTickets()).toBe(0);
      expect(h.dispose).toHaveBeenCalledOnce(); expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it.each(['project', 'layers'] as const)('refreshes media ownership after a held video ACK changes the %s', async change => {
    const h = nativeMedia('video'), replacement = { ...h.makeAsset(), id: 'replacement' };
    h.assets.set('replacement', replacement);
    h.PM.GL.renderToPixels.mockReturnValue(new Uint8Array(16).fill(9));
    h.prepare.mockImplementation(async (_PM: any, time: number) => {
      const frames = new Map();
      for (const layer of h.PM.proj.layers) if (layer.type === 'video') {
        const canvas = document.createElement('canvas'); canvas.width = canvas.height = 2;
        frames.set(`${layer.id}@${time}`, canvas);
        h.assets.get(layer.d.asset)!.el.currentTime = time;
      }
      h.PM.preparedVideoFrames = frames; h.PM.preparedVideoVersion = (h.PM.preparedVideoVersion || 0) + 1;
    });
    let reads = 0;
    h.controls.read = async target => {
      target.fill(++reads);
      if (reads === 2) { await Promise.resolve(); replacement.el.src = 'blob:late-replacement'; }
      return true;
    };
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.prepare).toHaveBeenCalledOnce();
      const layers = [{ ...h.PM.proj.layers[0], d: { asset: 'replacement' } }];
      if (change === 'project') h.PM.proj = { ...h.PM.proj, layers };
      else h.PM.proj.layers = layers;
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.prepare.mock.calls.map(call => call[1])).toEqual([0, 1 / 3, 1 / 3, 2 / 3]);
      expect(h.PM.GL.renderToPixels.mock.calls[0]![0]).toBe(1 / 3);
      expect(h.received[1]).toEqual(Array(16).fill(9));
      expect(h.activeTickets()).toBe(0); expect(h.maxTickets()).toBe(1); expect(h.maxWrites()).toBe(1);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { h.resolve(); await result; }
  });

  it.each((['image', 'video'] as const).flatMap(kind => ['true', 'omitted'].map(option => ({ kind, option }))))(
    'allows $kind PBO capture with $option motion-blur option when visual flags are literal false', async ({ kind, option }) => {
      const h = nativeMedia(kind); h.resolve(); h.PM.proj.layers[0].mblur = false;
      h.PM.proj.layers.push({ id: 'audio', type: 'audio', mblur: true });
      const opts: any = { ...options, format: 'mp4', fps: 3 };
      if (option === 'true') opts.mblur = true; else delete opts.mblur;
      expect(await h.X.run(opts)).toEqual({ cancelled: false });
      expect(h.begin).toHaveBeenCalledTimes(3);
      expect(h.begin.mock.calls[0]![1]).toMatchObject({ mblur: true, mbSamples: 16, shutter: .5 });
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled(); expect(h.dispose).toHaveBeenCalledOnce();
    });

  it.each((['image', 'video'] as const).flatMap(kind => ['layer', 'group'].flatMap(owner => ['true', 'property'].map(flag => ({ kind, owner, flag })))))(
    'keeps $kind with a $flag $owner blur flag on its serial path', async ({ kind, owner, flag }) => {
      const h = nativeMedia(kind); h.resolve();
      const layer = owner === 'group' ? { id: 'group', type: 'group' } : h.PM.proj.layers[0];
      if (owner === 'group') h.PM.proj.layers.push(layer);
      layer.mblur = flag === 'true' ? true : { v: false, kf: [] };
      expect(await h.run({ mblur: true })).toEqual({ cancelled: false });
      expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
    });

  it.each(['motion-blur', 'mutable-image', 'missing-frame', 'alpha'] as const)('keeps media %s outside opaque asynchronous capture', async safeguard => {
    const h = nativeMedia(safeguard === 'missing-frame' ? 'video' : 'image');
    if (safeguard === 'motion-blur') h.PM.proj.layers[0].mblur = true;
    if (safeguard === 'mutable-image') h.assets.get('media')!.el = document.createElement('canvas');
    if (safeguard === 'missing-frame') h.prepare.mockImplementation(async () => { h.PM.preparedVideoFrames = new Map(); });
    if (safeguard === 'alpha') vi.mocked(HTMLCanvasElement.prototype.getContext).mockImplementation(() => ({ drawImage() {}, putImageData() {}, createImageData: () => ({ data: new Uint8ClampedArray(16) }), getImageData: () => ({ data: new Uint8ClampedArray(16) }) }) as any);
    const result = h.run({ mblur: safeguard === 'motion-blur', alpha: safeguard === 'alpha' });
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.begin).not.toHaveBeenCalled(); h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.create).not.toHaveBeenCalled(); expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
    } finally { h.resolve(); await result; }
  });

  it('overlaps native GPU submission with one write while reusing exactly one acknowledged CPU frame', async () => {
    const h = nativeReadback(); h.PM.quality = .5;
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(h.write).toHaveBeenCalledOnce();
      expect(h.targets).toHaveLength(1);
      expect([...h.targets[0]!]).toEqual(Array(16).fill(1));
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
      expect(h.finish).not.toHaveBeenCalled();
      h.resolve();
      expect(await result).toEqual({ cancelled: false });
      expect(h.received).toEqual([Array(16).fill(1), Array(16).fill(2), Array(16).fill(3)]);
      expect(new Set(h.targets).size).toBe(1);
      expect(h.maxWrites()).toBe(1); expect(h.maxTickets()).toBe(1);
      expect(h.activeTickets()).toBe(0);
      expect(h.tickets.every(ticket => ticket.dispose.mock.calls.length === 1)).toBe(true);
      expect(h.dispose).toHaveBeenCalledOnce();
      expect(h.begin.mock.calls.map(call => call[0])).toEqual([0, 1 / 3, 2 / 3]);
      expect(h.begin.mock.calls[0]![1]).toEqual({ mblur: false, mbSamples: 1, shutter: .5, opaque: true, topDownOpaque: true });
      expect(h.PM.quality).toBe(.5);
    } finally { h.resolve(); await result; }
  });

  it.each(['saved', 'cancelled', 'failed'] as const)('keeps the HD CPU frame immutable until its final chunk ACK: %s', async outcome => {
    const h = nativeReadback(), limit = 4 * 1024 * 1024;
    h.PM.proj.w = 1920; h.PM.proj.h = 1080; h.PM.proj.fps = 2;
    const frameBytes = 1920 * 1080 * 4;
    const acknowledgements: Array<{ resolve(): void; reject(error: Error): void }> = [];
    const sent: Uint8Array[] = [];
    let active = 0, maximum = 0;
    h.write.mockImplementation(async (_token: string, bytes: Uint8Array) => {
      active++; maximum = Math.max(maximum, active);
      sent.push(bytes.slice());
      try {
        if (sent.length <= 2) await new Promise<void>((resolve, reject) => { acknowledgements.push({ resolve, reject }); });
      } finally { active--; }
    });
    let settled = false;
    const result = h.run({ fps: 2 }).finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      // Both chunks travel together as owned copies, never views of the target.
      expect(h.write).toHaveBeenCalledTimes(2);
      expect(h.targets).toHaveLength(1);
      expect(sent.map(chunk => chunk.length)).toEqual([limit, frameBytes - limit]);
      expect(h.write.mock.calls.every(call => (call[1] as Uint8Array).buffer !== h.targets[0]!.buffer)).toBe(true);
      acknowledgements[0]!.resolve();
      await new Promise(resolve => setTimeout(resolve, 0));
      // The next picture is queued on the GPU, but the final chunk of the
      // preceding picture is still unacknowledged, so the shared CPU target waits.
      expect(h.write).toHaveBeenCalledTimes(2);
      expect(h.targets).toHaveLength(1);
      expect(h.targets[0]!.every(byte => byte === 1)).toBe(true);
      expect(h.tickets[1]!.readInto).not.toHaveBeenCalled();
      expect(active).toBe(1); expect(settled).toBe(false);
      expect(h.finish).not.toHaveBeenCalled(); expect(h.cancel).not.toHaveBeenCalled();
      if (outcome === 'cancelled') h.X.cancel = true;
      if (outcome === 'failed') acknowledgements[1]!.reject(new Error('Final chunk failed'));
      else acknowledgements[1]!.resolve();
      expect(await result).toEqual(outcome === 'failed' ? { error: 'Final chunk failed' } : { cancelled: outcome === 'cancelled' });
      expect(maximum).toBe(2); expect(active).toBe(0);
      expect(h.maxTickets()).toBe(1); expect(h.activeTickets()).toBe(0);
      expect(h.dispose).toHaveBeenCalledOnce();
      const lengths = outcome === 'saved' ? [limit, frameBytes - limit, limit, frameBytes - limit] : [limit, frameBytes - limit];
      expect(sent.map(chunk => chunk.length)).toEqual(lengths);
      expect(sent.every((chunk, index) => chunk.every(byte => byte === (index < 2 ? 1 : 2)))).toBe(true);
      expect(new Set(h.targets).size).toBe(1);
      expect(h.targets).toHaveLength(outcome === 'saved' ? 2 : 1);
      if (outcome === 'saved') { expect(h.finish).toHaveBeenCalledOnce(); expect(h.cancel).not.toHaveBeenCalled(); }
      else { expect(h.finish).not.toHaveBeenCalled(); expect(h.cancel).toHaveBeenCalledOnce(); }
    } finally {
      for (const acknowledgement of acknowledgements) acknowledgement.resolve();
      h.resolve(); await result;
    }
  });

  it.each([new Error('Encoder write failed'), undefined])('retains a prior native write rejection over speculative capture failure: %s', async failure => {
    const h = nativeReadback();
    h.begin.mockImplementationOnce((...args: any[]) => {
      // Keep the first submission, then fail the speculative second one.
      const entry = { paint: 1, readInto: vi.fn(async (target: Uint8Array) => { target.fill(1); return true; }), dispose: vi.fn() };
      h.tickets.push(entry); h.controls.beginFailure = true; return entry;
    });
    let settled = false;
    const result = h.run().finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(settled).toBe(false); expect(h.cancel).not.toHaveBeenCalled();
      h.reject(failure);
      expect(await result).toEqual({ error: failure instanceof Error ? failure.message : 'undefined' });
      expect(h.write).toHaveBeenCalledOnce(); expect(h.finish).not.toHaveBeenCalled();
      expect(h.cancel).toHaveBeenCalledWith('render'); expect(h.dispose).toHaveBeenCalledOnce();
    } finally { h.resolve(); await result; }
  });

  it('drains a native write before cancellation and disposes the unused GPU ticket', async () => {
    const h = nativeReadback(); let settled = false;
    const result = h.run().finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      h.X.cancel = true;
      await Promise.resolve(); await Promise.resolve();
      expect(settled).toBe(false); expect(h.cancel).not.toHaveBeenCalled();
      h.resolve();
      expect(await result).toEqual({ cancelled: true });
      expect(h.targets).toHaveLength(1); expect(h.write).toHaveBeenCalledOnce();
      expect(h.activeTickets()).toBe(0); expect(h.dispose).toHaveBeenCalledOnce(); expect(h.finish).not.toHaveBeenCalled();
    } finally { h.resolve(); await result; }
  });

  it('retains a native write failure even when cancellation happens during the write', async () => {
    const h = nativeReadback(), result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      h.X.cancel = true; h.reject(new Error('Write rejected during cancellation'));
      expect(await result).toEqual({ error: 'Write rejected during cancellation' });
    } finally { h.resolve(); await result; }
  });

  it.each(['edit', 'project', 'fonts', 'draw', 'theme', 'extensions', 'eligibility'] as const)
    ('recaptures a speculative native frame serially after %s invalidation', async change => {
      const h = nativeReadback(); h.PM.proj.fps = 2;
      let version = 0; h.PM.animVersion = () => version;
      const handlers = new Map<string, Set<() => void>>();
      const on = (event: string, handler: () => void) => {
        let listeners = handlers.get(event); if (!listeners) { listeners = new Set(); handlers.set(event, listeners); }
        listeners.add(handler); return () => listeners!.delete(handler);
      };
      h.PM.bus = { on }; h.PM.Kernel = { events: { on } };
      h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(16).fill(9));
      const result = h.run({ fps: 2 });
      try {
        await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
        if (change === 'edit') version++;
        else if (change === 'project') h.PM.proj = { ...h.PM.proj };
        else if (change === 'eligibility') h.PM.proj.layers = [{ type: 'video' }];
        else for (const handler of handlers.get(change === 'theme' ? 'theme:changed' : change === 'extensions' ? 'extensions:changed' : change) || []) handler();
        h.resolve();
        expect(await result).toEqual({ cancelled: false });
        expect(h.received).toEqual([Array(16).fill(1), Array(16).fill(9)]);
        expect(h.targets).toHaveLength(1);
        expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
        expect(h.activeTickets()).toBe(0);
      } finally { h.resolve(); await result; }
    });

  it('discards a native frame if its source changes while its fence is pending', async () => {
    const h = nativeReadback(); h.PM.proj.fps = 2;
    let version = 0; h.PM.animVersion = () => version;
    h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(16).fill(9));
    let complete!: () => void;
    h.controls.read = async target => {
      await new Promise<void>(resolve => { complete = resolve; });
      target.fill(1); return true;
    };
    const result = h.run({ fps: 2 });
    try {
      await vi.waitFor(() => expect(h.targets).toHaveLength(1));
      version++; h.controls.read = null; complete();
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
      expect(h.received[0]).toEqual(Array(16).fill(9));
      h.resolve();
      expect(await result).toEqual({ cancelled: false });
    } finally { complete?.(); h.resolve(); await result; }
  });

  it.each(['loss', 'copy', 'cancel'] as const)('cleans native readback resources after %s during a fence wait', async outcome => {
    const h = nativeReadback(); let complete!: () => void;
    h.controls.read = async (_target, cancelled) => {
      await new Promise<void>(resolve => { complete = resolve; });
      if (outcome === 'copy') throw new Error('GPU copy failed');
      if (outcome === 'loss') { h.PM.GL.contextLost = true; throw new Error('GPU lost'); }
      h.X.cancel = true; return !cancelled();
    };
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.targets).toHaveLength(1));
      complete();
      expect(await result).toEqual(outcome === 'cancel' ? { cancelled: true } : { error: outcome === 'loss' ? 'GPU lost' : 'GPU copy failed' });
      expect(h.write).not.toHaveBeenCalled(); expect(h.finish).not.toHaveBeenCalled();
      expect(h.activeTickets()).toBe(0); expect(h.dispose).toHaveBeenCalledOnce(); expect(h.cancel).toHaveBeenCalledOnce();
    } finally { complete?.(); h.resolve(); await result; }
  });

  it.each(['static', 'video', 'precomp', 'extension', 'effect', 'mask', '3D', 'expression', 'background-expression', 'fonts', 'unsupported'] as const)
    ('keeps the native %s fallback on its existing serial capture path', async safeguard => {
      const h = nativeReadback();
      if (safeguard === 'static') h.PM.proj.layers = [];
      else if (safeguard === 'fonts') h.fonts.mockReturnValue(false);
      else if (safeguard === 'unsupported') h.create.mockReturnValue(null as any);
      else if (safeguard === 'effect') h.PM.proj.layers[0].fx = [{}];
      else if (safeguard === 'mask') h.PM.proj.layers[0].masks = [{}];
      else if (safeguard === '3D') h.PM.proj.layers[0].threeD = true;
      else if (safeguard === 'expression') h.PM.proj.layers[0].p.rotation.expr = 'time';
      else if (safeguard === 'background-expression') h.PM.proj.backgroundFill = { color: { expr: 'time' } };
      else h.PM.proj.layers = [{ type: safeguard }];
      const result = h.run();
      try {
        await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce());
        expect(h.begin).not.toHaveBeenCalled(); expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
        h.resolve(); expect(await result).toEqual({ cancelled: false });
        expect(h.begin).not.toHaveBeenCalled();
        if (safeguard === 'unsupported') expect(h.create).toHaveBeenCalledOnce();
        else expect(h.create).not.toHaveBeenCalled();
      } finally { h.resolve(); await result; }
    });

  it('keeps native audio layers eligible and matches opaque motion-blur capture options', async () => {
    const h = nativeReadback(); h.PM.proj.layers.push({ type: 'audio' }); h.PM.proj.shutter = -.25;
    const result = h.run({ mblur: true });
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2)); h.resolve();
      expect(await result).toEqual({ cancelled: false });
      expect(h.begin.mock.calls[0]![1]).toEqual({ mblur: true, mbSamples: 16, shutter: -.25, opaque: true, topDownOpaque: true });
    } finally { h.resolve(); await result; }
  });

  it.each(['saved', 'failed', 'cancelled'] as const)('drains native writes after speculative preparation fails while the prior frame is %s', async outcome => {
    const h = nativeReadback();
    let releasePreparation!: () => void;
    h.prepare.mockResolvedValueOnce().mockImplementationOnce(async () => {
      await new Promise<void>(resolve => { releasePreparation = resolve; });
      throw new Error('Speculative preparation failed');
    });
    let settled = false;
    const result = h.run().finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.prepare).toHaveBeenCalledTimes(2));
      if (outcome === 'failed') h.reject(new Error('Prior encoder failed'));
      else h.resolve();
      if (outcome === 'cancelled') h.X.cancel = true;
      await new Promise(resolve => setTimeout(resolve, 10));
      expect(settled).toBe(false); expect(h.cancel).not.toHaveBeenCalled();
      releasePreparation();
      expect(await result).toEqual(outcome === 'cancelled' ? { cancelled: true }
        : { error: outcome === 'failed' ? 'Prior encoder failed' : 'Speculative preparation failed' });
      expect(h.write).toHaveBeenCalledOnce(); expect(h.finish).not.toHaveBeenCalled(); expect(h.activeTickets()).toBe(0);
    } finally { releasePreparation?.(); h.resolve(); await result; }
  });

  it('stops a disposed readback without finishing a partial native video', async () => {
    const h = nativeReadback(); h.controls.read = async () => false;
    expect(await h.run()).toEqual({ cancelled: true });
    expect(h.write).not.toHaveBeenCalled(); expect(h.finish).not.toHaveBeenCalled();
    expect(h.cancel).toHaveBeenCalledOnce(); expect(h.activeTickets()).toBe(0);
  });

  it('restores native quality and releases readback after a synchronous GPU submission failure', async () => {
    const h = nativeReadback(); h.PM.quality = .25; h.controls.beginFailure = true;
    expect(await h.run({ mblur: true })).toEqual({ error: 'GPU submission failed' });
    expect(h.PM.quality).toBe(.25);
    expect(h.write).not.toHaveBeenCalled(); expect(h.finish).not.toHaveBeenCalled();
    expect(h.dispose).toHaveBeenCalledOnce(); expect(h.cancel).toHaveBeenCalledOnce();
  });

  it('keeps a loaded-font readiness invalidation caused by the GPU submission itself', async () => {
    const h = nativeReadback(); h.PM.proj.dur = 1 / 3;
    const begin = h.begin.getMockImplementation()!;
    h.begin.mockImplementation((time, opts) => {
      const ticket = begin(time, opts); h.fonts.mockReturnValue(false); return ticket;
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce()); h.resolve();
      expect(await result).toEqual({ cancelled: false });
      expect(h.received).toEqual([Array(16).fill(1)]);
      expect(h.PM.GL.renderToPixels).not.toHaveBeenCalled();
    } finally { h.resolve(); await result; }
  });

  it('keeps native alpha captures on their existing Canvas2D delivery path', async () => {
    const h = nativeReadback();
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({ drawImage() {}, putImageData() {},
      createImageData: () => ({ data: new Uint8ClampedArray(16) }), getImageData: () => ({ data: new Uint8ClampedArray(16) }) }) as any);
    const result = h.run({ format: 'prores', alpha: true });
    try {
      await vi.waitFor(() => expect(h.write).toHaveBeenCalledOnce()); h.resolve();
      expect(await result).toEqual({ cancelled: false });
      expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
      expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(3);
    } finally { h.resolve(); await result; }
  });

  it('keeps native frames above the 32MiB overlap limit serial and bounded', async () => {
    const h = nativeReadback(); h.PM.proj.w = 4096; h.PM.proj.h = 2050;
    h.PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(4096 * 2050 * 4));
    h.write.mockImplementation(async () => { h.X.cancel = true; });
    expect(await h.run()).toEqual({ cancelled: true });
    expect(h.create).not.toHaveBeenCalled(); expect(h.begin).not.toHaveBeenCalled();
    expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
    expect(h.write.mock.calls.every(([, bytes]) => bytes.length <= 4 * 1024 * 1024)).toBe(true);
  });

  it('scans a large keyframe set once while an unchanged native export reuses its eligibility', async () => {
    const h = nativeReadback(); let inspected = 0;
    h.PM.proj.layers[0].p.rotation.kf = Array.from({ length: 5000 }, (_, index) => {
      const frame = { t: index / 5000, v: index };
      Object.defineProperty(frame, 'expr', { get() { inspected++; return null; } });
      return frame;
    });
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      expect(inspected).toBe(5000);
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.begin).toHaveBeenCalledTimes(3);
      expect(inspected).toBe(5000);
    } finally { h.resolve(); await result; }
  });

  it.each(['touch', 'draw'] as const)('refreshes cached native eligibility after an expression edit announced by %s', async notification => {
    const h = nativeReadback(); let version = 0;
    h.PM.animVersion = () => version;
    const handlers = new Map<string, Set<() => void>>();
    h.PM.bus = { on(event: string, handler: () => void) {
      let listeners = handlers.get(event); if (!listeners) { listeners = new Set(); handlers.set(event, listeners); }
      listeners.add(handler); return () => listeners!.delete(handler);
    } };
    const result = h.run();
    try {
      await vi.waitFor(() => expect(h.begin).toHaveBeenCalledTimes(2));
      h.PM.proj.layers[0].p.rotation.expr = 'time';
      if (notification === 'touch') version++;
      else for (const handler of handlers.get('draw') || []) handler();
      h.resolve(); expect(await result).toEqual({ cancelled: false });
      expect(h.begin).toHaveBeenCalledTimes(2);
      expect(h.PM.GL.renderToPixels).toHaveBeenCalledTimes(2);
      expect(h.targets).toHaveLength(1);
    } finally { h.resolve(); await result; }
  });

  it('streams opaque image pixels directly without a canvas readback or an extra chunk copy', async () => {
    const { X, PM, frame } = setup();
    const captured = Uint8Array.from([120,135,150,255,180,195,210,255,0,15,30,255,60,75,90,255]);
    PM.GL.renderToPixels.mockImplementation(() => captured);
    frame.getContext = () => { throw new Error('The opaque encoder must not read a canvas'); };
    const slice = vi.spyOn(Uint8Array.prototype, 'slice');
    const write = vi.fn(async () => {});
    (window as any).powermove = { render: {
      start: async () => 'render', write, finish: async () => ({ path: '/tmp/render.mp4' }), cancel: vi.fn(),
    } };
    expect(await X.run({ ...options, format: 'mp4' })).toEqual({ cancelled: false });
    expect(write).toHaveBeenCalledTimes(2);
    const bytes = (write.mock.calls[0] as any)[1] as Uint8Array;
    expect([...bytes]).toEqual([120,135,150,255,180,195,210,255,0,15,30,255,60,75,90,255]);
    expect(bytes.buffer).toBe(captured.buffer);
    expect(bytes.byteLength).toBe(bytes.buffer.byteLength);
    expect(slice.mock.contexts.some(view => view instanceof Uint8Array && view.buffer === bytes.buffer)).toBe(false);
  });

  it('keeps large-frame IPC chunks bounded and owned, and cancels after the in-flight frame', async () => {
    const { X, PM } = setup();
    PM.proj.w = 1024; PM.proj.h = 1026;
    const frameBytes = 1024 * 1026 * 4;
    PM.GL.renderToPixels.mockImplementation(() => new Uint8Array(frameBytes));
    const releases: Array<() => void> = [];
    const write = vi.fn((_token: string, _bytes: Uint8Array) => new Promise<void>(resolve => releases.push(resolve)));
    const finish = vi.fn(), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, finish, cancel } };
    const pending = X.run({ ...options, format: 'mp4' });
    // One frame's chunks are sent together; the next frame waits for all of them.
    await vi.waitFor(() => expect(write).toHaveBeenCalledTimes(2));
    expect(PM.GL.renderToPixels).toHaveBeenCalledOnce();
    const first = write.mock.calls[0]![1];
    expect(first.byteLength).toBe(4 * 1024 * 1024);
    expect(first.buffer.byteLength).toBe(first.byteLength);
    X.cancel = true;
    releases.shift()!();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(write).toHaveBeenCalledTimes(2);
    const second = write.mock.calls[1]![1];
    expect(second.byteLength).toBe(frameBytes - first.byteLength);
    expect(second.buffer.byteLength).toBe(second.byteLength);
    expect(finish).not.toHaveBeenCalled();
    expect(cancel).not.toHaveBeenCalled();
    releases.shift()!();
    expect(await pending).toEqual({ cancelled: true });
    expect(PM.GL.renderToPixels).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledWith('render');
    expect(finish).not.toHaveBeenCalled();
    expect(X.busy).toBe(false);
  });

  it('reuses static pixels in bounded batches and cancels after the current batch', async () => {
    const { X, PM } = setup();
    PM.proj.fps = 30; PM.proj.dur = 2;
    let release!: () => void;
    const repeat = vi.fn((_token: string, _count: number) => new Promise<void>(resolve => { release = resolve; }));
    const write = vi.fn(async () => {}), finish = vi.fn(), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, repeat, finish, cancel } };
    const pending = X.run({ ...options, format: 'mp4', fps: 30 });
    await vi.waitFor(() => expect(repeat).toHaveBeenCalledOnce());
    expect(repeat).toHaveBeenCalledWith('render', 8);
    expect(PM.GL.renderToPixels).toHaveBeenCalledOnce();
    expect(write).toHaveBeenCalledOnce();
    expect(write.mock.calls[0]).toEqual(['render', expect.any(Uint8Array), false, true]);
    X.cancel = true; release();
    expect(await pending).toEqual({ cancelled: true });
    expect(cancel).toHaveBeenCalledOnce();
    expect(finish).not.toHaveBeenCalled();
  });

  it.each(['replace', 'close', 'assets-event'] as const)('repeats static bitmap frames one ACK at a time and observes %s on the following frame', async change => {
    const h = staticImage(), old = h.assets.get('photo')!;
    const paints: number[] = [];
    let retained = 0, release!: () => void;
    const write = vi.fn(async (_token: string, bytes: Uint8Array, _audio: boolean, keep: boolean) => {
      expect(keep).toBe(change !== 'close' || paints.length < 2);
      retained = bytes[0]!; paints.push(retained);
    });
    const repeat = vi.fn(async (_token: string, count: number) => {
      expect(count).toBe(1); paints.push(retained);
      if (repeat.mock.calls.length === 1) await new Promise<void>(yes => { release = yes; });
    });
    const finish = vi.fn(async () => ({ path: '/tmp/bitmap.mp4' })), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, repeat, finish, cancel } };
    const running = h.X.run({ ...options, format: 'mp4', fps: 4 });
    try {
      await vi.waitFor(() => expect(repeat).toHaveBeenCalledOnce());
      expect(paints).toEqual([1, 1]); expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
      if (change === 'replace') h.assets.set('photo', h.makeAsset(2));
      else if (change === 'close') old.el.close();
      else h.PM.bus.emit('assets');
      release(); expect(await running).toEqual({ cancelled: false });
      expect(paints).toEqual([1, 1, change === 'replace' ? 2 : change === 'close' ? 0 : 1, change === 'replace' ? 2 : change === 'close' ? 0 : 1]);
      expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual(change === 'close' ? [0, .5, .75] : [0, .5]);
      expect(repeat.mock.calls.every(call => call[1] === 1)).toBe(true);
      expect(finish).toHaveBeenCalledOnce(); expect(cancel).not.toHaveBeenCalled();
      expect(old.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
      expect(h.listeners.get('assets')!.size).toBe(0);
    } finally { release?.(); await running; }
  });

  it.each(['replace', 'close'] as const)('refreshes the native bitmap key after an empty join permits a %s microtask', async change => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let queued = false;
    const write = vi.fn(async (_token: string, _bytes: Uint8Array) => {}), repeat = vi.fn(async () => {});
    h.PM.assets.get.mockImplementation((id: string) => {
      const asset = h.assets.get(id);
      if (!queued && write.mock.calls.length === 1) {
        queued = true; queueMicrotask(() => {
          if (change === 'replace') h.assets.set('photo', h.makeAsset(2)); else old.el.close();
        });
      }
      return asset;
    });
    (window as any).powermove = { render: { start: async () => 'render', write, repeat,
      finish: async () => ({ path: '/tmp/bitmap.mp4' }), cancel: vi.fn(async () => {}) } };
    expect(await h.X.run({ ...options, format: 'mp4' })).toEqual({ cancelled: false });
    expect(queued).toBe(true); expect(repeat).not.toHaveBeenCalled();
    expect(h.PM.GL.renderToPixels.mock.calls.map((call: any[]) => call[0])).toEqual([0, .5]);
    expect(write.mock.calls.map(call => call[1][0])).toEqual([1, change === 'replace' ? 2 : 0]);
    expect(old.el.close).toHaveBeenCalledTimes(change === 'close' ? 1 : 0);
    expect(h.listeners.get('assets')!.size).toBe(0);
  });

  it.each(['cancelled', 'error', 'undefined'] as const)('preserves a native bitmap repeat ACK outcome %s before reading a replacement', async outcome => {
    const h = staticImage(), old = h.assets.get('photo')!;
    let release!: () => void, reject!: (error: unknown) => void;
    const write = vi.fn(async () => {}), repeat = vi.fn((_token: string, count: number) => {
      expect(count).toBe(1); return new Promise<void>((yes, no) => { release = yes; reject = no; });
    });
    const finish = vi.fn(async () => ({ path: '/tmp/bitmap.mp4' })), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, repeat, finish, cancel } };
    const running = h.X.run({ ...options, format: 'mp4', fps: 4 });
    try {
      await vi.waitFor(() => expect(repeat).toHaveBeenCalledOnce());
      h.assets.set('photo', h.makeAsset(2)); h.X.cancel = true;
      if (outcome === 'cancelled') release(); else reject(outcome === 'error' ? new Error('Earlier bitmap repeat failed') : undefined);
      expect(await running).toEqual(outcome === 'cancelled' ? { cancelled: true } : { error: outcome === 'error' ? 'Earlier bitmap repeat failed' : 'undefined' });
      expect(write).toHaveBeenCalledOnce(); expect(h.PM.GL.renderToPixels).toHaveBeenCalledOnce();
      expect(finish).not.toHaveBeenCalled(); expect(cancel).toHaveBeenCalledOnce();
      expect(old.el.close).not.toHaveBeenCalled(); expect(h.assets.get('photo')!.el.close).not.toHaveBeenCalled();
      expect(h.listeners.get('assets')!.size).toBe(0);
      repeat.mockImplementation(async () => {});
      expect(await h.X.run({ ...options, format: 'mp4', fps: 4 })).toEqual({ cancelled: false });
      expect(finish).toHaveBeenCalledOnce(); expect(h.assets.get('photo')!.el.close).not.toHaveBeenCalled();
    } finally { release?.(); await running; }
  });

  it('invalidates retained pixels after an edit during export', async () => {
    const { X, PM } = setup();
    let version = 0;
    PM.animVersion = () => version;
    const write = vi.fn(async () => { if (version === 0) version++; });
    (window as any).powermove = { render: {
      start: async () => 'render', write, finish: async () => ({ path: '/tmp/out.mp4' }), cancel: vi.fn(),
    } };
    expect(await X.run({ ...options, format: 'mp4' })).toEqual({ cancelled: false });
    expect(PM.GL.renderToPixels).toHaveBeenCalledTimes(2);
  });

  it('rejects context loss before repeating a retained frame', async () => {
    const { X, PM } = setup();
    const write = vi.fn(async () => { PM.GL.contextLost = true; });
    const repeat = vi.fn(), finish = vi.fn(), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, repeat, finish, cancel } };
    expect(await X.run({ ...options, format: 'mp4' })).toEqual({ error: 'The GPU could not capture the export frame' });
    expect(repeat).not.toHaveBeenCalled();
    expect(finish).not.toHaveBeenCalled();
    expect(cancel).toHaveBeenCalledOnce();
  });

  it.each([false, true].flatMap(alpha => ['capture', 'write', 'finish'].map(failure => ({ alpha, failure }))))('releases native alpha=$alpha resources after a $failure failure, then allows retry', async ({ alpha, failure }) => {
    const { X, PM } = setup();
    PM.time = .25; PM.quality = .5;
    PM.setTime = vi.fn();
    let preview: HTMLCanvasElement | undefined;
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockImplementation(function (this: HTMLCanvasElement) {
      if (this.width === 2 && this.height === 2) preview = this;
      return { drawImage() {}, putImageData() {},
        createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4), width: w, height: h }),
        getImageData: () => ({ data: new Uint8ClampedArray(16) }),
      } as any;
    });
    const write = vi.fn(async () => {}), finish = vi.fn(async () => ({ path: '/tmp/out.mp4' })), cancel = vi.fn(async () => {});
    (window as any).powermove = { render: { start: async () => 'render', write, finish, cancel } };
    if (failure === 'capture') PM.GL.renderToPixels.mockImplementationOnce(() => { throw new Error('Injected failure'); });
    if (failure === 'write') write.mockRejectedValueOnce(new Error('Injected failure'));
    if (failure === 'finish') finish.mockRejectedValueOnce(new Error('Injected failure'));
    const nativeOptions = { ...options, format: alpha ? 'prores' : 'mp4', alpha };
    expect(await X.run(nativeOptions)).toEqual({ error: 'Injected failure' });
    expect(cancel).toHaveBeenCalledOnce();
    expect(preview).toMatchObject({ width: 0, height: 0 });
    expect(PM.quality).toBe(.5);
    expect(PM.setTime).toHaveBeenCalledWith(.25, { force: true });
    expect(X.busy).toBe(false);
    expect(await X.run(nativeOptions)).toEqual({ cancelled: false });
    expect(cancel).toHaveBeenCalledOnce();
    expect(preview).toMatchObject({ width: 0, height: 0 });
  });
  it('cancels PNG export when destination selection is cancelled, without opening frame saves', async () => {
    const { X, toast, download } = setup();
    (window as any).showDirectoryPicker = vi.fn(async () => { throw new DOMException('Cancelled', 'AbortError'); });

    expect(await X.run(options)).toEqual({ cancelled: true });
    expect(download).not.toHaveBeenCalled();
    expect(toast).not.toHaveBeenCalledWith(expect.stringContaining('finished'), expect.anything());
  });

  it('writes every PNG to the selected folder and reports a write failure', async () => {
    const written: string[] = [];
    const { X, download } = setup();
    (window as any).showDirectoryPicker = async () => ({
      getFileHandle: async (name: string) => ({ createWritable: async () => ({
        async write() { written.push(name); }, async close() {},
      }) }),
    });
    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(written).toEqual(['Test_00000.png', 'Test_00001.png']);
    expect(download).not.toHaveBeenCalled();

    const abort = vi.fn(async () => {});
    (window as any).showDirectoryPicker = async () => ({
      getFileHandle: async () => ({ createWritable: async () => ({
        async write() { throw new Error('Disk full'); }, async close() {}, abort,
      }) }),
    });
    expect(await X.run(options)).toEqual({ error: 'Disk full' });
    expect(abort).toHaveBeenCalledOnce();
  });

  it('awaits sequential PNG saves and stops after a cancelled save', async () => {
    let release!: (result: any) => void;
    const download = vi.fn((_blob: Blob, _name: string) => new Promise<any>(resolve => { release = resolve; }));
    const { X } = setup(download);
    const result = X.run(options);
    await vi.waitFor(() => expect(download).toHaveBeenCalledTimes(1));
    expect(download.mock.calls[0]?.[1]).toBe('Test_00000.png');
    release({ ok: false, cancelled: true });
    expect(await result).toEqual({ cancelled: true });
    expect(download).toHaveBeenCalledTimes(1);
  });

  function heldPNG() {
    let resolve!: (value: any) => void, reject!: (error: unknown) => void;
    const first = new Promise<any>((done, fail) => { resolve = done; reject = fail; });
    const download = vi.fn(async () => ({ ok: true })).mockImplementationOnce(() => first);
    const state = setup(download);
    state.PM.proj.layers = [{ type: 'solid', from: 0, dur: 1, p: { rotation: { expr: 'time' } } }];
    state.PM.renderFrameTo = vi.fn(() => state.frame);
    return { ...state, resolve, reject };
  }

  it('captures the next artwork PNG while its prior save is held, with ordered filenames and one active save', async () => {
    const h = heldPNG();
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      expect(h.download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png']);
      const target = h.PM.renderFrameTo.mock.calls[0][4];
      expect(target.width).toBeGreaterThan(0);
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(h.download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png', 'Test_00001.png']);
      expect(target).toMatchObject({ width: 0, height: 0 });
    } finally { h.resolve({ ok: true }); await result; }
  });

  it.each(['failure', 'cancelled'] as const)('drops the speculative PNG after a prior save %s and releases its surface', async outcome => {
    const h = heldPNG();
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      if (outcome === 'failure') h.reject(new Error('Prior disk failure'));
      else h.resolve({ cancelled: true });
      expect(await result).toEqual(outcome === 'failure' ? { error: 'Prior disk failure' } : { cancelled: true });
      expect(h.download.mock.calls.length).toBe(1);
      expect(h.PM.renderFrameTo.mock.calls[0][4]).toMatchObject({ width: 0, height: 0 });
      expect(h.X.busy).toBe(false);
      expect(h.PM.preparedVideoFrames).toBeNull();
    } finally { h.resolve({ ok: true }); await result; }
  });

  it.each(['saved', 'failure', 'cancelled'] as const)('joins a prior PNG save %s before reporting a speculative capture failure', async outcome => {
    const h = heldPNG();
    const later = new Error('Later capture failure');
    h.PM.renderFrameTo.mockImplementationOnce(() => h.frame).mockImplementationOnce(() => { throw later; });
    let settled = false;
    const result = h.X.run(options).finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      expect(settled).toBe(false);
      expect(h.PM.renderFrameTo.mock.calls[0][4].width).toBeGreaterThan(0);
      if (outcome === 'failure') h.reject(new Error('Prior disk failure'));
      else h.resolve(outcome === 'cancelled' ? { cancelled: true } : { ok: true });
      expect(await result).toEqual(outcome === 'failure' ? { error: 'Prior disk failure' }
        : outcome === 'cancelled' ? { cancelled: true } : { error: 'Later capture failure' });
      expect(h.download.mock.calls.length).toBe(1);
      expect(h.PM.renderFrameTo.mock.calls[0][4]).toMatchObject({ width: 0, height: 0 });
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('preserves even an undefined prior PNG rejection instead of delivering the next frame', async () => {
    const h = heldPNG();
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      h.reject(undefined);
      expect(await result).toEqual({ error: 'undefined' });
      expect(h.download.mock.calls.length).toBe(1);
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('handles a save rejection while the next PNG encoding is still pending and drains before cleanup', async () => {
    const h = heldPNG();
    let entered!: () => void, encode!: () => void;
    const blocked = new Promise<void>(resolve => { entered = resolve; });
    const originalEncode = h.frame.toBlob;
    let calls = 0;
    h.frame.toBlob = callback => {
      if (++calls === 2) { encode = () => originalEncode(callback); entered(); }
      else originalEncode(callback);
    };
    let settled = false;
    const result = h.X.run(options).finally(() => { settled = true; });
    try {
      await blocked;
      h.reject(new Error('Save rejected during encode'));
      await new Promise(resolve => setTimeout(resolve, 15));
      expect(settled).toBe(false);
      expect(h.PM.renderFrameTo.mock.calls[0][4].width).toBeGreaterThan(0);
      encode();
      expect(await result).toEqual({ error: 'Save rejected during encode' });
      expect(h.download.mock.calls.length).toBe(1);
      expect(h.PM.renderFrameTo.mock.calls[0][4]).toMatchObject({ width: 0, height: 0 });
    } finally { h.resolve({ ok: true }); encode?.(); await result; }
  });

  it.each(['edit', 'project', 'fonts', 'draw', 'theme', 'extensions'] as const)('recaptures a speculative PNG invalidated by %s before delivering it', async change => {
    const h = heldPNG();
    let version = 0, color = 'old';
    h.PM.animVersion = () => version;
    const handlers = new Map<string, Set<() => void>>();
    const on = (event: string, handler: () => void) => {
      let entries = handlers.get(event); if (!entries) { entries = new Set(); handlers.set(event, entries); }
      entries.add(handler); return () => entries!.delete(handler);
    };
    h.PM.bus = { on }; h.PM.Kernel = { events: { on } };
    h.PM.renderFrameTo.mockImplementation((_time: number, _width: number, _height: number, _options: any, target: HTMLCanvasElement) => {
      const captured = color;
      target.toBlob = callback => callback(new Blob([captured]));
      return target;
    });
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      color = 'new';
      if (change === 'edit') version++;
      else if (change === 'project') h.PM.proj = { ...h.PM.proj };
      else for (const handler of handlers.get(change === 'theme' ? 'theme:changed' : change === 'extensions' ? 'extensions:changed' : change) || []) handler();
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(3);
      expect(h.download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png', 'Test_00001.png']);
      expect(await (h.download.mock.calls[0][0] as Blob).text()).toBe('old');
      expect(await (h.download.mock.calls[1][0] as Blob).text()).toBe('new');
      expect(new Set(h.PM.renderFrameTo.mock.calls.map((args: any[]) => args[4])).size).toBe(1);
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('drops a speculative PNG when the GPU is lost while its prior file is still saving', async () => {
    const h = heldPNG();
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      h.PM.GL.contextLost = true;
      h.resolve({ ok: true });
      expect(await result).toEqual({ error: 'The GPU could not capture the export frame' });
      expect(h.download.mock.calls.length).toBe(1);
      expect(h.PM.renderFrameTo.mock.calls[0][4]).toMatchObject({ width: 0, height: 0 });
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('refreshes a reused static PNG Blob after an edit while its preceding file is still saving', async () => {
    const h = heldPNG(); h.PM.proj.layers = [];
    let version = 0, color = 'old'; h.PM.animVersion = () => version;
    h.PM.renderFrameTo.mockImplementation((_time: number, _width: number, _height: number, _options: any, target: HTMLCanvasElement) => {
      const captured = color; target.toBlob = callback => callback(new Blob([captured])); return target;
    });
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.download.mock.calls.length).toBe(1));
      await new Promise(resolve => setTimeout(resolve, 15));
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(1);
      color = 'new'; version++;
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(2);
      expect(await (h.download.mock.calls[1][0] as Blob).text()).toBe('new');
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('does not cache a PNG under an animation version created during that frame capture', async () => {
    const { X, PM, download } = setup();
    let version = 0; PM.animVersion = () => version;
    PM.renderFrameTo = vi.fn((_time: number, _width: number, _height: number, _options: any, target: HTMLCanvasElement) => {
      const paint = String(version++);
      target.toBlob = callback => callback(new Blob([paint]));
      return target;
    });
    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(PM.renderFrameTo.mock.calls.length).toBe(2);
    expect(await (download.mock.calls[0][0] as Blob).text()).toBe('0');
    expect(await (download.mock.calls[1][0] as Blob).text()).toBe('1');
  });

  it('keeps a speculative paint whose own render invalidated already-loaded font readiness', async () => {
    const h = heldPNG();
    const fonts = Object.assign(new EventTarget(), { status: 'loaded', size: 0, ready: Promise.resolve() });
    const actualDocument = document;
    vi.stubGlobal('document', new Proxy(actualDocument, { get(target, key) {
      if (key === 'fonts') return fonts;
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    } }));
    const handlers = new Map<string, Set<() => void>>();
    const on = (event: string, handler: () => void) => {
      let entries = handlers.get(event); if (!entries) { entries = new Set(); handlers.set(event, entries); }
      entries.add(handler); return () => entries!.delete(handler);
    };
    h.PM.bus = { on }; h.PM.Kernel = { events: { on } };
    let paints = 0;
    h.PM.renderFrameTo.mockImplementation((_time: number, _width: number, _height: number, _options: any, target: HTMLCanvasElement) => {
      const paint = String(paints++);
      if (paints === 2) for (const handler of handlers.get('fonts') || []) handler();
      target.toBlob = callback => callback(new Blob([paint]));
      return target;
    });
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      expect(framePreparation.frameFontsReady(h.PM)).toBe(false);
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(2);
      expect(await (h.download.mock.calls[0][0] as Blob).text()).toBe('0');
      expect(await (h.download.mock.calls[1][0] as Blob).text()).toBe('1');
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('drains an active PNG save after external cancellation, drops the next frame and permits retry', async () => {
    const h = heldPNG();
    let settled = false;
    const result = h.X.run(options).finally(() => { settled = true; });
    try {
      await vi.waitFor(() => expect(h.PM.renderFrameTo.mock.calls.length).toBe(2));
      h.X.cancel = true;
      await Promise.resolve(); await Promise.resolve();
      expect(settled).toBe(false);
      expect(h.PM.renderFrameTo.mock.calls[0][4].width).toBeGreaterThan(0);
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: true });
      expect(h.download.mock.calls.length).toBe(1);
      expect(h.PM.renderFrameTo.mock.calls[0][4]).toMatchObject({ width: 0, height: 0 });
      expect(await h.X.run(options)).toEqual({ cancelled: false });
      expect(h.download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png', 'Test_00000.png', 'Test_00001.png']);
    } finally { h.resolve({ ok: true }); await result; }
  });

  it.each(['video', 'precomp', 'project-change', 'large-frame'] as const)('keeps PNG preparation serial with %s safeguards', async type => {
    const h = heldPNG();
    vi.spyOn(framePreparation, 'prepareFrame').mockResolvedValue();
    vi.spyOn(framePreparation, 'frameFontsReady').mockReturnValue(true);
    if (type === 'project-change') h.download.mockReset().mockResolvedValue({ ok: true }).mockImplementationOnce(async () => {
      h.PM.proj = { ...h.PM.proj, layers: [{ type: 'video' }] };
      return await new Promise<any>(resolve => { h.resolve = resolve; });
    });
    else if (type === 'large-frame') { h.PM.proj.w = 7680; h.PM.proj.h = 4320; }
    else h.PM.proj.layers = [{ type }];
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.download.mock.calls.length).toBe(1));
      await new Promise(resolve => setTimeout(resolve, 15));
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(1);
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(h.PM.renderFrameTo.mock.calls.length).toBe(2);
    } finally { h.resolve({ ok: true }); await result; }
  });

  it('drains the prior PNG write before starting a fresh font wait and releases export listeners', async () => {
    const h = heldPNG();
    const fonts = Object.assign(new EventTarget(), { status: 'loaded', size: 0, ready: Promise.resolve() });
    const actualDocument = document;
    vi.stubGlobal('document', new Proxy(actualDocument, { get(target, key) {
      if (key === 'fonts') return fonts;
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    } }));
    const listeners = new Map<string, Set<() => void>>();
    const on = (event: string, handler: () => void) => {
      let entries = listeners.get(event); if (!entries) { entries = new Set(); listeners.set(event, entries); }
      entries.add(handler); return () => entries!.delete(handler);
    };
    h.PM.bus = { on }; h.PM.Kernel = { events: { on } };
    h.download.mockReset().mockResolvedValue({ ok: true }).mockImplementationOnce(() => {
      fonts.status = 'loading'; fonts.dispatchEvent(new Event('loading'));
      return new Promise<any>(resolve => { h.resolve = resolve; });
    });
    const prepare = vi.spyOn(framePreparation, 'prepareFrame');
    const result = h.X.run(options);
    try {
      await vi.waitFor(() => expect(h.download.mock.calls.length).toBe(1));
      await new Promise(resolve => setTimeout(resolve, 15));
      expect(prepare.mock.calls.length).toBe(1);
      fonts.status = 'loaded'; fonts.dispatchEvent(new Event('loadingdone'));
      h.resolve({ ok: true });
      expect(await result).toEqual({ cancelled: false });
      expect(prepare.mock.calls.length).toBe(2);
      // Font readiness belongs to PM for its lifetime; export retention's
      // duplicate listeners and its draw listener must all be released.
      expect(listeners.get('draw')!.size).toBe(0);
      for (const event of ['fonts', 'layers', 'project', 'extensions:changed', 'theme:changed']) expect(listeners.get(event)!.size).toBe(1);
    } finally {
      fonts.status = 'loaded'; fonts.dispatchEvent(new Event('loadingdone'));
      h.resolve({ ok: true }); await result;
    }
  });

  it.each(['preparation', 'encoding'])('stops PNG delivery after Cancel during frame %s and permits retry', async phase => {
    const { X, PM, frame, download } = setup();
    PM.proj.layers = [{ type: 'solid', from: 0, dur: 1, p: { rotation: { expr: 'time' } } }];
    let entered!: () => void, release!: () => void;
    const blocked = new Promise<void>(resolve => { entered = resolve; });
    const pending = new Promise<void>(resolve => { release = resolve; });
    const originalPrepare = framePreparation.prepareFrame;
    let prepared = 0, encoded = 0;
    vi.spyOn(framePreparation, 'prepareFrame').mockImplementation(async (...args) => {
      if (++prepared === 2 && phase === 'preparation') { entered(); await pending; }
      return originalPrepare(...args);
    });
    const encode = frame.toBlob;
    frame.toBlob = (callback: (blob: Blob) => void) => {
      if (++encoded === 2 && phase === 'encoding') {
        entered(); void pending.then(() => encode(callback));
      } else encode(callback);
    };
    let cancel!: () => void;
    const close = vi.fn();
    PM.modal = (options: any) => {
      cancel = options.actions[0].run;
      return { el: document.createElement('div'), close };
    };
    PM.playing = true;
    PM.pause = vi.fn(() => { PM.playing = false; });
    PM.play = vi.fn(() => { PM.playing = true; });
    PM.setTime = vi.fn();
    const result = X.run(options);
    await blocked;
    expect(download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png']);
    cancel();
    release();
    expect(await result).toEqual({ cancelled: true });
    // The completed first frame stays; the interrupted second frame is never delivered.
    expect(download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png']);
    expect(close).toHaveBeenCalledOnce();
    expect(X.busy).toBe(false);
    expect(PM.preparedVideoFrames).toBeNull();
    expect(PM.setTime).toHaveBeenCalledWith(0, { force: true });
    expect(PM.playing).toBe(true);
    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(download.mock.calls.map(([, name]: any[]) => name)).toEqual(['Test_00000.png', 'Test_00000.png', 'Test_00001.png']);
    expect(X.busy).toBe(false);
  });

  it('delivers a PNG sequence one save at a time without a folder picker', async () => {
    let active = 0;
    let peak = 0;
    const download = vi.fn(async (_blob: Blob, _name: string) => {
      peak = Math.max(peak, ++active);
      await new Promise(resolve => setTimeout(resolve, 5));
      active--;
      return { ok: true, path: '/tmp/frame.png' };
    });
    const { X } = setup(download);

    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(download.mock.calls.map(([, name]) => name)).toEqual(['Test_00000.png', 'Test_00001.png']);
    expect(peak).toBe(1);
  });

  it('encodes PNG frames from the captured 2D frame, not the live WebGL canvas', async () => {
    const { X, PM, frame } = setup();
    PM.GL.canvas = { toBlob: () => { throw new Error('Live canvas readback failed'); } };
    PM.renderFrameTo = vi.fn(() => frame);

    expect(await X.run(options)).toEqual({ cancelled: false });
    expect(PM.renderFrameTo).toHaveBeenCalledOnce();
  });

  it('waits for realtime native delivery and propagates its cancellation and errors', async () => {
    const videoTrack = { requestFrame() {}, stop() {} };
    const { X, download, frame } = setup();
    (frame as any).captureStream = () => ({ getVideoTracks: () => [videoTrack] });
    class Recorder {
      static isTypeSupported() { return true; }
      onstop?: () => void;
      ondataavailable?: (event: any) => void;
      start() {}
      stop() { this.ondataavailable?.({ data: new Blob(['video']) }); this.onstop?.(); }
    }
    (window as any).MediaRecorder = Recorder;
    expect(await X.run({ ...options, format: 'rec' })).toEqual({ cancelled: false });
    download.mockResolvedValueOnce({ ok: false, cancelled: true });
    expect(await X.run({ ...options, format: 'rec' })).toEqual({ cancelled: true });
    download.mockRejectedValueOnce(new Error('Disk full'));
    expect(await X.run({ ...options, format: 'rec' })).toEqual({ error: 'Disk full' });
  });

  it('writes caption sidecars beside a video, never beside an image sequence', async () => {
    const videoTrack = { requestFrame() {}, stop() {} };
    const download = vi.fn(async (_blob: Blob, name: string) => ({ ok: true, path: `/tmp/${name}` }));
    const { X, PM, frame } = setup(download);
    PM.active = () => false;
    PM.proj.layers.push({ id: 'cc', type: 'captions', name: 'Captions', from: 0, dur: 1, d: { cues: [{ id: 'a', start: 0, end: 0.5, text: 'Hi' }], style: {} } });
    (frame as any).captureStream = () => ({ getVideoTracks: () => [videoTrack] });
    class Recorder {
      static isTypeSupported() { return true; }
      onstop?: () => void;
      ondataavailable?: (event: any) => void;
      start() {}
      stop() { this.ondataavailable?.({ data: new Blob(['video']) }); this.onstop?.(); }
    }
    (window as any).MediaRecorder = Recorder;
    const exportSidecar = vi.fn(async (_request: any) => ({ ok: true }));
    (window as any).powermove = { exportSidecar };

    expect(await X.run({ ...options, format: 'png', captionsSidecar: 'srt' })).toEqual({ cancelled: false });
    expect(exportSidecar).not.toHaveBeenCalled();
    expect(await X.run({ ...options, format: 'rec', captionsSidecar: 'srt' })).toEqual({ cancelled: false });
    expect(exportSidecar).toHaveBeenCalledOnce();
    expect(exportSidecar.mock.calls[0]![0]).toMatchObject({ extension: 'srt', text: expect.stringContaining('Hi') });
  });

  it('does not claim a still frame was exported when native save is cancelled', async () => {
    const { X, toast, download } = setup();
    download.mockResolvedValueOnce({ ok: false, cancelled: true });
    expect(await X.run({ ...options, format: 'still' })).toEqual({ cancelled: true });
    expect(toast).not.toHaveBeenCalledWith('Frame exported');
  });

  it('propagates native delivery cancellation for a web animation', async () => {
    const { X, toast, download } = setup();
    download.mockResolvedValueOnce({ ok: false, cancelled: true });
    expect(await X.run({ format: 'web' })).toEqual({ cancelled: true });
    expect(toast).not.toHaveBeenCalledWith('Web animation exported');
  });
});
