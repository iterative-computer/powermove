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
    expect(write).not.toHaveBeenCalled();
    expect(X.busy).toBe(false);
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

  it('streams the owned pixel buffer without copying a second complete frame', async () => {
    const { X, frame } = setup();
    const pixels = new Uint8ClampedArray(new ArrayBuffer(24), 4, 16);
    pixels.set(Array.from({ length: 16 }, (_, i) => i * 15));
    frame.getContext = () => ({ getImageData: () => ({ data: pixels }) });
    const slice = vi.spyOn(Uint8Array.prototype, 'slice');
    const write = vi.fn(async () => {});
    (window as any).powermove = { render: {
      start: async () => 'render', write, finish: async () => ({ path: '/tmp/render.mp4' }), cancel: vi.fn(),
    } };
    expect(await X.run({ ...options, format: 'mp4' })).toEqual({ cancelled: false });
    expect(write).toHaveBeenCalledTimes(2);
    expect(Array.from((write.mock.calls[0] as any)[1])).toEqual(Array.from(pixels));
    expect(slice.mock.contexts.some(view => view instanceof Uint8Array && view.buffer === pixels.buffer && view.byteOffset === pixels.byteOffset)).toBe(true);
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
