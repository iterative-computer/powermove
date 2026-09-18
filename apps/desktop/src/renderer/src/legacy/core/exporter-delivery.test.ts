// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { install } from './exporter';

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
    round: (number: number) => number, tc: () => '0:00',
    Audio: { hasAudibleLayers: () => false },
    GL: { canvas: frame, resize() {}, render() {} },
    renderFrameTo: () => frame,
  };
  install(PM);
  return { X: PM.Export, PM, toast, download, frame };
}

const options = { format: 'png', fps: 2, scale: 1, audio: false, mblur: false };
let originalMediaRecorder: any;

beforeEach(() => {
  originalMediaRecorder = (window as any).MediaRecorder;
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({ drawImage() {} }) as any);
  delete (window as any).showDirectoryPicker;
  delete (window as any).powermove;
});
afterEach(() => {
  vi.restoreAllMocks();
  delete (window as any).showDirectoryPicker;
  delete (window as any).powermove;
  if (originalMediaRecorder === undefined) delete (window as any).MediaRecorder;
  else (window as any).MediaRecorder = originalMediaRecorder;
});

describe('export delivery', () => {
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
    expect(PM.renderFrameTo).toHaveBeenCalledTimes(2);
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
